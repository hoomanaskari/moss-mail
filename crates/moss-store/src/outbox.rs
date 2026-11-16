//! What waits to go out, kept here so a restart or a dropped connection can't lose it:
//! - Changes waiting to reach the server (archive, seen, back to the Inbox). The engine
//!   decides what a change is; the store keeps its JSON and the order it was made in.
//! - Emails set to send later. Each keeps its draft, to open again, and the email the engine
//!   sends (its JSON, the engine's to read). Scheduling one takes it out of Drafts;
//!   cancelling puts it back.

use moss_core::{Draft, ScheduledEmail};
use rusqlite::{params, OptionalExtension, Row};

use crate::codec::json;
use crate::{Result, Store};

const RETRIABLE: &str = "NOT EXISTS (SELECT 1 FROM deliveries WHERE id = scheduled.operation_id AND (resolution IS NOT NULL OR state IN ('claimed', 'uncertain', 'cancelled')))";

const SCHEDULED: &str = "draft, send_at, next_try, tries, error, operation_id, revision";

fn scheduled(r: &Row) -> rusqlite::Result<ScheduledEmail> {
    Ok(ScheduledEmail {
        operation_id: r.get(5)?,
        revision: r.get(6)?,
        draft: json(r, 0)?,
        send_at: r.get(1)?,
        next_try: r.get(2)?,
        tries: r.get(3)?,
        error: r.get(4)?,
    })
}

/// A persistent identity for one queued action. This is a concurrency guard,
/// not an authentication credential; SQLite row numbers are never receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeId(pub(crate) [u8; 16]);

impl ChangeId {
    /// As the app hands it to the UI and back: 32 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// `None` for anything that isn't exactly what [`ChangeId::to_hex`] makes.
    pub fn from_hex(text: &str) -> Option<Self> {
        let digits = text.as_bytes();
        if digits.len() != 32 || !digits.iter().all(|d| d.is_ascii_digit() || (b'a'..=b'f').contains(d)) { return None; }
        let mut bytes = [0u8; 16];
        for (byte, pair) in bytes.iter_mut().zip(digits.chunks(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(ChangeId(bytes))
    }
}

impl Store {
    /// Saves an immutable change for later and returns its persistent identity.
    pub fn queue_change(&self, account_id: &str, change: &str, now: i64) -> Result<ChangeId> {
        Ok(self.conn.query_row(
            "INSERT INTO outbox (account_id, change, created_at) VALUES (?1, ?2, ?3) RETURNING token",
            params![account_id, change, now],
            |row| row.get(0).map(ChangeId),
        )?)
    }

    /// An account's waiting changes, oldest first, as `(identity, change)`.
    pub fn pending_changes(&self, account_id: &str) -> Result<Vec<(ChangeId, String)>> {
        let mut stmt = self.conn.prepare("SELECT token, change FROM outbox WHERE account_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([account_id], |row| Ok((ChangeId(row.get(0)?), row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Removes only an unclaimed action. Claimed and blocked intent require revision checks.
    /// A stale identity is harmless and returns false.
    /// Ignored removal of an existing row is an error, not a successful completion.
    pub fn drop_change(&mut self, id: ChangeId) -> Result<bool> {
        self.finish_unclaimed_change(id, crate::UnclaimedOutcome::Dropped)
    }

    /// Creates a fresh schedule operation. Existing schedules must be explicitly rescheduled.
    pub fn schedule_email(&mut self, draft: &Draft, outgoing: &str, send_at: i64, now: i64) -> Result<()> {
        let operation: String = self.conn.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.schedule_operation(&operation, draft, outgoing, send_at, now)?;
        Ok(())
    }

    /// Emails set to send later, the soonest first.
    pub fn scheduled_emails(&self) -> Result<Vec<ScheduledEmail>> {
        let sql = format!("SELECT {SCHEDULED} FROM scheduled ORDER BY send_at, draft_id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], scheduled)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    #[cfg(test)]
    pub fn scheduled_email(&self, id: &str) -> Result<Option<ScheduledEmail>> {
        let sql = format!("SELECT {SCHEDULED} FROM scheduled WHERE draft_id = ?1");
        Ok(self.conn.query_row(&sql, [id], scheduled).optional()?)
    }

    pub(crate) fn scheduled_email_for_operation(&self, operation: &str) -> Result<Option<ScheduledEmail>> {
        let sql = format!("SELECT {SCHEDULED} FROM scheduled WHERE operation_id = ?1");
        Ok(self.conn.query_row(&sql, [operation], scheduled).optional()?)
    }

    pub fn scheduled_operation(&self, draft_id: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT operation_id FROM scheduled WHERE draft_id = ?1", [draft_id], |row| row.get(0)).optional()?)
    }

    pub fn is_scheduled_delivery(&self, operation: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM scheduled WHERE operation_id = ?1)", [operation], |row| row.get(0))?)
    }

    /// Emails due to be tried by `now`, longest waiting first, with what the engine sends.
    pub fn due_emails(&self, now: i64) -> Result<Vec<(ScheduledEmail, String)>> {
        let sql = format!(
            "SELECT {SCHEDULED}, outgoing FROM scheduled WHERE next_try <= ?1 AND {RETRIABLE} ORDER BY next_try, draft_id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([now], |r| Ok((scheduled(r)?, r.get(7)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Rechecks one snapshot candidate just before claiming it, without rereading the queue.
    pub fn due_email(&self, id: &str, now: i64) -> Result<Option<(ScheduledEmail, String)>> {
        let sql = format!("SELECT {SCHEDULED}, outgoing FROM scheduled WHERE draft_id = ?1 AND next_try <= ?2 AND {RETRIABLE}");
        Ok(self.conn.query_row(&sql, params![id, now], |r| Ok((scheduled(r)?, r.get(7)?))).optional()?)
    }

    /// When the next email is due to be tried (ms), if any is waiting.
    pub fn next_due(&self) -> Result<Option<i64>> {
        Ok(self.conn.query_row(&format!("SELECT MIN(next_try) FROM scheduled WHERE {RETRIABLE}"), [], |r| r.get(0))?)
    }

    /// A new time for a scheduled email, tried afresh. False when it isn't scheduled.
    pub fn reschedule_email(&mut self, id: &str, send_at: i64) -> Result<bool> {
        self.delivery_transaction(|store| {
        let exhausted: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM scheduled WHERE draft_id = ?1 AND revision = 9007199254740991)", [id], |row| row.get(0))?;
        if exhausted { return Err(crate::Error::Invalid("This schedule cannot be changed again. Cancel it and create a new schedule.".into())); }
        let changed = store.conn.execute(
            "UPDATE scheduled SET revision = revision + 1, send_at = ?2, next_try = ?2, tries = 0, error = NULL WHERE draft_id = ?1 AND NOT EXISTS (SELECT 1 FROM deliveries WHERE id = scheduled.operation_id AND state != 'held')",
            params![id, send_at],
        )?;
        Ok(changed > 0)
        })
    }

    /// A try failed: says why, and when to try again.
    /// A recovery wait is not a provider send attempt. Preserve counters and deadlines.
    pub fn note_due_email_wait(&mut self,now:i64,reason:&str)->Result<()> {
        self.conn.execute(&format!("UPDATE scheduled SET error=?2 WHERE next_try<=?1 AND {RETRIABLE} AND error IS NOT ?2"),params![now,reason])?;
        Ok(())
    }

    pub fn email_failed(&mut self, id: &str, error: &str, next_try: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE scheduled SET tries = tries + 1, error = ?2, next_try = ?3 WHERE draft_id = ?1",
            params![id, error, next_try],
        )?;
        Ok(())
    }

    /// The email went: forgets it. False when it wasn't scheduled.
    pub fn email_sent(&mut self, id: &str, now: i64) -> Result<bool> {
        self.delivery_transaction(|store| {
            store.conn.execute("INSERT OR IGNORE INTO schedule_outcomes (id, draft_id, outcome, at) SELECT operation_id, draft_id, 'completed', ?2 FROM scheduled WHERE draft_id = ?1", params![id, now])?;
            Ok(store.conn.execute("DELETE FROM scheduled WHERE draft_id = ?1", [id])? > 0)
        })
    }

    /// Won't send it: the email goes back to Drafts as it was, edited `now`, and is returned.
    /// A reply whose conversation is gone comes back without it. `None` when it wasn't
    /// scheduled (sent meanwhile, or cancelled already).
    pub fn unschedule_email(&mut self, id: &str, now: i64) -> Result<Option<Draft>> {
        self.delivery_transaction(|store| store.unschedule_email_in_transaction(id, now))
    }

    pub(crate) fn unschedule_email_in_transaction(&self, id: &str, now: i64) -> Result<Option<Draft>> {
            if self.conn.is_autocommit() { return Err(crate::Error::Invalid("Unscheduling requires an owning transaction.".into())); }
            let store = self;
            // Read the email and its operation together. Once this row exists in
            // the transaction, a second optional lookup cannot legitimately disappear.
            let sql = format!("SELECT {SCHEDULED} FROM scheduled WHERE draft_id = ?1");
            let Some((email, operation)) = store.conn.query_row(&sql, [id], |row| {
                Ok((scheduled(row)?, row.get::<_, String>(5)?))
            }).optional()? else { return Ok(None); };
            match store.delivery_state(&operation)? {
                Some(crate::DeliveryState::Held) => {
                    store.conn.execute("UPDATE deliveries SET state = 'cancelled' WHERE id = ?1 AND state = 'held'", [&operation])?;
                }
                None | Some(crate::DeliveryState::Cancelled) => {}
                _ => return Err(crate::Error::Invalid("This email has a delivery attempt. Check its status before sending or editing it.".into())),
            }
            store.conn.execute("INSERT OR IGNORE INTO delivery_cancellations (id) VALUES (?1)", [&operation])?;
            store.conn.execute("INSERT OR IGNORE INTO schedule_outcomes (id, draft_id, outcome, at) VALUES (?1, ?2, 'cancelled', ?3)", params![operation, id, now])?;
            let recorded: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE id = ?1)", [&operation], |r| r.get(0))?;
            let bound: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM schedule_outcomes WHERE id = ?1 AND draft_id = ?2 AND outcome = 'cancelled')", params![operation, id], |r| r.get(0))?;
            if !recorded || !bound { return Err(crate::Error::Invalid("The schedule cancellation was not saved consistently.".into())); }
            let mut draft = Draft { updated_at: now, ..email.draft };
            if let Some(thread) = &draft.thread_id {
                let found: Option<i64> = store.conn.query_row("SELECT id FROM threads WHERE id = ?1", [thread], |r| r.get(0)).optional()?;
                if found.is_none() { draft.thread_id = None; }
            }
            store.conn.execute("DELETE FROM scheduled WHERE draft_id = ?1", [id])?;
            store.save_draft_in_transaction(&draft)?;
            Ok(Some(draft))
    }

}

#[cfg(test)]
mod tests {
    use crate::test_support::*;
    use crate::Store;
    use moss_core::{ComposeMode, ScheduledEmail};

    const HOUR: i64 = 3_600_000;

    #[test]
    fn a_change_identity_reads_back_from_its_text() {
        let store = store_with_work();
        let id = store.queue_change("work", "seen", 1).unwrap();
        let text = id.to_hex();
        assert_eq!(text.len(), 32);
        assert!(text.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(crate::ChangeId::from_hex(&text), Some(id));
        for bad in [String::new(), "zz".into(), text[..30].into(), format!("{text}00"), "g".repeat(32), text.to_uppercase(), "é".repeat(16)] {
            assert_eq!(crate::ChangeId::from_hex(&bad), None, "{bad}");
        }
    }

    #[test]
    fn recovery_wait_reasons_preserve_attempts_and_only_change_due_eligible_mail() {
        let mut store=store_with_work();
        for (id,at) in [("due",NOV_1),("future",NOV_1+DAY),("claimed",NOV_1)] {
            let d=draft(id);
            store.schedule_email(&d,"{}",at,NOV_1).unwrap();
            if id=="claimed" {
                let operation=store.scheduled_operation(id).unwrap().unwrap();
                store.prepare_delivery(&operation,&d,"{}","claimed@example.test",NOV_1).unwrap();
                store.claim_delivery(&operation).unwrap();
            }
        }
        store.note_due_email_wait(NOV_1,"Waiting for recovery").unwrap();
        for email in store.scheduled_emails().unwrap() {
            assert_eq!(email.tries,0);
            assert_eq!(email.next_try,email.send_at);
            assert_eq!(email.error,if email.draft.id=="due" {Some("Waiting for recovery".into())} else {None});
        }
        store.conn.execute_batch("CREATE TRIGGER refuse_wait BEFORE UPDATE OF error ON scheduled BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
        store.note_due_email_wait(NOV_1,"Waiting for recovery").unwrap();
        assert!(store.note_due_email_wait(NOV_1,"A changed reason").is_err());
        store.conn.execute_batch("DROP TRIGGER refuse_wait; DROP TABLE scheduled").unwrap();
        assert!(store.note_due_email_wait(NOV_1,"Waiting").is_err());
    }

    #[test]
    fn unscheduling_requires_both_retained_cancellation_records() {
        for prepared in [false, true] {
            for table in ["delivery_cancellations", "schedule_outcomes"] {
                for action in ["IGNORE", "ABORT, 'unschedule failed'"] {
                    let mut store = store_with_work();
                    let d = draft("scheduled");
                    store.schedule_operation("operation", &d, "{}", NOV_1, NOV_1).unwrap();
                    if prepared { store.prepare_delivery("operation", &d, "{}", "mail@example.test", NOV_1).unwrap(); }
                    store.conn.execute_batch(&format!("CREATE TRIGGER fail_unschedule BEFORE INSERT ON {table} BEGIN SELECT RAISE({action}); END;")).unwrap();
                    assert!(store.unschedule_email(&d.id, NOV_1).is_err(), "{prepared}: {table}: {action}");
                    assert!(store.drafts(None).unwrap().is_empty());
                    assert_eq!(store.scheduled_emails().unwrap().len(), 1);
                    assert_eq!(store.delivery_state("operation").unwrap(), prepared.then_some(crate::DeliveryState::Held));
                    assert_eq!(store.conn.query_row("SELECT count(*) FROM delivery_cancellations", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
                    assert_eq!(store.conn.query_row("SELECT count(*) FROM schedule_outcomes", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
                    store.conn.execute_batch("DROP TRIGGER fail_unschedule").unwrap();
                    assert!(store.unschedule_email(&d.id, NOV_1).unwrap().is_some());
                    assert_eq!(store.delivery_state("operation").unwrap(), Some(crate::DeliveryState::Cancelled));
                    assert!(!store.prepare_delivery("operation", &d, "{}", "mail@example.test", NOV_1).unwrap());
                }
            }
        }
    }

    #[test]
    fn unscheduling_refuses_a_conflicting_terminal_identity() {
        for (owner, outcome) in [("scheduled", "completed"), ("another-draft", "cancelled")] {
            let mut store = store_with_work();
            let d = draft("scheduled");
            store.schedule_operation("operation", &d, "{}", NOV_1, NOV_1).unwrap();
            store.conn.execute("INSERT INTO schedule_outcomes (id, draft_id, outcome, at) VALUES ('operation', ?1, ?2, 123)", rusqlite::params![owner, outcome]).unwrap();
            assert!(store.unschedule_email(&d.id, NOV_1).is_err());
            assert!(store.drafts(None).unwrap().is_empty());
            assert_eq!(store.scheduled_emails().unwrap().len(), 1);
            assert_eq!(store.delivery_state("operation").unwrap(), None);
            assert_eq!(store.conn.query_row("SELECT draft_id, outcome, at FROM schedule_outcomes", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))).unwrap(), (owner.into(), outcome.into(), 123));
        }
    }

    fn waiting(store: &crate::Store, draft: moss_core::Draft, send_at: i64) -> ScheduledEmail {
        ScheduledEmail { operation_id: store.scheduled_operation(&draft.id).unwrap().unwrap(), revision: store.scheduled_email(&draft.id).unwrap().unwrap().revision, draft, send_at, next_try: send_at, tries: 0, error: None }
    }

    #[test]
    fn scheduling_commits_with_full_durability() {
        let mut store = store_with_work();
        store.conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
        store.conn.execute_batch("CREATE TABLE observed_sync (value INTEGER); CREATE TRIGGER observe_schedule BEFORE INSERT ON scheduled BEGIN INSERT INTO observed_sync SELECT synchronous FROM pragma_synchronous; END;").unwrap();
        store.schedule_email(&draft("d1"), "{}", NOV_1, NOV_1).unwrap();
        assert_eq!(store.conn.query_row("SELECT value FROM observed_sync", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(store.conn.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn rescheduling_commits_with_full_durability() {
        let mut store = store_with_work();
        store.schedule_email(&draft("d1"), "{}", NOV_1, NOV_1).unwrap();
        store.conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
        store.conn.execute_batch("CREATE TABLE observed_sync (value INTEGER); CREATE TRIGGER observe_reschedule BEFORE UPDATE ON scheduled BEGIN INSERT INTO observed_sync SELECT synchronous FROM pragma_synchronous; END;").unwrap();
        assert!(store.reschedule_email("d1", NOV_1 + HOUR).unwrap());
        assert_eq!(store.conn.query_row("SELECT value FROM observed_sync", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(store.conn.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn scheduled_attempts_share_one_durable_operation_and_uncertainty_blocks_retry() {
        let mut store = store_with_work();
        let d = draft("d1");
        store.schedule_email(&d, "{}", NOV_1, NOV_1).unwrap();
        let operation = store.scheduled_operation(&d.id).unwrap().unwrap();
        assert!(store.is_scheduled_delivery(&operation).unwrap());
        store.prepare_delivery(&operation, &d, "{}", "message", NOV_1).unwrap();
        store.claim_delivery(&operation).unwrap();
        assert!(store.due_emails(NOV_1 + HOUR).unwrap().is_empty());
        assert_eq!(store.next_due().unwrap(), None);
        assert!(store.unschedule_email(&d.id, NOV_1).is_err());
        assert!(!store.reschedule_email(&d.id, NOV_1).unwrap());
        store.uncertain_delivery(&operation, "lost reply").unwrap();
        assert!(store.due_emails(NOV_1 + HOUR).unwrap().is_empty());
        assert!(store.due_email(&d.id, NOV_1 + HOUR).unwrap().is_none());
        assert_eq!(store.scheduled_operation(&d.id).unwrap().as_deref(), Some(operation.as_str()));
        assert_eq!(store.scheduled_emails().unwrap().len(), 1, "uncertain mail stays visible");
    }

    #[test]
    fn scheduling_takes_a_draft_out_of_drafts_until_it_is_sent() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya@fastmail.com")).unwrap();
        let lunch = draft("d1");
        store.save_draft(&lunch).unwrap();
        let later = moss_core::Draft { from_account_id: "personal".into(), ..draft("d2") };
        store.schedule_email(&lunch, r#"{"to":"daniel"}"#, NOV_1 + 2 * HOUR, NOV_1).unwrap();
        store.schedule_email(&later, "{}", NOV_1 + HOUR, NOV_1).unwrap();
        assert!(store.drafts(None).unwrap().is_empty(), "no longer a draft");
        assert_eq!(
            store.scheduled_emails().unwrap(),
            [waiting(&store, later.clone(), NOV_1 + HOUR), waiting(&store, lunch.clone(), NOV_1 + 2 * HOUR)],
            "the soonest first"
        );
        assert_eq!(store.scheduled_email("d1").unwrap(), Some(waiting(&store, lunch.clone(), NOV_1 + 2 * HOUR)));
        assert_eq!(store.scheduled_email("nope").unwrap(), None);
        assert_eq!(store.next_due().unwrap(), Some(NOV_1 + HOUR));

        assert!(store.due_emails(NOV_1).unwrap().is_empty());
        let due = store.due_emails(NOV_1 + 2 * HOUR).unwrap();
        assert_eq!(due[0].0.draft.id, "d2");
        assert_eq!(due[1], (waiting(&store, lunch, NOV_1 + 2 * HOUR), r#"{"to":"daniel"}"#.into()));

        assert!(store.email_sent("d2", NOV_1).unwrap());
        assert!(!store.email_sent("d2", NOV_1).unwrap());
        assert!(store.email_sent("d1", NOV_1).unwrap());
        assert_eq!(store.next_due().unwrap(), None);
    }

    #[test]
    fn a_failed_try_waits_says_why_and_a_new_time_starts_afresh() {
        let mut store = store_with_work();
        store.schedule_email(&draft("d1"), "{}", NOV_1, NOV_1).unwrap();
        store.email_failed("d1", "can't reach the server", NOV_1 + 60_000).unwrap();
        store.email_failed("d1", "still offline", NOV_1 + 120_000).unwrap();
        let failed = store.scheduled_email("d1").unwrap().unwrap();
        assert_eq!((failed.tries, failed.error.as_deref()), (2, Some("still offline")));
        assert_eq!((failed.send_at, failed.next_try), (NOV_1, NOV_1 + 120_000));
        assert!(store.due_emails(NOV_1 + 60_000).unwrap().is_empty(), "waits before trying again");

        assert!(store.reschedule_email("d1", NOV_1 + HOUR).unwrap());
        assert_eq!(store.scheduled_email("d1").unwrap(), Some(waiting(&store, draft("d1"), NOV_1 + HOUR)));
        assert!(!store.reschedule_email("nope", NOV_1).unwrap());

        // Creating another operation must not replace an existing schedule.
        assert!(store.schedule_email(&draft("d1"), "{}", NOV_1 + 3 * HOUR, NOV_1).is_err());
        assert_eq!(store.scheduled_emails().unwrap(), [waiting(&store, draft("d1"), NOV_1 + HOUR)]);
    }

    #[test]
    fn unscheduled_operations_stay_cancelled_after_account_removal() {
        for prepared in [false, true] {
            let mut store = store_with_work();
            let d = draft("unscheduled-tombstone");
            store.schedule_email(&d, "{}", NOV_1, NOV_1).unwrap();
            let operation = store.scheduled_operation(&d.id).unwrap().unwrap();
            if prepared { store.prepare_delivery(&operation, &d, "{}", "scheduled@fixture", NOV_1).unwrap(); }
            store.unschedule_email(&d.id, NOV_1).unwrap();
            assert!(store.remove_account("work").unwrap());
            if prepared { assert!(store.delivery(&operation).unwrap().is_some()); }
            store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
            assert!(!store.prepare_delivery(&operation, &d, "{}", "late@fixture", NOV_1).unwrap());
            assert_eq!(store.delivery_state(&operation).unwrap(), Some(crate::DeliveryState::Cancelled));
        }
    }

    #[test]
    fn unscheduling_cancels_a_prepared_hold_before_restoring_its_draft() {
        let mut store = store_with_work();
        let saved = store.attach_and_save_draft(&draft("held-schedule"), &[("saved.txt", "text/plain", b"saved")]).unwrap();
        store.schedule_email(&saved, "{}", NOV_1, NOV_1).unwrap();
        let operation = store.scheduled_operation(&saved.id).unwrap().unwrap();
        store.prepare_delivery(&operation, &saved, "{}", "held@fixture", NOV_1).unwrap();
        let staged = store.add_draft_file(&saved.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
        let restored = store.unschedule_email(&saved.id, NOV_1 + 1).unwrap().unwrap();
        assert_eq!(restored.attachments, saved.attachments);
        assert_eq!(store.draft(&saved.id).unwrap(), Some(restored));
        assert_eq!(store.delivery_state(&operation).unwrap(), Some(crate::DeliveryState::Cancelled));
        assert!(!store.claim_delivery(&operation).unwrap());
        assert!(store.scheduled_email(&saved.id).unwrap().is_none());
        assert_eq!(store.draft_file(&staged.id).unwrap().unwrap().2, b"staged");
    }

    #[test]
    fn cancelling_puts_the_email_back_in_drafts() {
        let mut store = store_with_work();
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        let reply = moss_core::Draft { mode: ComposeMode::Reply, thread_id: Some(thread), ..draft("d1") };
        store.schedule_email(&reply, "{}", NOV_1 + HOUR, NOV_1).unwrap();
        let back = store.unschedule_email("d1", NOV_1 + 5).unwrap();
        let expected = moss_core::Draft { updated_at: NOV_1 + 5, ..reply };
        assert_eq!(back, Some(expected.clone()));
        assert_eq!(store.drafts(None).unwrap(), [expected]);
        assert!(store.scheduled_emails().unwrap().is_empty());
        assert_eq!(store.unschedule_email("d1", NOV_1).unwrap(), None, "not scheduled any more");
        // A new email has no conversation to look for.
        store.schedule_email(&draft("d2"), "{}", NOV_1, NOV_1).unwrap();
        assert_eq!(store.unschedule_email("d2", NOV_1).unwrap().unwrap().thread_id, None);
    }

    #[test]
    fn a_reply_whose_conversation_is_gone_comes_back_as_a_draft_without_it() {
        let mut store = store_with_work();
        let reply = moss_core::Draft { mode: ComposeMode::Reply, thread_id: Some("999".into()), ..draft("d1") };
        store.schedule_email(&reply, "{}", NOV_1, NOV_1).unwrap();
        let back = store.unschedule_email("d1", NOV_1).unwrap().unwrap();
        assert_eq!(back.thread_id, None);
        assert_eq!(store.drafts(None).unwrap(), [back]);
    }

    #[test]
    fn removing_an_account_refuses_to_forget_its_scheduled_emails() {
        let mut store = store_with_work();
        store.schedule_email(&draft("d1"), "{}", NOV_1, NOV_1).unwrap();
        let before = store.scheduled_emails().unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.scheduled_emails().unwrap(), before);
    }

    #[test]
    fn scheduling_failures_are_errors() {
        let mut store = store_with_work();
        let nobody = moss_core::Draft { from_account_id: "nobody".into(), ..draft("d1") };
        assert!(store.schedule_email(&nobody, "{}", NOV_1, NOV_1).is_err(), "only for a known account");
        store.schedule_email(&draft("d2"), "{}", NOV_1, NOV_1).unwrap();
        break_schema(&store, "UPDATE scheduled SET draft = 'not json'");
        assert!(store.scheduled_emails().is_err());
        assert!(store.unschedule_email("d2", NOV_1).is_err());
        break_schema(&store, "DROP TABLE drafts");
        assert!(store.schedule_email(&draft("d3"), "{}", NOV_1, NOV_1).is_err());
        break_schema(&store, "DROP TABLE scheduled");
        assert!(store.schedule_email(&draft("d1"), "{}", NOV_1, NOV_1).is_err());
        assert!(store.scheduled_emails().is_err());
        assert!(store.scheduled_email("d1").is_err());
        assert!(store.due_emails(NOV_1).is_err());
        assert!(store.next_due().is_err());
        assert!(store.reschedule_email("d1", NOV_1).is_err());
        assert!(store.email_failed("d1", "no", NOV_1).is_err());
        assert!(store.email_sent("d1", NOV_1).is_err());
        assert!(store.unschedule_email("d1", NOV_1).is_err());
    }

    #[test]
    fn queue_identity_migration_preserves_existing_work_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            crate::migrate(&mut conn, &crate::schema::MIGRATIONS[..31]).unwrap();
            let mut old = Store { conn, delivery_session: None };
            old.upsert_account(&account("work", "maya@example.test")).unwrap();
            old.conn.execute_batch("INSERT INTO outbox(id,account_id,change,created_at) VALUES(7,'work','seen',10),(19,'work','archive',20);").unwrap();
        }
        let store = Store::open(&path).unwrap();
        let valid: i64 = store.conn.query_row("SELECT count(DISTINCT token) FROM outbox WHERE typeof(token)='blob' AND length(token)=16", [], |r| r.get(0)).unwrap();
        assert_eq!(valid, 2);
        let rows: Vec<(i64, String, i64)> = store.conn.prepare("SELECT id,change,created_at FROM outbox ORDER BY id").unwrap()
            .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(rows, [(7,"seen".into(),10),(19,"archive".into(),20)]);
        let pending = store.pending_changes("work").unwrap();
        assert_eq!(pending.iter().map(|(_,change)|change.as_str()).collect::<Vec<_>>(), ["seen","archive"]);
        drop(store);
        let mut reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.pending_changes("work").unwrap(), pending);
        reopened.drop_change(pending[0].0).unwrap();
        assert_eq!(reopened.pending_changes("work").unwrap(), pending[1..]);
    }

    #[test]
    fn stale_completion_cannot_remove_a_reused_queue_row() {
        for recreate_account in [false, true] {
            let mut store = store_with_work();
            let old = store.queue_change("work", "seen", 10).unwrap();
            let old_row: i64 = store.conn.query_row("SELECT id FROM outbox", [], |r| r.get(0)).unwrap();
            if recreate_account {
                assert!(store.remove_account("work").is_err());
                // Simulate a legacy cascade; stale receipts must still be harmless.
                store.conn.execute("DELETE FROM accounts WHERE id='work'", []).unwrap();
                store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
            } else {
                store.drop_change(old).unwrap();
            }
            let new = store.queue_change("work", "seen", 10).unwrap();
            let new_row: i64 = store.conn.query_row("SELECT id FROM outbox", [], |r| r.get(0)).unwrap();
            assert_eq!(old_row, new_row, "exercise actual SQLite row-number reuse");
            assert!(!store.drop_change(old).unwrap());
            assert_eq!(store.pending_changes("work").unwrap(), [(new, "seen".into())], "a delayed completion must preserve new intent");
            assert_ne!(old, new);
            assert!(store.drop_change(new).unwrap());
            assert!(store.pending_changes("work").unwrap().is_empty());
        }
    }

    #[test]
    fn queued_action_payloads_are_immutable() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya@example.test")).unwrap();
        let receipt = store.queue_change("work", "seen", 10).unwrap();
        for assignment in ["change='archive'", "account_id='personal'", "created_at=11", "id=id+1", "token=randomblob(16)"] {
            assert!(store.conn.execute(&format!("UPDATE outbox SET {assignment}"), []).is_err(), "{assignment}");
            assert_eq!(store.pending_changes("work").unwrap(), [(receipt, "seen".into())]);
        }
    }

    #[test]
    fn corrupted_queue_identities_are_errors_not_default_receipts() {
        for value in ["x'01'", "'synthetic-not-a-token'"] {
            let store = store_with_work();
            store.queue_change("work", "seen", 10).unwrap();
            store.conn.execute_batch(&format!("DROP TRIGGER outbox_payload_immutable; PRAGMA ignore_check_constraints=ON; UPDATE outbox SET token={value};")).unwrap();
            assert!(store.pending_changes("work").is_err());
        }
    }

    #[test]
    fn ignored_queue_removal_is_reported_and_preserves_intent() {
        let mut store = store_with_work();
        let receipt = store.queue_change("work", "seen", 10).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_queue_removal BEFORE DELETE ON outbox BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.drop_change(receipt).is_err());
        assert_eq!(store.pending_changes("work").unwrap(), [(receipt, "seen".into())]);
    }

    #[test]
    fn changes_wait_in_order_until_dropped() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya@fastmail.com")).unwrap();
        let archive = store.queue_change("work", r#"{"kind":"archive"}"#, 1).unwrap();
        let other = store.queue_change("personal", "seen", 2).unwrap();
        let back = store.queue_change("work", r#"{"kind":"unarchive"}"#, 3).unwrap();
        assert_eq!(
            store.pending_changes("work").unwrap(),
            [(archive, r#"{"kind":"archive"}"#.into()), (back, r#"{"kind":"unarchive"}"#.into())]
        );
        store.drop_change(archive).unwrap();
        assert_eq!(store.pending_changes("work").unwrap(), [(back, r#"{"kind":"unarchive"}"#.into())]);
        assert_eq!(store.pending_changes("personal").unwrap(), [(other, "seen".into())]);
    }

    #[test]
    fn removing_an_account_refuses_to_forget_its_changes() {
        let mut store = store_with_work();
        let id = store.queue_change("work", "seen", 1).unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.pending_changes("work").unwrap(), [(id,"seen".into())]);
    }

    #[test]
    fn failures_are_errors() {
        let mut store = store_with_work();
        assert!(store.queue_change("nobody", "seen", 1).is_err(), "only for a known account");
        let receipt = store.queue_change("work", "seen", 1).unwrap();
        break_schema(&store, "DROP TABLE outbox");
        assert!(store.queue_change("work", "seen", 1).is_err());
        assert!(store.pending_changes("work").is_err());
        assert!(store.drop_change(receipt).is_err());
    }
}
