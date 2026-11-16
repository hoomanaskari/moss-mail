//! Downloads one file with progress, resumes a partial one, and checks its SHA-256.
//!
//! While downloading, the file is `<name>.part`. Once its hash matches, it's renamed to
//! `<name>` and `<name>.ok` records the hash, so Moss knows it's whole without reading it again.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DownloadError {
    #[error("can't reach the download server: {0}")]
    Network(String),
    #[error("the download server answered {0}")]
    Http(u16),
    #[error("the download stopped early")]
    Incomplete,
    #[error("the downloaded file was damaged, so Moss deleted it")]
    Damaged,
    #[error("stopped")]
    Cancelled,
    #[error("can't save the file: {0}")]
    Disk(String),
}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        DownloadError::Disk(e.to_string())
    }
}

/// What to fetch and where to put it.
pub struct Request<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub bytes: u64,
    pub sha256: &'a str,
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The partial file while it downloads.
pub fn part_path(dest: &Path) -> PathBuf {
    with_suffix(dest, ".part")
}

/// Records that the file is whole and checked.
pub fn marker_path(dest: &Path) -> PathBuf {
    with_suffix(dest, ".ok")
}

/// Whether the file is downloaded and was checked.
pub fn is_ready(dest: &Path, sha256: &str) -> bool {
    dest.is_file() && std::fs::read_to_string(marker_path(dest)).is_ok_and(|m| m.trim() == sha256)
}

/// How many bytes of it are on disk so far: all of it once ready.
pub fn on_disk(dest: &Path, sha256: &str, bytes: u64) -> u64 {
    if is_ready(dest, sha256) {
        return bytes;
    }
    std::fs::metadata(part_path(dest))
        .map(|m| m.len())
        .ok()
        .filter(|&n| n <= bytes)
        .unwrap_or(0)
}

/// Records that `dest` is whole and matches `sha256`.
pub fn mark_ready(dest: &Path, sha256: &str) -> std::io::Result<()> {
    std::fs::write(marker_path(dest), sha256)
}

/// Downloads `request.url` to `request.dest`, picking up a partial file where it stopped.
/// `progress` gets the bytes on disk so far. Setting `cancel` stops it and keeps the part.
pub async fn fetch(
    client: &reqwest::Client,
    request: Request<'_>,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<(), DownloadError> {
    let Request {
        url,
        dest,
        bytes,
        sha256,
    } = request;
    let part = part_path(dest);
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > bytes {
        std::fs::remove_file(&part)?;
        have = 0;
    }
    let mut hasher = hash_start(&part, have).await?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part)?;
    if have < bytes {
        let network = |e: reqwest::Error| DownloadError::Network(e.to_string());
        let mut get = client.get(url);
        if have > 0 {
            get = get.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut response = get.send().await.map_err(network)?;
        match response.status().as_u16() {
            206 => {}
            // The whole file, from the start: drop what was there.
            200 => {
                file.set_len(0)?;
                have = 0;
                hasher = Sha256::new();
            }
            status => return Err(DownloadError::Http(status)),
        }
        progress(have);
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            file.write_all(&chunk)?;
            hasher.update(&chunk);
            have += chunk.len() as u64;
            progress(have);
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
        }
        file.sync_all()?;
        if have < bytes {
            return Err(DownloadError::Incomplete);
        }
    }
    drop(file);
    if hex(&hasher.finalize()) != sha256 {
        std::fs::remove_file(&part)?;
        return Err(DownloadError::Damaged);
    }
    std::fs::rename(&part, dest)?;
    mark_ready(dest, sha256)?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash of the first `len` bytes already on disk, read off the async threads: a part
/// can be many gigabytes.
async fn hash_start(part: &Path, len: u64) -> Result<Sha256, DownloadError> {
    let part = part.to_path_buf();
    let read = move || -> std::io::Result<Sha256> {
        let mut hasher = Sha256::new();
        if len > 0 {
            let mut file = std::fs::File::open(&part)?.take(len);
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }
        Ok(hasher)
    };
    Ok(tokio::task::spawn_blocking(read)
        .await
        .expect("hashing never panics")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> Vec<u8> {
        (0..100_000u32).map(|i| (i % 251) as u8).collect()
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    struct Setup {
        server: MockServer,
        dir: tempfile::TempDir,
        dest: PathBuf,
        url: String,
        sha: String,
    }

    async fn setup() -> Setup {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let url = format!("{}/model.gguf", server.uri());
        Setup {
            server,
            dir,
            dest,
            url,
            sha: sha(&body()),
        }
    }

    impl Setup {
        async fn run(&self, cancel: bool) -> (Result<(), DownloadError>, Vec<u64>) {
            let mut seen = vec![];
            let request = Request {
                url: &self.url,
                dest: &self.dest,
                bytes: body().len() as u64,
                sha256: &self.sha,
            };
            let result = fetch(
                &crate::http::local(),
                request,
                &AtomicBool::new(cancel),
                &mut |n| seen.push(n),
            )
            .await;
            (result, seen)
        }
    }

    #[tokio::test]
    async fn downloads_checks_and_marks_the_file() {
        let s = setup().await;
        Mock::given(method("GET"))
            .and(path("/model.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body()))
            .expect(1)
            .mount(&s.server)
            .await;
        let (result, seen) = s.run(false).await;
        assert_eq!(result, Ok(()));
        assert_eq!(std::fs::read(&s.dest).unwrap(), body());
        assert!(!part_path(&s.dest).exists());
        assert!(is_ready(&s.dest, &s.sha));
        assert!(!is_ready(&s.dest, &sha(b"other")));
        assert_eq!(seen.last(), Some(&(body().len() as u64)));
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(on_disk(&s.dest, &s.sha, 100_000), 100_000);
    }

    #[tokio::test]
    async fn picks_up_where_a_partial_download_stopped() {
        let s = setup().await;
        std::fs::write(part_path(&s.dest), &body()[..40_000]).unwrap();
        assert_eq!(on_disk(&s.dest, &s.sha, 100_000), 40_000);
        Mock::given(method("GET"))
            .and(header("range", "bytes=40000-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(&body()[40_000..]))
            .expect(1)
            .mount(&s.server)
            .await;
        let (result, seen) = s.run(false).await;
        assert_eq!(result, Ok(()));
        assert_eq!(std::fs::read(&s.dest).unwrap(), body());
        assert!(
            seen.first().copied().unwrap_or_default() >= 40_000,
            "{seen:?}"
        );
    }

    #[tokio::test]
    async fn starts_over_when_the_server_ignores_the_range() {
        let s = setup().await;
        std::fs::write(part_path(&s.dest), b"stale bytes").unwrap();
        Mock::given(method("GET"))
            .and(header_exists("range"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body()))
            .expect(1)
            .mount(&s.server)
            .await;
        assert_eq!(s.run(false).await.0, Ok(()));
        assert_eq!(std::fs::read(&s.dest).unwrap(), body());
    }

    #[tokio::test]
    async fn a_part_bigger_than_the_file_is_thrown_away() {
        let s = setup().await;
        std::fs::write(part_path(&s.dest), vec![0u8; 100_001]).unwrap();
        assert_eq!(on_disk(&s.dest, &s.sha, 100_000), 0);
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body()))
            .mount(&s.server)
            .await;
        assert_eq!(s.run(false).await.0, Ok(()));
        assert!(is_ready(&s.dest, &s.sha));
        let asked = s.server.received_requests().await.unwrap();
        assert!(
            asked.iter().all(|r| !r.headers.contains_key("range")),
            "asked for the whole file"
        );
    }

    #[tokio::test]
    async fn a_complete_part_is_checked_without_downloading_again() {
        let s = setup().await;
        std::fs::write(part_path(&s.dest), body()).unwrap();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&s.server)
            .await;
        assert_eq!(s.run(false).await.0, Ok(()));
        assert!(is_ready(&s.dest, &s.sha));
    }

    #[tokio::test]
    async fn a_damaged_file_is_deleted() {
        let s = setup().await;
        let mut wrong = body();
        wrong[5] ^= 1;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(wrong))
            .mount(&s.server)
            .await;
        assert_eq!(s.run(false).await.0, Err(DownloadError::Damaged));
        assert!(!s.dest.exists());
        assert!(!part_path(&s.dest).exists());
        assert!(!is_ready(&s.dest, &s.sha));
    }

    #[tokio::test]
    async fn a_short_download_keeps_the_part_to_resume() {
        let s = setup().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(&body()[..10_000]))
            .mount(&s.server)
            .await;
        assert_eq!(s.run(false).await.0, Err(DownloadError::Incomplete));
        assert_eq!(std::fs::metadata(part_path(&s.dest)).unwrap().len(), 10_000);
        assert!(!s.dest.exists());
    }

    #[tokio::test]
    async fn stopping_keeps_what_has_downloaded() {
        let s = setup().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body()))
            .mount(&s.server)
            .await;
        assert_eq!(s.run(true).await.0, Err(DownloadError::Cancelled));
        assert!(part_path(&s.dest).exists());
        assert!(!s.dest.exists());
    }

    #[tokio::test]
    async fn server_and_network_errors_say_what_happened() {
        let s = setup().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s.server)
            .await;
        assert_eq!(s.run(false).await.0, Err(DownloadError::Http(404)));

        let gone = Setup {
            url: "http://127.0.0.1:9/model.gguf".into(),
            ..setup().await
        };
        assert!(matches!(
            gone.run(false).await.0,
            Err(DownloadError::Network(_))
        ));
    }

    #[tokio::test]
    async fn a_folder_it_cannot_write_to_is_a_disk_error() {
        let s = setup().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body()))
            .mount(&s.server)
            .await;
        let blocked = Setup {
            dest: s.dir.path().join("missing").join("model.gguf"),
            ..setup().await
        };
        let blocked = Setup {
            server: s.server,
            url: s.url.clone(),
            ..blocked
        };
        assert!(matches!(
            blocked.run(false).await.0,
            Err(DownloadError::Disk(_))
        ));
        assert_eq!(
            DownloadError::from(std::io::Error::other("full")),
            DownloadError::Disk("full".into())
        );
    }
}
