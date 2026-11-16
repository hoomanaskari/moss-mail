//! Logical server actions retain explicit per-part outcomes while work remains.
use crate::{ChangeId, Error, Result, Store, MAX_CHANGE_PARTS, MAX_CHANGE_PAYLOAD_BYTES};
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchId(pub(crate) [u8; 16]);
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BatchKind { Legacy, Done, Undone, Read, Delete, Restore, Move, Forwarded, DraftCleanup }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOutcome { Pending, Completed, Discarded, Dropped }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnclaimedOutcome { Applied, Dropped }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPart { pub id: ChangeId, pub ordinal: usize, pub outcome: BatchOutcome }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeBatch { pub id: BatchId, pub kind: BatchKind, pub created_at: i64, pub parts: Vec<BatchPart> }

fn invalid() -> Error { Error::Invalid("The server action batch is inconsistent. Its remaining work has been kept.".into()) }

impl BatchKind {
    fn text(self) -> &'static str {
        match self { Self::Legacy=>"legacy",Self::Done=>"done",Self::Undone=>"undone",Self::Read=>"read",Self::Delete=>"delete",Self::Restore=>"restore",Self::Move=>"move",Self::Forwarded=>"forwarded",Self::DraftCleanup=>"draftCleanup" }
    }
    fn parse(text: &str) -> Result<Self> {
        Ok(match text { "legacy"=>Self::Legacy,"done"=>Self::Done,"undone"=>Self::Undone,"read"=>Self::Read,"delete"=>Self::Delete,"restore"=>Self::Restore,"move"=>Self::Move,"forwarded"=>Self::Forwarded,"draftCleanup"=>Self::DraftCleanup,_=>return Err(invalid()) })
    }
}

impl Store {
    /// Records the legacy sender's disposition, not independent verification of server state.
    pub fn finish_unclaimed_change(&mut self, id: ChangeId, outcome: UnclaimedOutcome) -> Result<bool> {
        self.delivery_transaction(|store| {
            let account: Option<String> = store.conn.query_row("SELECT o.account_id FROM outbox o WHERE o.token=?1 AND o.state='queued' AND o.failure IS NULL AND NOT EXISTS(SELECT 1 FROM outbox h WHERE h.account_id=o.account_id AND h.id<o.id AND (h.state<>'queued' OR h.failure IS NOT NULL))",params![id.0],|r|r.get(0)).optional()?;
            let Some(account) = account else {
                let exists: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE token=?1)",params![id.0],|r|r.get(0))?;
                if exists { return Err(crate::change_recovery::stale()); }
                return Ok(false);
            };
            store.batch_for_change(&account,id)?.ok_or_else(invalid)?;
            store.resolve_change_part(id,match outcome { UnclaimedOutcome::Applied=>BatchOutcome::Completed,UnclaimedOutcome::Dropped=>BatchOutcome::Dropped })?;
            if store.conn.execute("DELETE FROM outbox WHERE token=?1 AND state='queued' AND failure IS NULL",params![id.0])? != 1 { return Err(invalid()); }
            Ok(true)
        })
    }

    /// Dismisses the remaining parts of the exact blocked account head, atomically.
    pub fn discard_blocked_batch(&mut self, account: &str, id: ChangeId, revision: i64) -> Result<()> {
        self.delivery_transaction(|store| {
            let eligible: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE account_id=?1 AND token=?2 AND state_revision=?3 AND state_revision BETWEEN 0 AND 9007199254740991 AND state='blocked' AND failure IS NOT NULL AND id=(SELECT min(id) FROM outbox WHERE account_id=?1))",params![account,id.0,revision],|r|r.get(0))?;
            if !eligible { return Err(invalid()); }
            let batch = store.batch_for_change(account,id)?.ok_or_else(invalid)?;
            let pending: Vec<ChangeId> = batch.parts.iter().filter(|p|p.outcome==BatchOutcome::Pending).map(|p|p.id).collect();
            for part in &pending {
                let safe: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE token=?1 AND account_id=?2 AND state IN ('queued','blocked'))",params![part.0,account],|r|r.get(0))?;
                if !safe { return Err(invalid()); }
            }
            for part in &pending { store.resolve_change_part(*part,BatchOutcome::Discarded)?; }
            for part in &pending {
                if store.conn.execute("DELETE FROM outbox WHERE token=?1 AND account_id=?2 AND state IN ('queued','blocked')",params![part.0,account])? != 1 { return Err(invalid()); }
            }
            Ok(())
        })
    }

    /// Bounded metadata only, including resolved parts while another part remains.
    pub fn batch_for_change(&self, account: &str, id: ChangeId) -> Result<Option<ChangeBatch>> {
        let header = self.conn.query_row(
            "SELECT b.id,b.kind,b.parts,b.created_at FROM outbox_batches b JOIN outbox_batch_parts p ON p.batch=b.id WHERE p.token=?1 AND b.account_id=?2",
            params![id.0,account], |r| Ok((BatchId(r.get(0)?),r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).optional()?;
        let Some((batch,kind,total,created_at)) = header else {
            let exists: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE token=?1 AND account_id=?2)",params![id.0,account],|r|r.get(0))?;
            if exists { return Err(invalid()); }
            return Ok(None);
        };
        if !(1..=MAX_CHANGE_PARTS as i64).contains(&total) { return Err(invalid()); }
        let kind = BatchKind::parse(&kind)?;
        let mut statement = self.conn.prepare("SELECT p.token,p.ordinal,p.outcome,o.account_id FROM outbox_batch_parts p LEFT JOIN outbox o ON o.token=p.token WHERE p.batch=?1 ORDER BY p.ordinal LIMIT ?2")?;
        let mut rows = statement.query(params![batch.0,MAX_CHANGE_PARTS as i64+1])?;
        let mut parts = Vec::new();
        while let Some(row) = rows.next()? {
            let ordinal: i64 = row.get(1)?;
            if ordinal != parts.len() as i64 { return Err(invalid()); }
            let outcome = match row.get::<_,String>(2)?.as_str() {
                "pending"=>BatchOutcome::Pending,"completed"=>BatchOutcome::Completed,"discarded"=>BatchOutcome::Discarded,"dropped"=>BatchOutcome::Dropped,_=>return Err(invalid()),
            };
            let owner: Option<String> = row.get(3)?;
            if (outcome==BatchOutcome::Pending && owner.as_deref()!=Some(account)) || (outcome!=BatchOutcome::Pending && owner.is_some()) { return Err(invalid()); }
            parts.push(BatchPart {id:ChangeId(row.get(0)?),ordinal:ordinal as usize,outcome});
        }
        if parts.len() != total as usize { return Err(invalid()); }
        Ok(Some(ChangeBatch {id:batch,kind,created_at,parts}))
    }

    /// Pre-network draft actions must retain cleanup intent, never an empty batch.
    pub(crate) fn queue_draft_cleanup_in_transaction(&self, account: &str, changes: &[String], now: i64) -> Result<Vec<ChangeId>> {
        if changes.is_empty() {
            return Err(Error::Invalid("The server draft cleanup could not be prepared. No local changes were saved.".into()));
        }
        self.queue_batch_in_transaction(account, BatchKind::DraftCleanup, changes, now)
    }

    pub(crate) fn queue_batch_in_transaction(&self, account: &str, kind: BatchKind, changes: &[String], now: i64) -> Result<Vec<ChangeId>> {
        if self.conn.is_autocommit() { return Err(Error::Invalid("Queue batches require an owning transaction.".into())); }
        if changes.is_empty() { return Ok(vec![]); }
        if changes.len()>MAX_CHANGE_PARTS || changes.iter().any(|s| s.len()>MAX_CHANGE_PAYLOAD_BYTES) { return Err(Error::Invalid("This server action exceeds the queue limits.".into())); }
        let batch: [u8;16] = self.conn.query_row("SELECT randomblob(16)",[],|r|r.get(0))?;
        if self.conn.execute("INSERT INTO outbox_batches(id,account_id,kind,parts,created_at) VALUES (?1,?2,?3,?4,?5)",params![batch,account,kind.text(),changes.len() as i64,now])? != 1 { return Err(invalid()); }
        let mut ids = Vec::with_capacity(changes.len());
        for (ordinal,change) in changes.iter().enumerate() {
            let token: [u8;16] = self.conn.query_row("SELECT randomblob(16)",[],|r|r.get(0))?;
            if self.conn.execute("INSERT INTO outbox_batch_parts(token,batch,ordinal) VALUES (?1,?2,?3)",params![token,batch,ordinal as i64])? != 1 { return Err(invalid()); }
            if self.conn.execute("INSERT INTO outbox(account_id,change,created_at,token) VALUES (?1,?2,?3,?4)",params![account,change,now,token])? != 1 { return Err(invalid()); }
            ids.push(ChangeId(token));
        }
        let saved = self.batch_for_change(account,ids[0])?.ok_or_else(invalid)?;
        if saved.parts.len()!=changes.len() || saved.parts.iter().any(|p|p.outcome!=BatchOutcome::Pending) { return Err(invalid()); }
        Ok(ids)
    }

    pub(crate) fn resolve_change_part(&self, id: ChangeId, outcome: BatchOutcome) -> Result<()> {
        let text = match outcome { BatchOutcome::Completed=>"completed",BatchOutcome::Discarded=>"discarded",BatchOutcome::Dropped=>"dropped",BatchOutcome::Pending=>return Err(invalid()) };
        if self.conn.is_autocommit() { return Err(invalid()); }
        if self.conn.execute("UPDATE outbox_batch_parts SET outcome=?2 WHERE token=?1 AND outcome='pending'",params![id.0,text])? != 1 { return Err(invalid()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::ThreadChange;

    #[test]
    fn one_thread_action_keeps_its_completed_prefix_until_the_rest_finishes() {
        let mut store = store_with_work();
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        store.change_thread_with_changes(&thread, "work", ThreadChange::Done(true), NOV_1,
            |_, _| Ok(vec!["first".into(), "second".into(), "third".into()])).unwrap();
        let waiting = store.pending_changes("work").unwrap();
        let batch = store.batch_for_change("work", waiting[0].0).unwrap().unwrap();
        assert_eq!(batch.kind, BatchKind::Done);
        assert_eq!(batch.created_at, NOV_1);
        assert_eq!(batch.parts, waiting.iter().enumerate().map(|(ordinal,(id,_))| BatchPart { id:*id, ordinal, outcome:BatchOutcome::Pending }).collect::<Vec<_>>());
        assert_eq!(store.batch_for_change("work", waiting[1].0).unwrap().unwrap().id, batch.id);
        assert_eq!(store.batch_for_change("other", waiting[0].0).unwrap(), None);
        let revision = store.claim_change("work", waiting[0].0, 0).unwrap();
        store.complete_change("work", waiting[0].0, revision).unwrap();
        let retained = store.batch_for_change("work", waiting[1].0).unwrap().unwrap();
        assert_eq!(retained.parts[0].outcome, BatchOutcome::Completed);
        assert_eq!(retained.parts[1].outcome, BatchOutcome::Pending);
        assert_eq!(store.batch_for_change("work", waiting[0].0).unwrap(), Some(retained));
        store.drop_change(waiting[1].0).unwrap();
        let retained = store.batch_for_change("work", waiting[2].0).unwrap().unwrap();
        assert_eq!(retained.parts[1].outcome, BatchOutcome::Dropped);
        store.drop_change(waiting[2].0).unwrap();
        assert_eq!(store.batch_for_change("work", waiting[0].0).unwrap(), None);
    }

    #[test]
    fn a_failed_part_insert_rolls_back_local_state_and_all_batch_metadata() {
        let mut store = store_with_work();
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        store.conn.execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON outbox WHEN NEW.change='second' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.change_thread_with_changes(&thread, "work", ThreadChange::Done(true), NOV_1,
            |_, _| Ok(vec!["first".into(), "second".into()])).is_err());
        assert!(!store.thread(&thread, NOV_1).unwrap().unwrap().summary.done);
        assert!(store.pending_changes("work").unwrap().is_empty());
        assert_eq!(store.conn.query_row("SELECT count(*) FROM outbox_batches", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(store.conn.query_row("SELECT count(*) FROM outbox_batch_parts", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    }
    fn queued_batch(store: &mut Store) -> Vec<ChangeId> {
        store.delivery_transaction(|s| s.queue_batch_in_transaction("work", BatchKind::Restore, &["first".into(),"second".into(),"third".into()], NOV_1)).unwrap()
    }

    #[test]
    fn a_single_part_discard_cannot_split_a_multi_part_recovery_decision() {
        let mut store = store_with_work();
        let ids = queued_batch(&mut store);
        let revision = store.block_change("work",ids[0],0,crate::ChangeFailure::Refused).unwrap();
        assert!(store.discard_blocked_change("work",ids[0],revision).is_err());
        assert_eq!(store.pending_changes("work").unwrap().len(),3);
        assert!(store.batch_for_change("work",ids[0]).unwrap().unwrap().parts.iter().all(|p|p.outcome==BatchOutcome::Pending));
    }

    #[test]
    fn whole_batch_discard_checks_the_observed_head_and_preserves_later_work() {
        let mut store = store_with_work();
        let ids = queued_batch(&mut store);
        let later = store.queue_change("work","later",NOV_1).unwrap();
        let revision = store.claim_change("work",ids[0],0).unwrap();
        store.complete_change("work",ids[0],revision).unwrap();
        let revision = store.block_change("work",ids[1],0,crate::ChangeFailure::Uncertain).unwrap();
        assert!(store.discard_blocked_batch("work",ids[1],revision-1).is_err());
        assert!(store.discard_blocked_batch("other",ids[1],revision).is_err());
        store.conn.execute_batch("CREATE TRIGGER ignore_last_batch_delete BEFORE DELETE ON outbox WHEN OLD.change='third' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.discard_blocked_batch("work",ids[1],revision).is_err());
        assert_eq!(store.pending_changes("work").unwrap().len(),3);
        let batch = store.batch_for_change("work",ids[1]).unwrap().unwrap();
        assert_eq!(batch.parts.iter().map(|p|p.outcome).collect::<Vec<_>>(),[BatchOutcome::Completed,BatchOutcome::Pending,BatchOutcome::Pending]);
        store.conn.execute_batch("DROP TRIGGER ignore_last_batch_delete").unwrap();
        store.discard_blocked_batch("work",ids[1],revision).unwrap();
        assert_eq!(store.pending_changes("work").unwrap(),[(later,"later".into())]);
        assert_eq!(store.batch_for_change("work",ids[0]).unwrap(),None);
    }

    #[test]
    fn legacy_provider_success_and_dropped_intent_are_distinct() {
        let mut store = store_with_work();
        let ids = queued_batch(&mut store);
        assert!(store.finish_unclaimed_change(ids[0],UnclaimedOutcome::Applied).unwrap());
        let batch = store.batch_for_change("work",ids[1]).unwrap().unwrap();
        assert_eq!(batch.parts[0].outcome,BatchOutcome::Completed);
        assert!(!store.finish_unclaimed_change(ids[0],UnclaimedOutcome::Applied).unwrap());
        assert!(store.finish_unclaimed_change(ids[1],UnclaimedOutcome::Dropped).unwrap());
        assert_eq!(store.batch_for_change("work",ids[2]).unwrap().unwrap().parts[1].outcome,BatchOutcome::Dropped);
        store.drop_change(ids[2]).unwrap();
        assert_eq!(store.batch_for_change("work",ids[0]).unwrap(),None);
    }

    #[test]
    fn writer_limits_kinds_and_transaction_ownership_are_enforced() {
        let mut store = store_with_work();
        assert!(store.queue_batch_in_transaction("work",BatchKind::Done,&[],NOV_1).is_err());
        assert!(store.resolve_change_part(ChangeId([0;16]),BatchOutcome::Completed).is_err());
        assert!(store.delivery_transaction(|s|s.resolve_change_part(ChangeId([0;16]),BatchOutcome::Pending)).is_err());
        assert!(store.delivery_transaction(|s|s.queue_batch_in_transaction("work",BatchKind::Done,&[],NOV_1)).unwrap().is_empty());
        for kind in [BatchKind::Legacy,BatchKind::Done,BatchKind::Undone,BatchKind::Read,BatchKind::Delete,BatchKind::Restore,BatchKind::Move,BatchKind::Forwarded,BatchKind::DraftCleanup] {
            let ids = store.delivery_transaction(|s|s.queue_batch_in_transaction("work",kind,&["part".into()],NOV_1)).unwrap();
            assert_eq!(store.batch_for_change("work",ids[0]).unwrap().unwrap().kind,kind);
            store.drop_change(ids[0]).unwrap();
        }
        assert!(store.delivery_transaction(|s|s.queue_batch_in_transaction("work",BatchKind::Done,&vec!["x".into();MAX_CHANGE_PARTS+1],NOV_1)).is_err());
        assert!(store.delivery_transaction(|s|s.queue_batch_in_transaction("work",BatchKind::Done,&["x".repeat(MAX_CHANGE_PAYLOAD_BYTES+1)],NOV_1)).is_err());
        let ids = store.delivery_transaction(|s|s.queue_batch_in_transaction("work",BatchKind::Done,&vec!["x".into();MAX_CHANGE_PARTS],NOV_1)).unwrap();
        assert_eq!(store.batch_for_change("work",ids[0]).unwrap().unwrap().parts.len(),MAX_CHANGE_PARTS);
        for id in ids { store.drop_change(id).unwrap(); }
        let ids=store.delivery_transaction(|s|s.queue_batch_in_transaction("work",BatchKind::Done,&["x".repeat(MAX_CHANGE_PAYLOAD_BYTES)],NOV_1)).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().payload.unwrap().len(),MAX_CHANGE_PAYLOAD_BYTES);
        store.drop_change(ids[0]).unwrap();
    }

    #[test]
    fn migration_preserves_legacy_tokens_states_and_oversized_payloads() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("legacy.db");
        let mut conn=rusqlite::Connection::open(&path).unwrap();
        crate::migrate(&mut conn,&crate::schema::MIGRATIONS[..33]).unwrap();
        let mut old=Store {conn,delivery_session:None};
        old.upsert_account(&account("work","maya@example.test")).unwrap();
        let first=old.queue_change("work","first",1).unwrap();
        let second=old.queue_change("work",&"x".repeat(MAX_CHANGE_PAYLOAD_BYTES+1),2).unwrap();
        old.block_change("work",first,0,crate::ChangeFailure::Refused).unwrap();
        drop(old);
        let mut store=Store::open(&path).unwrap();
        for (id,time) in [(first,1),(second,2)] {
            let batch=store.batch_for_change("work",id).unwrap().unwrap();
            assert_eq!((batch.id,batch.kind,batch.created_at),(BatchId(id.0),BatchKind::Legacy,time));
            assert_eq!(batch.parts,[BatchPart {id,ordinal:0,outcome:BatchOutcome::Pending}]);
        }
        let head=store.earliest_change("work").unwrap().unwrap();
        assert_eq!((head.id,head.state,head.failure),(first,crate::ChangeState::Blocked,Some(crate::ChangeFailure::Refused)));
        store.discard_blocked_change("work",first,head.revision).unwrap();
        assert_eq!(store.earliest_change("work").unwrap().unwrap().payload,None);
        let before=store.batch_for_change("work",second).unwrap();
        drop(store);
        assert_eq!(Store::open(&path).unwrap().batch_for_change("work",second).unwrap(),before);
    }

    #[test]
    fn ignored_metadata_and_outcome_writes_cannot_partly_commit() {
        for trigger in [
            "CREATE TRIGGER reject_metadata BEFORE INSERT ON outbox_batches BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER reject_metadata BEFORE INSERT ON outbox_batch_parts WHEN NEW.ordinal=1 BEGIN SELECT RAISE(IGNORE); END;",
        ] {
            let mut store=store_with_work();store.conn.execute_batch(trigger).unwrap();
            assert!(store.delivery_transaction(|s|s.queue_batch_in_transaction("work",BatchKind::Restore,&["a".into(),"b".into()],NOV_1)).is_err());
            assert!(store.pending_changes("work").unwrap().is_empty());
            assert_eq!(store.conn.query_row("SELECT count(*) FROM outbox_batches",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
        let mut store=store_with_work();let ids=queued_batch(&mut store);
        for trigger in [
            "CREATE TRIGGER reject_outcome BEFORE UPDATE OF outcome ON outbox_batch_parts BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER reject_outcome BEFORE DELETE ON outbox BEGIN SELECT RAISE(IGNORE); END;",
        ] {
            store.conn.execute_batch(trigger).unwrap();
            assert!(store.finish_unclaimed_change(ids[0],UnclaimedOutcome::Applied).is_err());
            assert_eq!(store.batch_for_change("work",ids[0]).unwrap().unwrap().parts[0].outcome,BatchOutcome::Pending);
            store.conn.execute_batch("DROP TRIGGER reject_outcome").unwrap();
        }
        let revision=store.claim_change("work",ids[0],0).unwrap();
        store.conn.execute_batch("CREATE TRIGGER ignore_completion BEFORE DELETE ON outbox BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.complete_change("work",ids[0],revision).is_err());
        assert_eq!(store.batch_for_change("work",ids[0]).unwrap().unwrap().parts[0].outcome,BatchOutcome::Pending);
        store.conn.execute_batch("DROP TRIGGER ignore_completion").unwrap();
        store.complete_change("work",ids[0],revision).unwrap();
        assert_eq!(store.batch_for_change("work",ids[1]).unwrap().unwrap().parts[0].outcome,BatchOutcome::Completed);
    }

    #[test]
    fn account_cascade_removes_partial_batches_without_inventing_success() {
        let mut store=store_with_work();let ids=queued_batch(&mut store);
        store.finish_unclaimed_change(ids[0],UnclaimedOutcome::Applied).unwrap();
        store.conn.execute("DELETE FROM accounts WHERE id='work'",[]).unwrap();
        assert_eq!(store.batch_for_change("work",ids[0]).unwrap(),None);
        for table in ["outbox","outbox_batches","outbox_batch_parts"] {
            assert_eq!(store.conn.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
    }

    #[test]
    fn damaged_batch_metadata_is_never_reported_as_pending_or_successful() {
        for damage in [
            "DROP TRIGGER outbox_batch_immutable; PRAGMA ignore_check_constraints=ON; UPDATE outbox_batches SET kind='future';",
            "DROP TRIGGER outbox_batch_immutable; PRAGMA ignore_check_constraints=ON; UPDATE outbox_batches SET parts=0;",
            "DROP TRIGGER outbox_part_immutable; UPDATE outbox_batch_parts SET ordinal=4 WHERE ordinal=2;",
            "DELETE FROM outbox_batch_parts WHERE ordinal=2;",
            "DELETE FROM outbox_batch_parts WHERE ordinal=0;",
            "DROP TRIGGER outbox_batch_delete; DELETE FROM outbox WHERE change='second';",
            "UPDATE outbox_batch_parts SET outcome='completed' WHERE ordinal=1;",
            "DROP TRIGGER outbox_part_outcome; PRAGMA ignore_check_constraints=ON; UPDATE outbox_batch_parts SET outcome='future' WHERE ordinal=1;",
        ] {
            let mut store=store_with_work();let ids=queued_batch(&mut store);
            store.conn.execute_batch(damage).unwrap();
            assert!(store.batch_for_change("work",ids[0]).is_err(),"{damage}");
        }
    }

    #[test]
    fn batch_discard_refuses_a_claimed_sibling_and_immutable_metadata_cannot_change() {
        let mut store=store_with_work();let ids=queued_batch(&mut store);
        for sql in ["UPDATE outbox_batches SET kind='legacy'","UPDATE outbox_batch_parts SET ordinal=ordinal","UPDATE outbox_batch_parts SET outcome='pending'"] {
            assert!(store.conn.execute(sql,[]).is_err());
        }
        let batch=store.batch_for_change("work",ids[0]).unwrap().unwrap();
        assert!(store.conn.execute("INSERT INTO outbox_batch_parts(token,batch,ordinal) VALUES(randomblob(16),?1,3)",[batch.id.0]).is_err());
        let revision=store.block_change("work",ids[0],0,crate::ChangeFailure::Refused).unwrap();
        store.conn.execute("UPDATE outbox SET state='claimed' WHERE token=?1",[ids[1].0]).unwrap();
        assert!(store.discard_blocked_batch("work",ids[0],revision).is_err());
        assert_eq!(store.pending_changes("work").unwrap().len(),3);
    }

    #[test]
    fn an_unlabelled_delete_is_dropped_and_cannot_reuse_a_resolved_part() {
        let mut store=store_with_work();let ids=queued_batch(&mut store);
        store.conn.execute("DELETE FROM outbox WHERE token=?1",[ids[1].0]).unwrap();
        let batch=store.batch_for_change("work",ids[0]).unwrap().unwrap();
        assert_eq!(batch.parts[1].outcome,BatchOutcome::Dropped);
        assert!(store.conn.execute("UPDATE outbox_batch_parts SET outcome='completed' WHERE token=?1",[ids[1].0]).is_err());
        assert!(store.conn.execute("INSERT INTO outbox(account_id,change,created_at,token) VALUES('work','again',1,?1)",[ids[1].0]).is_err());
        assert_eq!(store.pending_changes("work").unwrap().len(),2);
    }

    #[test]
    fn completion_refuses_damaged_batch_ownership_without_losing_intent() {
        for claimed in [false,true] {
            let mut store=store_with_work();let ids=queued_batch(&mut store);
            store.upsert_account(&account("other","other@example.test")).unwrap();
            let revision=if claimed { store.claim_change("work",ids[0],0).unwrap() } else { 0 };
            store.conn.execute_batch("DROP TRIGGER outbox_batch_immutable; UPDATE outbox_batches SET account_id='other';").unwrap();
            let failed=if claimed { store.complete_change("work",ids[0],revision).is_err() }
                else { store.finish_unclaimed_change(ids[0],UnclaimedOutcome::Applied).is_err() };
            assert!(failed);
            assert_eq!(store.pending_changes("work").unwrap().len(),3);
            assert_eq!(store.conn.query_row("SELECT count(*) FROM outbox_batch_parts WHERE outcome='pending'",[],|r|r.get::<_,i64>(0)).unwrap(),3);
        }
    }

    #[test]
    fn a_normal_queue_barrier_is_not_reported_as_damaged_metadata() {
        let mut store=store_with_work();let ids=queued_batch(&mut store);
        let claim=store.claim_change("work",ids[0],0).unwrap();
        for id in [ids[0],ids[1]] {
            let error=store.finish_unclaimed_change(id,UnclaimedOutcome::Applied).unwrap_err();
            assert_eq!(error.to_string(),"This queued change is no longer in the expected state. Refresh its recovery status.");
        }
        store.block_change("work",ids[0],claim,crate::ChangeFailure::Uncertain).unwrap();
        let error=store.finish_unclaimed_change(ids[1],UnclaimedOutcome::Dropped).unwrap_err();
        assert_eq!(error.to_string(),"This queued change is no longer in the expected state. Refresh its recovery status.");
        assert!(store.batch_for_change("work",ids[0]).unwrap().unwrap().parts.iter().all(|p|p.outcome==BatchOutcome::Pending));
        assert_eq!(store.pending_changes("work").unwrap().len(),3);
    }

    #[test]
    fn every_stored_terminal_outcome_remains_distinguishable() {
        for outcome in [BatchOutcome::Completed,BatchOutcome::Discarded,BatchOutcome::Dropped] {
            let mut store=store_with_work();let ids=queued_batch(&mut store);
            store.delivery_transaction(|s| {
                s.resolve_change_part(ids[0],outcome)?;
                assert_eq!(s.conn.execute("DELETE FROM outbox WHERE token=?1",[ids[0].0])?,1);
                Ok(())
            }).unwrap();
            let batch=store.batch_for_change("work",ids[1]).unwrap().unwrap();
            assert_eq!(batch.parts[0].outcome,outcome);
            assert_eq!(batch.parts[1].outcome,BatchOutcome::Pending);
        }
    }

}
