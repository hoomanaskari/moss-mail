//! The AI end to end with the real launcher, against the fake `llama-server` in `tests/fake/`:
//! download, start each model on its first job, and talk to it with its private key.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use moss_ai::catalog::{Asset, Writer};
use moss_ai::manager::{Catalog, Llama};
use moss_ai::{Ai, Download, Machine, Parts};
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-llama-server");

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

async fn serve(server: &MockServer, id: &'static str, bytes: Vec<u8>) -> Asset {
    let sha = leak(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect());
    let asset = Asset {
        id,
        url: leak(format!("{}/{id}", server.uri())),
        file: leak(format!("{id}.gguf")),
        bytes: bytes.len() as u64,
        sha256: sha,
    };
    Mock::given(method("GET"))
        .and(path(format!("/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(server)
        .await;
    asset
}

#[tokio::test]
async fn downloads_then_runs_both_models_on_this_mac() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer {
        id: "w",
        name: "Gemma",
        sub: "",
        asset: serve(&server, "writer", vec![2; 3000]).await,
        needs_gb: 0,
        recommended_from_gb: 0,
    };
    let catalog = Catalog {
        runtime: moss_ai::catalog::RUNTIME,
        runtime_dir: "unused",
        embedder: serve(&server, "embedder", vec![3; 1000]).await,
        decider: serve(&server, "decider", vec![1; 2000]).await,
        writers: vec![writer],
    };
    let ai = Ai::new(Parts {
        dir: dir.path().join("ai"),
        machine: Machine::detect(),
        catalog,
        launcher: Arc::new(Llama),
        events: Arc::new(|_| {}),
        free_space: moss_ai::manager::free_space,
        runtime: Some(PathBuf::from(FAKE)),
        conditions: || None,
        recheck: std::time::Duration::from_secs(30),
        apple: None,
    });
    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);

    assert_eq!(ai.needs_reply("From: Dana\n\nCan you send the deck?").await, Ok(0.9));
    assert_eq!(ai.ask_line("From: Dana\n\nCan you send the deck?").await, Ok(Some("Reply to the fake.".into())));
    let key = std::fs::read_to_string(dir.path().join("ai/run/decider.key")).unwrap();
    assert_eq!(key.len(), 64);
    assert!(Path::new(&dir.path().join("ai/run/writer.key")).is_file());
}
