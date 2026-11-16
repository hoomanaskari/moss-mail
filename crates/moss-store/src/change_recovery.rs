//! Durable authority for queued server changes and explicit recovery.
use crate::{ChangeId, Error, Result, Store};
use rusqlite::{params, OptionalExtension};

/// Maximum UTF-8 JSON bytes returned by the queue reader and emitted per new part.
pub const MAX_CHANGE_PAYLOAD_BYTES: usize = 1_048_576;
pub const MAX_CHANGE_PARTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeState { Queued, Claimed, Blocked }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeFailure { Refused, Unreadable, Uncertain, TooLarge }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedChange {
    pub id: ChangeId,
    pub revision: i64,
    pub state: ChangeState,
    pub failure: Option<ChangeFailure>,
    /// None means the stored payload exceeds the bounded reader's budget.
    pub payload: Option<String>,
}

/// Queue authority without reading the serialized server action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeHead {
    pub id: ChangeId,
    pub revision: i64,
    pub state: ChangeState,
    pub failure: Option<ChangeFailure>,
}

pub(crate) fn stale() -> Error { Error::Invalid("This queued change is no longer in the expected state. Refresh its recovery status.".into()) }

fn changed_one(changed: usize) -> Result<()> {
    if changed != 1 { return Err(stale()); }
    Ok(())
}

impl Store {
    /// One read snapshot for UI status. Neither query reads action payloads.
    pub fn change_recovery_snapshot(&self, account: &str) -> Result<Option<(ChangeHead, crate::ChangeBatch)>> {
        let tx = self.conn.unchecked_transaction()?;
        let snapshot = match self.earliest_change_metadata(account)? {
            None => None,
            Some(head) => {
                let batch = self.batch_for_change(account, head.id)?.ok_or_else(stale)?;
                Some((head, batch))
            }
        };
        tx.commit()?;
        Ok(snapshot)
    }

    pub fn earliest_change_metadata(&self, account: &str) -> Result<Option<ChangeHead>> {
        Ok(self.read_change_head(account, false)?.map(|head| ChangeHead {
            id: head.id, revision: head.revision, state: head.state, failure: head.failure,
        }))
    }

    /// Reads exactly the oldest action, including blocked and interrupted attempts.
    /// The column byte count uses SQLite metadata; only bounded payloads enter Rust.
    /// Oversized intent stays visible for recovery instead of being skipped.
    pub fn earliest_change(&self, account: &str) -> Result<Option<QueuedChange>> {
        self.read_change_head(account, true)
    }

    fn read_change_head(&self, account: &str, with_payload: bool) -> Result<Option<QueuedChange>> {
        let encoding: String = self.conn.pragma_query_value(None, "encoding", |row| row.get(0))?;
        if encoding != "UTF-8" {
            return Err(Error::Invalid("This mail database uses an unsupported text encoding for queue recovery.".into()));
        }
        let payload = if with_payload {
            format!("CASE WHEN octet_length(change)<={} THEN change ELSE NULL END", MAX_CHANGE_PAYLOAD_BYTES)
        } else { "NULL".into() };
        let sql = format!("SELECT token,state_revision,state,failure,{payload} FROM outbox WHERE account_id=?1 ORDER BY id LIMIT 1");
        let row = self.conn.query_row(&sql, [account], |r| {
                Ok((ChangeId(r.get(0)?),r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?))
            }).optional()?;
        let Some((id,revision,state,failure,payload))=row else { return Ok(None); };
        let state=match state.as_str() {
            "queued" => ChangeState::Queued,
            "claimed" => ChangeState::Claimed,
            "blocked" => ChangeState::Blocked,
            _ => return Err(stale()),
        };
        let failure=match failure.as_deref() {
            None => None,
            Some("refused") => Some(ChangeFailure::Refused),
            Some("unreadable") => Some(ChangeFailure::Unreadable),
            Some("uncertain") => Some(ChangeFailure::Uncertain),
            Some("tooLarge") => Some(ChangeFailure::TooLarge),
            _ => return Err(stale()),
        };
        if !(0..=9007199254740991).contains(&revision) || (state==ChangeState::Blocked)!=failure.is_some() { return Err(stale()); }
        Ok(Some(QueuedChange {id,revision,state,failure,payload}))
    }

    /// Claims only the account's current head. Commit precedes any network mutation.
    pub fn claim_change(&mut self, account: &str, id: ChangeId, revision: i64) -> Result<i64> {
        self.delivery_transaction(|store| {
            store.batch_for_change(account, id)?.ok_or_else(stale)?;
            changed_one(store.conn.execute(
                "UPDATE outbox SET state='claimed',state_revision=state_revision+1
                 WHERE account_id=?1 AND token=?2 AND state_revision=?3 AND state_revision BETWEEN 0 AND 9007199254740990
                 AND state='queued' AND failure IS NULL AND id=(SELECT min(id) FROM outbox WHERE account_id=?1)",
                params![account,id.0,revision])?)?;
            Ok(revision+1)
        })
    }

    /// Retains unreadable, refused, uncertain or oversized intent. Repeated/stale transitions fail.
    pub fn block_change(&mut self, account: &str, id: ChangeId, revision: i64, failure: ChangeFailure) -> Result<i64> {
        let failure=match failure { ChangeFailure::Refused=>"refused",ChangeFailure::Unreadable=>"unreadable",ChangeFailure::Uncertain=>"uncertain",ChangeFailure::TooLarge=>"tooLarge" };
        self.delivery_transaction(|store| {
            changed_one(store.conn.execute(
                "UPDATE outbox SET state='blocked',failure=?4,state_revision=state_revision+1
                 WHERE account_id=?1 AND token=?2 AND state_revision=?3 AND state_revision BETWEEN 0 AND 9007199254740990
                 AND state IN ('queued','claimed') AND failure IS NULL
                 AND id=(SELECT min(id) FROM outbox WHERE account_id=?1)",
                params![account,id.0,revision,failure])?)?;
            Ok(revision+1)
        })
    }

    /// Acknowledges only the exact claimed attempt. It cannot discard a recovery record.
    pub fn complete_change(&mut self, account: &str, id: ChangeId, revision: i64) -> Result<()> {
        self.delivery_transaction(|store| {
            store.batch_for_change(account,id)?.ok_or_else(stale)?;
            store.resolve_change_part(id, crate::BatchOutcome::Completed)?;
            changed_one(store.conn.execute(
                "DELETE FROM outbox WHERE account_id=?1 AND token=?2 AND state_revision=?3 AND state_revision BETWEEN 0 AND 9007199254740991 AND state='claimed' AND failure IS NULL",
                params![account,id.0,revision])?)
        })
    }

    /// An attempt with no definite outcome goes back to the queue, in its place, to be tried
    /// again later. Only for changes the engine knows are safe to apply twice.
    pub fn release_change(&mut self, account: &str, id: ChangeId, revision: i64) -> Result<i64> {
        self.requeue_head(account, id, revision, "state='claimed' AND failure IS NULL")
    }

    /// You asked to try a blocked change again: it waits in its place for the next attempt.
    pub fn retry_blocked_change(&mut self, account: &str, id: ChangeId, revision: i64) -> Result<i64> {
        self.requeue_head(account, id, revision, "state='blocked' AND failure IS NOT NULL")
    }

    fn requeue_head(&mut self, account: &str, id: ChangeId, revision: i64, from: &str) -> Result<i64> {
        self.delivery_transaction(|store| {
            changed_one(store.conn.execute(&format!(
                "UPDATE outbox SET state='queued',failure=NULL,state_revision=state_revision+1
                 WHERE account_id=?1 AND token=?2 AND state_revision=?3 AND state_revision BETWEEN 0 AND 9007199254740990
                 AND {from} AND id=(SELECT min(id) FROM outbox WHERE account_id=?1)"),
                params![account,id.0,revision])?)?;
            Ok(revision+1)
        })
    }

    /// The server no longer has what this claimed attempt was for, so there is nothing to do.
    /// Its part is recorded as dropped, not as done, and later changes can go.
    pub fn drop_claimed_change(&mut self, account: &str, id: ChangeId, revision: i64) -> Result<()> {
        self.delivery_transaction(|store| {
            store.batch_for_change(account,id)?.ok_or_else(stale)?;
            store.resolve_change_part(id, crate::BatchOutcome::Dropped)?;
            changed_one(store.conn.execute(
                "DELETE FROM outbox WHERE account_id=?1 AND token=?2 AND state_revision=?3 AND state_revision BETWEEN 0 AND 9007199254740991 AND state='claimed' AND failure IS NULL",
                params![account,id.0,revision])?)
        })
    }

    /// At startup: an attempt a crash or force quit left claimed goes back to the queue. The
    /// caller holds the account's mutation guard, so the attempt can't still be on its way.
    /// Returns whether there was one.
    pub fn release_interrupted_change(&mut self, account: &str) -> Result<bool> {
        self.delivery_transaction(|store| {
            let claimed = "account_id=?1 AND state='claimed' AND failure IS NULL AND state_revision BETWEEN 0 AND 9007199254740990";
            let expected: i64 = store.conn.query_row(&format!("SELECT count(*) FROM outbox WHERE {claimed}"), [account], |r| r.get(0))?;
            let changed = store.conn.execute(&format!("UPDATE outbox SET state='queued',state_revision=state_revision+1 WHERE {claimed}"), [account])?;
            if changed as i64 != expected { return Err(stale()); }
            Ok(changed > 0)
        })
    }

    /// Explicit recovery dismissal checks the state observed by the user.
    #[cfg(test)]
    pub fn discard_blocked_change(&mut self, account: &str, id: ChangeId, revision: i64) -> Result<()> {
        self.delivery_transaction(|store| {
            let batch = store.batch_for_change(account,id)?.ok_or_else(stale)?;
            if batch.parts.len()!=1 { return Err(Error::Invalid("Dismiss the remaining parts of this action together.".into())); }
            store.resolve_change_part(id,crate::BatchOutcome::Discarded)?;
            changed_one(store.conn.execute(
                "DELETE FROM outbox WHERE account_id=?1 AND token=?2 AND state_revision=?3 AND state_revision BETWEEN 0 AND 9007199254740991 AND state='blocked' AND failure IS NOT NULL",
                params![account,id.0,revision])?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn an_attempt_without_a_definite_outcome_goes_back_to_the_queue_in_its_place() {
        let mut store=store_with_work();
        let first=store.queue_change("work","first",1).unwrap();
        let second=store.queue_change("work","second",2).unwrap();
        assert!(store.release_change("work",first,0).is_err(),"only a claimed attempt");
        let claim=store.claim_change("work",first,0).unwrap();
        assert!(store.release_change("work",first,claim-1).is_err(),"stale");
        assert!(store.release_change("other",first,claim).is_err(),"another account");
        assert!(store.release_change("work",second,0).is_err(),"not the head");
        let queued=store.release_change("work",first,claim).unwrap();
        assert_eq!(queued,claim+1);
        assert_eq!(store.earliest_change("work").unwrap().unwrap(),QueuedChange {id:first,revision:queued,state:ChangeState::Queued,failure:None,payload:Some("first".into())});
        assert!(store.release_change("work",first,queued).is_err(),"already queued");
        assert!(store.batch_for_change("work",first).unwrap().unwrap().parts.iter().all(|p|p.outcome==crate::BatchOutcome::Pending));
        let claim=store.claim_change("work",first,queued).unwrap();
        store.complete_change("work",first,claim).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().id,second);
    }

    #[test]
    fn a_blocked_change_is_tried_again_only_as_the_user_saw_it() {
        let mut store=store_with_work();
        let first=store.queue_change("work","first",1).unwrap();
        store.queue_change("work","second",2).unwrap();
        assert!(store.retry_blocked_change("work",first,0).is_err(),"a queued change isn't blocked");
        let claim=store.claim_change("work",first,0).unwrap();
        assert!(store.retry_blocked_change("work",first,claim).is_err(),"a claimed change isn't blocked");
        let blocked=store.block_change("work",first,claim,ChangeFailure::Refused).unwrap();
        assert!(store.retry_blocked_change("work",first,claim).is_err(),"stale");
        assert!(store.retry_blocked_change("other",first,blocked).is_err(),"another account");
        let queued=store.retry_blocked_change("work",first,blocked).unwrap();
        assert_eq!(queued,blocked+1);
        let head=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((head.id,head.state,head.failure,head.revision),(first,ChangeState::Queued,None,queued));
        assert!(store.retry_blocked_change("work",first,queued).is_err(),"once");
        store.claim_change("work",first,queued).unwrap();
    }

    #[test]
    fn a_claimed_change_whose_target_is_gone_is_dropped_and_later_work_can_go() {
        let mut store=store_with_work();
        let ids=store.delivery_transaction(|s|s.queue_batch_in_transaction("work",crate::BatchKind::Done,&["first".into(),"second".into()],NOV_1)).unwrap();
        assert!(store.drop_claimed_change("work",ids[0],0).is_err(),"only a claimed attempt");
        let claim=store.claim_change("work",ids[0],0).unwrap();
        assert!(store.drop_claimed_change("work",ids[0],claim-1).is_err(),"stale");
        assert!(store.drop_claimed_change("other",ids[0],claim).is_err(),"another account");
        store.drop_claimed_change("work",ids[0],claim).unwrap();
        assert!(store.drop_claimed_change("work",ids[0],claim).is_err(),"once");
        let batch=store.batch_for_change("work",ids[1]).unwrap().unwrap();
        assert_eq!(batch.parts.iter().map(|p|p.outcome).collect::<Vec<_>>(),[crate::BatchOutcome::Dropped,crate::BatchOutcome::Pending]);
        let head=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((head.id,head.state),(ids[1],ChangeState::Queued));
    }

    #[test]
    fn attempts_left_claimed_by_a_crash_go_back_to_the_queue_at_startup() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("queue.db");
        let mut store=Store::open(&path).unwrap();
        for id in ["work","other","third"] { store.upsert_account(&account(id,&format!("{id}@example.test"))).unwrap(); }
        let claimed=store.queue_change("work","claimed",1).unwrap();
        let claim=store.claim_change("work",claimed,0).unwrap();
        let blocked=store.queue_change("other","blocked",2).unwrap();
        let refused=store.block_change("other",blocked,0,ChangeFailure::Uncertain).unwrap();
        store.queue_change("third","queued",3).unwrap();
        assert!(store.release_interrupted_change("work").unwrap());
        assert!(!store.release_interrupted_change("work").unwrap());
        for id in ["other","third","missing"] { assert!(!store.release_interrupted_change(id).unwrap()); }
        drop(store);
        let store=Store::open(&path).unwrap();
        let head=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((head.state,head.revision),(ChangeState::Queued,claim+1));
        let other=store.earliest_change("other").unwrap().unwrap();
        assert_eq!((other.state,other.failure,other.revision),(ChangeState::Blocked,Some(ChangeFailure::Uncertain),refused));
        assert_eq!(store.earliest_change("third").unwrap().unwrap().state,ChangeState::Queued);
    }

    #[test]
    fn recovery_metadata_never_reads_the_action_payload() {
        unsafe extern "C" fn deny_payload(_: *mut std::ffi::c_void, action: std::ffi::c_int,
            table: *const std::ffi::c_char, column: *const std::ffi::c_char,
            _: *const std::ffi::c_char, _: *const std::ffi::c_char) -> std::ffi::c_int {
            if action == rusqlite::ffi::SQLITE_READ && !table.is_null() && !column.is_null()
                && unsafe { std::ffi::CStr::from_ptr(table).to_bytes() } == b"outbox"
                && unsafe { std::ffi::CStr::from_ptr(column).to_bytes() } == b"change" {
                return rusqlite::ffi::SQLITE_DENY;
            }
            rusqlite::ffi::SQLITE_OK
        }
        let mut store = store_with_work();
        let first = store.queue_change("work", &"x".repeat(MAX_CHANGE_PAYLOAD_BYTES + 1), 1).unwrap();
        store.queue_change("work", "later", 2).unwrap();
        let revision = store.block_change("work", first, 0, ChangeFailure::TooLarge).unwrap();
        unsafe { assert_eq!(rusqlite::ffi::sqlite3_set_authorizer(store.conn.handle(), Some(deny_payload), std::ptr::null_mut()), rusqlite::ffi::SQLITE_OK); }
        assert!(store.earliest_change("work").is_err(), "the authorizer must reject any payload read");
        assert_eq!(store.earliest_change_metadata("work").unwrap(), Some(ChangeHead {
            id: first, revision, state: ChangeState::Blocked, failure: Some(ChangeFailure::TooLarge),
        }));
        assert_eq!(store.earliest_change_metadata("missing").unwrap(), None);
        let (head, batch) = store.change_recovery_snapshot("work").unwrap().unwrap();
        assert_eq!(head.id, first);
        assert_eq!(batch.parts.len(), 1);
        assert_eq!(batch.kind, crate::BatchKind::Legacy);
        assert_eq!(store.change_recovery_snapshot("missing").unwrap(), None);
    }

    #[test]
    fn oversized_legacy_intent_has_a_distinct_persistent_recovery_reason() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("queue.db");
        let mut store=Store::open(&path).unwrap();
        store.upsert_account(&account("work","maya@example.test")).unwrap();
        let payload="x".repeat(1048577);
        let id=store.queue_change("work",&payload,1).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().payload,None);
        let revision=store.block_change("work",id,0,ChangeFailure::TooLarge).unwrap();
        drop(store);
        let mut store=Store::open(&path).unwrap();
        let row=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((row.id,row.revision,row.state,row.failure,row.payload),(id,revision,ChangeState::Blocked,Some(ChangeFailure::TooLarge),None));
        assert_eq!(store.pending_changes("work").unwrap(),[(id,payload)]);
        assert!(store.claim_change("work",id,revision).is_err());
        store.discard_blocked_change("work",id,revision).unwrap();
    }

    #[test]
    fn non_utf8_databases_cannot_exceed_the_rust_payload_budget() {
        for encoding in ["UTF-16le","UTF-16be"] {
            let dir=tempfile::tempdir().unwrap();let path=dir.path().join("legacy.db");
            let conn=rusqlite::Connection::open(&path).unwrap();
            conn.pragma_update(None,"encoding",encoding).unwrap();
            conn.execute_batch("CREATE TABLE encoding_probe(value TEXT)").unwrap();
            drop(conn);
            let mut store=Store::open(&path).unwrap();
            store.upsert_account(&account("work","maya@example.test")).unwrap();
            let payload="界".repeat(400000);
            let id=store.queue_change("work",&payload,1).unwrap();
            assert_eq!(store.conn.query_row("SELECT octet_length(change) FROM outbox",[],|r|r.get::<_,i64>(0)).unwrap(),800000);
            assert_eq!(payload.len(),1200000);
            assert!(matches!(store.earliest_change("work"),Err(Error::Invalid(message)) if message=="This mail database uses an unsupported text encoding for queue recovery."),"{encoding}: the storage size is not the Rust UTF-8 size");
            assert_eq!(store.pending_changes("work").unwrap(),[(id,payload)]);
        }
    }

    #[test]
    fn legacy_completion_cannot_bypass_an_earlier_recovery_barrier() {
        for claimed in [false,true] {
            let mut store=store_with_work();
            store.upsert_account(&account("other","other@example.test")).unwrap();
            let first=store.queue_change("work","first",1).unwrap();
            let second=store.queue_change("work","second",2).unwrap();
            let other=store.queue_change("other","other",3).unwrap();
            if claimed {store.claim_change("work",first,0).unwrap();}
            else {store.block_change("work",first,0,ChangeFailure::Refused).unwrap();}
            assert!(store.drop_change(second).is_err());
            assert_eq!(store.pending_changes("work").unwrap(),[(first,"first".into()),(second,"second".into())]);
            assert!(store.drop_change(other).unwrap());
        }
    }

    #[test]
    fn only_the_observed_head_can_enter_recovery() {
        let mut store=store_with_work();
        let first=store.queue_change("work","first",1).unwrap();
        let second=store.queue_change("work","second",2).unwrap();
        assert!(store.block_change("work",second,0,ChangeFailure::Unreadable).is_err());
        assert_eq!(store.earliest_change("work").unwrap().unwrap().id,first);
        store.drop_change(first).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().state,ChangeState::Queued);
        store.block_change("work",second,0,ChangeFailure::Unreadable).unwrap();
    }

    #[test]
    fn corrupted_revisions_cannot_authorize_a_transition() {
        for operation in ["claim","block","complete","discard","release","retry","drop"] {
            for revision in [-1,9007199254740992] {
                let mut store=store_with_work();
                let id=store.queue_change("work","payload",1).unwrap();
                if matches!(operation,"complete"|"release"|"drop") {store.claim_change("work",id,0).unwrap();}
                if matches!(operation,"discard"|"retry") {store.block_change("work",id,0,ChangeFailure::Refused).unwrap();}
                store.conn.execute_batch("PRAGMA ignore_check_constraints=ON").unwrap();
                store.conn.execute("UPDATE outbox SET state_revision=?1",[revision]).unwrap();
                let result=match operation {
                    "claim"=>store.claim_change("work",id,revision).map(|_|()),
                    "block"=>store.block_change("work",id,revision,ChangeFailure::Refused).map(|_|()),
                    "complete"=>store.complete_change("work",id,revision),
                    "release"=>store.release_change("work",id,revision).map(|_|()),
                    "retry"=>store.retry_blocked_change("work",id,revision).map(|_|()),
                    "drop"=>store.drop_claimed_change("work",id,revision),
                    _=>store.discard_blocked_change("work",id,revision),
                };
                assert!(result.is_err(),"{operation}: {revision}");
                assert_eq!(store.pending_changes("work").unwrap(),[(id,"payload".into())]);
            }
        }
    }

    #[test]
    fn recovery_migration_preserves_identity_payload_and_order() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("legacy.db");
        let mut conn=rusqlite::Connection::open(&path).unwrap();
        crate::migrate(&mut conn,&crate::schema::MIGRATIONS[..32]).unwrap();
        let mut old=Store {conn,delivery_session:None};
        old.upsert_account(&account("work","maya@example.test")).unwrap();
        let first=old.queue_change("work","first",1).unwrap();
        let second=old.queue_change("work","second",2).unwrap();
        drop(old);
        let mut store=Store::open(&path).unwrap();
        let head=store.earliest_change("work").unwrap().unwrap();
        assert_eq!(head,QueuedChange {id:first,revision:0,state:ChangeState::Queued,failure:None,payload:Some("first".into())});
        let claim=store.claim_change("work",first,0).unwrap();
        store.complete_change("work",first,claim).unwrap();
        assert_eq!(store.pending_changes("work").unwrap(),[(second,"second".into())]);
    }

    #[test]
    fn legacy_completion_cannot_remove_claimed_or_blocked_intent() {
        let mut store=store_with_work();
        let id=store.queue_change("work","payload",1).unwrap();
        let claim=store.claim_change("work",id,0).unwrap();
        assert!(store.drop_change(id).is_err());
        let blocked=store.block_change("work",id,claim,ChangeFailure::Uncertain).unwrap();
        assert!(store.drop_change(id).is_err());
        store.discard_blocked_change("work",id,blocked).unwrap();
    }

    #[test]
    fn unknown_or_inconsistent_recovery_metadata_fails_closed() {
        for assignment in ["state='future'", "failure='future'", "state='blocked'", "failure='refused'", "state_revision=-1", "state_revision=9007199254740992", "state_revision='bad'"] {
            let store=store_with_work();
            store.queue_change("work","payload",1).unwrap();
            assert!(store.conn.execute(&format!("UPDATE outbox SET {assignment}"),[]).is_err(),"{assignment}");
            store.conn.execute_batch("PRAGMA ignore_check_constraints=ON;").unwrap();
            store.conn.execute(&format!("UPDATE outbox SET {assignment}"),[]).unwrap();
            assert!(store.earliest_change("work").is_err(),"{assignment}");
        }
    }

    #[test]
    fn ignored_transitions_preserve_the_exact_previous_state() {
        for action in ["IGNORE", "ABORT, 'fixture'"] {
            for operation in ["claim","block","complete","discard","release","retry","drop","startup"] {
                let mut store=store_with_work();
                let id=store.queue_change("work","payload",1).unwrap();
                let revision=match operation {
                    "complete"|"release"|"drop"|"startup" => store.claim_change("work",id,0).unwrap(),
                    "discard"|"retry" => store.block_change("work",id,0,ChangeFailure::Refused).unwrap(),
                    _ => 0,
                };
                let before=store.earliest_change("work").unwrap();
                let event=if matches!(operation,"claim"|"block"|"release"|"retry"|"startup") {"UPDATE"} else {"DELETE"};
                store.conn.execute_batch(&format!("CREATE TRIGGER refuse_transition BEFORE {event} ON outbox BEGIN SELECT RAISE({action}); END;")).unwrap();
                let result=match operation {
                    "claim" => store.claim_change("work",id,revision).map(|_|()),
                    "block" => store.block_change("work",id,revision,ChangeFailure::Unreadable).map(|_|()),
                    "complete" => store.complete_change("work",id,revision),
                    "release" => store.release_change("work",id,revision).map(|_|()),
                    "retry" => store.retry_blocked_change("work",id,revision).map(|_|()),
                    "drop" => store.drop_claimed_change("work",id,revision),
                    "startup" => store.release_interrupted_change("work").map(|_|()),
                    _ => store.discard_blocked_change("work",id,revision),
                };
                assert!(result.is_err(),"{operation}: {action}");
                assert_eq!(store.earliest_change("work").unwrap(),before);
            }
        }
    }

    #[test]
    fn transitions_commit_with_full_durability_and_restore_performance_settings() {
        let mut store=store_with_work();
        store.conn.pragma_update(None,"synchronous","NORMAL").unwrap();
        store.conn.pragma_update(None,"fullfsync",false).unwrap();
        store.conn.execute_batch("CREATE TABLE observed (sync INTEGER,flush INTEGER);
            CREATE TRIGGER observe_update BEFORE UPDATE ON outbox BEGIN INSERT INTO observed SELECT synchronous,fullfsync FROM pragma_synchronous,pragma_fullfsync; END;
            CREATE TRIGGER observe_delete BEFORE DELETE ON outbox BEGIN INSERT INTO observed SELECT synchronous,fullfsync FROM pragma_synchronous,pragma_fullfsync; END;").unwrap();
        let id=store.queue_change("work","first",1).unwrap();
        let claim=store.claim_change("work",id,0).unwrap();
        store.complete_change("work",id,claim).unwrap();
        let id=store.queue_change("work","second",2).unwrap();
        let blocked=store.block_change("work",id,0,ChangeFailure::Refused).unwrap();
        store.discard_blocked_change("work",id,blocked).unwrap();
        let values=store.conn.prepare("SELECT sync,flush FROM observed").unwrap().query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(values,vec![(2,1);4]);
        assert_eq!(store.conn.pragma_query_value(None,"synchronous",|r|r.get::<_,i64>(0)).unwrap(),1);
        assert!(!store.conn.pragma_query_value(None,"fullfsync",|r|r.get::<_,bool>(0)).unwrap());
    }

    #[test]
    fn exhausted_revisions_and_database_failures_are_errors() {
        let mut store=store_with_work();
        let id=store.queue_change("work","payload",1).unwrap();
        store.conn.execute("UPDATE outbox SET state_revision=9007199254740991",[]).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().revision,9007199254740991);
        assert!(store.claim_change("work",id,9007199254740991).is_err());
        assert!(store.block_change("work",id,9007199254740991,ChangeFailure::Unreadable).is_err());
        store.conn.execute_batch("DROP TABLE outbox").unwrap();
        assert!(store.earliest_change("work").is_err());
        assert!(store.claim_change("work",id,0).is_err());
        assert!(store.block_change("work",id,0,ChangeFailure::Refused).is_err());
        assert!(store.complete_change("work",id,0).is_err());
        assert!(store.discard_blocked_change("work",id,0).is_err());
        assert!(store.release_change("work",id,0).is_err());
        assert!(store.retry_blocked_change("work",id,0).is_err());
        assert!(store.drop_claimed_change("work",id,0).is_err());
        assert!(store.release_interrupted_change("work").is_err());
    }

    #[test]
    fn claims_and_recovery_preserve_order_and_reject_stale_controls() {
        let mut store=store_with_work();
        assert_eq!(store.earliest_change("work").unwrap(),None);
        let first=store.queue_change("work","first",1).unwrap();
        let second=store.queue_change("work","second",2).unwrap();
        assert_eq!(store.earliest_change("work").unwrap(),Some(QueuedChange { id:first,revision:0,state:ChangeState::Queued,failure:None,payload:Some("first".into()) }));
        assert!(store.claim_change("work",second,0).is_err());
        assert!(store.claim_change("other",first,0).is_err());
        assert!(store.discard_blocked_change("work",first,0).is_err());
        assert!(store.complete_change("work",first,0).is_err());
        let claim=store.claim_change("work",first,0).unwrap();
        assert_eq!(claim,1);
        assert_eq!(store.earliest_change("work").unwrap().unwrap().state,ChangeState::Claimed);
        assert!(store.claim_change("work",first,0).is_err());
        assert!(store.claim_change("work",first,claim).is_err());
        assert!(store.complete_change("work",first,0).is_err());
        let blocked=store.block_change("work",first,claim,ChangeFailure::Uncertain).unwrap();
        assert_eq!(blocked,2);
        assert!(store.complete_change("work",first,claim).is_err());
        assert!(store.discard_blocked_change("work",first,claim).is_err());
        assert!(store.block_change("work",first,blocked,ChangeFailure::Refused).is_err());
        assert!(store.claim_change("work",second,0).is_err());
        let head=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((head.state,head.failure,head.payload),(ChangeState::Blocked,Some(ChangeFailure::Uncertain),Some("first".into())));
        store.discard_blocked_change("work",first,blocked).unwrap();
        assert!(store.discard_blocked_change("work",first,blocked).is_err());
        let claim=store.claim_change("work",second,0).unwrap();
        store.complete_change("work",second,claim).unwrap();
        assert_eq!(store.earliest_change("work").unwrap(),None);
    }

    #[test]
    fn queue_recovery_survives_reopening_without_replaying_claims() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("queue.db");
        let mut store=Store::open(&path).unwrap();
        store.upsert_account(&account("work","maya@example.test")).unwrap();
        let id=store.queue_change("work","payload",1).unwrap();
        let revision=store.claim_change("work",id,0).unwrap();
        drop(store);
        let mut store=Store::open(&path).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().state,ChangeState::Claimed);
        assert!(store.claim_change("work",id,revision).is_err());
        let revision=store.block_change("work",id,revision,ChangeFailure::Uncertain).unwrap();
        drop(store);
        let mut store=Store::open(&path).unwrap();
        let row=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((row.revision,row.failure),(revision,Some(ChangeFailure::Uncertain)));
        store.discard_blocked_change("work",id,revision).unwrap();
    }

    #[test]
    fn unreadable_and_refused_intent_stays_immutable_and_account_scoped() {
        for reason in [ChangeFailure::Unreadable,ChangeFailure::Refused] {
            let mut store=store_with_work();
            store.upsert_account(&account("other","other@example.test")).unwrap();
            let id=store.queue_change("work","bad json",1).unwrap();
            let other=store.queue_change("other","other",2).unwrap();
            assert!(store.block_change("other",id,0,reason).is_err());
            let revision=store.block_change("work",id,0,reason).unwrap();
            assert_eq!(store.earliest_change("work").unwrap().unwrap().failure,Some(reason));
            assert!(store.conn.execute("UPDATE outbox SET change='rewritten'",[]).is_err());
            assert!(store.discard_blocked_change("other",id,revision).is_err());
            let claim=store.claim_change("other",other,0).unwrap();
            assert!(store.complete_change("work",other,claim).is_err());
            store.complete_change("other",other,claim).unwrap();
            assert_eq!(store.pending_changes("work").unwrap(),[(id,"bad json".into())]);
        }
    }

    #[test]
    fn oldest_payload_is_byte_bounded_and_never_skipped() {
        let store=store_with_work();
        let text="é".repeat(524288);
        let id=store.queue_change("work",&text,1).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().payload,Some(text));
        let mut store=store;
        store.drop_change(id).unwrap();
        let id=store.queue_change("work",&"é".repeat(524289),2).unwrap();
        store.queue_change("work","small later action",3).unwrap();
        let head=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((head.id,head.payload),(id,None));
    }
}
