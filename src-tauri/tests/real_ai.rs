//! Both models on real mail, on this Mac. Ignored by default: it needs the downloaded models
//! and a copy of a real mail database (never the live one: this upgrades its schema).
//!
//! ```sh
//! MOSS_DATA_DIR=/path/to/copy MOSS_LLAMA_SERVER=/opt/homebrew/bin/llama-server \
//! cargo test -p moss --test real_ai -- --ignored --nocapture
//! ```
//! `MOSS_DATA_DIR` holds `moss.db` and `ai/` (the models).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moss_core::Section;
use moss_engine::{Event, Events};

struct Quiet;
impl Events for Quiet {
    fn emit(&self, _: Event) {}
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models and a copy of real mail"]
async fn the_models_sort_and_write_for_real_mail() {
    let dir = PathBuf::from(std::env::var("MOSS_DATA_DIR").expect("MOSS_DATA_DIR"));
    let runtime = std::env::var_os("MOSS_LLAMA_SERVER").map(PathBuf::from);
    let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
    let roots = moss_providers::TlsRoots::Platform;
    let engine = moss_lib::engine(&dir, secrets, Arc::new(Quiet), roots).unwrap();
    let free = moss_ai::manager::free_space;
    let ai = moss_lib::ai(&dir, Arc::new(|_| {}), runtime.as_deref(), free);
    assert!(ai.decider_ready() && ai.writer_ready(), "{:?}", ai.status().download);

    let sorted = |engine: &moss_engine::Engine| {
        engine.threads(None).unwrap().into_iter().filter(|t| t.ai_sure.is_some()).count()
    };
    let before = sorted(&engine);
    let started = Instant::now();
    engine.set_sorter(Arc::new(moss_lib::AiSorter(ai.clone())));
    // Sorting runs in the background: wait until it has been quiet for a while.
    let (mut last, mut quiet) = (before, Instant::now());
    while quiet.elapsed() < Duration::from_secs(30) && started.elapsed() < Duration::from_secs(900)
    {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let now = sorted(&engine);
        if now != last {
            (last, quiet) = (now, Instant::now());
        }
    }
    let threads = engine.threads(None).unwrap();
    let ai_sorted: Vec<_> = threads.iter().filter(|t| t.ai_sure.is_some()).collect();
    let reply = ai_sorted.iter().filter(|t| t.section == Section::Reply).count();
    eprintln!(
        "decision model: {} threads sorted in {:?} ({reply} need a reply, {} for your information)",
        ai_sorted.len(),
        started.elapsed(),
        ai_sorted.len() - reply
    );
    // Unsure answers (between 20% and 80%) move nothing, so count the questions asked too.
    let store = moss_store::Store::open(dir.join("moss.db")).unwrap();
    let asked = store.undecided(moss_store::NEEDS_REPLY, 0, 10_000, true).unwrap().len();
    eprintln!("still waiting for the decision model: {asked}");
    assert_eq!(asked, 0, "every recent email was asked about");

    let thread = threads.iter().find(|t| t.section == Section::Reply).expect("mail to reply to");
    let started = Instant::now();
    let ask = ai.ask_for(&engine.thread(&thread.id).unwrap()).await.unwrap();
    eprintln!("writing model, the ask ({:?} incl. load): {ask:?}", started.elapsed());
}
