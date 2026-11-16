//! Apple Intelligence: Apple's own on-device model, built into macOS 27, as a writing model.
//!
//! Moss reaches it through `moss-apple`, a small Swift helper that ships beside Moss (its code is
//! in `apple/` at the top of the repo). The helper serves Apple's model through the same
//! OpenAI-style chat endpoint as llama-server, so Moss starts it and talks to it the same way;
//! only the model and its smaller context differ. Nothing is downloaded.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::catalog::{ModelDownload, WriterInfo};

/// The writing model's id, in Settings and `settings.json`.
pub const ID: &str = "apple";

/// Apple Intelligence on this Mac, through the helper at `binary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apple {
    pub binary: PathBuf,
    /// Tokens Apple's model reads and writes at once: 8,192 in macOS 27.
    pub context: usize,
}

/// Apple Intelligence as Settings lists it: built in, so always ready.
pub fn info(recommended: bool) -> WriterInfo {
    WriterInfo {
        id: ID.into(),
        name: "Apple Intelligence".into(),
        sub: "Built into macOS: quick, with nothing to download.".into(),
        size: "Built in".into(),
        bytes: 0,
        recommended,
        too_big: false,
        download: ModelDownload::Ready,
    }
}

/// How long the helper may take to say whether Apple's model can run.
const CHECK: Duration = Duration::from_secs(5);

/// Asks the helper whether Apple's model can run on this Mac. `None` when it can't: an older
/// macOS or Mac, Apple Intelligence turned off, or no helper.
pub fn detect(binary: &Path) -> Option<Apple> {
    detect_within(binary, CHECK)
}

fn detect_within(binary: &Path, wait: Duration) -> Option<Apple> {
    let mut child = std::process::Command::new(binary)
        .arg("--available")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + wait;
    while child.try_wait().ok()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    let said: serde_json::Value = serde_json::from_str(out.trim()).ok()?;
    let context = said["context"].as_u64().filter(|_| said["available"] == true)?;
    Some(Apple { binary: binary.to_path_buf(), context: context as usize }).filter(|a| a.context > 0)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in helper: a script that says `said`, after `pause` seconds.
    fn helper(dir: &Path, said: &str, pause: u32) -> PathBuf {
        let path = dir.join("moss-apple");
        std::fs::write(&path, format!("#!/bin/sh\n[ \"$1\" = --available ] || exit 3\nsleep {pause}\necho '{said}'\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn settings_lists_it_as_built_in_and_ready() {
        let info = info(true);
        assert_eq!((info.id.as_str(), info.name.as_str(), info.size.as_str()), (ID, "Apple Intelligence", "Built in"));
        assert_eq!(info.download, ModelDownload::Ready);
        assert!(info.recommended && !info.too_big && info.bytes == 0);
        assert!(!info.sub.contains('—'));
        assert!(!super::info(false).recommended);
    }

    #[test]
    fn a_mac_with_apple_intelligence_on_has_it() {
        let dir = tempfile::tempdir().unwrap();
        let binary = helper(dir.path(), r#"{"available":true,"context":8192}"#, 0);
        assert_eq!(detect(&binary), Some(Apple { binary, context: 8192 }));
    }

    #[test]
    fn a_mac_without_it_or_a_helper_that_says_nothing_useful_has_none() {
        let dir = tempfile::tempdir().unwrap();
        for said in [r#"{"available":false,"reason":"notEnabled"}"#, r#"{"available":true}"#, r#"{"available":true,"context":0}"#, "nonsense"] {
            assert_eq!(detect(&helper(dir.path(), said, 0)), None, "{said}");
        }
        assert_eq!(detect(&dir.path().join("missing")), None);
    }

    #[test]
    fn a_helper_that_hangs_is_given_up_on() {
        let dir = tempfile::tempdir().unwrap();
        let binary = helper(dir.path(), r#"{"available":true,"context":8192}"#, 5);
        let started = Instant::now();
        assert_eq!(detect_within(&binary, Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
