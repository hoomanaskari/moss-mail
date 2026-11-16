//! Summaries end to end: rules writing them in the background, asking for one, the setting.

use std::sync::atomic::Ordering;

use moss_core::{SummaryContent, SummaryKind, SummaryRequest, SummaryWhy};

use super::*;

fn of(kind: SummaryKind) -> SummaryRequest {
    SummaryRequest::of(kind)
}

fn from_paydesk(id: &str, subject: &str, at: i64) -> Vec<u8> {
    raw_from(id, "Paydesk <billing@paydesk.example>", "Maya Ross <maya.ross@gmail.com>", subject, at)
}

fn text(s: &str) -> SummaryContent {
    SummaryContent::Text { text: s.into() }
}

#[tokio::test(start_paused = true)]
async fn mail_from_a_rules_sender_is_summarised_before_you_open_it() {
    let s = setup();
    let summarizer = Arc::new(FakeSummarizer::default());
    s.gmail().await;
    s.synced().await;
    let rule = s.engine.save_summary_rule(None, "paydesk.example", &of(SummaryKind::Facts)).unwrap();
    assert_eq!(s.engine.summary_rules().unwrap(), [rule.clone()]);
    s.engine.set_summarizer(summarizer.clone());
    s.fake.add(message("INBOX", 1, from_paydesk("i1@x", "Your invoice", NOW), NOW));
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    let id = s.thread_id("Your invoice");
    let written = s.events.wait_for(|e| matches!(e, Event::SummaryWritten { .. })).await;
    assert_eq!(written, Event::SummaryWritten { thread_id: id.clone() });
    let kept = s.engine.thread_summaries(&id, "fake-writer").unwrap();
    assert_eq!(kept.rule, Some(rule.clone()));
    let facts = kept.summary.as_ref().unwrap();
    assert_eq!((facts.request.kind, facts.why, facts.stale), (SummaryKind::Facts, SummaryWhy::Rule, false));
    assert_eq!(facts.content, text("facts of Your invoice"));
    // Opening it needs no model: it's kept.
    let need = s.engine.summary_for(&id, &of(SummaryKind::Facts), "fake-writer", false).unwrap();
    assert_eq!(need, SummaryNeed::Kept(facts.clone()));
    // A one-message thread from a sender without a rule gets nothing.
    assert_eq!(summarizer.read.lock().unwrap().clone(), ["facts Your invoice"]);
    // While the model can't run, nothing is written, and nothing breaks.
    summarizer.down.store(true, Ordering::Relaxed);
    s.engine.save_summary_rule(Some(rule.id), "paydesk.example", &of(SummaryKind::Actions)).unwrap();
    settle().await;
    assert_eq!(s.engine.thread_summaries(&id, "fake-writer").unwrap().summary.unwrap().request.kind, SummaryKind::Facts);
    assert!(s.engine.remove_summary_rule(rule.id).unwrap());
    assert!(!s.engine.remove_summary_rule(rule.id).unwrap());
}

#[tokio::test(start_paused = true)]
async fn a_summary_you_ask_for_is_written_once_until_a_new_message_or_you_ask_again() {
    let s = setup();
    s.inbox(1, "q1@x", "Quote", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Quote");
    let actions = of(SummaryKind::Actions);
    let SummaryNeed::Write(c) = s.engine.summary_for(&id, &actions, "e4b", false).unwrap() else { panic!("write it") };
    assert_eq!((c.subject.as_str(), c.why, c.you.as_str()), ("Quote", SummaryWhy::Asked, "maya.ross@gmail.com"));
    let kept = s.engine.keep_summary(&c, &actions, "e4b", &SummaryContent::Actions { items: vec![] }).unwrap();
    assert_eq!((kept.made_at, kept.why), (NOW, SummaryWhy::Asked));
    assert_eq!(s.engine.summary_for(&id, &actions, "e4b", false).unwrap(), SummaryNeed::Kept(kept));
    assert!(matches!(s.engine.summary_for(&id, &actions, "e4b", true).unwrap(), SummaryNeed::Write(_)), "again");
    assert!(matches!(s.engine.summary_for(&id, &actions, "26b", false).unwrap(), SummaryNeed::Write(_)), "another model");
    s.fake.add(message("INBOX", 2, reply_to(&["q1@x"], raw("q2@x", "Re: Quote", NOW)), NOW));
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    let stale = s.engine.thread_summaries(&id, "e4b").unwrap();
    assert!(stale.summary.unwrap().stale);
    let SummaryNeed::Write(c) = s.engine.summary_for(&id, &actions, "e4b", false).unwrap() else { panic!("write it") };
    assert_eq!(c.messages.len(), 2);
    // A custom summary needs an instruction; a thread that isn't there is an error.
    let empty = SummaryRequest { kind: SummaryKind::Custom, instruction: "".into() };
    let e = s.engine.summary_for(&id, &empty, "e4b", false).unwrap_err();
    assert_eq!(serde_json::to_value(&e).unwrap(), serde_json::json!({"kind": "invalid", "message": "say how to summarise it"}));
    assert!(s.engine.summary_for("999", &actions, "e4b", false).is_err());
    assert!(s.engine.thread_summaries("999", "e4b").is_err());
    let bad_rule = s.engine.save_summary_rule(None, "gmail.com", &actions).unwrap_err();
    assert_eq!(serde_json::to_value(&bad_rule).unwrap()["kind"], "invalid");
}

#[tokio::test(start_paused = true)]
async fn the_automatic_one_liners_can_be_turned_off_and_on() {
    let s = setup();
    s.fake.add(message("INBOX", 1, raw("q1@x", "Quote", NOW - 2 * DAY), NOW - 2 * DAY));
    s.fake.add(message("INBOX", 2, reply_to(&["q1@x"], raw("q2@x", "Re: Quote", NOW - DAY)), NOW - DAY));
    let summarizer = Arc::new(FakeSummarizer::default());
    s.engine.set_summarizer(summarizer.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert!(s.engine.automatic_summaries().unwrap());
    let first = s.engine.threads(None).unwrap().remove(0);
    assert_eq!((first.ai_summary.as_deref(), first.ai_summary_why), (Some("2 messages about Quote."), Some(SummaryWhy::Important)));
    s.events.clear();
    s.engine.set_automatic_summaries(false).unwrap();
    assert!(!s.engine.automatic_summaries().unwrap());
    assert!(s.events.all().contains(&Event::MailChanged { account_id: "gmail".into() }));
    settle().await;
    assert_eq!(s.engine.threads(None).unwrap()[0].ai_summary, None);
    s.engine.set_automatic_summaries(true).unwrap();
    settle().await;
    assert_eq!(s.engine.threads(None).unwrap()[0].ai_summary.as_deref(), Some("2 messages about Quote."));
}

#[tokio::test(start_paused = true)]
async fn a_discarded_summary_leaves_the_conversation_and_its_list_line() {
    let s = setup();
    s.fake.add(message("INBOX", 1, raw("q1@x", "Quote", NOW - 2 * DAY), NOW - 2 * DAY));
    s.fake.add(message("INBOX", 2, reply_to(&["q1@x"], raw("q2@x", "Re: Quote", NOW - DAY)), NOW - DAY));
    let summarizer = Arc::new(FakeSummarizer::default());
    s.engine.set_summarizer(summarizer.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    let id = s.thread_id("Quote");
    assert!(s.engine.thread_summaries(&id, "fake-writer").unwrap().summary.is_some());
    s.events.clear();
    s.engine.discard_summary(&id).unwrap();
    assert_eq!(s.events.all(), [Event::MailChanged { account_id: "gmail".into() }]);
    assert_eq!(s.engine.thread_summaries(&id, "fake-writer").unwrap().summary, None);
    // The background writer leaves it alone.
    s.engine.wake_summarizer();
    settle().await;
    assert_eq!(s.engine.threads(None).unwrap()[0].ai_summary, None);
    assert!(s.engine.discard_summary("999").is_err());
}

#[tokio::test(start_paused = true)]
async fn rule_summaries_stop_when_the_database_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("moss.db");
    let s = setup_with(dir, FakeProvider::new(), Arc::new(MemorySecretStore::new()));
    s.gmail().await;
    s.synced().await;
    s.engine.save_summary_rule(None, "paydesk.example", &of(SummaryKind::Short)).unwrap();
    s.fake.add(message("INBOX", 1, from_paydesk("i1@x", "Invoice 1", NOW - DAY), NOW - DAY));
    s.fake.add(message("INBOX", 2, from_paydesk("i2@x", "Invoice 2", NOW), NOW));
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    let other = rusqlite::Connection::open(&db).unwrap();
    other
        .execute_batch("CREATE TRIGGER full BEFORE INSERT ON thread_summaries BEGIN SELECT RAISE(FAIL, 'disk full'); END;")
        .unwrap();
    let summarizer = FakeSummarizer::default();
    s.engine.shared.summarize_waiting(&summarizer).await;
    assert_eq!(summarizer.read.lock().unwrap().len(), 1, "stops at the first failed save");
    other.execute_batch("DROP TRIGGER full; DROP TABLE summary_rules").unwrap();
    s.engine.shared.summarize_waiting(&summarizer).await;
    assert_eq!(summarizer.read.lock().unwrap().len(), 1, "nothing to read rules from");
    assert!(s.engine.summary_rules().is_err());
    assert!(s.engine.set_automatic_summaries(true).is_ok());
    other.execute_batch("DROP TABLE settings").unwrap();
    assert!(s.engine.automatic_summaries().is_err());
    assert!(s.engine.set_automatic_summaries(false).is_err());
}

#[tokio::test(start_paused = true)]
async fn asking_now_writes_with_the_model_given_and_keeps_it() {
    let s = setup();
    s.inbox(1, "q1@x", "Quote", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Quote");
    let facts = of(SummaryKind::Facts);
    let asked = std::sync::atomic::AtomicUsize::new(0);
    let write = |c: moss_store::ToSummarize| {
        asked.fetch_add(1, Ordering::SeqCst);
        async move { Ok::<_, String>(text(&c.subject)) }
    };
    let written = s.engine.summarize_now(&id, &facts, "e4b", false, &write).await.unwrap();
    assert_eq!((written.content, written.why), (text("Quote"), SummaryWhy::Asked));
    // Kept: the model isn't asked again.
    let kept = s.engine.summarize_now(&id, &facts, "e4b", false, &write).await;
    assert_eq!(kept.unwrap().content, text("Quote"));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    // The model is never asked for these.
    let never = |_: moss_store::ToSummarize| async { Err::<SummaryContent, _>("not asked".to_string()) };
    // Asked again, and the model fails: its error comes back as it is.
    let failed = s.engine.summarize_now(&id, &facts, "e4b", true, |_| async { Err("no model".to_string()) }).await.unwrap_err();
    assert_eq!(serde_json::to_value(&failed).unwrap(), serde_json::json!("no model"));
    // A custom instruction is kept as you meant it, without the spaces.
    let sloppy = SummaryRequest { kind: SummaryKind::Custom, instruction: "  In French ".into() };
    s.engine.summarize_now(&id, &sloppy, "e4b", false, |_| async { Ok::<_, String>(text("En français")) }).await.unwrap();
    let tidy = SummaryRequest::new(SummaryKind::Custom, "In French").unwrap();
    assert!(matches!(s.engine.summary_for(&id, &tidy, "e4b", false).unwrap(), SummaryNeed::Kept(_)));
    let missing = s.engine.summarize_now("999", &facts, "e4b", false, &never).await.unwrap_err();
    assert_eq!(serde_json::to_value(&missing).unwrap()["kind"], "unknownThread");
    // A custom summary with no instruction is refused before anything is read.
    let blank = SummaryRequest { kind: SummaryKind::Custom, instruction: "   ".into() };
    let refused = s.engine.summarize_now(&id, &blank, "e4b", false, &never).await.unwrap_err();
    assert_eq!(serde_json::to_value(&refused).unwrap()["kind"], "invalid");
}
