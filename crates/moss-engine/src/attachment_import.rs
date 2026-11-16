//! Bounded imports of user-selected files or bytes already supplied by an external agent.
use std::io::Read;
use std::path::PathBuf;
use crate::{attach, Engine, Error, Result};

/// The largest provider allowance. Smaller provider limits are checked before sending.
const MAX_BYTES: u64 = 50 * 1024 * 1024;
const MAX_FILES: usize = 100;

fn oversized() -> Error {
    Error::Invalid("A draft can hold up to 50 MB in at most 100 attachments.".into())
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 255 && name != "." && name != ".."
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}

fn read_files(paths: &[PathBuf], budget: u64) -> Result<Vec<(String, Vec<u8>)>> {
    if paths.len() > MAX_FILES { return Err(oversized()); }
    let mut total = 0u64;
    // Validate the whole batch before allocating any file contents.
    for path in paths {
        let metadata = std::fs::metadata(path).map_err(|e| Error::Unavailable(format!("can't read {}: {e}", path.file_name().unwrap_or_default().to_string_lossy())))?;
        if !metadata.is_file() { return Err(Error::Invalid("Only regular files can be attached.".into())); }
        total = total.checked_add(metadata.len()).ok_or_else(oversized)?;
        if total > budget { return Err(oversized()); }
    }
    let mut left = budget;
    let mut read = Vec::with_capacity(paths.len());
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !valid_name(&name) { return Err(Error::Invalid("The attachment needs a plain filename.".into())); }
        let unavailable = |e| Error::Unavailable(format!("can't read {name}: {e}"));
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        // A file replaced by a FIFO between metadata and open must not block this worker.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = options.open(path).map_err(unavailable)?;
        let bytes = read_regular(file, left, &name)?;
        left -= bytes.len() as u64;
        read.push((name, bytes));
    }
    Ok(read)
}

fn read_regular(file: std::fs::File, budget: u64, name: &str) -> Result<Vec<u8>> {
    let unavailable = |e| Error::Unavailable(format!("can't read {name}: {e}"));
    if !file.metadata().map_err(unavailable)?.is_file() {
        return Err(Error::Invalid("Only regular files can be attached.".into()));
    }
    let mut bytes = Vec::new();
    file.take(budget + 1).read_to_end(&mut bytes).map_err(unavailable)?;
    if bytes.len() as u64 > budget { return Err(oversized()); }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_open_handle_and_actual_bytes_are_checked_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.txt");
        std::fs::write(&path, b"12345678").unwrap();
        assert!(read_regular(std::fs::File::open(&path).unwrap(), 4, "growing.txt").is_err());
        assert_eq!(read_regular(std::fs::File::open(&path).unwrap(), 8, "growing.txt").unwrap(), b"12345678");
        #[cfg(unix)]
        assert!(read_regular(std::fs::File::open("/dev/null").unwrap(), 8, "fixture").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_opened_is_unavailable_and_named() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.txt");
        std::fs::write(&path, b"fixture").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = read_files(&[path], 1024);
        assert!(matches!(&result, Err(Error::Unavailable(m)) if m.contains("locked.txt")), "{result:?}");
    }
}

impl Engine {
    /// Imports only user-selected paths. AI tools must use `attach_contents` instead.
    pub fn attach_files(&self, draft_id: &str, paths: &[PathBuf]) -> Result<Vec<moss_core::DraftFile>> {
        let (count, used) = self.shared.store().draft_file_usage(draft_id)?;
        if count.saturating_add(paths.len() as u64) > MAX_FILES as u64 { return Err(oversized()); }
        let read = read_files(paths, MAX_BYTES.saturating_sub(used))?;
        self.attach_contents(draft_id, &read)
    }

    /// Stores bytes the caller already has, without granting it filesystem access.
    /// Batch insertion is atomic and existing draft files count against the limits.
    pub fn attach_contents(&self, draft_id: &str, files: &[(String, Vec<u8>)]) -> Result<Vec<moss_core::DraftFile>> {
        let mut store = self.shared.store();
        validate_contents(&store, draft_id, files)?;
        let files: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), attach::content_type(name), bytes.as_slice())).collect();
        Ok(store.add_draft_files(draft_id, &files, self.shared.now())?)
    }

    /// A removed staged file is released only when no saved mail or pending send owns it.
    pub fn discard_draft_file(&self, draft_id: &str, id: &str) -> Result<bool> {
        Ok(self.shared.store().discard_staged_draft_file(draft_id, id)?)
    }

    /// An external agent's imported bytes always have a recoverable owning draft.
    pub fn attach_draft_contents(&self, draft: &moss_core::Draft, files: &[(String, Vec<u8>)]) -> Result<moss_core::Draft> {
        self.save_draft_with(draft, |store| {
            validate_contents(store, &draft.id, files)?;
            let files: Vec<_> = files.iter().map(|(name, bytes)| (name.as_str(), attach::content_type(name), bytes.as_slice())).collect();
            Ok(store.edit_draft(draft, &files, self.shared.now(), crate::drafts::cleanup_changes)?)
        })
    }

}

fn validate_contents(store: &moss_store::Store, draft_id: &str, files: &[(String, Vec<u8>)]) -> Result<()> {
        let (count, used) = store.draft_file_usage(draft_id)?;
        if count.saturating_add(files.len() as u64) > MAX_FILES as u64 { return Err(oversized()); }
        let mut total = used;
        for (name, bytes) in files {
            if !valid_name(name) { return Err(Error::Invalid("The attachment needs a plain filename.".into())); }
            total = total.checked_add(bytes.len() as u64).ok_or_else(oversized)?;
            if total > MAX_BYTES { return Err(oversized()); }
        }
    Ok(())
}
