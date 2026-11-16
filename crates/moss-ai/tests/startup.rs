//! Startup timing checks, isolated from the many starts in `server.rs`.
use std::path::{Path, PathBuf};
use std::time::Duration;
use moss_ai::server::{Server, ServerError};
const FAKE: &str = env!("CARGO_BIN_EXE_fake-llama-server");
fn key_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("decider.key");
    (dir, key)
}
async fn health(url: &str) -> Option<u16> {
    // Like Moss, never through a proxy: the first lookup of the system's can take seconds.
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    client.get(format!("{url}/health")).send().await.ok().map(|r| r.status().as_u16())
}
async fn gone(url: &str) -> bool {
    for _ in 0..50 {
        if health(url).await.is_none() { return true; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn a_start_never_waits_for_another_runtime_to_bind() {
    let (dir, held_key) = key_file();
    let log = dir.path().join("startup.log");
    // This one takes 1.5 s to start listening, longer than the others may take in all.
    let slow = ["--test-startup-log", log.to_str().unwrap(), "--test-bind-delay-ms", "1500"];
    let holder = Server::start(Path::new(FAKE), Path::new("m.gguf"), &slow, &held_key, Duration::from_secs(10));
    let others = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let began = tokio::time::Instant::now();
        // A missing runtime is reported as one at once, not as a model that took too long.
        let missing = Server::start(Path::new("/nope/llama-server"), Path::new("m.gguf"), &[], &dir.path().join("missing.key"), Duration::from_secs(1)).await;
        assert!(matches!(missing, Err(ServerError::Spawn(_))), "{:?}", missing.err());
        assert!(began.elapsed() < Duration::from_millis(500));
        // A runtime that starts quickly is ready within its own wait.
        let quick = Server::start(Path::new(FAKE), Path::new("m.gguf"), &[], &dir.path().join("quick.key"), Duration::from_secs(1)).await.unwrap();
        assert_eq!(health(&quick.endpoint().url).await, Some(200));
        assert!(!std::fs::read_to_string(&log).unwrap().contains("bound"), "ready before the slow one listened");
        quick
    };
    let (holder, quick) = tokio::join!(holder, others);
    let holder = holder.unwrap();
    assert_ne!(holder.endpoint().url, quick.endpoint().url);
    let urls = [holder.endpoint().url.clone(), quick.endpoint().url.clone()];
    drop((holder, quick));
    for url in urls { assert!(gone(&url).await); }
}

#[tokio::test]
async fn an_unanswered_health_check_still_ends_at_the_deadline() {
    let (_dir, key) = key_file();
    let started = tokio::time::Instant::now();
    let result = Server::start(Path::new(FAKE), Path::new("slow-health.gguf"), &[], &key, Duration::from_millis(300)).await;
    assert_eq!(result.err(), Some(ServerError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(1), "an unanswered health request must not outlast the wait");
    assert!(!key.with_extension("pid").exists());
}
