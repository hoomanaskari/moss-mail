//! AI replies against the real writing model on this Mac, to read what it writes. Ignored by
//! default: it needs the downloaded models.
//!
//! ```sh
//! MOSS_AI_DIR="$HOME/Library/Application Support/app.moss.mail/ai" \
//! MOSS_LLAMA_SERVER=/opt/homebrew/bin/llama-server \
//! cargo test -p moss-ai --test real_replies -- --ignored --nocapture
//! ```
//!
//! `ONLY=<name>` runs one conversation; `SIGNATURE="Best|Maya"` adds a signature below each reply.

use std::path::PathBuf;
use std::sync::Arc;

use moss_ai::manager::{free_space, Catalog, Llama};
use moss_ai::reply::{Context, Job};
use moss_ai::{Ai, Download, Machine, Parts};
use moss_core::Person;

fn person(name: &str, email: &str) -> Person {
    Person { name: name.into(), email: email.into() }
}

/// Sat 31 Oct 2026, and today, Thu 5 Nov 2026.
const DAY: i64 = 1_793_448_000_000;
const TODAY: i64 = 1_793_880_000_000;

/// The signature added below each reply: `SIGNATURE="Best|Maya"`, one line per `|`; none by default.
fn signature() -> Vec<String> {
    std::env::var("SIGNATURE").map(|s| s.split('|').map(str::to_string).collect()).unwrap_or_default()
}

fn alex() -> Person {
    person("Alex Morgan Studio Build", "alex@studio-build.example.test")
}

fn ai() -> Ai {
    let dir = PathBuf::from(std::env::var("MOSS_AI_DIR").expect("MOSS_AI_DIR"));
    Ai::new(Parts {
        dir,
        machine: Machine::detect(),
        catalog: Catalog::pinned(),
        launcher: Arc::new(Llama),
        events: Arc::new(|_| {}),
        free_space,
        runtime: std::env::var_os("MOSS_LLAMA_SERVER").map(PathBuf::from),
        conditions: moss_ai::power::conditions,
        recheck: std::time::Duration::from_secs(30),
        apple: None,
    })
}

/// Realistic conversations: a work ask, a friend's plan, a client, one in Dutch, a long
/// thread whose latest is a two-line note, an FYI.
fn conversations() -> Vec<(&'static str, Context)> {
    let maya = person("Maya Ross", "maya@rossstudio-art.example.test");
    let priya = person("Priya Nair", "priya@brightlane-com.example.test");
    let daniel = person("Daniel Okafor", "daniel@northwind-co.example.test");
    let sam = person("Sam", "sam@example.test");
    let joost = person("Joost de Vries", "joost@studiovries-nl.example.test");
    let it = person("Northwind IT", "it@northwind-co.example.test");
    vec![
        ("work ask", Context {
            subject: "Q4 design budget: sign-off needed".into(),
            messages: vec![(daniel.clone(), DAY, "Hi Maya,\n\nI’ve updated the Q4 design budget after Tuesday’s planning call. The main change is a new line for user testing (€2,400), which we agreed is worth doing before the onboarding launch. The new total is €18,400.\n\nCould you approve it by Thursday? If anything looks off, reply with what you’d change.\n\nThanks,\nDaniel".into())],
            me: maya.clone(),
            now: TODAY,
            signature: signature(),
            to: None,
            past: vec!["Hi Daniel,\n\nThanks for pulling this together so quickly. Looks good to me, go ahead.\n\nBest,\nMaya".into()],
        }),
        ("client thread", Context {
            subject: "Spring campaign: contract and launch plan".into(),
            messages: vec![
                (priya.clone(), DAY, "Hi Maya,\n\nGreat news: legal signed off, so the final contract is attached. Could you countersign and send it back by Thursday 6 November?\n\nThe budget stays at €24,000, half on signing, half after launch.\n\nLaunch moves to Monday 17 November, because the print run needs an extra week. Jonas will send the print list on Tuesday.\n\nThanks,\nPriya".into()),
            ],
            me: maya.clone(),
            past: vec![],
            now: TODAY,
            signature: signature(),
            to: None,
        }),
        ("friend", Context {
            subject: "Saturday?".into(),
            messages: vec![
                (sam.clone(), DAY, "hey! are you around saturday? thinking of a long walk along the coast then lunch somewhere. bring the dog if you want 🐕".into()),
                (maya.clone(), DAY, "Ooh yes, Saturday works. What time are you thinking?".into()),
                (sam.clone(), DAY, "10ish at the station? or is that too early after your week".into()),
            ],
            me: maya.clone(),
            now: TODAY,
            signature: signature(),
            to: None,
            past: vec!["haha yes!! see you then x".into()],
        }),
        ("dutch", Context {
            subject: "Offerte fotoshoot".into(),
            messages: vec![(joost.clone(), DAY, "Hoi Maya,\n\nDank voor je offerte. Die ziet er goed uit. Kunnen we de shoot verplaatsen naar dinsdag 18 november? En is het mogelijk om ook een paar portretten van het team te maken?\n\nGroet,\nJoost".into())],
            me: maya.clone(),
            past: vec![],
            now: TODAY,
            signature: signature(),
            to: None,
        }),
        // A long email thread whose latest message is a two-line note: you still write emails here.
        ("contractor", Context {
            subject: "Re: Studio display cabinet estimate".into(),
            messages: vec![
                (maya.clone(), DAY, "Hi Alex,\n\nOur design team needs a display cabinet for the studio samples. We would like adjustable shelves and two drawers, with a simple painted finish. The sketch is ready for review. Could you send an estimate and suggest a suitable material?\n\nBest\nMaya".into()),
                (alex(), DAY, "Thanks for the sketch. I will compare two material options.\nAlex".into()),
                (alex(), DAY, "The samples are ready. Is the project still going ahead?\nAlex".into()),
                (maya.clone(), DAY, "Hi Alex,\n\nThanks for checking. The team is reviewing the display layout this week. Once the dimensions are confirmed, I will send the updated drawing so you can prepare the final estimate.\n\nBest\nMaya".into()),
                (alex(), DAY, "The sample is ready.\nPlease message me tomorrow.\nAlex".into()),
            ],
            me: maya.clone(),
            past: vec![],
            now: TODAY,
            signature: signature(),
            to: None,
        }),
        ("fyi", Context {
            subject: "Laptop update on Thursday".into(),
            messages: vec![(it.clone(), DAY, "Hi all,\n\nWe’re rolling out a security update to every laptop on Thursday evening. Please restart before you leave on Thursday and save your work. It takes about 20 minutes.\n\nNorthwind IT".into())],
            me: maya.clone(),
            past: vec![],
            now: TODAY,
            signature: signature(),
            to: None,
        }),
    ]
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models"]
async fn the_real_model_writes_replies() {
    let ai = ai();
    assert_eq!(ai.status().download, Download::Ready, "{:?}", ai.status());
    let only = std::env::var("ONLY").ok();
    for (name, c) in conversations() {
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let t = std::time::Instant::now();
        match ai.reply_options(&c, &mut |_| true).await {
            Ok(options) => {
                eprintln!("\n===== {name}: quick replies ({:?})", t.elapsed());
                for o in options {
                    eprintln!("--- [{}]\n{}", o.label, o.draft);
                }
            }
            Err(e) => eprintln!("\n===== {name}: quick replies failed: {e}"),
        }
        for (label, job, text) in [
            ("accept", Job::Accept, ""),
            ("decline", Job::Decline, ""),
            ("note", Job::Instructions { instructions: "say yes but ask if we can do it a day later".into() }, ""),
            ("shorter", Job::Shorter, "Hi there,\n\nThanks so much for this, I really appreciate you taking the time to send it over. I had a look and I think it all makes sense to me, so I am happy to go ahead with it as it is. If anything changes on my side I will let you know as soon as I can.\n\nThanks again"),
        ] {
            let t = std::time::Instant::now();
            let written = ai.reply(&job, text, &c, &mut |_| true).await;
            match written {
                Ok(w) => eprintln!("\n===== {name}: {label} ({:?})\n{w}", t.elapsed()),
                Err(e) => eprintln!("\n===== {name}: {label} failed: {e}"),
            }
        }
    }
}
