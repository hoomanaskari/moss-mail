//! Quick reply and the emails Ask Moss writes, from the real writing model on this Mac, over a
//! few conversations, from short notes. Prints every email so a person can read it (whether it
//! says the note and nothing else is for a person to judge) and counts the ones that run long.
//! Ignored by default: it needs the models.
//!
//! ```sh
//! MOSS_AI_DIR="$HOME/Library/Application Support/app.moss.mail/ai" \
//! cargo test -p moss-ai --test real_short_replies -- --ignored --nocapture
//! ```
//! `MOSS_WRITER=apple|e2b|e4b` picks the writing model (Apple's needs `MOSS_APPLE`, the helper
//! built from `apple/`); otherwise the one Moss has set. The models are linked from
//! `MOSS_AI_DIR` into a folder of its own, so a running Moss isn't touched.

use std::path::PathBuf;
use std::sync::Arc;

use moss_ai::manager::{free_space, Catalog, Llama};
use moss_ai::reply::{Context, Job};
use moss_ai::{Ai, Machine, Parts};
use moss_core::Person;

const DAY: i64 = 86_400_000;

fn person(name: &str, email: &str) -> Person {
    Person { name: name.into(), email: email.into() }
}

fn maya() -> Person {
    person("Maya Ross", "maya@northwind.example.test")
}

fn real_ai(dir: &std::path::Path) -> Ai {
    let real = PathBuf::from(std::env::var("MOSS_AI_DIR").expect("MOSS_AI_DIR"));
    for name in ["models", "runtime"] {
        std::os::unix::fs::symlink(real.join(name), dir.join(name)).unwrap();
    }
    std::fs::copy(real.join("settings.json"), dir.join("settings.json")).unwrap();
    if let Ok(writer) = std::env::var("MOSS_WRITER") {
        std::fs::write(dir.join("settings.json"), format!(r#"{{"writer":"{writer}","plan":"recommended"}}"#)).unwrap();
    }
    let ai = Ai::new(Parts {
        dir: dir.to_path_buf(),
        machine: Machine::detect(),
        catalog: Catalog::pinned(),
        launcher: Arc::new(Llama),
        events: Arc::new(|_| {}),
        free_space,
        runtime: None,
        conditions: moss_ai::power::conditions,
        recheck: std::time::Duration::from_secs(30),
        apple: std::env::var_os("MOSS_APPLE").and_then(|h| moss_ai::apple::detect(std::path::Path::new(&h))),
    });
    assert!(ai.writer_ready(), "download the writing model first");
    ai
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// The conversations: who wrote what, oldest first, and what you wrote to them before.
fn conversations() -> Vec<(&'static str, Context)> {
    let alex = person("Alex Morgan Studio Build", "alex@studio-build.example.test");
    let priya = person("Priya Shah", "priya@acme.example.test");
    let dana = person("Dana Lee", "dana@acme.example.test");
    let at = now() - 2 * DAY;
    let context = |subject: &str, messages: Vec<(Person, &str)>, past: Vec<&str>| Context {
        subject: subject.into(),
        messages: messages.into_iter().map(|(p, t)| (p, at, t.to_string())).collect(),
        me: maya(),
        past: past.into_iter().map(String::from).collect(),
        now: now(),
        signature: vec![],
        to: None,
    };
    vec![
        (
            "alex",
            context(
                "Estimate for a studio display cabinet",
                vec![
                    (maya(), "Hi Alex,\n\nOur team needs a display cabinet for the studio. Could you review our sketch next week and send an estimate?\n\nThanks,\nMaya"),
                    (alex.clone(), "Hi Maya, thanks for reaching out. I can review the sample cabinet at the studio next Tuesday. Please send a time that suits your team. Alex"),
                ],
                vec![],
            ),
        ),
        (
            "priya",
            context("Lunch on Thursday?", vec![(priya, "Shall we have lunch on Thursday at noon? Priya")], vec!["Hi Priya,\n\nSounds good, see you there.\n\nCheers,\nMaya"]),
        ),
        (
            "dana",
            context("Contract for the spring shoot", vec![(dana, "Hi Maya, the contract is ready. Can you sign it by Friday? Dana")], vec![]),
        ),
        ("new to alex", Context { to: Some(alex), ..context("Texting", vec![], vec!["Hi Alex,\n\nOur team needs a display cabinet for the studio. Could you review our sketch next week and send an estimate?\n\nThanks,\nMaya"]) }),
    ]
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models on this Mac"]
async fn the_real_writing_model_writes_short_replies_in_your_voice() {
    let dir = tempfile::tempdir().unwrap();
    let ai = real_ai(dir.path());
    let mut misses = vec![];
    for (name, context) in conversations() {
        let notes: &[&str] = match name {
            "alex" | "new to alex" => &["I'll text you soon", "Thursday at 10 works"],
            "priya" => &["yes"],
            _ => &["I'll sign it tomorrow"],
        };
        for note in notes {
            let job = Job::Instructions { instructions: note.to_string() };
            let written = ai.reply(&job, "", &context, &mut |_| true).await.unwrap();
            eprintln!("\n[{name}] from \u{201c}{note}\u{201d} ({} words):\n{written}", words(&written));
            if words(&written) > 30 {
                misses.push(format!("{name}: \u{201c}{note}\u{201d} ran to {} words", words(&written)));
            }
        }
        if context.messages.is_empty() {
            continue;
        }
        let options = ai.reply_options(&context, &mut |_| true).await.unwrap();
        for o in &options {
            eprintln!("\n[{name}] quick reply \u{201c}{}\u{201d} ({} words):\n{}", o.label, words(&o.draft), o.draft);
            if words(&o.draft) > 45 {
                misses.push(format!("{name}: quick reply {} ran to {} words", o.label, words(&o.draft)));
            }
        }
    }
    ai.stop_all().await;
    eprintln!("\nmisses: {misses:?}");
}
