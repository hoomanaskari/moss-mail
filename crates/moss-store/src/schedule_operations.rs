//! Stable identities for schedule creation, retries and cancellation.
use moss_core::{Draft, ScheduledEmail};
use serde::{Deserialize, Serialize};
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use crate::{codec::{named, to_json}, DeliveryState, Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ScheduleOutcome {
    Scheduled,
    AlreadyScheduled,
    Rescheduled { revision: i64 },
    Changed { current: ScheduledEmail },
    Sending,
    Cancelled { restored: Option<Draft>, #[serde(default, skip_serializing_if = "is_false")] early: bool },
    Completed,
    InDelivery { state: DeliveryState },
}

fn is_false(value: &bool) -> bool { !*value }

fn request_digest(draft: &Draft, outgoing: &str) -> String {
    let mut content = draft.clone();
    content.updated_at = 0;
    content.revision = None;
    format!("{:x}", Sha256::digest(to_json(&(content, outgoing)).as_bytes()))
}

fn binding(actual: &str, expected: &str) -> Result<()> {
    if actual != expected { return Err(Error::Invalid("This schedule operation belongs to another draft.".into())); }
    Ok(())
}

impl Store {
    /// A later ownership transfer permanently revokes earlier snapshot recovery.
    pub(crate) fn retire_early_schedule_recovery(&self, draft_id: &str) -> Result<()> {
        self.conn.execute("UPDATE schedule_outcomes SET never_prepared=0 WHERE draft_id=?1 AND never_prepared=1", [draft_id])?;
        let retained: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM schedule_outcomes WHERE draft_id=?1 AND never_prepared=1)", [draft_id], |row| row.get(0))?;
        if retained { return Err(Error::Invalid("Earlier cancellation recovery could not be retired. No changes were saved.".into())); }
        Ok(())
    }

    fn draft_has_outgoing_owner(&self, draft_id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM scheduled WHERE draft_id=?1) OR EXISTS(SELECT 1 FROM deliveries WHERE draft_id=?1 AND state!='cancelled')",
            [draft_id], |row| row.get(0),
        )?)
    }

    fn schedule_outcome(&self, operation: &str, draft_id: &str) -> Result<Option<ScheduleOutcome>> {
        let ended: Option<(String, String, bool)> = self.conn.query_row(
            "SELECT draft_id, outcome, never_prepared FROM schedule_outcomes WHERE id = ?1", [operation],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        let Some((owner, outcome, early)) = ended else { return Ok(None); };
        binding(&owner, draft_id)?;
        match outcome.as_str() {
            "cancelled" => Ok(Some(ScheduleOutcome::Cancelled {
                early: early && !self.draft_has_outgoing_owner(draft_id)?,
                restored: if early { None } else { self.editable_draft(draft_id)? },
            })),
            "completed" => Ok(Some(ScheduleOutcome::Completed)),
            _ => Err(Error::Invalid("The schedule outcome cannot be read.".into())),
        }
    }

    fn schedule_delivery(&self, operation: &str, draft_id: &str) -> Result<Option<DeliveryState>> {
        let delivery: Option<(String, DeliveryState)> = self.conn.query_row(
            "SELECT draft_id, state FROM deliveries WHERE id = ?1", [operation],
            |row| Ok((row.get(0)?, named(row, 1)?)),
        ).optional()?;
        let Some((owner, state)) = delivery else { return Ok(None); };
        binding(&owner, draft_id)?;
        Ok(Some(state))
    }

    pub fn schedule_operation(&mut self, operation: &str, draft: &Draft, outgoing: &str, send_at: i64, now: i64) -> Result<ScheduleOutcome> {
        self.schedule_operation_with_change(operation, draft, outgoing, send_at, now, |_| Ok(None))
    }

    /// Read existing identity before validating a new request. Creation rechecks this
    /// inside its transaction, so this lookup alone never authorizes ownership transfer.
    pub fn existing_schedule_outcome(&self, operation: &str, draft: &Draft, outgoing: &str) -> Result<Option<ScheduleOutcome>> {
        if let Some(outcome) = self.schedule_outcome(operation, &draft.id)? { return Ok(Some(outcome)); }
        let digest = request_digest(draft, outgoing);
        let active: Option<(String, Option<String>)> = self.conn.query_row(
            "SELECT draft_id, request_digest FROM scheduled WHERE operation_id = ?1", [operation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        if let Some((owner, previous)) = &active {
            binding(owner, &draft.id)?;
            if previous.as_deref() != Some(&digest) { return Err(Error::Invalid("This schedule operation already belongs to different mail.".into())); }
        }
        if let Some(state) = self.schedule_delivery(operation, &draft.id)? {
            if state == DeliveryState::Cancelled && active.is_some() {
                return Err(Error::Invalid("This email was cancelled. Take it back to Drafts before scheduling it again.".into()));
            }
            return Ok(Some(match state {
                DeliveryState::Complete => ScheduleOutcome::Completed,
                DeliveryState::Cancelled => ScheduleOutcome::Cancelled { early: false, restored: None },
                state => ScheduleOutcome::InDelivery { state },
            }));
        }
        if active.is_some() { return Ok(Some(ScheduleOutcome::AlreadyScheduled)); }
        if self.scheduled_operation(&draft.id)?.is_some() {
            return Err(Error::Invalid("This draft already has a schedule. Cancel or reschedule it first.".into()));
        }
        let owned: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE draft_id = ?1 AND state != 'cancelled')", [&draft.id], |row| row.get(0))?;
        if owned { return Err(Error::Invalid("This draft belongs to a delivery.".into())); }
        let cancelled: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE id = ?1)", [operation], |row| row.get(0))?;
        if cancelled { return Ok(Some(ScheduleOutcome::Cancelled { early: false, restored: None })); }
        Ok(None)
    }

    /// Identity checks, file cleanup, server cleanup and ownership transfer share one commit.
    pub fn schedule_operation_with_change(&mut self, operation: &str, draft: &Draft, outgoing: &str, send_at: i64, now: i64, change: impl FnOnce(&Self) -> Result<Option<(String, Vec<String>)>>) -> Result<ScheduleOutcome> {
        self.delivery_transaction(|store|store.schedule_in_transaction(operation,draft,outgoing,send_at,now,change))
    }

    pub(crate) fn schedule_in_transaction(&self, operation: &str, draft: &Draft, outgoing: &str, send_at: i64, now: i64, change: impl FnOnce(&Self) -> Result<Option<(String, Vec<String>)>>) -> Result<ScheduleOutcome> {
        let snapshot = Draft { revision: None, ..draft.clone() };
        let draft = &snapshot;
        let store=self;
        if let Some(outcome) = store.existing_schedule_outcome(operation, draft, outgoing)? { return Ok(outcome); }
        let digest = request_digest(draft, outgoing);
        if store.conn.execute(
            "INSERT INTO scheduled (draft_id, account_id, draft, outgoing, send_at, next_try, tries, error, operation_id, request_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, 0, NULL, ?6, ?7)",
            params![draft.id, draft.from_account_id, to_json(draft), outgoing, send_at, operation, digest],
        )? != 1 { return Err(Error::Invalid("The scheduled send could not be saved. No local changes were saved.".into())); }
        let cleanup = change(store)?;
        store.remember_draft_identity(&draft.id)?;
        store.retire_early_schedule_recovery(&draft.id)?;
        store.discard_removed_draft_files_in_transaction(draft)?;
        if let Some((account, changes)) = cleanup { store.queue_draft_cleanup_in_transaction(&account, &changes, now)?; }
        store.delete_transferred_draft(&draft.id)?;
        Ok(ScheduleOutcome::Scheduled)
    }

    pub fn reschedule_at_revision(&mut self, operation: &str, draft_id: &str, send_at: i64, expected_revision: i64) -> Result<ScheduleOutcome> {
        self.reschedule_checked(operation, draft_id, send_at, Some(expected_revision))
    }

    pub fn reschedule_operation(&mut self, operation: &str, draft_id: &str, send_at: i64) -> Result<ScheduleOutcome> {
        self.reschedule_checked(operation, draft_id, send_at, None)
    }

    fn reschedule_checked(&mut self, operation: &str, draft_id: &str, send_at: i64, expected_revision: Option<i64>) -> Result<ScheduleOutcome> {
        self.delivery_transaction(|store| {
            if let Some(outcome) = store.schedule_outcome(operation, draft_id)? { return Ok(outcome); }
            let current = store.scheduled_email_for_operation(operation)?;
            if let Some(current) = &current { binding(&current.draft.id, draft_id)?; }
            if let Some(state) = store.schedule_delivery(operation, draft_id)? {
                match state {
                    DeliveryState::Complete => return Ok(ScheduleOutcome::Completed),
                    DeliveryState::Cancelled => return Err(Error::Invalid("This email was cancelled. Take it back to Drafts before rescheduling it.".into())),
                    DeliveryState::Held => {},
                    state => return Ok(ScheduleOutcome::InDelivery { state }),
                }
            }
            let Some(current) = current else { return Err(Error::Invalid("This schedule is no longer available.".into())); };
            if let Some(expected) = expected_revision {
                if !(1..=9_007_199_254_740_991).contains(&expected) { return Err(Error::Invalid("The schedule revision is invalid.".into())); }
                if current.revision != expected { return Ok(ScheduleOutcome::Changed { current }); }
            }
            if current.revision == 9_007_199_254_740_991 { return Err(Error::Invalid("This schedule cannot be changed again. Cancel it and create a new schedule.".into())); }
            let changed = store.conn.execute(
                "UPDATE scheduled SET revision = revision + 1, send_at = ?3, next_try = ?3, tries = 0, error = NULL WHERE operation_id = ?1 AND draft_id = ?2 AND revision = ?4",
                params![operation, draft_id, send_at, current.revision],
            )?;
            if changed == 0 { return Err(Error::Invalid("The schedule could not be changed.".into())); }
            Ok(if expected_revision.is_some() { ScheduleOutcome::Rescheduled { revision: current.revision + 1 } } else { ScheduleOutcome::Scheduled })
        })
    }

    pub fn cancel_schedule_operation(&mut self, operation: &str, draft_id: &str, now: i64) -> Result<ScheduleOutcome> {
        self.cancel_schedule_checked(operation, draft_id, now, true)
    }

    /// Agents can only cancel identities already issued by the app, never pre-create one.
    pub fn cancel_known_schedule_operation(&mut self, operation: &str, draft_id: &str, now: i64) -> Result<ScheduleOutcome> {
        self.cancel_schedule_checked(operation, draft_id, now, false)
    }

    fn cancel_schedule_checked(&mut self, operation: &str, draft_id: &str, now: i64, allow_early: bool) -> Result<ScheduleOutcome> {
        self.delivery_transaction(|store| {
            if let Some(outcome) = store.schedule_outcome(operation, draft_id)? { return Ok(outcome); }
            if !allow_early && store.scheduled_email_for_operation(operation)?.is_none() {
                return Err(Error::Invalid("That operation is not scheduled. Call list_scheduled and pass its operationId.".into()));
            }
            let state = store.schedule_delivery(operation, draft_id)?;
            if let Some(state) = state.filter(|state| !matches!(state, DeliveryState::Held | DeliveryState::Cancelled)) {
                return Ok(if state == DeliveryState::Complete { ScheduleOutcome::Completed } else { ScheduleOutcome::InDelivery { state } });
            }
            let owner: Option<String> = store.conn.query_row("SELECT draft_id FROM scheduled WHERE operation_id = ?1", [operation], |row| row.get(0)).optional()?;
            if let Some(owner) = owner {
                binding(&owner, draft_id)?;
                return Ok(ScheduleOutcome::Cancelled { early: false, restored: store.unschedule_email_in_transaction(draft_id, now)? });
            }
            if state == Some(DeliveryState::Held) { return Err(Error::Invalid("This operation belongs to an ordinary send, not a schedule.".into())); }
            let owned_elsewhere = store.draft_has_outgoing_owner(draft_id)?;
            let early = state.is_none() && !owned_elsewhere;
            let inserted = store.conn.execute("INSERT INTO schedule_outcomes (id, draft_id, outcome, at, never_prepared) VALUES (?1, ?2, 'cancelled', ?3, ?4)", params![operation, draft_id, now, early])?;
            if inserted != 1 { return Err(Error::Invalid("The schedule cancellation was not saved.".into())); }
            store.conn.execute("INSERT OR IGNORE INTO delivery_cancellations (id) VALUES (?1)", [operation])?;
            let recorded: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE id = ?1)", [operation], |r| r.get(0))?;
            if !recorded { return Err(Error::Invalid("The schedule cancellation was not saved.".into())); }
            Ok(ScheduleOutcome::Cancelled { early, restored: if early || owned_elsewhere { None } else { store.editable_draft(draft_id)? } })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn work_cleanup(_: &Store) -> Result<Option<(String, Vec<String>)>> {
        Ok(Some(("work".into(), vec!["cleanup".into()])))
    }

    #[test]
    fn early_replay_refuses_unreadable_ownership_metadata() {
        let mut store = store_with_work();
        store.cancel_schedule_operation("early", "draft", NOV_1).unwrap();
        store.conn.execute_batch("ALTER TABLE deliveries RENAME TO unavailable_deliveries;").unwrap();
        assert!(store.cancel_schedule_operation("early", "draft", NOV_1).is_err());
        store.conn.execute_batch("ALTER TABLE unavailable_deliveries RENAME TO deliveries;").unwrap();
        assert_eq!(store.cancel_schedule_operation("early", "draft", NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
    }

    #[test]
    fn early_replay_cannot_offer_a_draft_that_was_handed_off_after_cancellation() {
        for scheduled in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("mail.db");
            let mut store = Store::open(&path).unwrap();
            store.upsert_account(&account("work", "maya@example.test")).unwrap();
            let d = draft("new-owner");
            assert_eq!(store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
            if scheduled { store.schedule_operation("new", &d, "{}", NOV_1 + DAY, NOV_1).unwrap(); }
            else { store.prepare_delivery("new", &d, "{}", "message@fixture", NOV_1).unwrap(); }
            assert_eq!(store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
            if scheduled { store.cancel_schedule_operation("new", &d.id, NOV_1).unwrap(); }
            else { store.cancel_delivery("new").unwrap(); }
            drop(store);
            let mut store = Store::open(&path).unwrap();
            let current = store.editable_draft(&d.id).unwrap();
            assert!(current.is_some());
            assert_eq!(store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: current });
        }
    }

    #[test]
    fn retiring_recovery_seeks_the_draft_instead_of_scanning_the_outcome_history() {
        let mut store = store_with_work();
        store.conn.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<5000) INSERT INTO schedule_outcomes(id,draft_id,outcome,at,never_prepared) SELECT 'operation-'||x,'draft-'||x,'cancelled',1,1 FROM n;").unwrap();
        let steps = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = steps.clone();
        store.conn.progress_handler(1, Some(move || { counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed); false })).unwrap();
        store.retire_early_schedule_recovery("draft-5000").unwrap();
        store.conn.progress_handler(0, None::<fn() -> bool>).unwrap();
        let steps = steps.load(std::sync::atomic::Ordering::Relaxed);
        assert!(steps < 200, "retiring one draft took {steps} VM steps");
        assert_eq!(store.conn.query_row("SELECT count(*) FROM schedule_outcomes WHERE never_prepared=1", [], |row| row.get::<_, i64>(0)).unwrap(), 4999);
    }

    #[test]
    fn failed_recovery_retirement_rolls_back_the_entire_handoff() {
        for scheduled in [false, true] {
            for refusal in ["IGNORE", "ABORT, 'fixture'"] {
                let mut store = store_with_work();
                let d = draft("retire-fault");
                store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap();
                store.conn.execute_batch(&format!("CREATE TRIGGER refuse_retirement BEFORE UPDATE OF never_prepared ON schedule_outcomes BEGIN SELECT RAISE({refusal}); END;")).unwrap();
                let result = if scheduled { store.schedule_operation("new", &d, "{}", NOV_1 + DAY, NOV_1).map(|_| ()) }
                    else { store.prepare_delivery("new", &d, "{}", "message@fixture", NOV_1).map(|_| ()) };
                assert!(result.is_err());
                assert!(store.scheduled_emails().unwrap().is_empty());
                assert!(store.delivery("new").unwrap().is_none());
                assert_eq!(store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
            }
        }
    }

    #[test]
    fn an_unknown_cancellation_cannot_offer_another_operations_draft_for_editing() {
        for scheduled in [false, true] {
            let mut store = store_with_work();
            let d = draft("already-owned");
            if scheduled { store.schedule_operation("owner", &d, "{}", NOV_1 + DAY, NOV_1).unwrap(); }
            else { store.prepare_delivery("owner", &d, "{}", "message@fixture", NOV_1).unwrap(); }
            for _ in 0..2 {
                let outcome = store.cancel_schedule_operation("unknown", &d.id, NOV_1).unwrap();
                assert_eq!(outcome, ScheduleOutcome::Cancelled { early: false, restored: None });
            }
            assert!(store.draft(&d.id).unwrap().is_none());
            if scheduled { assert_eq!(store.scheduled_operation(&d.id).unwrap().as_deref(), Some("owner")); }
            else { assert_eq!(store.delivery_state("owner").unwrap(), Some(DeliveryState::Held)); }
        }
    }

    #[test]
    fn early_schedule_cancellation_requires_both_durable_records() {
        for table in ["schedule_outcomes", "delivery_cancellations"] {
            for action in ["IGNORE", "ABORT, 'cancel failed'"] {
                let mut store = store_with_work();
                let d = draft("early");
                store.conn.execute_batch(&format!("CREATE TRIGGER fail_cancel BEFORE INSERT ON {table} BEGIN SELECT RAISE({action}); END;")).unwrap();
                assert!(store.cancel_schedule_operation("operation", &d.id, NOV_1).is_err(), "{table}: {action}");
                assert_eq!(store.conn.query_row("SELECT count(*) FROM schedule_outcomes", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
                assert_eq!(store.delivery_state("operation").unwrap(), None);
                store.conn.execute_batch("DROP TRIGGER fail_cancel").unwrap();
                assert_eq!(store.cancel_schedule_operation("operation", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
                assert!(!store.prepare_delivery("operation", &d, "{}", "mail@example.test", NOV_1).unwrap());
            }
        }
    }

    #[test]
    fn cancellation_before_creation_survives_reopen_and_preserves_the_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedule.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        let d = draft("early-schedule");
        store.save_draft(&d).unwrap();
        assert_eq!(store.cancel_schedule_operation("op", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.schedule_operation("op", &d, "{}", NOV_1 + DAY, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
        assert_eq!(store.draft(&d.id).unwrap(), Some(d.clone()));
        assert!(store.scheduled_emails().unwrap().is_empty());
        assert!(!store.prepare_delivery("op", &d, "{}", "never@fixture", NOV_1).unwrap());
    }

    #[test]
    fn retry_preserves_a_rescheduled_time_and_rejects_changed_content() {
        let mut store = store_with_work();
        let mut d = draft("retry-schedule");
        assert_eq!(store.schedule_operation("op", &d, "payload", NOV_1, NOV_1).unwrap(), ScheduleOutcome::Scheduled);
        assert!(store.reschedule_email(&d.id, NOV_1 + DAY).unwrap());
        d.updated_at += 1;
        assert_eq!(store.schedule_operation("op", &d, "payload", NOV_1, NOV_1).unwrap(), ScheduleOutcome::AlreadyScheduled);
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1 + DAY);
        assert!(store.schedule_operation("op", &d, "changed", NOV_1, NOV_1).is_err());
        d.body.push_str("changed");
        assert!(store.schedule_operation("op", &d, "payload", NOV_1, NOV_1).is_err());
    }

    #[test]
    fn a_token_cannot_be_rebound_and_an_old_undo_cannot_cancel_a_new_schedule() {
        let mut store = store_with_work();
        let d = draft("bound-schedule");
        store.schedule_operation("old", &d, "{}", NOV_1, NOV_1).unwrap();
        assert!(store.cancel_schedule_operation("old", "other-draft", NOV_1).is_err());
        assert!(store.schedule_operation("new", &d, "{}", NOV_1, NOV_1).is_err());
        assert!(matches!(store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: Some(_) }));
        store.schedule_operation("new", &d, "{}", NOV_1 + DAY, NOV_1).unwrap();
        assert_eq!(store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
        assert_eq!(store.schedule_operation("old", &d, "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
        assert_eq!(store.scheduled_operation(&d.id).unwrap().as_deref(), Some("new"));
    }

    #[test]
    fn a_prepared_schedule_still_rejects_changed_content_for_its_token() {
        let mut store = store_with_work();
        let d = draft("prepared-content");
        store.schedule_operation("op", &d, "original", NOV_1, NOV_1).unwrap();
        store.prepare_delivery("op", &d, "prepared bytes", "prepared@fixture", NOV_1).unwrap();
        assert!(store.schedule_operation("op", &d, "changed", NOV_1, NOV_1).is_err());
        assert_eq!(store.delivery_state("op").unwrap(), Some(DeliveryState::Held));
    }

    #[test]
    fn internal_unschedule_requires_an_owning_transaction() {
        let mut store = store_with_work();
        let d = draft("unguarded-unschedule");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        assert!(store.unschedule_email_in_transaction(&d.id, NOV_1).is_err());
        assert!(store.scheduled_email(&d.id).unwrap().is_some());
        assert_eq!(store.delivery_state("op").unwrap(), None);
    }

    #[test]
    fn delivery_ownership_is_reported_without_recreating_or_cancelling_claimed_mail() {
        for state in [DeliveryState::Held, DeliveryState::Claimed, DeliveryState::Accepted, DeliveryState::Uncertain, DeliveryState::Complete, DeliveryState::Cancelled] {
            let mut store = store_with_work();
            let d = draft("delivery-owner");
            store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
            store.prepare_delivery("op", &d, "prepared", "id@fixture", NOV_1).unwrap();
            store.conn.execute("UPDATE deliveries SET state = ?1 WHERE id = 'op'", [crate::codec::name(&state)]).unwrap();
            let expected = match state {
                DeliveryState::Complete => ScheduleOutcome::Completed,
                DeliveryState::Cancelled => ScheduleOutcome::Cancelled { early: false, restored: None },
                state => ScheduleOutcome::InDelivery { state },
            };
            if state == DeliveryState::Cancelled {
                assert!(store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).is_err());
            } else {
                assert_eq!(store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap(), expected);
            }
            let cancelled = store.cancel_schedule_operation("op", &d.id, NOV_1).unwrap();
            if matches!(state, DeliveryState::Held | DeliveryState::Cancelled) {
                assert!(matches!(cancelled, ScheduleOutcome::Cancelled { early: false, restored: Some(_) }));
                assert!(store.scheduled_email(&d.id).unwrap().is_none());
            } else {
                assert_eq!(cancelled, expected);
                assert!(store.scheduled_email(&d.id).unwrap().is_some());
            }
        }
    }

    #[test]
    fn ordinary_deliveries_and_finished_tokens_keep_their_original_owner() {
        let mut store = store_with_work();
        let d = draft("ordinary");
        store.prepare_delivery("op", &d, "{}", "ordinary@fixture", NOV_1).unwrap();
        assert!(store.cancel_schedule_operation("op", &d.id, NOV_1).is_err());
        assert!(store.schedule_operation("op", &draft("other"), "{}", NOV_1, NOV_1).is_err());
        assert!(store.cancel_schedule_operation("op", "other", NOV_1).is_err());
        assert!(store.schedule_operation("another-op", &d, "{}", NOV_1, NOV_1).is_err());
        store.cancel_delivery("op").unwrap();
        assert_eq!(store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
        store.cancel_schedule_operation("cancelled", "first-owner", NOV_1).unwrap();
        assert!(store.cancel_schedule_operation("cancelled", "other-owner", NOV_1).is_err());
        assert!(store.schedule_operation("cancelled", &d, "{}", NOV_1, NOV_1).is_err());
        store.cancel_delivery("early-send-cancel").unwrap();
        assert_eq!(store.schedule_operation("early-send-cancel", &draft("another"), "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
    }

    #[test]
    fn corrupt_outcomes_and_legacy_requests_fail_closed() {
        let mut store = store_with_work();
        let d = draft("legacy");
        store.schedule_operation("legacy-op", &d, "{}", NOV_1, NOV_1).unwrap();
        store.conn.execute("UPDATE scheduled SET request_digest = NULL", []).unwrap();
        assert!(store.schedule_operation("legacy-op", &d, "{}", NOV_1, NOV_1).is_err());
        assert!(matches!(store.cancel_schedule_operation("legacy-op", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: Some(_) }));
        store.conn.execute_batch("PRAGMA ignore_check_constraints = ON; UPDATE schedule_outcomes SET outcome = 'corrupt';").unwrap();
        assert!(store.schedule_operation("legacy-op", &d, "{}", NOV_1, NOV_1).is_err());
        assert!(store.cancel_schedule_operation("legacy-op", &d.id, NOV_1).is_err());
        assert!(store.scheduled_emails().unwrap().is_empty());
    }

    #[test]
    fn transfer_cleanup_failures_roll_back_the_entire_batch_and_files() {
        for scheduled in [false, true] {
            for failure in ["empty", "too-many", "builder", "second-part"] {
                let mut store = store_with_work();
                let saved = store.attach_and_save_draft(&draft("cleanup-transfer"), &[("kept.txt", "text/plain", b"kept")]).unwrap();
                let mut edited = saved.clone();
                edited.attachments.clear();
                if failure == "second-part" {
                    store.conn.execute_batch("CREATE TRIGGER refuse_second BEFORE INSERT ON outbox WHEN NEW.change = 'second' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
                }
                let builder = |inside: &Store| {
                    assert!(!inside.conn.is_autocommit());
                    assert_eq!(inside.draft(&saved.id).unwrap(), Some(saved.clone()));
                    match failure {
                        "builder" => Err(Error::Invalid("fixture builder failure".into())),
                        "empty" => Ok(Some(("work".into(), vec![]))),
                        "too-many" => Ok(Some(("work".into(), vec!["cleanup".into(); 65]))),
                        _ => Ok(Some(("work".into(), vec!["first".into(), "second".into()]))),
                    }
                };
                let result = if scheduled {
                    store.schedule_operation_with_change("transfer-cleanup", &edited, "{}", NOV_1 + 1, NOV_1, builder).map(|_| ())
                } else {
                    store.prepare_delivery_with_change("transfer-cleanup", &edited, "{}", "sent@example.test", NOV_1, builder).map(|_| ())
                };
                assert!(result.is_err(), "scheduled={scheduled}, failure={failure}");
                assert_eq!(store.draft(&saved.id).unwrap(), Some(saved.clone()));
                assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_some());
                assert!(store.delivery("transfer-cleanup").unwrap().is_none());
                assert!(store.scheduled_operation(&saved.id).unwrap().is_none());
                assert!(store.pending_changes("work").unwrap().is_empty());
                let batches: i64 = store.conn.query_row("SELECT count(*) FROM outbox_batches", [], |row| row.get(0)).unwrap();
                assert_eq!(batches, 0);
            }
        }
    }

    #[test]
    fn ignored_ownership_transfer_writes_preserve_the_editable_draft() {
        for action in ["schedule-insert", "schedule-delete", "prepare-delete"] {
            let mut store = store_with_work();
            let saved = store.attach_and_save_draft(&draft("retained"), &[("kept.txt", "text/plain", b"kept")]).unwrap();
            store.conn.execute_batch(if action == "schedule-insert" {
                "CREATE TRIGGER ignore_transfer BEFORE INSERT ON scheduled BEGIN SELECT RAISE(IGNORE); END;"
            } else {
                "CREATE TRIGGER ignore_transfer BEFORE DELETE ON drafts BEGIN SELECT RAISE(IGNORE); END;"
            }).unwrap();
            let result = if action == "prepare-delete" {
                store.prepare_delivery("transfer", &saved, "{}", "sent@example.test", NOV_1).map(|_| ())
            } else {
                store.schedule_operation("transfer", &saved, "{}", NOV_1 + 1, NOV_1).map(|_| ())
            };
            assert!(result.is_err(), "{action}");
            assert_eq!(store.draft(&saved.id).unwrap(), Some(saved.clone()));
            assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_some());
            assert!(store.delivery("transfer").unwrap().is_none());
            assert!(store.scheduled_operation(&saved.id).unwrap().is_none());
        }
    }

    #[test]
    fn server_cleanup_and_removed_files_commit_or_roll_back_with_the_schedule() {
        for fail in [None, Some("ABORT, 'fixture'"), Some("IGNORE")] {
            let mut store = store_with_work();
            let saved = store.attach_and_save_draft(&draft("atomic"), &[("file.txt", "text/plain", b"keep")]).unwrap();
            let mut edited = saved.clone();
            edited.attachments.clear();
            if let Some(action) = fail { store.conn.execute_batch(&format!("CREATE TRIGGER refuse_cleanup BEFORE INSERT ON outbox BEGIN SELECT RAISE({action}); END;")).unwrap(); }
            let result = store.schedule_operation_with_change("op", &edited, "{}", NOV_1, NOV_1, work_cleanup);
            if fail.is_some() {
                assert!(result.is_err());
                assert_eq!(store.draft(&saved.id).unwrap(), Some(saved.clone()));
                assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_some());
                assert!(store.scheduled_email(&saved.id).unwrap().is_none());
                assert!(store.pending_changes("work").unwrap().is_empty());
            } else {
                assert_eq!(result.unwrap(), ScheduleOutcome::Scheduled);
                assert!(store.draft(&saved.id).unwrap().is_none());
                assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_none());
                assert_eq!(store.pending_changes("work").unwrap()[0].1, "cleanup");
            }
        }
    }

    #[test]
    fn failed_terminal_transitions_leave_no_partial_outcome() {
        let mut store = store_with_work();
        let d = draft("atomic-outcome");
        store.conn.execute_batch("CREATE TRIGGER refuse_cancel BEFORE INSERT ON delivery_cancellations BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.cancel_schedule_operation("op", &d.id, NOV_1).is_err());
        store.conn.execute_batch("DROP TRIGGER refuse_cancel").unwrap();
        assert_eq!(store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::Scheduled);
        store.conn.execute_batch("CREATE TRIGGER refuse_finish BEFORE DELETE ON scheduled BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.email_sent(&d.id, NOV_1).is_err());
        assert_eq!(store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::AlreadyScheduled);
        store.conn.execute_batch("DROP TRIGGER refuse_finish").unwrap();
        assert!(store.email_sent(&d.id, NOV_1).unwrap());
        assert_eq!(store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::Completed);
    }

    #[test]
    fn outcomes_have_stable_tagged_wire_values() {
        let current = ScheduledEmail { operation_id: "wire-op".into(), revision: 7, draft: draft("wire-draft"), send_at: NOV_1, next_try: NOV_1, tries: 0, error: None };
        assert_eq!(serde_json::to_value(ScheduleOutcome::Rescheduled { revision: 7 }).unwrap(), serde_json::json!({ "status": "rescheduled", "revision": 7 }));
        let conflict = serde_json::to_value(ScheduleOutcome::Changed { current: current.clone() }).unwrap();
        assert_eq!(conflict["current"]["operationId"], "wire-op");
        assert_eq!(conflict["current"]["revision"], 7);
        assert_eq!(conflict["current"]["draft"]["id"], "wire-draft");
        for (outcome, status) in [
            (ScheduleOutcome::Scheduled, "scheduled"),
            (ScheduleOutcome::AlreadyScheduled, "alreadyScheduled"),
            (ScheduleOutcome::Sending, "sending"),
            (ScheduleOutcome::Rescheduled { revision: 7 }, "rescheduled"),
            (ScheduleOutcome::Changed { current }, "changed"),
            (ScheduleOutcome::Cancelled { early: true, restored: None }, "cancelled"),
            (ScheduleOutcome::Cancelled { early: false, restored: None }, "cancelled"),
            (ScheduleOutcome::Cancelled { early: false, restored: Some(draft("restored")) }, "cancelled"),
            (ScheduleOutcome::Completed, "completed"),
            (ScheduleOutcome::InDelivery { state: DeliveryState::Uncertain }, "inDelivery"),
        ] {
            let wire = serde_json::to_value(&outcome).unwrap();
            assert_eq!(wire["status"], status);
            assert_eq!(serde_json::from_value::<ScheduleOutcome>(wire).unwrap(), outcome);
        }
    }

    #[test]
    fn stale_rescheduling_cannot_change_a_newer_operation() {
        let mut store = store_with_work();
        let d = draft("reschedule-owner");
        store.schedule_operation("old", &d, "{}", NOV_1, NOV_1).unwrap();
        store.cancel_schedule_operation("old", &d.id, NOV_1).unwrap();
        store.schedule_operation("new", &d, "{}", NOV_1 + DAY, NOV_1).unwrap();
        assert_eq!(store.reschedule_operation("old", &d.id, NOV_1).unwrap(), ScheduleOutcome::Cancelled { early: false, restored: None });
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1 + DAY);
        assert!(store.reschedule_operation("new", "another-draft", NOV_1).is_err());
        assert!(store.reschedule_operation("unknown", &d.id, NOV_1).is_err());
        assert_eq!(store.reschedule_operation("new", &d.id, NOV_1 + 2 * DAY).unwrap(), ScheduleOutcome::Scheduled);
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1 + 2 * DAY);
        assert_eq!(store.schedule_operation("new", &d, "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::AlreadyScheduled);
    }

    #[test]
    fn rescheduling_does_not_reactivate_submitted_or_completed_mail() {
        for state in [DeliveryState::Claimed, DeliveryState::Accepted, DeliveryState::Uncertain, DeliveryState::Complete] {
            let mut store = store_with_work();
            let d = draft("submitted-reschedule");
            store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
            store.prepare_delivery("op", &d, "prepared", "id@fixture", NOV_1).unwrap();
            store.conn.execute("UPDATE deliveries SET state = ?1 WHERE id = 'op'", [crate::codec::name(&state)]).unwrap();
            let expected = if state == DeliveryState::Complete { ScheduleOutcome::Completed } else { ScheduleOutcome::InDelivery { state } };
            assert_eq!(store.reschedule_operation("op", &d.id, NOV_1 + DAY).unwrap(), expected);
            assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1);
        }
    }

    #[test]
    fn moving_a_prepared_hold_preserves_its_exact_delivery() {
        let mut store = store_with_work();
        let d = draft("held-move");
        store.schedule_operation("op", &d, "original", NOV_1, NOV_1).unwrap();
        store.prepare_delivery("op", &d, "frozen Date and payload", "stable@fixture", NOV_1).unwrap();
        let before = store.delivery("op").unwrap();
        assert_eq!(store.reschedule_operation("op", &d.id, NOV_1 + DAY).unwrap(), ScheduleOutcome::Scheduled);
        assert_eq!(store.delivery("op").unwrap(), before);
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1 + DAY);
        store.conn.execute("UPDATE deliveries SET state = 'cancelled' WHERE id = 'op'", []).unwrap();
        assert!(store.reschedule_operation("op", &d.id, NOV_1 + 2 * DAY).is_err());
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1 + DAY);
    }

    #[test]
    fn a_stale_revision_cannot_overwrite_a_newer_schedule_time() {
        let mut store = store_with_work();
        let d = draft("revision-race");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        store.reschedule_at_revision("op", &d.id, NOV_1 + DAY, 1).unwrap();
        let stale = store.reschedule_at_revision("op", &d.id, NOV_1 + 2 * DAY, 1).unwrap();
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1 + DAY);
        assert_eq!(serde_json::to_value(stale).unwrap()["status"], "changed");
    }

    #[test]
    fn checked_moves_chain_revisions_without_counting_retry_bookkeeping() {
        let mut store = store_with_work();
        let d = draft("revision-chain");
        store.schedule_operation("op", &d, "payload", NOV_1, NOV_1).unwrap();
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().revision, 1);
        store.email_failed(&d.id, "offline", NOV_1 + DAY).unwrap();
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().revision, 1);
        assert_eq!(store.reschedule_at_revision("op", &d.id, NOV_1 + 2 * DAY, 1).unwrap(), ScheduleOutcome::Rescheduled { revision: 2 });
        let current = store.scheduled_email(&d.id).unwrap().unwrap();
        assert_eq!((current.tries, current.error, current.next_try), (0, None, NOV_1 + 2 * DAY));
        assert_eq!(store.reschedule_at_revision("op", &d.id, NOV_1 + 3 * DAY, 2).unwrap(), ScheduleOutcome::Rescheduled { revision: 3 });
        assert_eq!(serde_json::to_value(store.scheduled_email(&d.id).unwrap().unwrap()).unwrap()["revision"], 3);
        assert_eq!(store.due_email(&d.id, NOV_1 + 3 * DAY).unwrap().unwrap().1, "payload");
    }

    #[test]
    fn revisions_remain_positive_javascript_safe_integers() {
        let mut store = store_with_work();
        let d = draft("revision-bound");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        for invalid in [-1, 0, 9_007_199_254_740_992] {
            assert!(store.reschedule_at_revision("op", &d.id, NOV_1 + DAY, invalid).is_err());
        }
        store.conn.execute("UPDATE scheduled SET revision = 9007199254740991", []).unwrap();
        assert!(store.reschedule_at_revision("op", &d.id, NOV_1 + DAY, 9_007_199_254_740_991).is_err());
        assert!(matches!(store.reschedule_email(&d.id, NOV_1 + DAY), Err(Error::Invalid(_))));
        let current = store.scheduled_email(&d.id).unwrap().unwrap();
        assert_eq!((current.revision, current.send_at), (9_007_199_254_740_991, NOV_1));
    }

    #[test]
    fn failed_revision_updates_roll_back_time_and_retry_state() {
        let mut store = store_with_work();
        let d = draft("revision-rollback");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        store.email_failed(&d.id, "offline", NOV_1 + DAY).unwrap();
        let before = store.scheduled_email(&d.id).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_move BEFORE UPDATE ON scheduled BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.reschedule_at_revision("op", &d.id, NOV_1 + 2 * DAY, 1).is_err());
        assert!(store.reschedule_email(&d.id, NOV_1 + 2 * DAY).is_err());
        assert_eq!(store.scheduled_email(&d.id).unwrap(), before);
    }

    #[test]
    fn legacy_moves_also_invalidate_older_revisions() {
        let mut store = store_with_work();
        let d = draft("legacy-revision");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        assert!(store.reschedule_email(&d.id, NOV_1 + DAY).unwrap());
        store.reschedule_operation("op", &d.id, NOV_1 + 2 * DAY).unwrap();
        let current = store.scheduled_email(&d.id).unwrap().unwrap();
        assert_eq!(current.revision, 3);
        assert_eq!(store.reschedule_at_revision("op", &d.id, NOV_1, 2).unwrap(), ScheduleOutcome::Changed { current });
    }

    #[test]
    fn existing_schedules_gain_revision_one_on_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old-schedule.db");
        let mut store = crate::test_support::historical_store(&path, 27);
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        let d = draft("migrated-revision");
        store.conn.execute("INSERT INTO scheduled(draft_id,account_id,draft,outgoing,send_at,next_try,operation_id) VALUES (?1,?2,?3,'payload',?4,?4,'op')",
            rusqlite::params![d.id,d.from_account_id,serde_json::to_string(&d).unwrap(),NOV_1]).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let current = store.scheduled_email(&d.id).unwrap().unwrap();
        assert_eq!(current.revision, 1);
        assert_eq!(current.operation_id, "op");
        assert_eq!(current.draft, d);
        assert_eq!(current.send_at, NOV_1);
        assert_eq!(store.due_email(&current.draft.id, NOV_1).unwrap().unwrap().1, "payload");
    }

    #[test]
    fn an_ignored_reschedule_is_not_reported_as_success() {
        let mut store = store_with_work();
        let d = draft("ignored-reschedule");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        store.conn.execute_batch("CREATE TRIGGER ignore_time_change BEFORE UPDATE ON scheduled BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.reschedule_operation("op", &d.id, NOV_1 + DAY).is_err());
        assert_eq!(store.scheduled_email(&d.id).unwrap().unwrap().send_at, NOV_1);
    }

    #[test]
    fn scheduled_rows_expose_their_operation_identity() {
        let mut store = store_with_work();
        let d = draft("visible-operation");
        store.schedule_operation("visible-token", &d, "payload", NOV_1, NOV_1).unwrap();
        let rows = store.scheduled_emails().unwrap();
        assert_eq!(serde_json::to_value(&rows[0]).unwrap()["operationId"], "visible-token");
        let due = store.due_email(&d.id, NOV_1).unwrap().unwrap();
        assert_eq!(serde_json::to_value(&due.0).unwrap()["operationId"], "visible-token");
        assert_eq!(due.1, "payload");
    }

    #[test]
    fn completion_records_the_callers_clock() {
        let mut store = store_with_work();
        let d = draft("completion-clock");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        store.email_sent(&d.id, NOV_1 + DAY).unwrap();
        let at: i64 = store.conn.query_row("SELECT at FROM schedule_outcomes WHERE id = 'op'", [], |row| row.get(0)).unwrap();
        assert_eq!(at, NOV_1 + DAY);
    }

    #[test]
    fn completed_operations_cannot_recreate_a_schedule() {
        let mut store = store_with_work();
        let d = draft("completed-schedule");
        store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap();
        assert!(store.email_sent(&d.id, NOV_1).unwrap());
        store.save_draft(&d).unwrap();
        assert_eq!(store.schedule_operation("op", &d, "{}", NOV_1, NOV_1).unwrap(), ScheduleOutcome::Completed);
        assert_eq!(store.cancel_schedule_operation("op", &d.id, NOV_1).unwrap(), ScheduleOutcome::Completed);
        assert_eq!(store.draft(&d.id).unwrap(), Some(d));
        assert!(store.scheduled_emails().unwrap().is_empty());
    }
}
