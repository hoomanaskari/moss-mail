//! Ask Moss acting on mail with the real writing model and the app's own actions (the MCP
//! tools), over a small made-up mailbox. Ignored by default: it needs the downloaded models.
//!
//! ```sh
//! MOSS_AI_DIR="$HOME/Library/Application Support/app.moss.mail/ai" \
//! cargo test -p moss --test real_actions -- --ignored --nocapture --test-threads=1
//! ```
//! `MOSS_WRITER=e2b` (or `e4b`) picks the writing model; otherwise the one Moss has set.
//! The models and runtime are linked from `MOSS_AI_DIR`; everything else is in a folder of its
//! own, so a running Moss isn't touched.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use moss_ai::agent::{Answer, AskEvent};
use moss_ai::ask::{Ask, Chats};
use moss_ai::Ai;
use moss_core::{Account, AccountColor, MessageSource, ProviderKind};
use moss_engine::{Engine, Event, Events};

struct Quiet;
impl Events for Quiet {
    fn emit(&self, _: Event) {}
}

fn raw(id: &str, from: &str, subject: &str, extra: &str, body: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: Maya Ross <maya@northwind.example.test>\r\n{extra}Subject: {subject}\r\nMessage-ID: <{id}>\r\n\
         Date: Tue, 03 Nov 2026 09:00:00 +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

fn mailbox(dir: &Path) -> Engine {
    let mut store = moss_store::Store::open(dir.join("moss.db")).unwrap();
    store
        .upsert_account(&Account {
            id: "work".into(),
            name: "Work".into(),
            letter: "W".into(),
            address: "maya@northwind.example.test".into(),
            sender_name: "Maya Ross".into(),
            color: AccountColor::Work,
            provider: ProviderKind::Imap,
            org: None,
            org_domain: None,
        })
        .unwrap();
    let list = "List-Unsubscribe: <mailto:out@news.example>\r\nList-Id: <news.example>\r\n";
    let emails = [
        raw(
            "n1",
            "Design Weekly <news@designweekly.example>",
            "Design Weekly: issue 112",
            list,
            "This week in design: grids, type and colour.",
        ),
        raw(
            "n2",
            "Type Foundry <hello@typefoundry.example>",
            "New fonts this month",
            list,
            "Three new families are out. Read the newsletter.",
        ),
        raw(
            "n3",
            "Studio Notes <notes@studionotes.example>",
            "Studio Notes, issue 42",
            list,
            "Our monthly newsletter about running a studio.",
        ),
        raw(
            "d1",
            "Dana Lee <dana@acme.example.test>",
            "Contract for the spring shoot",
            "",
            "Hi Maya, the contract is ready. Can you sign it by Friday? Dana",
        ),
        raw(
            "p1",
            "Priya Shah <priya@acme.example.test>",
            "Lunch on Thursday?",
            "",
            "Shall we have lunch on Thursday at noon? Priya",
        ),
    ];
    for (i, e) in emails.iter().enumerate() {
        let source = MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: i.to_string(),
            ..Default::default()
        };
        store.ingest(e, &source).unwrap();
    }
    drop(store);
    let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
    moss_lib::engine(
        dir,
        secrets,
        Arc::new(Quiet),
        moss_providers::TlsRoots::Platform,
    )
    .unwrap()
}

/// The real models, linked into `dir/ai`.
fn real_ai(dir: &Path) -> Ai {
    let real = PathBuf::from(std::env::var("MOSS_AI_DIR").expect("MOSS_AI_DIR"));
    let ai = dir.join("ai");
    std::fs::create_dir_all(&ai).unwrap();
    for name in ["models", "runtime"] {
        std::os::unix::fs::symlink(real.join(name), ai.join(name)).unwrap();
    }
    std::fs::copy(real.join("settings.json"), ai.join("settings.json")).unwrap();
    // MOSS_WRITER=e2b tries another downloaded writing model than the one Moss has set.
    if let Ok(writer) = std::env::var("MOSS_WRITER") {
        std::fs::write(
            ai.join("settings.json"),
            format!(r#"{{"writer":"{writer}","plan":"recommended"}}"#),
        )
        .unwrap();
    }
    let ai = moss_lib::ai(dir, Arc::new(|_| {}), None, moss_ai::manager::free_space);
    assert!(ai.writer_ready(), "download the writing model first");
    ai
}

fn id_of(engine: &Engine, subject: &str) -> String {
    engine
        .threads(None)
        .unwrap()
        .into_iter()
        .find(|t| t.subject == subject)
        .unwrap()
        .id
}

/// The app's actions, saying what each call was given and what went wrong.
struct Logged(moss_lib::EngineActions);

#[async_trait::async_trait]
impl moss_ai::tools::Actions for Logged {
    fn specs(&self) -> Vec<serde_json::Value> {
        self.0.specs()
    }
    fn label(&self, name: &str, args: &serde_json::Value) -> Option<String> {
        self.0.label(name, args)
    }
    async fn act(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<Result<moss_ai::tools::Acted, String>> {
        let got = self.0.act(name, args).await;
        if std::env::var_os("MOSS_LOG_CALLS").is_some() {
            let said = match &got {
                Some(Ok(a)) => a.text.chars().take(160).collect::<String>(),
                Some(Err(e)) => format!("ERROR {e}"),
                None => "not an action".into(),
            };
            eprintln!("      {name} {args} -> {said}");
        } else if let Some(Err(e)) = &got {
            eprintln!("      {name} {args} -> {e}");
        }
        got
    }
}

struct Run {
    engine: Engine,
    ai: Ai,
    chats: Chats,
    _dir: tempfile::TempDir,
}

impl Run {
    fn new() -> Run {
        let dir = tempfile::tempdir().unwrap();
        let engine = mailbox(dir.path());
        let ai = real_ai(dir.path());
        // Search by meaning too, as in the app: the embedding model indexes the mail first.
        engine.set_embedder(Arc::new(moss_lib::AiEmbedder(ai.clone())));
        std::thread::sleep(std::time::Duration::from_secs(8));
        Run {
            engine,
            ai,
            chats: Chats::default(),
            _dir: dir,
        }
    }

    async fn ask(&self, chat: &str, open: Option<&str>, question: &str) -> Answer {
        let mail = moss_lib::EngineMail(self.engine.clone());
        let actions = Logged(moss_lib::ask_actions(
            &self.engine,
            Arc::new(Quiet),
            self._dir.path(),
        ));
        let log = |e: AskEvent| {
            if let AskEvent::Tool { label, .. } = e {
                eprintln!("    {label}");
            }
        };
        let open = match open {
            Some(id) => Some(self.engine.thread(id).unwrap()),
            None => None,
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let acts: Option<&dyn moss_ai::tools::Actions> =
            if std::env::var_os("MOSS_NO_ACTIONS").is_some() {
                None
            } else {
                Some(&actions)
            };
        let ask = Ask {
            model: &self.ai,
            mail: &mail,
            actions: acts,
            now,
            emit: &log,
        };
        eprintln!(
            "\n> {question}{}",
            open.as_ref().map_or(String::new(), |t| format!(
                "   (open: {})",
                t.summary.subject
            ))
        );
        if std::env::var_os("MOSS_SHOW_PROMPT").is_some() {
            eprintln!(
                "--- prompt ---\n{}\n---",
                moss_ai::ask::moss_system(now, open.as_ref(), acts.is_some())
            );
        }
        let started = Instant::now();
        let answer = self
            .chats
            .ask(ask, chat, open.as_ref(), question)
            .await
            .unwrap();
        eprintln!(
            "  {:?}\n  did {:?}, drafts {:?}, send later {:?} ({:?})",
            answer.text,
            answer.did,
            answer
                .drafts
                .iter()
                .map(|d| (&d.subject, &d.body))
                .collect::<Vec<_>>(),
            answer.send_at,
            started.elapsed()
        );
        answer
    }

    fn done(&self) -> Vec<String> {
        self.engine
            .done_threads(None, 50)
            .unwrap()
            .into_iter()
            .map(|t| t.subject)
            .collect()
    }
}

/// Whether the answer writes out an email Moss shows under it: most of its lines are in the
/// answer. Quoting its gist in a sentence is fine.
fn writes_out(answer: &Answer) -> bool {
    answer.drafts.iter().any(|d| {
        let lines: Vec<&str> = d.body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let repeated = lines.iter().filter(|l| answer.text.contains(*l)).count();
        repeated * 2 > lines.len()
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models on this Mac"]
async fn the_real_writing_model_does_what_the_window_does_with_mail() {
    let r = Run::new();
    let mut misses = vec![];
    let mut check = |ok: bool, what: &str| {
        eprintln!("  {} {what}", if ok { "OK  " } else { "MISS" });
        if !ok {
            misses.push(what.to_string());
        }
    };

    // Bulk: several emails it has to find first.
    r.ask("bulk", None, "Mark all the newsletters as done.")
        .await;
    let done = r.done();
    check(
        done.len() == 3
            && done
                .iter()
                .all(|s| !s.contains("Contract") && !s.contains("Lunch")),
        "the three newsletters are Done, nothing else",
    );

    // About the open email, without naming it.
    let dana = id_of(&r.engine, "Contract for the spring shoot");
    r.ask("open", Some(&dana), "Remind me about this tomorrow at 9am.")
        .await;
    let snoozed = r.engine.thread(&dana).unwrap().summary.snoozed_until;
    check(snoozed.is_some(), "the open email has a reminder");

    // A reply it readies but never sends.
    let lunch = id_of(&r.engine, "Lunch on Thursday?");
    let answer = r
        .ask(
            "send",
            None,
            "Reply to Priya that Thursday at noon works for me, and send it.",
        )
        .await;
    let ready = answer.drafts.first();
    check(
        ready.is_some_and(|d| d.thread_id.as_deref() == Some(&lunch)),
        "a reply to Priya waits under the answer",
    );
    check(r.engine.scheduled().unwrap().is_empty(), "nothing went out");
    check(!writes_out(&answer), "the answer doesn't write the email out again");

    // What went wrong in the app on 8 November: the answer wrote the draft out as well as the card.
    let answer = r
        .ask("draft", Some(&lunch), "Draft a response to her that I will text her soon")
        .await;
    check(
        answer.drafts.iter().any(|d| d.thread_id.as_deref() == Some(&lunch)),
        "a reply to Priya waits under the answer",
    );
    check(!writes_out(&answer), "the answer doesn't write the email out again");

    // A question about the open email that needs no tools.
    let answer = r.ask("ask", Some(&dana), "By when does she need it?").await;
    check(
        answer.text.to_lowercase().contains("friday"),
        "it answers from the open email",
    );

    // Undoable changes on the open email.
    r.ask("move", Some(&lunch), "Move this to Other.").await;
    let moved = r.engine.thread(&lunch).unwrap().summary;
    check(
        moved.split == moss_core::SplitId::Other,
        "the open email moved to Other",
    );

    // Send later, at the time asked for.
    let answer = r
        .ask(
            "later",
            None,
            "Reply to Dana that I'll sign it today. Send the reply tomorrow at 8am.",
        )
        .await;
    let later = answer
        .drafts
        .iter()
        .find(|d| d.thread_id.as_deref() == Some(&dana));
    check(
        later.is_some_and(|d| answer.send_at.contains_key(&d.id) && !d.body.trim().is_empty()),
        "a reply to Dana, set to go tomorrow",
    );
    check(
        answer.drafts.iter().all(|d| !d.body.contains("[Your")),
        "no name in the sign-off: Moss adds the signature",
    );

    // Forward, to someone new.
    let answer = r
        .ask(
            "forward",
            None,
            "Forward the contract email to sam@acme.example.test.",
        )
        .await;
    check(
        answer.drafts.iter().any(|d| {
            d.mode == moss_core::ComposeMode::Forward
                && d.to.iter().any(|p| p.email == "sam@acme.example.test")
        }),
        "a forward to Sam waits under the answer",
    );

    // Delete and block, only when asked.
    r.ask("delete", Some(&lunch), "Delete this email.").await;
    check(
        !r.engine
            .threads(None)
            .unwrap()
            .iter()
            .any(|t| t.id == lunch),
        "the open email is in Trash",
    );
    r.ask("block", None, "Block Studio Notes.").await;
    let blocked = r.engine.blocked_threads(None, 10).unwrap();
    check(
        blocked
            .iter()
            .any(|t| t.subject == "Studio Notes, issue 42")
            && blocked.len() == 1,
        "Studio Notes is blocked, no one else",
    );

    r.ai.stop_all().await;
    eprintln!("\nmisses: {misses:?}");
}

/// What went wrong in the app on 7 November: three test emails whose subjects start the same,
/// one of them set to come back later.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models on this Mac"]
async fn the_real_writing_model_finds_emails_by_a_phrase_and_stops_reminders() {
    let r = Run::new();
    for (i, (subject, body)) in [
        (
            "Moss chat test A: lunch on Thursday?",
            "Shall we have lunch on Thursday at 12:30? Let me know by Wednesday evening.",
        ),
        (
            "Moss chat test B: invoice 4417",
            "Invoice 4417 for 1,250 EUR is due on 20 November.",
        ),
        (
            "Moss chat test C: weekly notes",
            "Nothing to do here, just some weekly notes.",
        ),
    ]
    .iter()
    .enumerate()
    {
        let mut store = moss_store::Store::open(r._dir.path().join("moss.db")).unwrap();
        let source = MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: format!("t{i}"),
            ..Default::default()
        };
        store
            .ingest(
                &raw(
                    &format!("t{i}"),
                    "Alex Example <alex@example.test>",
                    subject,
                    "",
                    body,
                ),
                &source,
            )
            .unwrap();
    }
    let a = id_of(&r.engine, "Moss chat test A: lunch on Thursday?");
    let later = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + 86_400_000;
    r.engine.snooze(&a, Some(later)).unwrap();
    let mut misses = vec![];
    let mut check = |ok: bool, what: &str| {
        eprintln!("  {} {what}", if ok { "OK  " } else { "MISS" });
        if !ok {
            misses.push(what.to_string());
        }
    };

    r.ask(
        "stop",
        None,
        "Stop reminding me about the Moss chat test A email, I want it back in the inbox now.",
    )
    .await;
    check(
        r.engine.thread(&a).unwrap().summary.snoozed_until.is_none(),
        "the reminder on test A is stopped",
    );

    r.engine.snooze(&a, Some(later)).unwrap();
    r.ask("open", Some(&a), "Stop the reminder on this.").await;
    check(
        r.engine.thread(&a).unwrap().summary.snoozed_until.is_none(),
        "the reminder on the open email is stopped",
    );

    r.ask("bulk", None, "Mark all the Moss chat test emails as done.")
        .await;
    let done = r.done();
    check(
        done.iter()
            .filter(|s| s.starts_with("Moss chat test"))
            .count()
            == 3,
        "the three test emails are Done",
    );
    check(
        done.iter().all(|s| s.starts_with("Moss chat test")),
        "nothing else is Done",
    );

    r.ai.stop_all().await;
    eprintln!("\nmisses: {misses:?}");
}

/// Synthetic conversation covering a reply requested by name over several turns.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models on this Mac"]
async fn the_real_writing_model_drafts_a_reply_from_a_name() {
    let r = Run::new();
    {
        let mut store = moss_store::Store::open(r._dir.path().join("moss.db")).unwrap();
        let source = MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: "alex".into(),
            ..Default::default()
        };
        let body = "Hi Maya, thanks for reaching out. I can review the sample cabinet at the studio next Tuesday. Please send a time that suits your team. Alex";
        store
            .ingest(
                &raw(
                    "alex",
                    "Alex Morgan Studio Build <alex@studio-build.example.test>",
                    "Estimate for a studio display cabinet",
                    "",
                    body,
                ),
                &source,
            )
            .unwrap();
    }
    let alex = id_of(
        &r.engine,
        "Estimate for a studio display cabinet",
    );
    let mut misses = vec![];
    let mut check = |ok: bool, what: &str| {
        eprintln!("  {} {what}", if ok { "OK  " } else { "MISS" });
        if !ok {
            misses.push(what.to_string());
        }
    };
    let to_alex = |a: &Answer| {
        a.drafts
            .iter()
            .any(|d| d.thread_id.as_deref() == Some(alex.as_str()) && !d.body.trim().is_empty())
    };

    let a = r.ask("one", None, "Draft a reply to Alex MOrgan").await;
    check(to_alex(&a), "asked for a reply by name, it drafts one");

    let a = r
        .ask(
            "two",
            None,
            "Reply to Alex saying I will text him as soon as I settle",
        )
        .await;
    check(
        to_alex(&a)
            && a.drafts
                .iter()
                .any(|d| d.body.to_lowercase().contains("text")),
        "told what to say, the draft says it",
    );

    // The same, a turn at a time, as in the app.
    r.ask("turns", None, "Draft a reply to Alex MOrgan").await;
    let a = r
        .ask(
            "turns",
            None,
            "just say I will text him as soon as I settle",
        )
        .await;
    check(
        to_alex(&a)
            && a.drafts
                .iter()
                .any(|d| d.body.to_lowercase().contains("text")),
        "a follow-up with the words becomes the draft",
    );
    check(
        !a.text.contains("Subject:"),
        "the email is a draft card, not text in the answer",
    );

    r.ai.stop_all().await;
    eprintln!("\nmisses: {misses:?}");
}

/// The synthetic mailbox with the studio supplier conversation, served over MCP for
/// another agent to try against: Moss's harness next to theirs, on the same model. Writes the
/// address and token to `MOSS_SERVE_FILE` and keeps serving for `MOSS_SERVE_SECS` seconds.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "serves until stopped; for trying other agents"]
async fn serve_the_made_up_mailbox_over_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let engine = mailbox(dir.path());
    {
        let mut store = moss_store::Store::open(dir.path().join("moss.db")).unwrap();
        let body = "Hi Maya, thanks for reaching out. I can review the sample cabinet at the studio next Tuesday. Please send a time that suits your team. Alex";
        let source = MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: "alex".into(),
            ..Default::default()
        };
        store
            .ingest(
                &raw(
                    "alex",
                    "Alex Morgan Studio Build <alex@studio-build.example.test>",
                    "Estimate for a studio display cabinet",
                    "",
                    body,
                ),
                &source,
            )
            .unwrap();
    }
    let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
    let clock = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    };
    let mcp = moss_mcp::Mcp::new(moss_mcp::McpParts {
        engine: engine.clone(),
        events: Arc::new(Quiet),
        secrets,
        dir: dir.path().to_path_buf(),
        port: 0,
        clock,
    });
    let status = mcp.set_on(true).await.unwrap();
    let out = std::env::var("MOSS_SERVE_FILE").expect("MOSS_SERVE_FILE");
    std::fs::write(
        &out,
        serde_json::json!({"url": status.url, "token": status.token}).to_string(),
    )
    .unwrap();
    let secs: u64 = std::env::var("MOSS_SERVE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800);
    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
    eprintln!(
        "drafts: {:?}",
        engine
            .drafts()
            .unwrap()
            .iter()
            .map(|d| (&d.subject, &d.body))
            .collect::<Vec<_>>()
    );
}

/// One chat on a copy of real mail (never the live database: this upgrades its schema), with the
/// writing model it has set. `MOSS_DATA_DIR` holds `moss.db` and an `ai/` folder whose `models/`
/// and `runtime/` link to the real ones; `MOSS_ASKS` is the questions, split by `|`. Its copy has
/// no passwords, so nothing reaches a mail server. `MOSS_OPEN`, if set, is the thread on screen.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models and a copy of real mail"]
async fn a_chat_on_a_copy_of_real_mail() {
    let dir = PathBuf::from(std::env::var("MOSS_DATA_DIR").expect("MOSS_DATA_DIR"));
    let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
    let engine = moss_lib::engine(
        &dir,
        secrets,
        Arc::new(Quiet),
        moss_providers::TlsRoots::Platform,
    )
    .unwrap();
    let ai = moss_lib::ai(&dir, Arc::new(|_| {}), None, moss_ai::manager::free_space);
    assert!(ai.writer_ready(), "{:?}", ai.status().download);
    engine.set_embedder(Arc::new(moss_lib::AiEmbedder(ai.clone())));
    let r = Run {
        engine,
        ai,
        chats: Chats::default(),
        _dir: tempfile::tempdir().unwrap(),
    };
    let open = std::env::var("MOSS_OPEN").ok();
    for question in std::env::var("MOSS_ASKS").expect("MOSS_ASKS").split('|') {
        r.ask("real", open.as_deref(), question.trim()).await;
    }
    r.ai.stop_all().await;
}

/// Synthetic short-email scenario that exercises the previous failure mode:
/// padding a brief note with filler. Quick reply exercises the same writing path.
/// Moss's earlier email to Alex is in the mailbox, so it knows how the user writes to him.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the real models on this Mac"]
async fn the_real_writing_model_writes_short_emails_that_sound_like_you() {
    let r = Run::new();
    {
        let mut store = moss_store::Store::open(r._dir.path().join("moss.db")).unwrap();
        let source = |id: &str| MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: id.into(),
            ..Default::default()
        };
        let asked = "From: Maya Ross <maya@northwind.example.test>\r\nTo: Alex Morgan Studio Build <alex@studio-build.example.test>\r\n\
            Subject: Estimate for a studio display cabinet\r\nMessage-ID: <ask-alex>\r\n\
            Date: Mon, 02 Nov 2026 09:00:00 +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
            Hi Alex,\r\n\r\nOur team needs a display cabinet for the studio. Could you review our sketch next week and send an estimate?\r\n\r\nThanks,\r\nMaya\r\n";
        store.ingest(asked.as_bytes(), &source("ask-alex")).unwrap();
        let body = "Hi Maya, thanks for reaching out. I can review the sample cabinet at the studio next Tuesday. Please send a time that suits your team. Alex";
        let answer = raw(
            "alex",
            "Alex Morgan Studio Build <alex@studio-build.example.test>",
            "Re: Estimate for a studio display cabinet",
            "In-Reply-To: <ask-alex>\r\nReferences: <ask-alex>\r\n",
            body,
        );
        store.ingest(&answer, &source("alex")).unwrap();
    }
    let alex = r
        .engine
        .threads(None)
        .unwrap()
        .into_iter()
        .find(|t| t.from.email == "alex@studio-build.example.test")
        .unwrap()
        .id;
    let mut misses = vec![];
    let mut check = |ok: bool, what: &str| {
        eprintln!("  {} {what}", if ok { "OK  " } else { "MISS" });
        if !ok {
            misses.push(what.to_string());
        }
    };
    let words = |text: &str| text.split_whitespace().count();

    let a = r
        .ask(
            "alex",
            None,
            "Can you draft an email to Alex Morgan telling him that I will text him soon?",
        )
        .await;
    let body = a.drafts.first().map(|d| d.body.clone()).unwrap_or_default();
    eprintln!("\n  draft:\n{body}\n");
    check(body.to_lowercase().contains("text"), "the draft says it");
    check(words(&body) <= 25, "a one-line ask makes a short email");
    check(!writes_out(&a), "the answer doesn't write the email out");
    check(
        !a.text.contains("Body:") && !a.text.contains("Subject:"),
        "the answer has no Subject and Body of its own beside the card",
    );

    // Quick reply on Alex's email: the choices, then a reply from the user's own words.
    let thread = r.engine.thread(&alex).unwrap();
    let me = moss_core::Person {
        name: "Maya Ross".into(),
        email: "maya@northwind.example.test".into(),
    };
    let past = r
        .engine
        .my_words_to("work", "alex@studio-build.example.test", &alex)
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let context = moss_ai::reply::Context::from_thread(&thread, me, past, now);
    let options = r
        .ai
        .reply_options(&context, &mut |_| true)
        .await
        .unwrap();
    for o in &options {
        eprintln!("\n  quick reply \"{}\":\n{}\n", o.label, o.draft);
    }
    check(
        options.iter().all(|o| words(&o.draft) <= 60),
        "quick replies to a short email are short",
    );
    let job = moss_ai::reply::Job::Instructions {
        instructions: "I'll text you soon".into(),
    };
    let written = r.ai.reply(&job, "", &context, &mut |_| true).await.unwrap();
    eprintln!("\n  reply from \"I'll text you soon\":\n{written}\n");
    check(
        written.to_lowercase().contains("text") && words(&written) <= 40,
        "a reply from a few words stays a few words",
    );

    r.ai.stop_all().await;
    eprintln!("\nmisses: {misses:?}");
}
