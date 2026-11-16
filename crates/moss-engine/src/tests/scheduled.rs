//! Send later: emails wait on this Mac until their time, then the engine sends them.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use moss_core::{ComposeMode, Draft, Person, ScheduledEmail};

use super::*;

const HOUR: i64 = 3_600_000;
const MINUTE: i64 = 60_000;

fn draft(thread_id: Option<String>) -> Draft {
    Draft {
        revision: None,
        id: "d1".into(),
        mode: if thread_id.is_some() { ComposeMode::Reply } else { ComposeMode::New },
        thread_id,
        from_account_id: "gmail".into(),
        to: vec![Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Re: Q4 budget".into(),
        body: "Approved.".into(),
        body_html: "<p>Approved.</p>".into(),
        remind_if_no_reply: false,
        updated_at: NOW,
        signature_id: None,
        signature_chosen: false,
        from_address: None,
        attachments: vec![],
    }
}

fn outgoing(d: &Draft) -> OutgoingDraft {
    OutgoingDraft {
        mode: d.mode,
        thread_id: d.thread_id.clone(),
        from_account_id: d.from_account_id.clone(),
        to: d.to.clone(),
        cc: d.cc.clone(),
        bcc: d.bcc.clone(),
        subject: d.subject.clone(),
        body: d.body.clone(),
        body_html: d.body_html.clone(),
        signature: vec!["Maya".into()],
        from_address: d.from_address.clone(),
        remind_if_no_reply: d.remind_if_no_reply,
        attachments: d.attachments.clone(),
    }
}

/// A Gmail account, synced, with Daniel's "Q4 budget" in it. Sync is stopped, so only
/// sending reaches the fake server.
async fn with_thread(clock: fn() -> i64) -> (Setup, String) {
    let dir = tempfile::tempdir().unwrap();
    let s = setup_all(dir, FakeProvider::new(), Arc::new(MemorySecretStore::new()), Endpoints::default(), Arc::new(DnsMx::system()), clock);
    s.inbox(1, "q4@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    s.engine.shared.stop_worker("gmail");
    let id = s.thread_id("Q4 budget");
    (s, id)
}

fn sent_events(s: &Setup) -> Vec<(String, bool)> {
    s.events
        .all()
        .into_iter()
        .filter_map(|e| match e {
            Event::ScheduledSent { email, late } => Some((email.draft.id, late)),
            _ => None,
        })
        .collect()
}

static AT_TIME: AtomicI64 = AtomicI64::new(NOW);
fn at_time() -> i64 {
    AT_TIME.load(Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn an_email_set_to_send_later_waits_for_its_time_then_goes_and_is_never_done() {
    let (s, thread) = with_thread(at_time).await;
    let d = draft(Some(thread.clone()));
    let d = s.engine.save_draft(&d).unwrap();
    s.engine.schedule(&d, &outgoing(&d), NOW + HOUR).unwrap();
    assert!(s.engine.drafts().unwrap().is_empty(), "out of Drafts");
    let waiting = ScheduledEmail { revision: 1, operation_id: s.engine.shared.store().scheduled_operation(&d.id).unwrap().unwrap(), draft: Draft { revision: None, ..d.clone() }, send_at: NOW + HOUR, next_try: NOW + HOUR, tries: 0, error: None };
    assert_eq!(s.engine.scheduled().unwrap(), [waiting.clone()]);

    s.engine.send_due().await;
    assert!(s.fake.sent().is_empty(), "not yet");

    AT_TIME.store(NOW + HOUR, Ordering::SeqCst);
    s.engine.send_due().await;
    let sent = s.fake.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].in_reply_to.as_deref(), Some("q4@x"));
    assert!(sent[0].text.as_deref().unwrap().starts_with("Approved.\n\n-- \nMaya\n"), "signed as chosen");
    assert!(s.engine.scheduled().unwrap().is_empty());
    assert_eq!(sent_events(&s), [("d1".into(), false)]);
    assert!(s.events.all().contains(&Event::MailChanged { account_id: "gmail".into() }));
    // In the conversation, and never moved to Done: only you do that.
    assert_eq!(s.engine.thread(&thread).unwrap().messages.len(), 2);
    assert!(s.engine.done_threads(None, 50).unwrap().is_empty());
    assert!(s.engine.threads(None).unwrap().iter().any(|t| t.id == thread));
}

fn fixed() -> i64 {
    NOW
}

#[tokio::test(start_paused = true)]
async fn sending_later_starts_again_after_a_panic_and_never_sends_an_email_twice() {
    let (s, _) = with_thread(fixed).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    // A bug after the server took the email, before Moss could note it went.
    s.fake.panic_after_send(1);
    s.engine.send_later();
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(s.fake.sent().len(), 1);
    // Started again, it can't know whether the email went, so it doesn't send it again: it
    // asks you to check Sent, as after a crash, rather than showing "sending" for ever.
    tokio::time::sleep(Duration::from_secs(16)).await;
    assert_eq!(s.fake.sent().len(), 1, "never twice");
    let operation = s.engine.shared.store().scheduled_operation(&d.id).unwrap().unwrap();
    assert_eq!(s.engine.delivery_state(&operation).unwrap(), Some(moss_store::DeliveryState::Uncertain));
    assert!(s.engine.shared.store().delivery(&operation).unwrap().unwrap().error.unwrap().contains("Check Sent"));
    assert!(!s.engine.shared.scheduled_going.lock().unwrap().contains(&d.id));
    // And the next email still goes.
    let next = Draft { id: "d2".into(), ..draft(None) };
    s.engine.schedule(&next, &outgoing(&next), NOW).unwrap();
    tokio::time::sleep(Duration::from_secs(31)).await;
    assert_eq!(s.fake.sent().len(), 2);
    assert!(sent_events(&s).contains(&("d2".into(), false)));
}

#[tokio::test(start_paused = true)]
async fn scheduling_still_works_after_a_panic_while_its_list_was_held() {
    let (s, _) = with_thread(fixed).await;
    super::poison(&s.engine.shared.scheduled_going);
    super::poison(&s.engine.shared.scheduled_recovery_announced);
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    s.engine.send_due().await;
    assert_eq!(s.fake.sent().len(), 1);
}

static CLOSED: AtomicI64 = AtomicI64::new(NOW);
fn closed() -> i64 {
    CLOSED.load(Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn an_email_due_while_moss_was_closed_goes_soon_after_launch_and_says_it_is_late() {
    let (s, _) = with_thread(closed).await;
    let d = draft(None);
    // Set for 8 AM; Moss opens again at 11.
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    CLOSED.store(NOW + 3 * HOUR, Ordering::SeqCst);
    s.engine.start().unwrap();
    s.engine.shared.stop_worker("gmail");
    // Not in the first moments, so the window is up to hear that it went.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(s.fake.sent().is_empty());
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(sent_events(&s), [("d1".into(), true)]);
    assert_eq!(s.fake.sent().len(), 1);
}

static RETRY: AtomicI64 = AtomicI64::new(NOW);
fn retry() -> i64 {
    RETRY.load(Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn a_failed_send_is_kept_shown_and_tried_again_later_and_later() {
    let (s, _) = with_thread(retry).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    s.fake.fail_next(moss_providers::Error::Network("no route".into()));
    s.engine.send_due().await;
    let failed = s.engine.scheduled().unwrap();
    assert_eq!(failed.len(), 1, "never lost");
    assert_eq!((failed[0].tries, failed[0].next_try), (1, NOW + MINUTE));
    assert!(failed[0].error.as_deref().unwrap().contains("can't reach the server"));
    assert!(s.events.all().contains(&Event::ScheduledFailed { email: failed[0].clone() }));

    s.engine.send_due().await;
    assert!(s.fake.sent().is_empty(), "waits a minute before trying again");

    // Each failure waits longer: 1, 2, 5, 15, 30 minutes, then every hour.
    let mut at = NOW;
    for (tries, wait) in [(2, 2), (3, 5), (4, 15), (5, 30), (6, 60), (7, 60)] {
        at = s.engine.scheduled().unwrap()[0].next_try;
        RETRY.store(at, Ordering::SeqCst);
        s.fake.fail_next(moss_providers::Error::Protocol("451 try later".into()));
        s.engine.send_due().await;
        let email = &s.engine.scheduled().unwrap()[0];
        assert_eq!((email.tries, email.next_try), (tries, at + wait * MINUTE));
    }
    RETRY.store(at + HOUR, Ordering::SeqCst);
    s.engine.send_due().await;
    assert_eq!(s.fake.sent().len(), 1);
    assert!(s.engine.scheduled().unwrap().is_empty());
    assert_eq!(sent_events(&s), [("d1".into(), true)]);
}

#[tokio::test(start_paused = true)]
async fn a_scheduled_email_can_get_a_new_time_or_go_back_to_drafts() {
    let (s, thread) = with_thread(clock).await;
    let d = draft(Some(thread));
    s.engine.schedule(&d, &outgoing(&d), NOW + HOUR).unwrap();
    s.engine.reschedule("d1", NOW + 2 * HOUR).unwrap();
    assert_eq!(s.engine.scheduled().unwrap()[0].send_at, NOW + 2 * HOUR);

    let back = s.engine.unschedule("d1").unwrap();
    assert_eq!(back.revision.as_ref().map(String::len), Some(32));
    assert_eq!(draft_contents([back.clone()]), draft_contents([d.clone()]), "as it was written");
    assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([d.clone()]));
    assert!(s.engine.scheduled().unwrap().is_empty());
    // Gone (sent or cancelled already): says so.
    assert!(matches!(s.engine.unschedule("d1"), Err(Error::Unavailable(_))));
    assert!(matches!(s.engine.reschedule("d1", NOW), Err(Error::Unavailable(_))));
    // A time already past means now.
    s.engine.schedule(&back, &outgoing(&back), NOW - HOUR).unwrap();
    assert_eq!(s.engine.scheduled().unwrap()[0].send_at, NOW);
    s.engine.reschedule("d1", NOW - HOUR).unwrap();
    assert_eq!(s.engine.scheduled().unwrap()[0].send_at, NOW);
}

#[tokio::test(start_paused = true)]
async fn an_email_is_checked_before_it_is_set_to_send_later() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    let nobody = OutgoingDraft { to: vec![], ..outgoing(&d) };
    assert!(matches!(s.engine.schedule(&d, &nobody, NOW + HOUR), Err(Error::NoRecipients)));
    let stranger = OutgoingDraft { from_account_id: "nope".into(), ..outgoing(&d) };
    assert!(matches!(s.engine.schedule(&d, &stranger, NOW + HOUR), Err(Error::UnknownAccount(_))));
    let elsewhere = OutgoingDraft { from_address: Some("me@elsewhere.art".into()), ..outgoing(&d) };
    assert!(matches!(s.engine.schedule(&d, &elsewhere, NOW + HOUR), Err(Error::NotYourAddress { .. })));
    assert!(s.engine.scheduled().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_email_on_its_way_can_no_longer_be_cancelled() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    s.fake.delay_sends(Duration::from_secs(5));
    let engine = s.engine.clone();
    let sending = tokio::spawn(async move { engine.send_due().await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(matches!(s.engine.unschedule("d1"), Err(Error::Invalid(_))));
    assert!(matches!(s.engine.reschedule("d1", NOW + HOUR), Err(Error::Invalid(_))));
    sending.await.unwrap();
    assert_eq!(s.fake.sent().len(), 1);
    assert!(s.engine.drafts().unwrap().is_empty(), "sent, not back in Drafts");
}

static RUNNING: AtomicI64 = AtomicI64::new(NOW);
fn running() -> i64 {
    RUNNING.load(Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn while_moss_runs_an_email_goes_at_its_time() {
    let (s, _) = with_thread(running).await;
    s.engine.start().unwrap();
    s.engine.shared.stop_worker("gmail");
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW + 90_000).unwrap();
    settle().await;
    assert!(s.fake.sent().is_empty());
    // The clock moves on; the engine looks again at least every 30 s.
    RUNNING.store(NOW + 90_000, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(31)).await;
    settle().await;
    assert_eq!(s.fake.sent().len(), 1);
    assert_eq!(sent_events(&s), [("d1".into(), false)]);

    // Send now: a new time of now goes at once, without waiting for the next look.
    let now = OutgoingDraft { subject: "Now".into(), ..outgoing(&d) };
    let soon = Draft { id: "d2".into(), ..d.clone() };
    s.engine.schedule(&soon, &now, NOW + 2 * HOUR).unwrap();
    settle().await;
    s.engine.reschedule("d2", NOW).unwrap();
    settle().await;
    assert_eq!(s.fake.sent().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_broken_database_is_an_error_and_the_engine_keeps_going() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("UPDATE scheduled SET outgoing = 'not json'").unwrap();
    // An email Moss can't read stays and says why, rather than going wrong.
    s.engine.send_due().await;
    let kept = s.engine.scheduled().unwrap();
    assert_eq!(kept[0].tries, 1);
    assert!(s.fake.sent().is_empty());
    db.execute_batch("DROP TABLE scheduled").unwrap();
    assert!(s.engine.scheduled().is_err());
    assert!(s.engine.schedule(&d, &outgoing(&d), NOW).is_err());
    assert!(s.engine.reschedule("d1", NOW).is_err());
    assert!(s.engine.unschedule("d1").is_err());
    s.engine.send_due().await;
}

#[tokio::test(start_paused = true)]
async fn a_database_that_breaks_while_sending_is_said_and_the_engine_keeps_going() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    s.fake.delay_sends(Duration::from_secs(5));
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    let rename = |sql: &str| db.execute_batch(sql).unwrap();

    // It went, but Moss can't cross it off: it still says it went.
    let engine = s.engine.clone();
    let sending = tokio::spawn(async move { engine.send_due().await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    rename("ALTER TABLE scheduled RENAME TO hidden");
    sending.await.unwrap();
    assert_eq!(sent_events(&s), [("d1".into(), false)]);
    rename("ALTER TABLE hidden RENAME TO scheduled");

    // The accepted operation only needs cleanup. A different email fails next.
    let second = Draft { id: "second".into(), ..d.clone() };
    s.engine.schedule(&second, &outgoing(&second), NOW).unwrap();
    s.fake.fail_next(moss_providers::Error::Network("no route".into()));
    let engine = s.engine.clone();
    let failing = tokio::spawn(async move { engine.send_due().await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    rename("ALTER TABLE scheduled RENAME TO hidden");
    failing.await.unwrap();
    assert!(s.events.all().iter().any(|e| matches!(e, Event::ScheduledFailed { .. })));
    rename("ALTER TABLE hidden RENAME TO scheduled");
    assert_eq!(s.engine.scheduled().unwrap().len(), 1, "never lost");
    assert_eq!(s.fake.sent().len(), 1, "the accepted first email is never submitted again");
}

static WAITS: AtomicI64 = AtomicI64::new(NOW);
fn waits() -> i64 {
    WAITS.load(Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn a_scheduled_email_waits_on_an_answer_once_it_has_gone() {
    let (s, thread) = with_thread(waits).await;
    // Remind me if no reply, set in the composer.
    let d = Draft { remind_if_no_reply: true, ..draft(Some(thread.clone())) };
    s.engine.schedule(&d, &outgoing(&d), NOW + HOUR).unwrap();
    assert_eq!(s.engine.thread(&thread).unwrap().summary.waiting, None, "nothing waits before it goes");
    WAITS.store(NOW + HOUR, Ordering::SeqCst);
    s.engine.send_due().await;
    let row = s.engine.thread(&thread).unwrap().summary;
    assert_eq!(row.waiting.unwrap().why.by, moss_core::DecidedBy::You);
    assert_eq!(row.section, Section::Later);

    // Without it, a question still gets Moss's own check once it's sent.
    let asks = Draft { id: "d2".into(), subject: "Final numbers".into(), body: "Can you send them?".into(), ..draft(None) };
    s.engine.schedule(&asks, &outgoing(&asks), NOW + HOUR).unwrap();
    s.engine.send_due().await;
    let sorter = Arc::new(ReplyOnly);
    s.engine.shared.sort_waiting(sorter.as_ref()).await;
    let sent = s.engine.threads(None).unwrap().into_iter().find(|t| t.subject == "Final numbers").unwrap();
    assert_eq!(sent.waiting.unwrap().why.by, moss_core::DecidedBy::Rule);
}

#[tokio::test(start_paused = true)]
async fn cancelling_or_rescheduling_after_the_due_snapshot_prevents_delivery() {
    for cancel in [true, false] {
        let (s, _) = with_thread(clock).await;
        let first = draft(None);
        let second = Draft { id: "d2".into(), subject: "Must wait".into(), ..first.clone() };
        s.engine.schedule(&first, &outgoing(&first), NOW).unwrap();
        s.engine.schedule(&second, &outgoing(&second), NOW).unwrap();
        s.fake.delay_sends(Duration::from_secs(5));
        let engine = s.engine.clone();
        let sending = tokio::spawn(async move { engine.send_due().await });
        tokio::time::sleep(Duration::from_secs(1)).await;
        if cancel { s.engine.unschedule("d2").unwrap(); }
        else { s.engine.reschedule("d2", NOW + HOUR).unwrap(); }
        sending.await.unwrap();
        assert_eq!(s.fake.sent().len(), 1, "a stale due snapshot must not send the second email");
        if cancel { assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([second])); }
        else { assert_eq!(s.engine.scheduled().unwrap()[0].send_at, NOW + HOUR); }
    }
}

#[tokio::test(start_paused = true)]
async fn overlapping_scheduler_runs_do_not_send_the_same_email_twice() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    s.fake.delay_sends(Duration::from_secs(5));
    let engine = s.engine.clone();
    let first = tokio::spawn(async move { engine.send_due().await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    s.engine.send_due().await;
    first.await.unwrap();
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_uncertain_scheduled_submission_is_never_automatically_retried() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    let operation = s.engine.shared.store().scheduled_operation(&d.id).unwrap().unwrap();
    s.fake.fail_next(moss_providers::Error::DeliveryUnknown("lost reply after DATA".into()));
    s.engine.send_due().await;
    assert_eq!(s.engine.delivery_state(&operation).unwrap(), Some(moss_store::DeliveryState::Uncertain));
    s.engine.shared.store().email_failed(&d.id, "force next scan", NOW).unwrap();
    s.engine.send_due().await;
    assert!(s.fake.sent().is_empty());
    assert_eq!(s.engine.scheduled().unwrap().len(), 1);
    assert!(s.engine.reschedule(&d.id, NOW).is_err());
    assert!(s.engine.unschedule(&d.id).is_err());
}

#[tokio::test(start_paused = true)]
async fn scheduled_local_repair_reuses_the_accepted_operation_without_resending() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    let operation = s.engine.shared.store().scheduled_operation(&d.id).unwrap().unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    s.engine.send_due().await;
    assert_eq!(s.engine.delivery_state(&operation).unwrap(), Some(moss_store::DeliveryState::Accepted));
    assert_eq!(s.fake.sent().len(), 1);
    db.execute_batch("DROP TRIGGER refuse_sent").unwrap();
    s.engine.shared.store().email_failed(&d.id, "retry local repair", NOW).unwrap();
    s.engine.send_due().await;
    assert_eq!(s.fake.sent().len(), 1);
    assert!(s.engine.scheduled().unwrap().is_empty());
    assert_eq!(s.engine.delivery_state(&operation).unwrap(), Some(moss_store::DeliveryState::Complete));
}

#[tokio::test(start_paused = true)]
async fn restarting_preserves_a_safe_scheduled_retry_and_its_frozen_payload() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    let operation = s.engine.shared.store().scheduled_operation(&d.id).unwrap().unwrap();
    s.fake.fail_next(moss_providers::Error::Network("connection refused".into()));
    s.engine.send_due().await;
    let prepared: crate::delivery::PreparedSend = serde_json::from_str(&s.engine.shared.store().delivery(&operation).unwrap().unwrap().outgoing).unwrap();
    let Setup { _dir: dir, fake, secrets, engine, .. } = s;
    drop(engine);
    settle().await;
    let again = setup_with(dir, fake.clone(), secrets);
    again.engine.recover_deliveries().unwrap();
    assert_eq!(again.engine.delivery_state(&operation).unwrap(), Some(moss_store::DeliveryState::Held));
    assert!(again.engine.drafts().unwrap().is_empty(), "scheduled ownership survives recovery");
    again.engine.send_due().await;
    assert!(fake.sent().is_empty(), "the backoff survives restart");
    again.engine.reschedule(&d.id, NOW).unwrap();
    again.engine.send_due().await;
    assert_eq!(fake.sent(), [prepared.message]);
    assert_eq!(again.engine.delivery_state(&operation).unwrap(), Some(moss_store::DeliveryState::Complete));
}

#[tokio::test(start_paused = true)]
async fn completed_schedule_cleanup_failures_back_off_instead_of_spinning() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_cleanup BEFORE DELETE ON scheduled BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    s.engine.send_due().await;
    assert_eq!(s.fake.sent().len(), 1);
    assert!(s.engine.shared.store().next_due().unwrap().unwrap() > NOW);
    s.engine.send_due().await;
    assert_eq!(s.fake.sent().len(), 1);
    assert_eq!(sent_events(&s).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn operation_bound_undo_before_creation_is_honoured_by_the_engine() {
    use moss_store::ScheduleOutcome;
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.save_draft(&d).unwrap();
    assert_eq!(s.engine.cancel_schedule_with_operation("early", &d.id).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
    assert_eq!(s.engine.schedule_with_operation("early", &d, &outgoing(&d), NOW + HOUR).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
    assert!(s.engine.scheduled().unwrap().is_empty());
    assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([d]));
}

#[tokio::test(start_paused = true)]
async fn old_operation_actions_do_not_touch_a_new_schedule_in_the_engine() {
    use moss_store::ScheduleOutcome;
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule_with_operation("old", &d, &outgoing(&d), NOW + HOUR).unwrap();
    assert!(matches!(s.engine.cancel_schedule_with_operation("old", &d.id).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: Some(_) }));
    let d = s.engine.editable_draft(&d.id).unwrap().unwrap();
    s.engine.schedule_with_operation("new", &d, &outgoing(&d), NOW + 2 * HOUR).unwrap();
    assert_eq!(s.engine.reschedule_with_operation("old", &d.id, NOW).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
    assert_eq!(s.engine.cancel_schedule_with_operation("old", &d.id).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
    let waiting = s.engine.scheduled().unwrap();
    assert_eq!(waiting[0].operation_id, "new");
    assert_eq!(waiting[0].send_at, NOW + 2 * HOUR);
    assert_eq!(s.engine.reschedule_with_operation("new", &d.id, NOW + 3 * HOUR).unwrap(), ScheduleOutcome::Scheduled);
    assert_eq!(s.engine.scheduled().unwrap()[0].send_at, NOW + 3 * HOUR);
}

#[tokio::test(start_paused = true)]
async fn operation_actions_only_observe_the_matching_scheduler_claim() {
    use moss_store::ScheduleOutcome;
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.cancel_schedule_with_operation("old", &d.id).unwrap();
    s.engine.schedule_with_operation("busy", &d, &outgoing(&d), NOW + HOUR).unwrap();
    s.engine.shared.scheduled_going.lock().unwrap().insert(d.id.clone());
    assert_eq!(s.engine.cancel_schedule_with_operation("busy", &d.id).ok().map(|v| serde_json::to_value(v).unwrap()), Some(serde_json::json!({"status": "sending"})));
    assert_eq!(s.engine.reschedule_with_operation("busy", &d.id, NOW).ok().map(|v| serde_json::to_value(v).unwrap()), Some(serde_json::json!({"status": "sending"})));
    assert_eq!(s.engine.cancel_schedule_with_operation("old", &d.id).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
    assert_eq!(s.engine.reschedule_with_operation("old", &d.id, NOW).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
    let waiting = s.engine.scheduled().unwrap();
    assert_eq!(waiting[0].operation_id, "busy");
    assert_eq!(waiting[0].send_at, NOW + HOUR);
    s.engine.shared.scheduled_going.lock().unwrap().remove(&d.id);
}

#[tokio::test(start_paused = true)]
async fn operation_actions_report_the_live_send_then_its_completed_outcome() {
    use moss_store::ScheduleOutcome;
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule_with_operation("live", &d, &outgoing(&d), NOW).unwrap();
    s.fake.delay_sends(Duration::from_secs(5));
    let engine = s.engine.clone();
    let sending = tokio::spawn(async move { engine.send_due().await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(s.engine.schedule_with_operation("live", &d, &outgoing(&d), NOW).unwrap(), ScheduleOutcome::Sending);
    assert_eq!(s.engine.cancel_schedule_with_operation("live", &d.id).unwrap(), ScheduleOutcome::Sending);
    assert_eq!(s.engine.reschedule_with_operation("live", &d.id, NOW + HOUR).unwrap(), ScheduleOutcome::Sending);
    sending.await.unwrap();
    assert_eq!(s.fake.sent().len(), 1);
    assert_eq!(s.engine.schedule_with_operation("live", &d, &outgoing(&d), NOW).unwrap(), ScheduleOutcome::Completed);
    assert_eq!(s.engine.cancel_schedule_with_operation("live", &d.id).unwrap(), ScheduleOutcome::Completed);
    assert_eq!(s.engine.reschedule_with_operation("live", &d.id, NOW + HOUR).unwrap(), ScheduleOutcome::Completed);
    assert!(s.engine.drafts().unwrap().is_empty());
    assert!(s.engine.scheduled().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn retained_schedule_outcomes_survive_account_removal_and_stale_validation() {
    use moss_store::ScheduleOutcome;
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.cancel_schedule_with_operation("cancelled", &d.id).unwrap();
    s.engine.schedule_with_operation("completed", &d, &outgoing(&d), NOW + HOUR).unwrap();
    s.engine.shared.store().email_sent(&d.id, NOW).unwrap();
    s.engine.shared.store().remove_account("gmail").unwrap();
    let mut invalid = outgoing(&d);
    invalid.to.clear();
    for (op, expected) in [("cancelled", ScheduleOutcome::Cancelled { early: false, restored: None }), ("completed", ScheduleOutcome::Completed)] {
        assert_eq!(s.engine.schedule_with_operation(op, &d, &outgoing(&d), NOW).unwrap(), expected);
        assert_eq!(s.engine.schedule_with_operation(op, &d, &invalid, NOW).unwrap(), expected);
    }
    assert!(s.engine.scheduled().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn competing_moves_cannot_overwrite_the_first_committed_revision() {
    use moss_store::ScheduleOutcome;
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule_with_operation("op", &d, &outgoing(&d), NOW + HOUR).unwrap();
    s.engine.reschedule_with_revision("op", &d.id, NOW + 3 * HOUR, 1).unwrap();
    let stale = s.engine.reschedule_with_revision("op", &d.id, NOW + 2 * HOUR, 1).unwrap();
    let current = s.engine.scheduled().unwrap().remove(0);
    assert_eq!(current.send_at, NOW + 3 * HOUR);
    assert_eq!(current.revision, 2);
    assert_eq!(stale, ScheduleOutcome::Changed { current });
    assert_eq!(s.engine.reschedule_with_revision("op", &d.id, NOW + 4 * HOUR, 2).unwrap(), ScheduleOutcome::Rescheduled { revision: 3 });
}

#[tokio::test(start_paused = true)]
async fn schedule_mutations_notify_other_clients_only_after_actual_changes() {
    let (s, _) = with_thread(|| NOW).await;
    let d = draft(None);
    let changes = || s.events.all().into_iter().filter(|e| serde_json::to_value(e).unwrap()["type"] == "scheduledChanged").count();
    s.engine.schedule_with_operation("notify", &d, &outgoing(&d), NOW + HOUR).unwrap();
    assert_eq!(changes(), 1);
    s.engine.schedule_with_operation("notify", &d, &outgoing(&d), NOW + HOUR).unwrap();
    assert_eq!(changes(), 1, "replaying creation does not invalidate the list");
    s.engine.reschedule_with_revision("notify", &d.id, NOW + 2 * HOUR, 1).unwrap();
    assert_eq!(changes(), 2);
    s.engine.reschedule_with_revision("notify", &d.id, NOW + 3 * HOUR, 1).unwrap();
    assert_eq!(changes(), 2, "a stale revision does not change the schedule");
    s.engine.cancel_known_schedule("notify", &d.id).unwrap();
    assert_eq!(changes(), 3);
    s.engine.cancel_known_schedule("notify", &d.id).unwrap();
    assert_eq!(changes(), 3, "replaying cancellation does not restore another draft");
    let d = s.engine.editable_draft(&d.id).unwrap().unwrap();
    s.engine.schedule(&d, &outgoing(&d), NOW + HOUR).unwrap();
    assert_eq!(changes(), 4);
    s.engine.reschedule(&d.id, NOW + 2 * HOUR).unwrap();
    assert_eq!(changes(), 5);
    s.engine.unschedule(&d.id).unwrap();
    assert_eq!(changes(), 6);
}

#[tokio::test(start_paused = true)]
async fn scheduler_waits_for_recovery_without_recording_a_send_attempt() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    s.engine.schedule(&d, &outgoing(&d), NOW).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    let recovering = Draft { id: "recovering".into(), subject: "Uncertain ordinary delivery".into(), ..d.clone() };
    {
        let mut store = s.engine.shared.store();
        store.prepare_delivery("recovering", &recovering, "{}", "recovering@example.test", NOW).unwrap();
        assert!(store.claim_delivery("recovering").unwrap());
    }
    *s.engine.shared.delivery_recovered.lock().unwrap() = false;
    db.execute_batch("CREATE TRIGGER fail_recovery BEFORE UPDATE OF state ON deliveries WHEN OLD.id = 'recovering' BEGIN SELECT RAISE(ABORT, 'recovery storage fault'); END").unwrap();
    s.engine.send_due().await;
    assert!(s.fake.sent().is_empty());
    assert_eq!(s.engine.scheduled().unwrap()[0].tries, 0);
    assert!(s.events.all().iter().any(|e| matches!(e, Event::ScheduledWaiting { email } if email.tries==0 && email.error.as_deref().is_some_and(|why|why.contains("waiting for delivery recovery")))));
    assert!(s.engine.shared.scheduled_going.lock().unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail_recovery").unwrap();
    s.engine.send_due().await;
    assert_eq!(s.engine.delivery_state("recovering").unwrap(), Some(moss_store::DeliveryState::Uncertain));
    assert_eq!(s.fake.sent().len(), 1);
    assert_eq!(s.fake.sent()[0].subject, d.subject);
    assert!(s.engine.scheduled().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_failed_claim_read_preserves_the_unsent_candidate_and_releases_claims() {
    let (s, _) = with_thread(clock).await;
    let first = draft(None);
    let second = Draft { id: "d2".into(), subject: "Second schedule".into(), ..first.clone() };
    s.engine.schedule(&first, &outgoing(&first), NOW).unwrap();
    s.engine.schedule(&second, &outgoing(&second), NOW).unwrap();
    let first_operation = s.engine.shared.store().scheduled_operation(&first.id).unwrap().unwrap();
    s.fake.delay_sends(Duration::from_secs(5));
    let engine = s.engine.clone();
    let sending = tokio::spawn(async move { engine.send_due().await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("ALTER TABLE scheduled RENAME TO unavailable_scheduled").unwrap();
    sending.await.unwrap();
    assert_eq!(s.fake.sent().len(), 1);
    assert!(s.engine.shared.scheduled_going.lock().unwrap().is_empty());
    assert!(!s.events.all().iter().any(|e| matches!(e, Event::ScheduledFailed { email } if email.draft.id == second.id)));
    db.execute_batch("ALTER TABLE unavailable_scheduled RENAME TO scheduled").unwrap();
    assert_eq!(s.engine.scheduled().unwrap().iter().find(|e| e.draft.id == second.id).unwrap().tries, 0);
    s.engine.send_due().await;
    assert_eq!(s.fake.sent().iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), [first.subject.as_str(), second.subject.as_str()], "repair never resubmits the accepted first email");
    assert_eq!(s.engine.delivery_state(&first_operation).unwrap(), Some(moss_store::DeliveryState::Complete));
    let outcome: String = db.query_row("SELECT outcome FROM schedule_outcomes WHERE id = ?1", [&first_operation], |row| row.get(0)).unwrap();
    assert_eq!(outcome, "completed");
    // Events may repeat during local repair. Consumers deduplicate by operation ID;
    // the durable delivery, rather than event counts, is the send receipt.
    assert_eq!(sent_events(&s).iter().filter(|(id, _)| id == &first.id).count(), 2);
    assert!(s.engine.scheduled().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn scheduled_handoffs_require_the_due_operation_and_its_exact_content() {
    for mismatch in ["operation", "draft", "outgoing", "not-due"] {
        let (s, _) = with_thread(clock).await;
        let mut d = draft(None);
        let mut outgoing = outgoing(&d);
        s.engine.schedule_with_operation("scheduled-owner", &d, &outgoing, if mismatch == "not-due" { NOW + HOUR } else { NOW }).unwrap();
        let operation = if mismatch == "operation" { "foreign-operation" } else { "scheduled-owner" };
        if mismatch == "draft" { d.subject = "Stale subject".into(); }
        if mismatch == "outgoing" { outgoing.body = "Different outgoing body".into(); }
        assert!(s.engine.prepare_scheduled_send(operation, &d, &outgoing).await.is_err(), "{mismatch}");
        assert_eq!(s.engine.scheduled().unwrap().len(), 1);
        assert!(s.engine.shared.store().delivery(operation).unwrap().is_none());
        assert!(s.fake.sent().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn schedule_cancellation_distinguishes_early_replay_from_a_missing_restored_draft() {
    let (s, _) = with_thread(clock).await;
    let d = draft(None);
    for _ in 0..2 {
        let cancelled = s.engine.cancel_schedule_with_operation("never-prepared", &d.id).unwrap();
        assert_eq!(serde_json::to_value(cancelled).unwrap()["early"], true);
    }
    let cancelled = s.engine.schedule_with_operation("never-prepared", &d, &outgoing(&d), NOW + HOUR).unwrap();
    assert_eq!(serde_json::to_value(cancelled).unwrap()["early"], true);
    assert!(s.engine.drafts().unwrap().is_empty());
    let d = s.engine.save_draft(&d).unwrap();
    s.engine.schedule_with_operation("prepared", &d, &outgoing(&d), NOW + HOUR).unwrap();
    s.engine.cancel_schedule_with_operation("prepared", &d.id).unwrap();
    let mut current = s.engine.editable_draft(&d.id).unwrap().unwrap();
    current.body = "Edited after restoration".into();
    let current = s.engine.save_draft(&current).unwrap();
    let replay = serde_json::to_value(s.engine.cancel_schedule_with_operation("prepared", &d.id).unwrap()).unwrap();
    assert_eq!(replay["restored"], serde_json::to_value(&current).unwrap());
    assert_ne!(replay["early"], true);
    s.engine.delete_editable_draft(&d.id, current.revision.as_deref(), false).unwrap();
    let gone = serde_json::to_value(s.engine.cancel_schedule_with_operation("prepared", &d.id).unwrap()).unwrap();
    assert!(gone["restored"].is_null());
    assert_ne!(gone["early"], true);
}

static RECOVERY_CLOCK_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn recovery_loop_clock() -> i64 {
    RECOVERY_CLOCK_READS.fetch_add(1, Ordering::SeqCst);
    NOW
}

#[tokio::test(start_paused = true)]
async fn recovery_wait_backs_off_the_running_scheduler_and_announces_once() {
    let (s, _) = with_thread(recovery_loop_clock).await;
    let due = draft(None);
    s.engine.schedule(&due, &outgoing(&due), NOW).unwrap();
    let interrupted = Draft { id: "interrupted-loop".into(), ..due.clone() };
    {
        let mut store = s.engine.shared.store();
        store.prepare_delivery("interrupted-loop", &interrupted, "{}", "loop@example.test", NOW).unwrap();
        store.claim_delivery("interrupted-loop").unwrap();
    }
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_loop_recovery BEFORE UPDATE OF state ON deliveries WHEN NEW.state='uncertain' BEGIN SELECT RAISE(IGNORE); END").unwrap();
    *s.engine.shared.delivery_recovered.lock().unwrap() = false;
    s.events.clear();
    RECOVERY_CLOCK_READS.store(0, Ordering::SeqCst);
    s.engine.send_later();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(10)).await;
    for _ in 0..5 { tokio::task::yield_now().await; }
    tokio::time::advance(Duration::from_secs(30)).await;
    for _ in 0..5 { tokio::task::yield_now().await; }
    assert!(RECOVERY_CLOCK_READS.load(Ordering::SeqCst) < 80, "recovery must back off even though the email remains overdue");
    assert_eq!(s.events.all().iter().filter(|event| matches!(event, Event::ScheduledWaiting {email} if email.draft.id==due.id)).count(), 1);
    assert!(s.fake.sent().is_empty());
    assert_eq!(s.engine.scheduled().unwrap()[0].tries, 0);
    db.execute_batch("DROP TRIGGER refuse_loop_recovery").unwrap();
    tokio::time::advance(Duration::from_secs(30)).await;
    for _ in 0..10 { tokio::task::yield_now().await; }
    assert_eq!(s.fake.sent().len(),1);
    assert!(s.engine.scheduled().unwrap().is_empty());
}

static SLOW_PASS_TIME: AtomicI64 = AtomicI64::new(NOW);
fn slow_pass_clock() -> i64 { SLOW_PASS_TIME.load(Ordering::SeqCst) }

#[tokio::test(start_paused = true)]
async fn a_schedule_becoming_due_during_a_slow_send_runs_without_an_extra_poll_delay() {
    SLOW_PASS_TIME.store(NOW,Ordering::SeqCst);
    let (s,_) = with_thread(slow_pass_clock).await;
    s.engine.send_later();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(10)).await;
    for _ in 0..5 {tokio::task::yield_now().await;}
    s.fake.delay_sends(Duration::from_secs(3));
    let first=draft(None);
    let second=Draft {id:"second-during-pass".into(),subject:"Second scheduled email".into(),..first.clone()};
    s.engine.schedule(&first,&outgoing(&first),NOW+1_000).unwrap();
    s.engine.schedule(&second,&outgoing(&second),NOW+2_000).unwrap();
    // Let the worker consume both creation wakes while neither email is due.
    for _ in 0..10 {tokio::task::yield_now().await;}
    SLOW_PASS_TIME.store(NOW+1_000,Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..100 {
        if s.engine.shared.scheduled_going.lock().unwrap().contains(&first.id) {break;}
        tokio::task::yield_now().await;
    }
    assert!(s.engine.shared.scheduled_going.lock().unwrap().contains(&first.id));
    SLOW_PASS_TIME.store(NOW+4_000,Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(3)).await;
    for _ in 0..10 {tokio::task::yield_now().await;}
    SLOW_PASS_TIME.store(NOW+7_000,Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(3)).await;
    for _ in 0..10 {tokio::task::yield_now().await;}
    assert_eq!(s.fake.sent().len(),2,"a newly due email must not wait for the recovery backoff");
    assert!(s.engine.scheduled().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_schedule_during_ownership_recovery_never_attempts_a_send() {
    let (s,_) = with_thread(clock).await;
    let due=draft(None);
    s.engine.schedule(&due,&outgoing(&due),NOW).unwrap();
    let blocked=Draft {id:"blocked-unreadable-list".into(),..due.clone()};
    {
        let mut store=s.engine.shared.store();
        store.prepare_delivery("blocked-unreadable-list",&blocked,"{}","blocked@example.test",NOW).unwrap();
        store.claim_delivery("blocked-unreadable-list").unwrap();
    }
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_recovery BEFORE UPDATE OF state ON deliveries WHEN NEW.state='uncertain' BEGIN SELECT RAISE(IGNORE); END; ALTER TABLE scheduled RENAME TO unreadable_scheduled").unwrap();
    *s.engine.shared.delivery_recovered.lock().unwrap()=false;
    assert_eq!(s.engine.send_due().await,None);
    assert!(s.fake.sent().is_empty());
    db.execute_batch("ALTER TABLE unreadable_scheduled RENAME TO scheduled; DROP TRIGGER refuse_recovery").unwrap();
    assert_eq!(s.engine.send_due().await,Some(NOW));
    assert_eq!(s.fake.sent().len(),1);
    assert!(s.engine.scheduled().unwrap().is_empty());
}
