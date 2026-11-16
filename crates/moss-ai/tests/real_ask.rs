//! Ask Moss against the real writing model, on this Mac. Ignored by default: it needs the models
//! downloaded (Settings → AI) and takes a few seconds.
//!
//! ```sh
//! MOSS_AI_DIR="$HOME/Library/Application Support/app.moss.mail/ai" \
//! cargo test -p moss-ai --test real_ask -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use moss_ai::agent::AskEvent;
use moss_ai::ask::{Ask, Chats};
use moss_ai::manager::{free_space, Catalog, Llama};
use moss_ai::tools::Mail;
use moss_ai::{Ai, Machine, Parts};
use moss_core::{Contact, Draft, FileHit, FileQuery, Message, Person, SearchHit, Thread, ThreadSummary};

const NOV_4: i64 = 1_793_793_600_000;
const DAY: i64 = 86_400_000;

fn person(name: &str, email: &str) -> Person {
    Person { name: name.into(), email: email.into() }
}

fn thread(id: &str, subject: &str, from: Person, at: i64, text: &str) -> Thread {
    let message = Message {
        id: format!("m{id}"),
        thread_id: id.into(),
        from: from.clone(),
        to: vec![person("Maya Ross", "maya@northwind-co.example.test")],
        cc: vec![],
        bcc: vec![],
        subject: subject.into(),
        sent_at: at,
        paragraphs: vec![text.into()],
        body_text: text.into(),
        body_html: None,
        attachments: vec![],
        read: true,
        image_trust: Default::default(),
        sender_unverified: false,
        headers_only: false,
        images_missing: false,
        forwards: vec![],
        forwarded: false,
    };
    Thread {
        summary: ThreadSummary {
            id: id.into(),
            account_id: "work".into(),
            split: moss_core::SplitId::Important,
            section: moss_core::Section::Reply,
            from,
            subject: subject.into(),
            summary: text.into(),
            last_message_at: at,
            snoozed_until: None,
            unread: false,
            done: false,
            done_at: None,
            labels: vec![],
            message_count: 1,
            has_attachments: false,
            ai_sure: None,
            ai_summary: None,
            ai_summary_why: None,
            sender: Default::default(),
            due: None,
            split_why: None,
            waiting: None,
            moved_by_you: false,
            you_wrote_last: false,
            to_address: None,
        },
        messages: vec![message],
    }
}

struct Inbox {
    threads: Vec<Thread>,
    drafts: Mutex<Vec<Draft>>,
}

#[async_trait]
impl Mail for Inbox {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        Ok(self
            .threads
            .iter()
            .filter(|t| words.iter().any(|w| format!("{} {} {}", t.summary.subject, t.summary.from.name, t.summary.summary).to_lowercase().contains(w)))
            .take(limit)
            .map(|t| SearchHit { thread: t.summary.clone(), message_id: t.messages[0].id.clone(), snippet: vec![], rank: 1.0, file: None })
            .collect())
    }
    fn thread(&self, id: &str) -> Result<Thread, String> {
        self.threads.iter().find(|t| t.summary.id == id).cloned().ok_or_else(|| "no such thread".into())
    }
    fn files(&self, _: &FileQuery) -> Result<Vec<FileHit>, String> {
        Ok(vec![])
    }
    fn file(&self, _: &str, _: u32) -> Result<FileHit, String> {
        Err("no such file".into())
    }
    async fn file_text(&self, message_id: &str, index: u32) -> Result<Option<String>, String> {
        let messages = self.threads.iter().flat_map(|t| &t.messages).filter(|m| m.id == message_id);
        Ok(messages.flat_map(|m| &m.attachments).find(|a| a.index == index).and_then(|a| a.text.clone()))
    }
    fn people(&self, _: &str, _: usize) -> Result<Vec<Contact>, String> {
        Ok(vec![Contact { person: person("Dana Lee", "dana@northwind-co.example.test"), from_them: 3, to_them: 2, last_at: 0 }])
    }
    fn save_draft(&self, draft: &Draft) -> Result<(), String> {
        self.drafts.lock().unwrap().push(draft.clone());
        Ok(())
    }
    fn reply_context(&self, _: &Draft) -> Option<moss_ai::reply::Context> {
        None
    }
}

/// The AI over `MOSS_AI_DIR`, with the writing model downloaded.
fn real_ai() -> Ai {
    let dir = PathBuf::from(std::env::var("MOSS_AI_DIR").expect("MOSS_AI_DIR"));
    let ai = Ai::new(Parts {
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
    });
    assert!(ai.writer_ready(), "download the writing model first");
    ai
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models on this Mac"]
async fn the_real_writing_model_answers_with_tools_and_cites_its_emails() {
    let ai = real_ai();
    let dana = person("Dana Lee", "dana@northwind-co.example.test");
    let sam = person("Sam Ortiz", "sam@northwind-co.example.test");
    let mail = Inbox {
        threads: vec![
            thread("41", "Q4 budget", dana, NOV_4 - 7 * DAY, "Hi Maya, the final Q4 budget is 182k. Can you confirm by Friday?"),
            thread("57", "Re: Q4 budget", sam, NOV_4 - 5 * DAY, "Approved. Ship it."),
            thread("63", "Team lunch", person("Priya Shah", "priya@acme-co.example.test"), NOV_4 - DAY, "Lunch on Thursday at noon?"),
        ],
        drafts: Mutex::new(vec![]),
    };
    let chats = Chats::default();
    let log = |e: AskEvent| match e {
        AskEvent::Tool { label, .. } => eprintln!("  tool: {label}"),
        AskEvent::Step { step, .. } => eprintln!("  step {step}"),
        _ => {}
    };
    let ask = || Ask { model: &ai, mail: &mail, actions: None, now: NOV_4, emit: &log };

    let t = std::time::Instant::now();
    let answer = chats.ask(ask(), "real", None, "Did Sam approve the Q4 budget, and how much is it?").await.unwrap();
    eprintln!("answer: {:?} sources {:?} ({:?} incl. load)", answer.text, answer.sources.iter().map(|s| &s.id).collect::<Vec<_>>(), t.elapsed());
    assert!(answer.text.contains("182"), "{}", answer.text);
    assert!(!answer.sources.is_empty());

    let t = std::time::Instant::now();
    let answer = chats.ask(ask(), "real", None, "Draft a reply to Dana confirming it.").await.unwrap();
    eprintln!("answer: {:?} drafts {:?} ({:?})", answer.text, answer.drafts.iter().map(|d| &d.body).collect::<Vec<_>>(), t.elapsed());
    assert_eq!(answer.drafts.len(), 1);
    assert_eq!(answer.drafts[0].thread_id.as_deref(), Some("41"));

    let lunch = mail.threads[2].clone();
    let t = std::time::Instant::now();
    let answer = chats.ask(ask(), "real", Some(&lunch), "When is it?").await.unwrap();
    eprintln!("thread answer: {:?} ({:?})", answer.text, t.elapsed());
    assert!(answer.text.to_lowercase().contains("thursday"), "{}", answer.text);
    ai.stop_all().await;
}

/// A long conversation: a 22,000-character plan whose last line holds the one fact asked for,
/// then three replies after it. Any cap on what the model reads would lose that fact.
fn long_launch_thread() -> Thread {
    let dana = person("Dana Whitfield", "dana@brightside-agency.example.test");
    let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];
    let mut plan = String::from("Hi Maya,\n\nHere is the full plan for the spring launch, week by week.\n\n");
    for n in 1..=200 {
        plan.push_str(&format!(
            "Item {n}: the shot list for scene {n} moves to {}; the stylist needs {} outfits, and props for it cost €{}.\n",
            days[n % 5],
            n % 7 + 2,
            n * 40
        ));
    }
    plan.push_str("\nOne more thing: the entry code for the launch night venue is ORCHID-7731. Please keep it private.\n\nThanks, Dana");
    let mut t = thread("80", "Spring launch plan", dana.clone(), NOV_4 - 6 * DAY, &plan);
    t.summary.summary = "The full spring launch plan, week by week".into();
    let replies = [
        (person("Maya Ross", "maya@northwind-co.example.test"), "Thanks Dana, this is great. I'll go through it tonight."),
        (dana.clone(), "Also: the shot list is final now, no more changes."),
        (person("Sam Ortiz", "sam@northwind-co.example.test"), "Budget approved for all of it."),
    ];
    for (i, (from, text)) in replies.into_iter().enumerate() {
        let mut m = t.messages[0].clone();
        m.id = format!("m80-{i}");
        m.from = from;
        m.sent_at = NOV_4 - (5 - i as i64) * DAY;
        m.paragraphs = vec![text.into()];
        m.body_text = text.into();
        t.messages.push(m);
    }
    t.summary.message_count = t.messages.len() as u32;
    t
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models on this Mac"]
async fn the_real_writing_model_reads_a_long_thread_whole() {
    let ai = real_ai();
    let long = long_launch_thread();
    let chars: usize = long.messages.iter().map(|m| m.body_text.chars().count()).sum();
    eprintln!("the thread: {} messages, {chars} characters", long.messages.len());
    assert!(chars > 20_000);
    let mail = Inbox { threads: vec![long.clone()], drafts: Mutex::new(vec![]) };
    let chats = Chats::default();
    let log = |e: AskEvent| {
        if let AskEvent::Tool { label, .. } = e {
            eprintln!("  tool: {label}");
        }
    };
    let ask = || Ask { model: &ai, mail: &mail, actions: None, now: NOV_4, emit: &log };

    // With the email open, the whole thread is in the prompt.
    let t = std::time::Instant::now();
    let answer = chats.ask(ask(), "open", Some(&long), "What is the entry code for the launch night venue?").await.unwrap();
    eprintln!("this email: {:?} ({:?} incl. load)", answer.text, t.elapsed());
    assert!(answer.text.contains("7731"), "{}", answer.text);

    // A follow-up about the replies after the long email.
    let answer = chats.ask(ask(), "open", Some(&long), "Who approved the budget, and is the shot list final?").await.unwrap();
    eprintln!("follow-up: {:?}", answer.text);
    assert!(answer.text.contains("Sam"), "{}", answer.text);

    // No email open: it has to find the thread and read it, all of it.
    let t = std::time::Instant::now();
    let answer = chats.ask(ask(), "long", None, "What entry code did Dana give for the launch night venue?").await.unwrap();
    eprintln!("all mail: {:?} sources {:?} ({:?})", answer.text, answer.sources.iter().map(|s| &s.id).collect::<Vec<_>>(), t.elapsed());
    assert!(answer.text.contains("7731"), "{}", answer.text);
    ai.stop_all().await;
}
