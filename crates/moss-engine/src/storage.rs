//! What Moss keeps on this device, for Settings → Storage: the space each kind of data
//! takes, the offline setting, the cache and its limit, rebuilding the index, and loading
//! the bodies of mail someone is about to open.

use std::path::Path;

use serde::Serialize;

use moss_store::{Offline, Usage};

use crate::{Engine, Result, Shared};

/// Opened attachments saved as files for other apps to open. Their bytes are in the cache
/// too, so only the newest few stay as files.
pub(crate) const FILES_KEPT: usize = 20;

/// Settings → Storage, apart from the AI models (the app adds those).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub usage: Usage,
    pub offline: Offline,
    pub cache_limit: u64,
    /// The index is being rebuilt, so search finds less for now.
    pub rebuilding: bool,
    /// Moss reads the text inside attachments, so search and Ask Moss can use it.
    pub attachment_text: bool,
}

/// The size of the files under `dir`, in bytes. A folder that isn't there is empty.
pub(crate) fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            _ => e.metadata().map_or(0, |m| m.len()),
        })
        .sum()
}

/// Every file under `dir`, with when it was last changed. Folders starting with a dot,
/// like the thumbnails', keep their own count.
fn files_in(dir: &Path, out: &mut Vec<(std::time::SystemTime, std::path::PathBuf)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = e.path();
        if path.is_dir() {
            if !e.file_name().to_string_lossy().starts_with('.') {
                files_in(&path, out);
            }
        } else if let Ok(changed) = e.metadata().and_then(|m| m.modified()) {
            out.push((changed, path));
        }
    }
}

/// Deletes all but the `keep` newest files under `dir`.
pub(crate) fn keep_newest_files(dir: &Path, keep: usize) {
    let mut files = Vec::new();
    files_in(dir, &mut files);
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(&path);
    }
}

impl Shared {
    /// Trims the cache to its limit, oldest first.
    pub(crate) fn trim_cache(&self) -> Result<()> {
        let mut store = self.store();
        let limit = store.cache_limit()?;
        store.trim_cache(limit)?;
        Ok(())
    }
}

impl Engine {
    /// Space used and what's kept. `files` is where opened attachments are saved as files;
    /// they count as cache.
    pub fn storage(&self, files: &Path) -> Result<StorageInfo> {
        let store = self.shared.store();
        let mut usage = store.usage()?;
        usage.cache += folder_size(files);
        Ok(StorageInfo {
            usage,
            offline: store.offline()?,
            cache_limit: store.cache_limit()?,
            rebuilding: store.rebuilding()?,
            attachment_text: store.reading_attachments()?,
        })
    }

    pub fn set_offline(&self, offline: Offline) -> Result<()> {
        self.shared.store().set_offline(offline, self.shared.now())?;
        self.shared.trim_cache()
    }

    pub fn set_cache_limit(&self, bytes: u64) -> Result<()> {
        self.shared.store().set_cache_limit(bytes)?;
        self.shared.trim_cache()
    }

    /// Removes all cache, and the attachment files saved in `files`. The index, offline
    /// mail and settings stay. Returns the bytes freed.
    pub fn clear_cache(&self, files: &Path) -> Result<u64> {
        let on_disk = folder_size(files);
        let _ = std::fs::remove_dir_all(files);
        let freed = self.shared.store().clear_cache()? + on_disk - folder_size(files);
        Ok(freed)
    }

    /// Rebuilds the keyword index, details and people from the mail here, in the
    /// background, then tidies up.
    pub fn rebuild_index(&self) -> Result<()> {
        self.shared.store().start_rebuild()?;
        let engine = self.clone();
        tokio::spawn(async move { engine.tidy().await });
        Ok(())
    }

    /// Fetches the bodies that aren't here of the messages in these threads, so they open
    /// at once: the ones around the selection in the list, or the top search results.
    /// Mail that can't be fetched just opens as it would have.
    pub async fn preload(&self, thread_ids: &[String]) -> Result<()> {
        let now = self.shared.now();
        let mut missing = Vec::new();
        // One thread at a time, so opening one isn't kept waiting behind the rest.
        for id in thread_ids {
            let thread = self.shared.store().thread(id, now)?;
            if let Some(thread) = thread {
                missing.extend(thread.messages.into_iter().filter(|m| m.headers_only || m.images_missing).map(|m| m.id));
            }
        }
        for id in missing {
            let _ = self.load_message(&id).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_count_every_file_inside_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(folder_size(&dir.path().join("none")), 0);
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/one"), [0; 10]).unwrap();
        std::fs::write(dir.path().join("a/b/two"), [0; 5]).unwrap();
        assert_eq!(folder_size(dir.path()), 15);
    }

    #[test]
    fn only_the_newest_files_stay() {
        let dir = tempfile::tempdir().unwrap();
        for (n, name) in ["old", "mid", "new"].iter().enumerate() {
            let folder = dir.path().join(n.to_string());
            std::fs::create_dir_all(&folder).unwrap();
            let path = folder.join(name);
            std::fs::write(&path, [0; 3]).unwrap();
            let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 * n as u64 + 1);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
        }
        keep_newest_files(dir.path(), 2);
        assert!(!dir.path().join("0/old").exists());
        assert!(dir.path().join("1/mid").exists() && dir.path().join("2/new").exists());
        keep_newest_files(&dir.path().join("none"), 2);
    }
}
