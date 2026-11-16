//! Against the real models, on this Mac. Ignored by default: it downloads gigabytes.
//!
//! ```sh
//! MOSS_AI_DIR="$HOME/Library/Application Support/app.moss.mail/ai" \
//! MOSS_LLAMA_SERVER=/opt/homebrew/bin/llama-server \
//! cargo test -p moss-ai --test real -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use moss_ai::manager::{free_space, Catalog, Llama};
use moss_ai::{Ai, Download, Machine, Parts};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads the real models"]
async fn the_real_models_decide_and_write() {
    let dir = PathBuf::from(std::env::var("MOSS_AI_DIR").expect("MOSS_AI_DIR"));
    let ai = Ai::new(Parts {
        dir,
        machine: Machine::detect(),
        catalog: Catalog::pinned(),
        launcher: Arc::new(Llama),
        events: Arc::new(|s| {
            if let Download::Downloading { done, total } = s.download {
                eprintln!("{:.1}%", done as f64 * 100.0 / total as f64);
            }
        }),
        free_space,
        runtime: std::env::var_os("MOSS_LLAMA_SERVER").map(PathBuf::from),
        conditions: moss_ai::power::conditions,
        recheck: std::time::Duration::from_secs(30),
        apple: None,
    });
    eprintln!("{:?}", ai.status());
    if let Some(task) = ai.download() {
        task.await.unwrap();
    }
    assert_eq!(ai.status().download, Download::Ready, "{:?}", ai.status());

    let asks = "From: Dana Wu <dana@acme-co.example.test>\nSubject: Q3 deck\nAttachments: none\n\n\
        Hi Maya, could you send me the Q3 board deck by Thursday? I need it for the investor call.";
    let fyi = "From: Acme Status <status@acme-co.example.test>\nSubject: Weekly report\nAttachments: none\n\n\
        Here is your weekly summary. All systems were up 100% of the time. No action needed.";
    let t = std::time::Instant::now();
    let yes = ai.needs_reply(asks).await.unwrap();
    let no = ai.needs_reply(fyi).await.unwrap();
    eprintln!("needs reply: asks={yes:.2} fyi={no:.2} ({:?} incl. load)", t.elapsed());
    assert!(yes > no, "{yes} {no}");

    let t = std::time::Instant::now();
    let line = ai.ask_line(asks).await.unwrap();
    eprintln!("ask: {line:?} ({:?} incl. load)", t.elapsed());
    assert!(line.is_some());
}
