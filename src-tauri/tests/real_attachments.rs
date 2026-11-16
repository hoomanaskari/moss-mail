//! Ask Moss on a copy of real mail with the real models, where the answer is inside a file (a
//! PDF). Ignored by default: it needs the downloaded models and a copy of a real mail database
//! (never the live one: this upgrades its schema).
//!
//! ```sh
//! MOSS_DATA_DIR=/path/to/copy MOSS_ASK="when does my parking permit start?" MOSS_EXPECT=12 \
//! cargo test -p moss --test real_attachments -- --ignored --nocapture
//! ```
//! `MOSS_DATA_DIR` holds `moss.db` and an `ai/` folder whose `models/` and `runtime/` can be
//! links to the real ones; give it its own `run/`, so a running Moss isn't touched. Files
//! opened before are in the database's cache, so they're read without signing in.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moss_ai::agent::AskEvent;
use moss_ai::ask::{Ask, Chats};
use moss_engine::{Event, Events};

struct Quiet;
impl Events for Quiet {
    fn emit(&self, _: Event) {}
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models and a copy of real mail"]
async fn ask_moss_answers_from_inside_a_file() {
    let dir = PathBuf::from(std::env::var("MOSS_DATA_DIR").expect("MOSS_DATA_DIR"));
    let question = std::env::var("MOSS_ASK").expect("MOSS_ASK");
    let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
    let engine = moss_lib::engine(&dir, secrets, Arc::new(Quiet), moss_providers::TlsRoots::Platform).unwrap();
    let reader = moss_lib::MacReader { native: Arc::new(moss_extract::mac::Mac), conditions: || None };
    engine.set_reader(Arc::new(reader));
    let ai = moss_lib::ai(&dir, Arc::new(|_| {}), None, moss_ai::manager::free_space);
    assert!(ai.writer_ready(), "{:?}", ai.status().download);
    engine.set_embedder(Arc::new(moss_lib::AiEmbedder(ai.clone())));
    // Recent files are read in the background meanwhile.
    tokio::time::sleep(Duration::from_secs(20)).await;
    let mail = moss_lib::EngineMail(engine.clone());
    let log = |e: AskEvent| match e {
        AskEvent::Tool { label, .. } => eprintln!("  tool: {label}"),
        AskEvent::Step { step, .. } => eprintln!("  step {step}"),
        _ => {}
    };
    let started = Instant::now();
    let ask = Ask { model: &ai, mail: &mail, actions: None, now: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64, emit: &log };
    let answer = Chats::default().ask(ask, "real", None, &question).await.unwrap();
    let files: Vec<&str> = answer.files.iter().map(|f| f.name.as_str()).collect();
    eprintln!("answer ({:?}): {}\nfiles: {files:?}", started.elapsed(), answer.text);
    if let Ok(expect) = std::env::var("MOSS_EXPECT") {
        assert!(answer.text.contains(&expect), "{}", answer.text);
        assert!(!files.is_empty(), "it points to the file");
    }
    ai.stop_all().await;
}
