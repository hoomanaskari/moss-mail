//! Every kind of summary of a real email, from the real writing model on this Mac, to read
//! and judge by eye. Ignored by default. Give it the email's text as Moss cleans it:
//!
//! ```sh
//! MOSS_AI_DIR="$HOME/Library/Application Support/app.moss.mail/ai" \
//! MOSS_SUMMARY_EMAIL=email.txt MOSS_SUMMARY_FROM="Studio News <updates@newsletter.example.test>" \
//! MOSS_SUMMARY_SUBJECT="Quarterly studio project digest" \
//! cargo test -p moss-ai --test real_summaries -- --ignored --nocapture
//! ```
//!
//! `MOSS_APPLE=apple/.build/release/moss-apple` writes them with Apple Intelligence instead
//! (when Moss is set to write with it).

use std::path::PathBuf;
use std::sync::Arc;

use moss_ai::manager::{free_space, Catalog, Llama};
use moss_ai::summaries::input;
use moss_ai::{Ai, Download, Machine, Parts};
use moss_core::{Person, SummaryKind, SummaryRequest};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real writing model and an email"]
async fn every_kind_of_summary_of_a_real_email() {
    let var = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name}"));
    let ai = Ai::new(Parts {
        dir: PathBuf::from(var("MOSS_AI_DIR")),
        machine: Machine::detect(),
        catalog: Catalog::pinned(),
        launcher: Arc::new(Llama),
        events: Arc::new(|_| {}),
        free_space,
        runtime: std::env::var_os("MOSS_LLAMA_SERVER").map(PathBuf::from),
        conditions: moss_ai::power::conditions,
        recheck: std::time::Duration::from_secs(30),
        apple: std::env::var_os("MOSS_APPLE").and_then(|helper| moss_ai::apple::detect(&PathBuf::from(helper))),
    });
    assert_eq!(ai.status().download, Download::Ready, "{:?}", ai.status());
    let text = std::fs::read_to_string(var("MOSS_SUMMARY_EMAIL")).unwrap();
    let from = var("MOSS_SUMMARY_FROM");
    let (name, email) = from.split_once(" <").map_or(("", from.as_str()), |(n, e)| (n, e.trim_end_matches('>')));
    let person = Person { name: name.into(), email: email.into() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    eprintln!("The email has {} words.", text.split_whitespace().count());
    let conversation = input("maya@ross-art.example.test", now, &var("MOSS_SUMMARY_SUBJECT"), &[(person, now, text)]);
    let custom = SummaryRequest::new(SummaryKind::Custom, "Only the news about Anthropic").unwrap();
    for request in [SummaryKind::Short, SummaryKind::Detailed, SummaryKind::Actions, SummaryKind::Facts].map(SummaryRequest::of).into_iter().chain([custom]) {
        let started = std::time::Instant::now();
        // As the app asks for it: streaming, read the way its kind reads.
        let mut parts = 0;
        let got = ai.summarize_streaming(&conversation, &request, &mut |_| { parts += 1; true }).await;
        let words = match &got {
            Ok(moss_core::SummaryContent::Text { text }) => text.split_whitespace().count(),
            _ => 0,
        };
        eprintln!("\n===== {:?} ({:.1}s, {parts} parts, {words} words) =====\n{got:?}", request.kind, started.elapsed().as_secs_f64());
    }
    ai.stop_all().await;
}
