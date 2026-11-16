//! Durable evidence of a send attempt. A claimed delivery is never automatically retried:
//! a crash or a lost response cannot tell us whether the remote server accepted the mail.

use moss_core::Draft;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use crate::{codec::{json, named, to_json}, Result, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryState { Held, Claimed, Accepted, Uncertain, Complete, Cancelled }

/// An email you sent that Moss stopped before it went, back in Drafts (see
/// [`Store::cancel_interrupted_delivery`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedSend { pub id: String, pub subject: String, pub at: i64 }

/// Replay identity without loading message or attachment content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub id: String,
    pub state: DeliveryState,
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: String,
    pub draft: Draft,
    pub outgoing: String,
    pub message_id: String,
    pub state: DeliveryState,
    pub raw: Option<Vec<u8>>,
    pub source: Option<String>,
    pub error: Option<String>,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub source_revision: Option<String>,
}

/// Only the evidence needed to repair a server-accepted email locally.
pub struct DeliveryRepair {
    pub outgoing: String,
    pub raw: Option<Vec<u8>>,
    pub source: Option<String>,
}

const COLUMNS: &str = "id, draft, outgoing, message_id, state, raw, source, error, thread_id, source_revision";
fn delivery(row: &Row) -> rusqlite::Result<Delivery> {
    Ok(Delivery { id: row.get(0)?, draft: json(row, 1)?, outgoing: row.get(2)?,
        message_id: row.get(3)?, state: named(row, 4)?, raw: row.get(5)?, source: row.get(6)?, error: row.get(7)?, thread_id: row.get(8)?, source_revision: row.get(9)? })
}

fn finished_transition<T>(outcome: Result<T>, restored: rusqlite::Result<()>) -> Result<T> {
    // The commit is authoritative. A failed optimization reset must never invite
    // the caller to retry a claim or an accepted submission as though it rolled back.
    if restored.is_err() {
        eprintln!("Moss: database performance settings could not be restored after a delivery transaction.");
    }
    outcome
}

impl Store {
    /// Critical delivery transitions must survive power loss, not just a process crash.
    /// The usual sync/cache writes retain their existing durability setting.
    pub(crate) fn delivery_transaction<T>(&mut self, change: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let previous: i64 = self.conn.pragma_query_value(None, "synchronous", |row| row.get(0))?;
        let previous_fullfsync: bool = self.conn.pragma_query_value(None, "fullfsync", |row| row.get(0))?;
        self.conn.pragma_update(None, "fullfsync", true)?;
        self.conn.pragma_update(None, "synchronous", "FULL")?;
        let result = (|| {
            let tx = self.conn.unchecked_transaction()?;
            let value = change(self)?;
            tx.commit()?;
            Ok(value)
        })();
        let restored_sync = self.conn.pragma_update(None, "synchronous", previous);
        let restored_flush = self.conn.pragma_update(None, "fullfsync", previous_fullfsync);
        finished_transition(result, restored_sync.and(restored_flush))
    }

    /// The complete recoverable draft is committed before it leaves Drafts or waits for Undo.
    /// Reusing an operation ID cannot create another send, even after completion.
    pub fn prepare_delivery(&mut self, id: &str, draft: &Draft, outgoing: &str, message_id: &str, now: i64) -> Result<bool> {
        self.prepare_delivery_with_change(id, draft, outgoing, message_id, now, |_| Ok(None))
    }

    /// Queue removal of the server draft in the same durable commit as preparation.
    pub fn prepare_delivery_with_change(&mut self, id: &str, draft: &Draft, outgoing: &str, message_id: &str, now: i64, change: impl FnOnce(&Self) -> Result<Option<(String, Vec<String>)>>) -> Result<bool> {
        self.prepare_delivery_fingerprinted(id,draft,outgoing,message_id,now,None,change)
    }

    pub fn prepare_delivery_fingerprinted(&mut self, id: &str, draft: &Draft, outgoing: &str, message_id: &str, now: i64, logical_hash: Option<&str>, change: impl FnOnce(&Self) -> Result<Option<(String, Vec<String>)>>) -> Result<bool> {
        self.delivery_transaction(|store|store.prepare_delivery_in_transaction(id,draft,outgoing,message_id,now,logical_hash,change))
    }

    pub(crate) fn prepare_delivery_in_transaction(&self, id: &str, draft: &Draft, outgoing: &str, message_id: &str, now: i64, logical_hash: Option<&str>, change: impl FnOnce(&Self) -> Result<Option<(String, Vec<String>)>>) -> Result<bool> {
        let source_revision = draft.revision.clone();
        let snapshot = Draft { revision: None, ..draft.clone() };
        let draft = &snapshot;
        let store=self;
        let cancelled: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE id = ?1)", [id], |row| row.get(0))?;
        if cancelled { return Ok(false); }
        let strict_hash=crate::delivery_compaction::storage_fingerprint(draft,outgoing,message_id);
        if let Some(existing)=store.compacted_storage_hash(id)? {
            if existing.as_deref()!=Some(strict_hash.as_str()) {
                return Err(crate::Error::Invalid("This delivery operation already belongs to different mail.".into()));
            }
            return Ok(false);
        }
        if let Some(existing) = store.delivery(id)? {
            if existing.draft != *draft || existing.outgoing != outgoing || existing.message_id != message_id {
                return Err(crate::Error::Invalid("This delivery operation already belongs to different mail.".into()));
            }
            return Ok(false);
        }
        let account_exists: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)", [&draft.from_account_id], |row| row.get(0))?;
        if !account_exists { return Err(crate::Error::Invalid("The sending account is no longer on this Mac.".into())); }
        if store.scheduled_operation(&draft.id)?.is_some_and(|operation| operation != id) {
            return Err(crate::Error::Invalid("This draft belongs to a scheduled send. Cancel its schedule before sending it now.".into()));
        }
        let added = store.conn.execute(
            "INSERT INTO deliveries (id, account_id, draft, outgoing, message_id, state, created_at, draft_id, snooze_revision, source_revision, file_ceiling,storage_hash,logical_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, 'held', ?6, ?7,
                coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'snooze_edits'), 0), ?8, coalesce((SELECT max(id) FROM draft_files WHERE draft_id=?7),0),?9,?10) ON CONFLICT(id) DO NOTHING",
            params![id, draft.from_account_id, to_json(draft), outgoing, message_id, now, draft.id, source_revision,strict_hash,logical_hash])?;
        if added > 0 {
            let cleanup = change(store)?;
            store.remember_draft_identity(&draft.id)?;
            store.retire_early_schedule_recovery(&draft.id)?;
            store.discard_removed_draft_files_in_transaction(draft)?;
            store.delete_transferred_draft(&draft.id)?;
            if let Some((account, changes)) = cleanup {
                store.queue_draft_cleanup_in_transaction(&account, &changes, now)?;
            }
        }
        Ok(added > 0)
    }

    pub fn active_delivery_id(&self, draft_id: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT id FROM deliveries WHERE draft_id = ?1 AND state != 'cancelled'", [draft_id], |row| row.get(0)).optional()?)
    }

    pub fn delivery_receipt(&self, draft_id: &str) -> Result<Option<DeliveryReceipt>> {
        Ok(self.conn.query_row("SELECT id,state,source_revision FROM deliveries WHERE draft_id=?1 AND state!='cancelled'", [draft_id], |row| {
            Ok(DeliveryReceipt { id: row.get(0)?, state: named(row,1)?, source_revision: row.get(2)? })
        }).optional()?)
    }

    /// Status and cancellation checks must not load attachment-sized journal payloads.
    pub fn delivery_state(&self, id: &str) -> Result<Option<DeliveryState>> {
        Ok(self.conn.query_row("SELECT state FROM deliveries WHERE id = ?1 UNION ALL SELECT 'cancelled' FROM delivery_cancellations WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM deliveries WHERE id = ?1)", [id], |row| named(row, 0)).optional()?)
    }

    pub fn delivery(&self, id: &str) -> Result<Option<Delivery>> {
        if self.compacted_storage_hash(id)?.is_some() { return Err(crate::Error::Invalid("This delivery's payload has been retired. Its delivery receipt is retained.".into())); }
        Ok(self.conn.query_row(&format!("SELECT {COLUMNS} FROM deliveries WHERE id = ?1"), [id], delivery).optional()?)
    }

    pub fn delivery_repair(&self, id: &str) -> Result<Option<DeliveryRepair>> {
        Ok(self.conn.query_row("SELECT outgoing,raw,source FROM deliveries WHERE id=?1 AND state='accepted' AND resolution IS NULL",[id],|row|Ok(DeliveryRepair {outgoing:row.get(0)?,raw:row.get(1)?,source:row.get(2)?})).optional()?)
    }

    pub fn delivery_draft_id(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT draft_id FROM deliveries WHERE id=?1",[id],|row|row.get(0)).optional()?)
    }

    /// Everything requiring recovery or attention. Completed operation IDs stay for deduplication.
    pub fn pending_deliveries(&self) -> Result<Vec<Delivery>> {
        let mut statement = self.conn.prepare(&format!("SELECT {COLUMNS} FROM deliveries WHERE state NOT IN ('complete', 'cancelled') AND resolution IS NULL AND compacted_at IS NULL ORDER BY created_at, id"))?;
        let rows = statement.query_map([], delivery)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Only holds and claims require startup ownership recovery. Accepted payloads
    /// are read one at a time by the independent, backed-off local repair path.
    pub fn interrupted_deliveries(&self) -> Result<Vec<(String,DeliveryState,String)>> {
        let mut statement=self.conn.prepare("SELECT id,state,account_id FROM deliveries WHERE state IN ('held','claimed') ORDER BY created_at,id")?;
        let rows=statement.query_map([],|row|Ok((row.get(0)?,named(row,1)?,row.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Freeze an interrupted operation for manual attention without decoding or
    /// discarding any payload. Uncertainty never authorizes automatic resubmission.
    pub fn retain_interrupted_delivery(&mut self,id:&str,error:&str)->Result<()> {
        self.delivery_transaction(|store| {
            store.conn.execute("UPDATE deliveries SET interrupted_from=state,state='uncertain',error=?2 WHERE id=?1 AND state IN ('held','claimed')",params![id,error])?;
            let unresolved:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE id=?1 AND state IN ('held','claimed'))",[id],|row|row.get(0))?;
            if unresolved {return Err(crate::Error::Invalid("The interrupted delivery could not be retained for review.".into()));}
            Ok(())
        })
    }

    /// Reserve a bounded batch of accepted local repairs, never network submissions.
    pub fn claim_delivery_repairs(&mut self, now: i64) -> Result<Vec<String>> {
        self.delivery_transaction(|store| {
            let mut statement=store.conn.prepare("SELECT id, repair_attempt FROM deliveries WHERE state='accepted' AND resolution IS NULL AND (repair_next<=?1 OR repair_next>?2) ORDER BY repair_next,id LIMIT 16")?;
            let rows=statement.query_map(params![now,now.saturating_add(300_000)],|row| Ok((row.get::<_,String>(0)?,row.get::<_,u32>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id,attempt) in &rows {
                let delay=(30_000_i64 << (*attempt).min(4)).min(300_000);
                let next=now.saturating_add(delay);
                if store.conn.execute("UPDATE deliveries SET repair_next=?2, repair_attempt=?3 WHERE id=?1 AND state='accepted' AND resolution IS NULL AND (repair_next<=?4 OR repair_next>?5)",
                    params![id,next,attempt.saturating_add(1).min(4),now,now.saturating_add(300_000)])? != 1 {
                    return Err(crate::Error::Invalid("The local repair retry could not be recorded.".into()));
                }
            }
            Ok(rows.into_iter().map(|(id,_)|id).collect())
        })
    }

    pub fn claim_delivery(&mut self, id: &str) -> Result<bool> {
        self.delivery_transaction(|store| Ok(store.conn.execute("UPDATE deliveries SET state = 'claimed' WHERE id = ?1 AND state = 'held' AND compacted_at IS NULL AND cancel_intent=0 AND NOT EXISTS (SELECT 1 FROM delivery_cancellations WHERE id = ?1)", [id])? > 0))
    }

    /// Only a provider-proven rejection permits another submission attempt.
    pub fn release_delivery(&mut self, id: &str, error: &str) -> Result<bool> {
        self.delivery_transaction(|store| Ok(store.conn.execute(
            "UPDATE deliveries SET state = 'held', error = ?2 WHERE id = ?1 AND state = 'claimed'", params![id, error])? > 0))
    }

    pub fn accept_delivery(&mut self, id: &str, raw: &[u8], source: &str) -> Result<bool> {
        self.delivery_transaction(|store| Ok(store.conn.execute(
            "UPDATE deliveries SET state = 'accepted', raw = ?2, source = ?3, error = NULL WHERE id = ?1 AND state = 'claimed'",
            params![id, raw, source])? > 0))
    }

    pub fn uncertain_delivery(&mut self, id: &str, error: &str) -> Result<bool> {
        self.delivery_transaction(|store| Ok(store.conn.execute(
            "UPDATE deliveries SET state = 'uncertain', error = ?2 WHERE id = ?1 AND state = 'claimed'", params![id, error])? > 0))
    }

    /// Apply the send's reminder once, atomically with its repair marker. A later repair
    /// must not override a user edit made after the first attempt partially succeeded.
    pub fn apply_delivery_reminder(&mut self, id: &str, thread_id: &str, message_id: &str, wait: i64, now: i64) -> Result<bool> {
        self.delivery_transaction(|store| {
            if store.conn.execute("UPDATE deliveries SET reminder_applied = 1 WHERE id = ?1 AND state = 'accepted' AND resolution IS NULL AND reminder_applied = 0", [id])? == 0 {
                return Ok(false);
            }
            let snoozed_later: bool = store.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM snooze_edits s JOIN deliveries d ON d.id = ?1
                 WHERE s.thread_id = ?2 AND s.id > d.snooze_revision)",
                params![id, crate::codec::parse_id(thread_id)], |row| row.get(0))?;
            if snoozed_later { return Ok(false); }
            match store.remind_if_no_reply_inner(thread_id, true, wait, now, Some(message_id)) {
                Err(crate::Error::NotFound { kind: "thread", .. }) => Ok(false),
                result => result,
            }
        })
    }

    /// Resolve a completed delivery to its current local conversation.
    pub fn delivery_receipt_thread(&self, id: &str) -> Result<Option<String>> {
        // The message identity survives merges. A cached thread id can predate a merge,
        // or even have been reused after local deletion, so it is not replay authority.
        Ok(self.conn.query_row(
            "SELECT m.thread_id FROM deliveries d JOIN messages m
             ON m.account_id=d.account_id AND m.message_id=trim(d.message_id,'<>')
             WHERE d.id=?1 AND d.state='complete' AND d.account_retired=0 AND m.from_me=1",
            [id], |row| row.get::<_, i64>(0).map(|id| id.to_string()),
        ).optional()?)
    }

    pub fn complete_delivery(&mut self, id: &str) -> Result<bool> {
        self.complete_delivery_with_receipt(id, None)
    }

    pub fn complete_delivery_with_receipt(&mut self, id: &str, thread_id: Option<&str>) -> Result<bool> {
        self.delivery_transaction(|store| Ok(store.conn.execute(
            "UPDATE deliveries SET state = 'complete', raw = NULL, source = NULL, thread_id = ?2 WHERE id = ?1 AND state = 'accepted' AND resolution IS NULL", params![id, thread_id])? > 0))
    }

    /// A committed cancellation intent blocks submission even if draft restoration fails.
    pub fn delivery_cancellation_recorded(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE id = ?1)", [id], |row| row.get(0))?)
    }

    /// A cancellation marker only proves no submission for a matching unsent state.
    pub fn delivery_stopped(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE id=?1 AND state='cancelled') OR (EXISTS(SELECT 1 FROM delivery_cancellations WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM deliveries WHERE id=?1 AND NOT(state='held' OR (state='uncertain' AND interrupted_from IS 'held'))))",[id],|row|row.get(0))?)
    }

    /// Review receipts remain readable after their content has been compacted.
    pub fn delivery_resolved(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE id=?1 AND resolution IS NOT NULL)", [id], |row| row.get(0))?)
    }

    /// Metadata only: malformed saved content must not hide an unsent recovery.
    pub fn delivery_restoration_pending(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE id=?1 AND resolution IS NULL AND (state='held' OR (state='uncertain' AND interrupted_from='held')) AND EXISTS(SELECT 1 FROM delivery_cancellations WHERE id=?1))", [id], |row| row.get(0))?)
    }

    /// Moss stopped (a crash or force quit) while this email waited for Undo: it goes back to
    /// Drafts unsent, as Undo would, and is noted for you with [`Store::interrupted_sends`].
    pub fn cancel_interrupted_delivery(&mut self, id: &str, now: i64) -> Result<bool> {
        self.cancel_delivery_noting(id, Some(now))
    }

    /// Emails Moss stopped before sending, oldest first, until you have seen them.
    pub fn interrupted_sends(&self) -> Result<Vec<InterruptedSend>> {
        let mut statement = self.conn.prepare("SELECT id, subject, at FROM send_interruptions ORDER BY at, id")?;
        let rows = statement.query_map([], |r| Ok(InterruptedSend { id: r.get(0)?, subject: r.get(1)?, at: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// You have seen these: they aren't noted again.
    pub fn dismiss_interrupted_sends(&mut self, ids: &[String]) -> Result<()> {
        self.delivery_transaction(|store| {
            for id in ids { store.conn.execute("DELETE FROM send_interruptions WHERE id=?1", [id])?; }
            Ok(())
        })
    }

    pub fn cancel_delivery(&mut self, id: &str) -> Result<bool> {
        self.cancel_delivery_noting(id, None)
    }

    /// With `note`, the restored draft is also noted as a send Moss stopped, at that time.
    fn cancel_delivery_noting(&mut self, id: &str, note: Option<i64>) -> Result<bool> {
        // Commit the no-send intent independently. Rolling back a failed restoration
        // must never give a released timer permission to claim this delivery again.
        let stopped = self.delivery_transaction(|store| {
            if store.is_scheduled_delivery(id)? { return Ok(false); }
            let state = store.delivery_state(id)?;
            if state.is_some_and(|state| !matches!(state, DeliveryState::Held | DeliveryState::Cancelled)) { return Ok(false); }
            store.conn.execute("INSERT OR IGNORE INTO delivery_cancellations (id) VALUES (?1)", [id])?;
            if !store.delivery_cancellation_recorded(id)? { return Err(crate::Error::Invalid("The cancellation was not saved.".into())); }
            store.conn.execute("UPDATE deliveries SET cancel_intent=1 WHERE id=?1",[id])?;
            let missing: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE id=?1 AND cancel_intent!=1)",[id],|row|row.get(0))?;
            if missing { return Err(crate::Error::Invalid("The cancellation could not be indexed. Try Undo again.".into())); }
            Ok(true)
        })?;
        if !stopped { return Ok(false); }
        self.delivery_transaction(|store| {
            if store.delivery_state(id)? == Some(DeliveryState::Cancelled) && store.delivery_draft_id(id)?.is_some() { return Ok(false); }
            // Column 1 in COLUMNS is the draft JSON. Only its decoding failure is
            // irrecoverable payload damage; storage failures must remain retryable.
            let kept = store.delivery(id).map_err(|error| match error {
                crate::Error::Sqlite(rusqlite::Error::FromSqlConversionFailure(1,_,_))=>crate::Error::UnreadableDeliveryDraft,
                error=>error,
            })?;
            if kept.as_ref().is_some_and(|d| d.state != DeliveryState::Held) { return Ok(false); }
            if let Some(kept) = kept {
                store.conn.execute("UPDATE deliveries SET state = 'cancelled' WHERE id = ?1", [id])?;
                let mut draft = kept.draft;
                if let Some(thread) = &draft.thread_id {
                    let exists: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM threads WHERE id = ?1)", [thread], |row| row.get(0))?;
                    if !exists { draft.thread_id = None; }
                }
                store.save_draft_in_transaction(&draft)?;
                if let Some(at) = note {
                    store.conn.execute("INSERT INTO send_interruptions(id, account_id, subject, at) VALUES (?1, ?2, ?3, ?4)", params![id, draft.from_account_id, draft.subject, at])?;
                }
            }
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::*;
    use crate::{DeliveryState, InterruptedSend, Store};

    #[test]
    fn a_send_moss_stopped_during_undo_is_noted_once_until_you_have_seen_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        for id in ["first", "second"] {
            let d = moss_core::Draft { subject: format!("About {id}"), ..draft(id) };
            store.prepare_delivery(id, &d, "{}", &format!("{id}@moss.local"), NOV_1).unwrap();
        }
        assert!(store.interrupted_sends().unwrap().is_empty());
        assert!(store.cancel_interrupted_delivery("second", NOV_1 + 2).unwrap());
        assert!(store.cancel_interrupted_delivery("first", NOV_1 + 1).unwrap());
        assert!(!store.cancel_interrupted_delivery("first", NOV_1 + 3).unwrap(), "once");
        assert_eq!(store.delivery_state("first").unwrap(), Some(DeliveryState::Cancelled));
        assert_eq!(store.draft("first").unwrap().unwrap().subject, "About first", "back in Drafts");
        let noted = vec![
            InterruptedSend { id: "first".into(), subject: "About first".into(), at: NOV_1 + 1 },
            InterruptedSend { id: "second".into(), subject: "About second".into(), at: NOV_1 + 2 },
        ];
        assert_eq!(store.interrupted_sends().unwrap(), noted);
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.interrupted_sends().unwrap(), noted, "until you have seen it");
        store.dismiss_interrupted_sends(&["first".into(), "unknown".into()]).unwrap();
        assert_eq!(store.interrupted_sends().unwrap(), noted[1..]);
        // An Undo you pressed yourself is no news.
        store.prepare_delivery("undone", &draft("undone"), "{}", "undone@moss.local", NOV_1).unwrap();
        assert!(store.cancel_delivery("undone").unwrap());
        assert_eq!(store.interrupted_sends().unwrap(), noted[1..]);
        // Removing the account takes its notes with it.
        store.dismiss_interrupted_sends(&[]).unwrap();
        assert!(store.remove_account("work").unwrap());
        assert!(store.interrupted_sends().unwrap().is_empty());
    }

    #[test]
    fn a_note_that_cannot_be_saved_keeps_the_send_stopped_and_fails() {
        let mut store = store_with_work();
        store.prepare_delivery("op", &draft("d"), "{}", "d@moss.local", NOV_1).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_note BEFORE INSERT ON send_interruptions BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.cancel_interrupted_delivery("op", NOV_1).is_err());
        assert!(store.delivery_stopped("op").unwrap(), "never sent");
        assert!(store.interrupted_sends().unwrap().is_empty());
        store.conn.execute_batch("DROP TRIGGER refuse_note").unwrap();
        assert!(store.cancel_interrupted_delivery("op", NOV_1).unwrap());
        assert_eq!(store.interrupted_sends().unwrap().len(), 1);
        break_schema(&store, "DROP TABLE send_interruptions");
        assert!(store.interrupted_sends().is_err());
        assert!(store.dismiss_interrupted_sends(&["op".into()]).is_err());
    }

    #[test]
    fn cancelled_replay_does_not_decode_retired_payloads() {
        let mut store=store_with_work();
        store.prepare_delivery("retired-cancel",&draft("retired-draft"),"{}","fixture@example.test",0).unwrap();
        assert!(store.cancel_delivery("retired-cancel").unwrap());
        store.conn.execute("UPDATE deliveries SET draft='',outgoing='' WHERE id='retired-cancel'",[]).unwrap();
        assert!(!store.cancel_delivery("retired-cancel").unwrap());
        assert_eq!(store.delivery_state("retired-cancel").unwrap(),Some(DeliveryState::Cancelled));
        assert!(!store.claim_delivery("retired-cancel").unwrap());
    }

    #[test]
    fn cancellation_flag_and_tombstone_commit_together_or_not_at_all() {
        for action in ["IGNORE","ABORT, 'fixture'"] {
            let mut store=store_with_work();
            store.prepare_delivery("flag-fault",&draft("flag-draft"),"{}","flag@example.test",NOV_1).unwrap();
            store.conn.execute_batch(&format!("CREATE TRIGGER refuse_flag BEFORE UPDATE OF cancel_intent ON deliveries BEGIN SELECT RAISE({action}); END;")).unwrap();
            assert!(store.cancel_delivery("flag-fault").is_err());
            assert!(!store.delivery_cancellation_recorded("flag-fault").unwrap());
            assert_eq!(store.delivery_state("flag-fault").unwrap(),Some(DeliveryState::Held));
            assert_eq!(store.conn.query_row("SELECT cancel_intent FROM deliveries WHERE id='flag-fault'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
            store.conn.execute_batch("DROP TRIGGER refuse_flag").unwrap();
            assert!(store.cancel_delivery("flag-fault").unwrap());
            assert!(store.delivery_cancellation_recorded("flag-fault").unwrap());
            assert_eq!(store.conn.query_row("SELECT cancel_intent FROM deliveries WHERE id='flag-fault'",[],|row|row.get::<_,i64>(0)).unwrap(),1);
            assert!(!store.claim_delivery("flag-fault").unwrap());
        }
    }

    #[test]
    fn completed_receipts_resolve_only_the_sending_accounts_sent_copy() {
        let mut store=store_with_work();
        store.upsert_account(&account("other","maya@northwind.co")).unwrap();
        let bytes=mail().id("shared-receipt@example.test").from("maya@northwind.co").build();
        let other=store.ingest(&bytes,&moss_core::MessageSource {account_id:"other".into(),mailbox:"Sent".into(),..source("other-copy")}).unwrap();
        let own=store.ingest(&bytes,&moss_core::MessageSource {mailbox:"Sent".into(),..source("own-copy")}).unwrap();
        assert_ne!(other.thread_id,own.thread_id);
        store.prepare_delivery("shared-receipt",&draft("shared-receipt-draft"),"{}","<shared-receipt@example.test>",NOV_1).unwrap();
        assert_eq!(store.delivery_receipt_thread("shared-receipt").unwrap(),None);
        store.claim_delivery("shared-receipt").unwrap();
        store.accept_delivery("shared-receipt",&bytes,"{}").unwrap();
        store.complete_delivery_with_receipt("shared-receipt",Some(&own.thread_id)).unwrap();
        assert_eq!(store.delivery_receipt_thread("shared-receipt").unwrap(),Some(own.thread_id));
        store.conn.execute("DELETE FROM messages WHERE account_id='work'",[]).unwrap();
        assert_eq!(store.delivery_receipt_thread("shared-receipt").unwrap(),None);
        assert_eq!(store.delivery_receipt_thread("missing").unwrap(),None);
        break_schema(&store,"DROP TABLE deliveries");
        assert!(store.delivery_receipt_thread("shared-receipt").is_err());
    }

    #[test]
    fn accepted_repair_batches_are_bounded_fair_and_back_off() {
        let mut store=store_with_work();
        for n in 0..18 {
            let id=format!("repair-{n:02}");
            store.prepare_delivery(&id,&draft(&format!("draft-{n}")),"{}","stable@example.test",NOV_1).unwrap();
            if n < 17 {
                store.claim_delivery(&id).unwrap();
                store.accept_delivery(&id,b"accepted","{}").unwrap();
            }
        }
        let first=store.claim_delivery_repairs(NOV_1).unwrap();
        assert_eq!(first.len(),16);
        assert_eq!(store.claim_delivery_repairs(NOV_1).unwrap(),vec!["repair-16"]);
        assert!(store.claim_delivery_repairs(NOV_1+29_999).unwrap().is_empty());
        assert_eq!(store.claim_delivery_repairs(NOV_1+30_000).unwrap(),first);
        store.claim_delivery_repairs(NOV_1+30_000).unwrap();
        assert!(store.claim_delivery_repairs(NOV_1+89_999).unwrap().is_empty());
        assert_eq!(store.claim_delivery_repairs(NOV_1+90_000).unwrap().len(),16);
        store.claim_delivery_repairs(NOV_1+90_000).unwrap();
        for at in [210_000,450_000,750_000,1_050_000] {
            assert!(store.claim_delivery_repairs(NOV_1+at-1).unwrap().is_empty());
            assert_eq!(store.claim_delivery_repairs(NOV_1+at).unwrap().len(),16);
            assert_eq!(store.claim_delivery_repairs(NOV_1+at).unwrap().len(),1);
        }
        assert_eq!(store.delivery_state("repair-17").unwrap(),Some(DeliveryState::Held));
    }

    #[test]
    fn repair_backoff_survives_reopen_and_failed_reservations_roll_back() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("moss.db");
        let mut store=Store::open(&path).unwrap();
        store.upsert_account(&account("work","maya@northwind.co")).unwrap();
        for id in ["a","b"] {
            store.prepare_delivery(id,&draft(id),"{}","receipt@example.test",NOV_1).unwrap();
            store.claim_delivery(id).unwrap();
            store.accept_delivery(id,b"copy","{}").unwrap();
        }
        for refusal in ["RAISE(ABORT,'fixture')","RAISE(IGNORE)"] {
            store.conn.execute_batch(&format!("CREATE TRIGGER refuse_repair BEFORE UPDATE OF repair_next ON deliveries WHEN OLD.id='b' BEGIN SELECT {refusal}; END")).unwrap();
            assert!(store.claim_delivery_repairs(NOV_1).is_err());
            assert_eq!(store.conn.query_row("SELECT sum(repair_attempt) FROM deliveries",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            store.conn.execute_batch("DROP TRIGGER refuse_repair").unwrap();
        }
        assert_eq!(store.claim_delivery_repairs(NOV_1).unwrap(),vec!["a","b"]);
        drop(store);
        let mut reopened=Store::open(&path).unwrap();
        assert!(reopened.claim_delivery_repairs(NOV_1+29_999).unwrap().is_empty());
        assert_eq!(reopened.claim_delivery_repairs(NOV_1+30_000).unwrap(),vec!["a","b"]);
        break_schema(&reopened,"DROP TABLE deliveries");
        assert!(reopened.claim_delivery_repairs(NOV_1+90_000).is_err());
    }

    #[test]
    fn repair_reservations_recover_from_clock_correction_and_prioritize_fresh_mail() {
        let mut store=store_with_work();
        for n in 0..17 {
            let id=format!("clock-{n:02}");
            store.prepare_delivery(&id,&draft(&id),"{}","clock@example.test",NOV_1).unwrap();
            store.claim_delivery(&id).unwrap();
            store.accept_delivery(&id,b"accepted","{}").unwrap();
            if n==15 { assert_eq!(store.claim_delivery_repairs(NOV_1+DAY).unwrap().len(),16); }
        }
        let corrected=store.claim_delivery_repairs(NOV_1).unwrap();
        assert_eq!(corrected.len(),16);
        assert_eq!(corrected[0],"clock-16","fresh acceptance must precede corrupt or deferred backlog");
        assert_eq!(store.claim_delivery_repairs(NOV_1).unwrap().len(),1);
        assert!(store.claim_delivery_repairs(NOV_1).unwrap().is_empty());
    }

    #[test]
    fn interrupted_attention_preserves_payloads_and_checks_ignored_writes() {
        let mut store=store_with_work();
        let d=draft("interrupted");
        store.prepare_delivery("interrupted",&d,"saved outgoing","receipt@example.test",NOV_1).unwrap();
        store.conn.execute("UPDATE deliveries SET draft='unreadable' WHERE id='interrupted'",[]).unwrap();
        for refusal in ["RAISE(ABORT,'fixture')","RAISE(IGNORE)"] {
            store.conn.execute_batch(&format!("CREATE TRIGGER refuse_attention BEFORE UPDATE OF state ON deliveries BEGIN SELECT {refusal}; END")).unwrap();
            assert!(store.retain_interrupted_delivery("interrupted","Needs review").is_err());
            assert_eq!(store.delivery_state("interrupted").unwrap(),Some(DeliveryState::Held));
            store.conn.execute_batch("DROP TRIGGER refuse_attention").unwrap();
        }
        assert_eq!(store.interrupted_deliveries().unwrap(),vec![("interrupted".into(),DeliveryState::Held,"work".into())]);
        store.retain_interrupted_delivery("interrupted","Needs review").unwrap();
        assert!(store.interrupted_deliveries().unwrap().is_empty());
        assert_eq!(store.delivery_state("interrupted").unwrap(),Some(DeliveryState::Uncertain));
        assert_eq!(store.conn.query_row("SELECT interrupted_from FROM deliveries WHERE id='interrupted'",[],|r|r.get::<_,String>(0)).unwrap(),"held");
        assert_eq!(store.conn.query_row("SELECT draft,outgoing,error FROM deliveries WHERE id='interrupted'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap(),("unreadable".into(),"saved outgoing".into(),"Needs review".into()));
        store.retain_interrupted_delivery("missing","No operation").unwrap();
        store.retain_interrupted_delivery("interrupted","Do not overwrite").unwrap();
        assert_eq!(store.conn.query_row("SELECT error FROM deliveries WHERE id='interrupted'",[],|r|r.get::<_,String>(0)).unwrap(),"Needs review");
        break_schema(&store,"DROP TABLE deliveries");
        assert!(store.interrupted_deliveries().is_err());
        assert!(store.retain_interrupted_delivery("interrupted","Unreadable").is_err());
    }

    #[test]
    fn interrupted_restore_does_not_classify_storage_read_errors_as_payload_damage() {
        let mut store=store_with_work();
        store.prepare_delivery("read-error",&draft("read-error"),"outgoing bytes","read@example.test",NOV_1).unwrap();
        store.conn.execute_batch("ALTER TABLE deliveries RENAME COLUMN outgoing TO unavailable_outgoing").unwrap();
        assert!(matches!(store.cancel_delivery("read-error"),Err(crate::Error::Sqlite(_))));
        assert_eq!(store.delivery_state("read-error").unwrap(),Some(DeliveryState::Held));
        assert_eq!(store.conn.query_row("SELECT unavailable_outgoing FROM deliveries WHERE id='read-error'",[],|r|r.get::<_,String>(0)).unwrap(),"outgoing bytes");
    }

    fn work_cleanup(_: &Store) -> crate::Result<Option<(String, Vec<String>)>> {
        Ok(Some(("work".into(), vec!["cleanup".into()])))
    }

    #[test]
    fn early_cancellation_must_retain_its_record_before_reporting_success() {
        let mut store = store_with_work();
        store.conn.execute_batch("CREATE TRIGGER ignore_cancel BEFORE INSERT ON delivery_cancellations BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.cancel_delivery("early").is_err());
        assert_eq!(store.delivery_state("early").unwrap(), None);
        store.conn.execute_batch("DROP TRIGGER ignore_cancel").unwrap();
        assert!(store.cancel_delivery("early").unwrap());
        assert_eq!(store.delivery_state("early").unwrap(), Some(DeliveryState::Cancelled));
        assert!(!store.prepare_delivery("early", &draft("later"), "{}", "mail@example.test", NOV_1).unwrap());
    }

    #[test]
    fn a_retained_cancellation_prevents_claiming_even_if_the_row_is_held() {
        let mut store = store_with_work();
        store.prepare_delivery("operation", &draft("held"), "{}", "mail@example.test", NOV_1).unwrap();
        store.conn.execute("INSERT INTO delivery_cancellations (id) VALUES ('operation')", []).unwrap();
        assert!(!store.claim_delivery("operation").unwrap());
        assert_eq!(store.delivery("operation").unwrap().unwrap().state, DeliveryState::Held);
        assert_eq!(store.delivery_state("operation").unwrap(), Some(DeliveryState::Held), "restoration is still pending despite the no-send intent");
    }

    #[test]
    fn ignored_held_cancellation_updates_already_roll_back_through_restoration() {
        for scheduled in [false, true] {
            let mut store = store_with_work();
            let d = draft("held");
            if scheduled { store.schedule_operation("operation", &d, "{}", NOV_1, NOV_1).unwrap(); }
            store.prepare_delivery("operation", &d, "{}", "mail@example.test", NOV_1).unwrap();
            store.conn.execute_batch("CREATE TRIGGER ignore_cancel BEFORE UPDATE OF state ON deliveries WHEN NEW.state = 'cancelled' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            if scheduled { assert!(store.unschedule_email(&d.id, NOV_1).is_err()); }
            else { assert!(store.cancel_delivery("operation").is_err()); }
            assert_eq!(store.delivery("operation").unwrap().unwrap().state, DeliveryState::Held);
        assert_eq!(store.delivery_state("operation").unwrap(), Some(DeliveryState::Held), "restoration is still pending despite the no-send intent");
            assert_eq!(store.conn.query_row("SELECT count(*) FROM delivery_cancellations", [], |r| r.get::<_, i64>(0)).unwrap(), i64::from(!scheduled));
            assert_eq!(store.conn.query_row("SELECT count(*) FROM schedule_outcomes", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
            assert_eq!(store.scheduled_emails().unwrap().len(), usize::from(scheduled));
            assert!(store.drafts(None).unwrap().is_empty());
        }
    }

    #[test]
    fn merging_three_threads_keeps_real_order_without_inventing_an_edit() {
        for edited in [false, true] {
            let mut store = store_with_work();
            let a = ingest(&mut store, "a", &mail().id("a@example.test").subject("A").build());
            let b = ingest(&mut store, "b", &mail().id("b@example.test").subject("B").build());
            let c = ingest(&mut store, "c", &mail().id("c@example.test").subject("C").build());
            if edited {
                for thread in [&a.thread_id, &c.thread_id, &b.thread_id] { store.snooze(thread, None).unwrap(); }
            }
            let before: i64 = store.conn.query_row("SELECT coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'snooze_edits'), 0)", [], |r| r.get(0)).unwrap();
            let sent = ingest(&mut store, "sent", &mail().id("sent@example.test")
                .reply_to(&["a@example.test", "b@example.test", "c@example.test"]).build());
            assert_eq!(sent.thread_id, a.thread_id);
            let count: i64 = store.conn.query_row("SELECT count(*) FROM snooze_edits", [], |r| r.get(0)).unwrap();
            assert_eq!(count, i64::from(edited));
            let after: i64 = store.conn.query_row("SELECT coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'snooze_edits'), 0)", [], |r| r.get(0)).unwrap();
            assert_eq!(after, before, "merging is not a new user action");
            if edited {
                let kept: i64 = store.conn.query_row("SELECT id FROM snooze_edits WHERE thread_id = ?1", [&sent.thread_id], |r| r.get(0)).unwrap();
                assert_eq!(kept, before);
            }
        }
    }

    #[test]
    fn snooze_order_survives_deletion_reopen_and_the_undo_hold() {
        for retry in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("mail.db");
            let mut store = Store::open(&path).unwrap();
            store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
            let removed = ingest(&mut store, "removed", &mail().id("removed@example.test").subject("Removed").build());
            store.snooze(&removed.thread_id, Some(NOV_1 + DAY)).unwrap();
            let old: i64 = store.conn.query_row("SELECT id FROM snooze_edits", [], |r| r.get(0)).unwrap();
            store.conn.execute("DELETE FROM threads WHERE id = ?1", [&removed.thread_id]).unwrap();
            store.conn.execute_batch("VACUUM").unwrap();
            store.prepare_delivery("op", &draft("d1"), "{}", "sent", NOV_1).unwrap();
            let captured: i64 = store.conn.query_row("SELECT snooze_revision FROM deliveries WHERE id = 'op'", [], |r| r.get(0)).unwrap();
            assert_eq!(captured, old, "deleted edits must not reset the sequence");
            let sent = ingest(&mut store, "sent", &mail().id("sent@example.test")
                .from("Maya Ross <maya@northwind.co>").to("Daniel Okafor <daniel@northwind.co>").at(NOV_1).build());
            if retry {
                assert!(store.claim_delivery("op").unwrap());
                assert!(store.release_delivery("op", "definite refusal").unwrap());
            }
            store.mark_done(&sent.thread_id, false, NOV_1).unwrap();
            store.snooze(&sent.thread_id, None).unwrap();
            let newer: i64 = store.conn.query_row("SELECT id FROM snooze_edits", [], |r| r.get(0)).unwrap();
            assert!(newer > captured);
            drop(store);
            let mut store = Store::open(&path).unwrap();
            store.claim_delivery("op").unwrap();
            store.accept_delivery("op", b"accepted", "{}").unwrap();
            assert!(!store.apply_delivery_reminder("op", &sent.thread_id, &sent.message_id, DAY, NOV_1).unwrap(),
                "a choice after Send preparation wins, even during Undo or after restart");
            let thread = store.thread(&sent.thread_id, NOV_1).unwrap().unwrap();
            assert!(thread.summary.waiting.is_none());
            assert_eq!(thread.summary.snoozed_until, None);
        }
    }

    #[test]
    fn reminder_ordering_preserves_later_choices_across_thread_merges() {
        for edit_keep_last in [false, true] {
            let mut store = store_with_work();
            let first = ingest(&mut store, "first", &mail().id("first@example.test").subject("First").at(NOV_1).build());
            let second = ingest(&mut store, "second", &mail().id("second@example.test").subject("Second").at(NOV_1).build());
            store.prepare_delivery("op", &draft("d1"), "{}", "sent", NOV_1).unwrap();
            let (older, newer) = if edit_keep_last { (&second.thread_id, &first.thread_id) } else { (&first.thread_id, &second.thread_id) };
            store.snooze(older, Some(NOV_1 + DAY)).unwrap();
            store.snooze(newer, None).unwrap();
            let newest: i64 = store.conn.query_row("SELECT max(id) FROM snooze_edits", [], |r| r.get(0)).unwrap();
            let sent = ingest(&mut store, "sent", &mail().id("sent@example.test")
                .from("Maya Ross <maya@northwind.co>").to("Daniel Okafor <daniel@northwind.co>")
                .reply_to(&["first@example.test", "second@example.test"]).at(NOV_1 + 1_000).build());
            assert_eq!(sent.thread_id, first.thread_id);
            let kept: i64 = store.conn.query_row("SELECT id FROM snooze_edits WHERE thread_id = ?1", [&sent.thread_id], |r| r.get(0)).unwrap();
            assert_eq!(kept, newest, "merging threads must preserve the actual latest user choice");
            store.claim_delivery("op").unwrap();
            store.accept_delivery("op", b"accepted", "{}").unwrap();
            assert!(!store.apply_delivery_reminder("op", &sent.thread_id, &sent.message_id, 4 * DAY, NOV_1).unwrap());
            assert!(store.thread(&sent.thread_id, NOV_1).unwrap().unwrap().summary.waiting.is_none());
        }
    }

    #[test]
    fn older_and_unrelated_snoozes_do_not_suppress_a_new_send_reminder() {
        let mut store = store_with_work();
        let target = ingest(&mut store, "target", &mail().id("target@example.test").subject("Target").at(NOV_1).build());
        let other = ingest(&mut store, "other", &mail().id("other@example.test").subject("Other").at(NOV_1).build());
        store.snooze(&target.thread_id, Some(NOV_1 + DAY)).unwrap();
        store.prepare_delivery("op", &draft("d1"), "{}", "sent", NOV_1).unwrap();
        store.snooze(&other.thread_id, None).unwrap();
        let sent = ingest(&mut store, "sent", &mail().id("sent@example.test")
            .from("Maya Ross <maya@northwind.co>").to("Daniel Okafor <daniel@northwind.co>")
            .reply_to(&["target@example.test"]).at(NOV_1 + 1_000).build());
        store.claim_delivery("op").unwrap();
        store.accept_delivery("op", b"accepted", "{}").unwrap();
        assert!(store.apply_delivery_reminder("op", &sent.thread_id, &sent.message_id, 4 * DAY, NOV_1).unwrap());
        let thread = store.thread(&target.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.summary.snoozed_until, Some(NOV_1 + 1_000 + 4 * DAY));
        assert!(thread.summary.waiting.is_some());
    }

    #[test]
    fn reminder_repair_skips_unaccepted_missing_or_answered_mail() {
        for missing in [false, true] {
            let mut store = store_with_work();
            let incoming = ingest(&mut store, "incoming", &mail().id("incoming@example.test").at(NOV_1).build());
            let thread = if missing { "missing-thread" } else { incoming.thread_id.as_str() };
            store.prepare_delivery("op", &draft("d1"), "{}", "id", NOV_1).unwrap();
            assert!(!store.apply_delivery_reminder("op", thread, &incoming.message_id, DAY, NOV_1).unwrap());
            let applied: i64 = store.conn.query_row("SELECT reminder_applied FROM deliveries WHERE id = 'op'", [], |r| r.get(0)).unwrap();
            assert_eq!(applied, 0, "unaccepted mail cannot consume the reminder step");
            store.claim_delivery("op").unwrap();
            store.accept_delivery("op", b"accepted", "{}").unwrap();
            assert!(!store.apply_delivery_reminder("op", thread, &incoming.message_id, DAY, NOV_1).unwrap());
            let applied: i64 = store.conn.query_row("SELECT reminder_applied FROM deliveries WHERE id = 'op'", [], |r| r.get(0)).unwrap();
            assert_eq!(applied, 1, "missing or answered mail must not keep retrying a reminder");
            assert!(store.thread(&incoming.thread_id, NOV_1).unwrap().unwrap().summary.waiting.is_none());
            assert!(!store.apply_delivery_reminder("op", thread, &incoming.message_id, DAY, NOV_1).unwrap());
        }
    }

    #[test]
    fn restoring_performance_settings_cannot_reverse_a_committed_transition() {
        let restoration_error = || rusqlite::Error::InvalidParameterName("fixture".into());
        assert_eq!(super::finished_transition(Ok(true), Err(restoration_error())).unwrap(), true);
        let failed: crate::Result<()> = Err(crate::Error::Invalid("commit failed".into()));
        assert!(matches!(super::finished_transition(failed, Err(restoration_error())), Err(crate::Error::Invalid(reason)) if reason == "commit failed"));
        assert_eq!(super::finished_transition(Ok(false), Ok(())).unwrap(), false);
    }

    #[test]
    fn delivery_transitions_sync_to_disk_and_restore_normal_mode_after_failure() {
        let mut store = store_with_work();
        store.conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
        store.conn.pragma_update(None, "fullfsync", false).unwrap();
        for fail in [false, true] {
            let result = store.delivery_transaction(|s| {
                let synchronous: i64 = s.conn.pragma_query_value(None, "synchronous", |r| r.get(0))?;
                let fullfsync: i64 = s.conn.pragma_query_value(None, "fullfsync", |r| r.get(0))?;
                assert_eq!(synchronous, 2);
                assert_eq!(fullfsync, 1, "macOS must flush the device cache for critical delivery commits");
                if fail { Err(crate::Error::Invalid("fixture".into())) } else { Ok(()) }
            });
            assert_eq!(result.is_err(), fail);
            assert_eq!(store.conn.pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0)).unwrap(), 1);
            assert_eq!(store.conn.pragma_query_value(None, "fullfsync", |r| r.get::<_, i64>(0)).unwrap(), 0);
        }
    }

    #[test]
    fn ignored_preparation_keeps_the_draft_and_does_not_queue_server_deletion() {
        let mut store = store_with_work();
        let saved = draft("ignored-preparation");
        store.save_draft(&saved).unwrap();
        store.conn.execute_batch("CREATE TRIGGER ignore_preparation BEFORE INSERT ON deliveries BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        let before: i64 = store.conn.query_row("SELECT count(*) FROM outbox", [], |row| row.get(0)).unwrap();
        assert!(!store.prepare_delivery_with_change("ignored-operation", &saved, "{}", "ignored@fixture", NOV_1, work_cleanup).unwrap());
        assert_eq!(store.draft(&saved.id).unwrap(), Some(saved));
        assert!(store.delivery("ignored-operation").unwrap().is_none());
        assert_eq!(store.conn.query_row("SELECT count(*) FROM outbox", [], |row| row.get::<_, i64>(0)).unwrap(), before);
    }

    #[test]
    fn a_delivery_survives_reopen_and_can_only_be_claimed_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "maya@example.test")).unwrap();
        let d = draft("d1");
        store.save_draft(&d).unwrap();
        assert!(store.prepare_delivery("op1", &d, "outgoing-json", "stable@example.test", NOV_1).unwrap());
        assert!(!store.prepare_delivery("op1", &d, "outgoing-json", "stable@example.test", NOV_1).unwrap());
        assert!(store.drafts(None).unwrap().is_empty());
        drop(store);
        let mut store = Store::open(&path).unwrap();
        let kept = store.delivery("op1").unwrap().unwrap();
        assert_eq!(kept.draft, d);
        assert_eq!(kept.outgoing, "outgoing-json");
        assert_eq!(kept.message_id, "stable@example.test");
        assert_eq!(kept.state, DeliveryState::Held);
        assert!(store.claim_delivery("op1").unwrap());
        assert!(!store.claim_delivery("op1").unwrap());
        assert!(!store.claim_delivery("unknown").unwrap());
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.pending_deliveries().unwrap()[0].state, DeliveryState::Claimed);
    }

    #[test]
    fn preparation_and_server_cleanup_are_one_commit() {
        for action in ["ABORT, 'fixture'", "IGNORE"] {
            let mut store = store_with_work();
            let d = draft("d1");
            store.save_draft(&d).unwrap();
            store.conn.execute_batch(&format!("CREATE TRIGGER refuse_cleanup BEFORE INSERT ON outbox BEGIN SELECT RAISE({action}); END;")).unwrap();
            assert!(store.prepare_delivery_with_change("op", &d, "{}", "id", NOV_1, work_cleanup).is_err());
            assert!(store.delivery("op").unwrap().is_none());
            assert_eq!(store.drafts(None).unwrap(), [d.clone()]);
            store.conn.execute_batch("DROP TRIGGER refuse_cleanup").unwrap();
            assert!(store.prepare_delivery_with_change("op", &d, "{}", "id", NOV_1, work_cleanup).unwrap());
            assert_eq!(store.pending_changes("work").unwrap().len(), 1);
            assert!(!store.prepare_delivery_with_change("op", &d, "{}", "id", NOV_1, work_cleanup).unwrap());
            assert_eq!(store.pending_changes("work").unwrap().len(), 1);
        }
    }

    #[test]
    fn state_checks_do_not_load_or_decode_the_mail_payload() {
        let mut store = store_with_work();
        store.prepare_delivery("op", &draft("d1"), "{}", "id", NOV_1).unwrap();
        store.conn.execute("UPDATE deliveries SET draft = 'unreadable payload' WHERE id = 'op'", []).unwrap();
        assert!(store.delivery("op").is_err());
        assert_eq!(store.delivery_state("op").unwrap(), Some(DeliveryState::Held));
        assert_eq!(store.delivery_state("absent").unwrap(), None);
    }

    #[test]
    fn a_scheduled_draft_can_only_be_prepared_by_its_own_operation() {
        let mut store = store_with_work();
        let d = draft("d1");
        store.schedule_email(&d, "{}", NOV_1 + 60_000, NOV_1).unwrap();
        assert!(matches!(store.prepare_delivery("stale-composer", &d, "{}", "id", NOV_1), Err(crate::Error::Invalid(_))));
        assert!(store.delivery("stale-composer").unwrap().is_none());
        let operation = store.scheduled_operation(&d.id).unwrap().unwrap();
        assert!(store.prepare_delivery(&operation, &d, "{}", "id", NOV_1).unwrap());
        assert!(store.is_scheduled_delivery(&operation).unwrap());
    }

    #[test]
    fn stale_autosaves_cannot_recreate_mail_owned_by_a_delivery() {
        let mut store = store_with_work();
        let d = draft("d1");
        store.save_draft(&d).unwrap();
        store.prepare_delivery("op", &d, "{}", "id", NOV_1).unwrap();
        assert!(store.save_draft(&d).is_err(), "a late autosave must not resurrect held mail");
        assert!(store.drafts(None).unwrap().is_empty());
        assert!(store.prepare_delivery("duplicate", &d, "{}", "other", NOV_1).is_err());
        assert!(store.cancel_delivery("op").unwrap());
        store.save_draft(&d).unwrap();
        store.prepare_delivery("next", &d, "{}", "next-id", NOV_1).unwrap();
        store.claim_delivery("next").unwrap();
        assert!(store.save_draft(&d).is_err());
        store.accept_delivery("next", b"mail", "{}").unwrap();
        assert!(store.save_draft(&d).is_err());
        store.complete_delivery("next").unwrap();
        assert!(store.save_draft(&d).is_err(), "a late autosave must not resurrect sent mail either");
    }

    #[test]
    fn reused_operation_ids_require_the_identical_delivery() {
        let mut store = store_with_work();
        let original = draft("d1");
        store.prepare_delivery("op", &original, "payload", "message", NOV_1).unwrap();
        let mut changed = original.clone();
        changed.subject = "Different mail".into();
        let unrelated = draft("still-unsent");
        store.save_draft(&unrelated).unwrap();
        for (d, outgoing, message_id) in [(&changed, "payload", "message"), (&original, "changed", "message"), (&original, "payload", "different")] {
            assert!(matches!(store.prepare_delivery("op", d, outgoing, message_id, NOV_1), Err(crate::Error::Invalid(_))));
            assert_eq!(store.drafts(None).unwrap(), [unrelated.clone()], "rejection keeps unrelated unsent mail");
        }
        assert_eq!(store.delivery("op").unwrap().unwrap().draft, original);
    }

    #[test]
    fn cancellation_before_preparation_survives_reopen_and_reused_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel-before-prepare.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        let d = draft("not-yet-prepared");
        store.save_draft(&d).unwrap();
        assert!(store.cancel_delivery("early-undo").unwrap());
        assert_eq!(store.delivery_state("early-undo").unwrap(), Some(DeliveryState::Cancelled));
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(!store.prepare_delivery("early-undo", &d, "{}", "never@fixture", NOV_1 + 30 * DAY).unwrap());
        assert!(store.delivery("early-undo").unwrap().is_none());
        assert_eq!(store.draft(&d.id).unwrap(), Some(d.clone()));
        assert!(!store.claim_delivery("early-undo").unwrap());
        assert!(store.prepare_delivery("new-attempt", &d, "{}", "new@fixture", NOV_1).unwrap());
    }

    #[test]
    fn a_failed_cancellation_record_never_confirms_undo() {
        let mut store = store_with_work();
        store.conn.execute_batch("CREATE TRIGGER refuse_cancellation BEFORE INSERT ON delivery_cancellations BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.cancel_delivery("failed-cancel").is_err());
        assert_eq!(store.delivery_state("failed-cancel").unwrap(), None);
        store.conn.execute_batch("DROP TRIGGER refuse_cancellation").unwrap();
        assert!(store.prepare_delivery("failed-cancel", &draft("d1"), "{}", "not-cancelled@fixture", NOV_1).unwrap());
    }

    #[test]
    fn undo_restores_a_reply_after_its_original_thread_was_removed() {
        let mut store = store_with_work();
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        let mut d = draft("deleted-thread-undo");
        d.thread_id = Some(thread.clone());
        d.mode = moss_core::ComposeMode::Reply;
        store.prepare_delivery("deleted-thread-op", &d, "{}", "reply@fixture", NOV_1).unwrap();
        store.conn.execute("DELETE FROM threads WHERE id = ?1", [&thread]).unwrap();
        assert!(store.cancel_delivery("deleted-thread-op").unwrap());
        assert_eq!(store.delivery_state("deleted-thread-op").unwrap(), Some(DeliveryState::Cancelled));
        assert!(!store.claim_delivery("deleted-thread-op").unwrap());
        d.thread_id = None;
        assert_eq!(store.draft(&d.id).unwrap(), Some(d));
    }

    #[test]
    fn ordinary_send_cancellation_cannot_cancel_a_scheduled_operation() {
        for prepared in [false, true] {
            let mut store = store_with_work();
            let d = draft("schedule-owned-operation");
            store.schedule_email(&d, "{}", NOV_1, NOV_1).unwrap();
            let operation = store.scheduled_operation(&d.id).unwrap().unwrap();
            if prepared { store.prepare_delivery(&operation, &d, "{}", "scheduled@fixture", NOV_1).unwrap(); }
            assert!(!store.cancel_delivery(&operation).unwrap());
            let cancelled: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE id = ?1)", [&operation], |row| row.get(0)).unwrap();
            assert!(!cancelled);
            assert!(store.due_email(&d.id, NOV_1).unwrap().is_some());
            if !prepared { assert!(store.prepare_delivery(&operation, &d, "{}", "scheduled@fixture", NOV_1).unwrap()); }
            assert!(store.claim_delivery(&operation).unwrap());
        }
    }

    #[test]
    fn cancellation_restores_only_held_mail_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("undo.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "sender@example.test")).unwrap();
        let d = draft("d1");
        store.prepare_delivery("op", &d, "{}", "id", NOV_1).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_draft BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.cancel_delivery("op").is_err());
        assert_eq!(store.delivery("op").unwrap().unwrap().state, DeliveryState::Held);
        let cancelled: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM delivery_cancellations WHERE id = 'op')", [], |row| row.get(0)).unwrap();
        assert!(cancelled, "failed restoration must retain the durable no-send intent");
        assert!(!store.claim_delivery("op").unwrap(), "Undo must stop submission even when restoration fails");
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(!store.claim_delivery("op").unwrap(), "the no-send intent must survive reopening");
        assert_eq!(store.delivery("op").unwrap().unwrap().draft, d);
        store.conn.execute_batch("DROP TRIGGER refuse_draft").unwrap();
        assert!(store.cancel_delivery("op").unwrap());
        assert_eq!(store.drafts(None).unwrap(), [d.clone()]);
        assert!(!store.cancel_delivery("op").unwrap());
        store.prepare_delivery("next", &d, "{}", "id2", NOV_1).unwrap();
        store.claim_delivery("next").unwrap();
        assert!(!store.cancel_delivery("next").unwrap());
        assert!(store.drafts(None).unwrap().is_empty());
    }

    #[test]
    fn acceptance_keeps_the_sent_bytes_until_local_repair_completes() {
        let mut store = store_with_work();
        store.prepare_delivery("op", &draft("d1"), "{}", "id", NOV_1).unwrap();
        assert!(!store.accept_delivery("op", b"raw", "source-json").unwrap(), "must claim before delivery");
        store.claim_delivery("op").unwrap();
        assert!(store.accept_delivery("op", b"raw", "source-json").unwrap());
        let accepted = store.delivery("op").unwrap().unwrap();
        assert_eq!(accepted.raw.as_deref(), Some(b"raw".as_slice()));
        assert_eq!(accepted.source.as_deref(), Some("source-json"));
        assert_eq!(accepted.state, DeliveryState::Accepted);
        assert!(!store.claim_delivery("op").unwrap());
        assert!(!store.uncertain_delivery("op", "lost response").unwrap());
        assert!(store.complete_delivery("op").unwrap());
        assert!(!store.complete_delivery("op").unwrap());
        assert!(store.pending_deliveries().unwrap().is_empty());
        assert!(!store.prepare_delivery("op", &draft("d1"), "{}", "id", NOV_1).unwrap(), "completion retains the operation ID");
    }

    #[test]
    fn uncertain_mail_stays_visible_and_cannot_be_reclaimed() {
        let mut store = store_with_work();
        store.prepare_delivery("op", &draft("d1"), "{}", "id", NOV_1).unwrap();
        assert!(!store.complete_delivery("op").unwrap());
        store.claim_delivery("op").unwrap();
        assert!(store.uncertain_delivery("op", "response lost").unwrap());
        let kept = &store.pending_deliveries().unwrap()[0];
        assert_eq!(kept.state, DeliveryState::Uncertain);
        assert_eq!(kept.error.as_deref(), Some("response lost"));
        assert!(!store.claim_delivery("op").unwrap());
        assert!(!store.cancel_delivery("op").unwrap());
        assert!(!store.uncertain_delivery("unknown", "no").unwrap());
    }
}
