//! The runtime is on this Mac: starting it never asks a proxy. In its own test process, since it
//! sets the proxy for the whole process.
use std::path::Path;
use std::time::Duration;

use moss_ai::server::Server;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-llama-server");

#[tokio::test]
async fn a_proxy_never_stands_between_moss_and_its_runtime() {
    // A proxy that isn't there. Looking up the system's proxy can also take seconds, the first
    // time in a process, which a start would wait for.
    let nowhere = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    for name in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        std::env::set_var(name, format!("http://{nowhere}"));
    }
    let dir = tempfile::tempdir().unwrap();
    let started = Server::start(Path::new(FAKE), Path::new("m.gguf"), &[], &dir.path().join("k.key"), Duration::from_secs(5)).await;
    assert!(started.is_ok(), "{:?}", started.err());
}
