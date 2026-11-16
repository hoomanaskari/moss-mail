//! Starting and stopping `llama-server`, against the fake one in `tests/fake/`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use moss_ai::server::{free_port, new_key, stop_leftovers, Server, ServerError};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-llama-server");

fn key_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("decider.key");
    (dir, file)
}

async fn health(url: &str) -> Option<u16> {
    // Like Moss, never through a proxy: the first lookup of the system's can take seconds.
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    client.get(format!("{url}/health")).send()
        .await
        .ok()
        .map(|r| r.status().as_u16())
}

#[tokio::test]
async fn waits_for_the_model_to_load_then_answers_on_localhost() {
    let (_dir, key) = key_file();
    let server = Server::start(
        Path::new(FAKE),
        Path::new("model.gguf"),
        &["-c", "8192"],
        &key,
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    let endpoint = server.endpoint().clone();
    assert!(
        endpoint.url.starts_with("http://127.0.0.1:"),
        "{endpoint:?}"
    );
    assert_eq!(endpoint.key.len(), 64);
    // The key is in a file only this user can read, never in the process list.
    assert_eq!(std::fs::read_to_string(&key).unwrap(), endpoint.key);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert_eq!(health(&endpoint.url).await, Some(200));

    // Its process id sits by the key, so a later run can find it if this one ends abruptly.
    let pid = key.with_extension("pid");
    assert!(std::fs::read_to_string(&pid).unwrap().trim().parse::<u32>().is_ok());

    drop(server);
    assert!(!pid.exists(), "a server that stopped leaves no process id behind");
    // Stopped with the handle.
    let mut gone = false;
    for _ in 0..50 {
        if health(&endpoint.url).await.is_none() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "the server should stop when dropped");
}

#[tokio::test]
async fn a_runtime_that_quits_while_loading_is_an_error() {
    let (_dir, key) = key_file();
    let result = Server::start(
        Path::new(FAKE),
        Path::new("crash.gguf"),
        &[],
        &key,
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(result.err(), Some(ServerError::Exited));
    assert!(!key.with_extension("pid").exists());
}

#[tokio::test]
async fn a_model_that_never_loads_times_out() {
    let (_dir, key) = key_file();
    let result = Server::start(
        Path::new(FAKE),
        Path::new("stuck.gguf"),
        &[],
        &key,
        Duration::from_millis(600),
    )
    .await;
    assert_eq!(result.err(), Some(ServerError::Timeout));
    assert!(!key.with_extension("pid").exists());
}

#[tokio::test]
async fn a_missing_runtime_cannot_start() {
    let (_dir, key) = key_file();
    let result = Server::start(
        Path::new("/nope/llama-server"),
        Path::new("m.gguf"),
        &[],
        &key,
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(result, Err(ServerError::Spawn(_))));
}

#[tokio::test]
async fn a_key_file_it_cannot_write_stops_the_start() {
    let (dir, _) = key_file();
    let result = Server::start(
        Path::new(FAKE),
        Path::new("m.gguf"),
        &[],
        &dir.path().join("no/such/dir/k"),
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(result, Err(ServerError::Spawn(_))));
}

#[test]
fn keys_and_ports_are_fresh_each_time() {
    assert_ne!(new_key(), new_key());
    assert!(new_key().chars().all(|c| c.is_ascii_hexdigit()));
    assert!(free_port().unwrap() > 0);
}

async fn gone(url: &str) -> bool {
    for _ in 0..50 {
        if health(url).await.is_none() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn a_server_a_previous_run_left_running_is_stopped_next_time() {
    let (dir, key) = key_file();
    let server = Server::start(Path::new(FAKE), Path::new("m.gguf"), &[], &key, Duration::from_secs(10))
        .await
        .unwrap();
    let url = server.endpoint().url.clone();
    // The app quit or crashed without stopping it.
    std::mem::forget(server);
    assert_eq!(health(&url).await, Some(200));

    // Moss runs llama-server and Apple's helper: a leftover of either one is stopped.
    assert_eq!(stop_leftovers(dir.path(), &[Path::new("/nope/moss-apple"), Path::new(FAKE)]), 1);
    assert!(gone(&url).await, "the leftover server should stop");
    assert!(!key.with_extension("pid").exists());
}

#[tokio::test]
async fn leftovers_match_a_runtime_in_an_aliased_temporary_directory() {
    let (dir, key) = key_file();
    let binary = dir.path().join("fake-llama-server");
    std::fs::copy(FAKE, &binary).unwrap();
    let server = Server::start(&binary, Path::new("m.gguf"), &[], &key, Duration::from_secs(10))
        .await.unwrap();
    let url = server.endpoint().url.clone();
    // Retain the handle so this test cleans up its process even if the assertion fails.
    assert_eq!(stop_leftovers(dir.path(), &[&binary]), 1);
    assert!(gone(&url).await);
    drop(server);
}

#[cfg(unix)]
#[tokio::test]
async fn a_deleted_runtime_cannot_authorize_stopping_a_process() {
    let (dir, key) = key_file();
    let binary = dir.path().join("fake-llama-server");
    std::fs::copy(FAKE, &binary).unwrap();
    let server = Server::start(&binary, Path::new("m.gguf"), &[], &key, Duration::from_secs(10))
        .await.unwrap();
    let url = server.endpoint().url.clone();
    std::fs::remove_file(&binary).unwrap();
    assert_eq!(stop_leftovers(dir.path(), &[&binary]), 0);
    assert_eq!(health(&url).await, Some(200));
    drop(server);
    assert!(gone(&url).await);
}

#[test]
fn only_its_own_runtime_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    // Since reused by another program: this test itself.
    std::fs::write(dir.path().join("writer.pid"), std::process::id().to_string()).unwrap();
    std::fs::write(dir.path().join("decider.pid"), "not a number").unwrap();
    std::fs::write(dir.path().join("embedder.key"), "secret").unwrap();
    assert_eq!(stop_leftovers(dir.path(), &[Path::new(FAKE)]), 0);
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, ["embedder.key"], "old process ids are cleared either way");
    assert_eq!(stop_leftovers(&dir.path().join("missing"), &[Path::new(FAKE)]), 0);
    // A runtime that has since been deleted matches nothing.
    std::fs::write(dir.path().join("writer.pid"), std::process::id().to_string()).unwrap();
    assert_eq!(stop_leftovers(dir.path(), &[Path::new("/nope/llama-server")]), 0);
}

#[tokio::test]
async fn simultaneous_runtime_starts_keep_distinct_live_endpoints() {
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        tasks.spawn(async {
            let (dir, key) = key_file();
            let binary = dir.path().join("fake-llama-server");
            std::fs::copy(FAKE, &binary).unwrap();
            let server = Server::start(&binary, Path::new("m.gguf"), &[], &key, Duration::from_secs(10)).await.unwrap();
            (dir, server)
        });
    }
    let mut servers = Vec::new();
    while let Some(result) = tasks.join_next().await { servers.push(result.unwrap()); }
    let endpoints: std::collections::HashSet<_> = servers.iter().map(|(_, server)| server.endpoint().url.clone()).collect();
    assert_eq!(endpoints.len(), 12);
    for url in &endpoints { assert_eq!(health(url).await, Some(200)); }
    drop(servers);
    for url in &endpoints { assert!(gone(url).await); }
}
