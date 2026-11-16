//! Bounded metadata for deliveries that still need a user's decision.
use crate::{DeliveryState, Result, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use rusqlite::{params, OptionalExtension};

const PENDING: &str = "resolution IS NULL AND (state IN ('accepted','uncertain') OR (state='held' AND cancel_intent=1)) AND (state!='held' OR EXISTS(SELECT 1 FROM delivery_cancellations WHERE delivery_cancellations.id=deliveries.id))";
const RELEASABLE: &str = "draft_id=(SELECT draft_id FROM deliveries WHERE id=?1)
                AND id<=(SELECT file_ceiling FROM deliveries WHERE id=?1)
                AND NOT EXISTS(SELECT 1 FROM drafts WHERE drafts.id=draft_files.draft_id)
                AND NOT EXISTS(SELECT 1 FROM scheduled WHERE scheduled.draft_id=draft_files.draft_id AND scheduled.operation_id!=?1)
                AND NOT EXISTS(SELECT 1 FROM deliveries d WHERE d.draft_id=draft_files.draft_id AND d.id!=?1 AND d.resolution IS NULL AND d.state NOT IN ('complete','cancelled'))";
const MAX_PREVIEW_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryAttentionCursor {
    pub created_at: i64,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryAttentionEntry {
    pub id: String,
    pub account_id: String,
    pub state: DeliveryState,
    pub interrupted_from: Option<DeliveryState>,
    pub created_at: i64,
    pub file_count: i64,
    pub file_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryAttentionPage {
    pub entries: Vec<DeliveryAttentionEntry>,
    pub next: Option<DeliveryAttentionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReviewCopy {
    pub recovery_id: String,
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryResolution { Reviewed, Copied }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DeliveryResolutionOutcome {
    Resolved { #[serde(rename = "recoveryId")] recovery_id: Option<String> },
    AlreadyFinished,
    Sending,
}

struct ReviewRecord {
    state: DeliveryState,
    resolution: Option<String>,
    recovery_id: Option<String>,
    resolved_at: Option<i64>,
}

impl DeliveryResolution {
    fn name(self) -> &'static str {
        match self { Self::Reviewed => "reviewed", Self::Copied => "copied" }
    }
}

impl Store {
    fn review_record(&self, id: &str) -> Result<ReviewRecord> {
        self.conn.query_row("SELECT state,resolution,recovery_id,resolved_at FROM deliveries WHERE id=?1",[id],|row|Ok(ReviewRecord {
            state:crate::codec::named(row,0)?,resolution:row.get(1)?,recovery_id:row.get(2)?,resolved_at:row.get(3)?,
        })).optional()?.ok_or_else(||crate::Error::Invalid("This delivery is no longer available. Refresh the review list.".into()))
    }

    fn review_result(&self, id: &str, record: &ReviewRecord, action: DeliveryResolution) -> Result<Option<DeliveryResolutionOutcome>> {
        if let Some(resolution)=&record.resolution {
            if resolution!=action.name() { return Err(crate::Error::RequestConflict); }
            return Ok(Some(DeliveryResolutionOutcome::Resolved {recovery_id:record.recovery_id.clone()}));
        }
        match record.state {
            DeliveryState::Complete | DeliveryState::Cancelled => Ok(Some(DeliveryResolutionOutcome::AlreadyFinished)),
            DeliveryState::Claimed => Ok(Some(DeliveryResolutionOutcome::Sending)),
            DeliveryState::Accepted if action==DeliveryResolution::Copied => Err(crate::Error::Invalid("This email was accepted by the server. Its recovery data cannot become another send.".into())),
            DeliveryState::Held if !self.delivery_cancellation_recorded(id)? => Err(crate::Error::Invalid("This email is still waiting to send. Use Undo before reviewing its recovery.".into())),
            DeliveryState::Held if self.is_scheduled_delivery(id)? => Err(crate::Error::Invalid("This unsent email still belongs to a schedule. Cancel its schedule before reviewing it.".into())),
            _ => Ok(None),
        }
    }

    /// A copy commits first; a failed review mark never deletes its independent journal.
    pub fn resolve_delivery(&mut self, id: &str, action: DeliveryResolution, now: i64) -> Result<DeliveryResolutionOutcome> {
        if let Some(outcome)=self.review_result(id,&self.review_record(id)?,action)? { return Ok(outcome); }
        let recovery_id=if action==DeliveryResolution::Copied {Some(self.copy_delivery_recovery(id,now)?.recovery_id)} else {None};
        self.delivery_transaction(|store| {
            let record=store.review_record(id)?;
            if let Some(outcome)=store.review_result(id,&record,action)? { return Ok(outcome); }
            if let Some(recovery)=&recovery_id {
                let draft=store.delivery_attention_draft(id)?.ok_or(crate::Error::RequestConflict)?;
                if !store.local_recovery_matches_draft(recovery,&draft)? { return Err(crate::Error::RequestConflict); }
                let retained:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM local_draft_recoveries WHERE id=?1 AND state IN ('open','adopted'))",[recovery],|row|row.get(0))?;
                if !retained { return Err(crate::Error::Invalid("The recovery copy was discarded. The original has been kept. Use Discard recovery files only if you intend to remove it without a copy.".into())); }
            }
            let scheduled:Option<String>=store.conn.query_row("SELECT s.draft_id FROM scheduled s JOIN deliveries d ON d.id=s.operation_id AND d.draft_id=s.draft_id WHERE d.id=?1",[id],|r|r.get(0)).optional()?;
            if let Some(draft_id)=scheduled {
                if record.state==DeliveryState::Accepted {
                    store.conn.execute("INSERT OR IGNORE INTO schedule_outcomes (id,draft_id,outcome,at) VALUES (?1,?2,'completed',?3)",params![id,draft_id,now])?;
                    let recorded:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM schedule_outcomes WHERE id=?1 AND draft_id=?2 AND outcome='completed')",params![id,draft_id],|r|r.get(0))?;
                    if !recorded { return Err(crate::Error::Invalid("The accepted schedule receipt could not be saved.".into())); }
                }
                store.conn.execute("DELETE FROM scheduled WHERE operation_id=?1 AND draft_id=?2",params![id,draft_id])?;
                let remains:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM scheduled WHERE operation_id=?1 AND draft_id=?2)",params![id,draft_id],|r|r.get(0))?;
                if remains { return Err(crate::Error::Invalid("The reviewed schedule could not be removed.".into())); }
            }
            store.conn.execute("UPDATE deliveries SET resolution=?2,recovery_id=?3,resolved_at=?4,
                interrupted_from=CASE WHEN state='held' THEN 'held' ELSE interrupted_from END,
                state=CASE WHEN state='held' THEN 'uncertain' ELSE state END WHERE id=?1 AND resolution IS NULL",
                params![id,action.name(),recovery_id,now])?;
            let saved=store.review_record(id)?;
            let state=if record.state==DeliveryState::Held {DeliveryState::Uncertain} else {record.state};
            if saved.resolution.as_deref()!=Some(action.name()) || saved.recovery_id!=recovery_id || saved.resolved_at!=Some(now) || saved.state!=state {
                return Err(crate::Error::Invalid("The review could not be saved. Its recovery data has been kept.".into()));
            }
            // The monotonically allocated file IDs fence out imports made after
            // preparation, even if the wall clock moved backwards.

            store.conn.execute(&format!("DELETE FROM draft_files WHERE {RELEASABLE}"),[id])?;
            let remains:bool=store.conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM draft_files WHERE {RELEASABLE})"),[id],|r|r.get(0))?;
            if remains { return Err(crate::Error::Invalid("The reviewed delivery files could not be released.".into())); }
            Ok(DeliveryResolutionOutcome::Resolved {recovery_id})
        })
    }

    /// Copy first, without changing the delivery or removing any original files.
    pub fn copy_delivery_recovery(&mut self, id: &str, now: i64) -> Result<DeliveryReviewCopy> {
        let sql = format!("SELECT state FROM deliveries WHERE id=?1 AND {PENDING}");
        let state: Option<DeliveryState> = self.conn.query_row(&sql,[id],|row|crate::codec::named(row,0)).optional()?;
        match state {
            Some(DeliveryState::Held | DeliveryState::Uncertain) => {},
            Some(_) => return Err(crate::Error::Invalid("This email was accepted by the server. Its recovery data cannot become another send.".into())),
            None => return Err(crate::Error::Invalid("This delivery no longer needs a recovery copy. Refresh the review list.".into())),
        }
        let draft = self.delivery_attention_draft(id)?.ok_or_else(||crate::Error::Invalid("This delivery changed. Refresh the review list.".into()))?;
        let mut hash = Sha256::new();hash.update(b"moss-delivery-recovery-v1\0");hash.update(id.as_bytes());
        let recovery_id = format!("delivery-{:x}",hash.finalize());
        let available = match self.keep_local_draft_recovery(&recovery_id,None,&draft,now) {
            Ok(_) => true,
            Err(crate::Error::LocalDraftRecoveryResolved { .. }) => false,
            Err(error) => return Err(error),
        };
        if !self.local_recovery_matches_draft(&recovery_id,&draft)? { return Err(crate::Error::RequestConflict); }
        Ok(DeliveryReviewCopy {recovery_id,available})
    }

    /// Decode only the selected bounded snapshot; list entries remain usable if it is damaged.
    pub fn delivery_attention_draft(&self, id: &str) -> Result<Option<moss_core::Draft>> {
        let sql = format!("SELECT CASE WHEN length(CAST(draft AS BLOB))<=?2 THEN draft END,draft_id,account_id FROM deliveries WHERE id=?1 AND {PENDING}");
        let snapshot: Option<(Option<String>,String,String)> = self.conn.query_row(&sql, params![id,MAX_PREVIEW_BYTES as i64], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        match snapshot {
            None => Ok(None),
            Some((None,_,_)) => Err(crate::Error::Invalid("This saved draft exceeds the 16 MiB preview limit. Its data is retained for review.".into())),
            Some((Some(snapshot),draft_id,account_id)) => {
                let draft:moss_core::Draft=serde_json::from_str(&snapshot).map_err(|_|crate::Error::UnreadableDeliveryDraft)?;
                if draft.id!=draft_id || draft.from_account_id!=account_id { return Err(crate::Error::UnreadableDeliveryDraft); }
                Ok(Some(draft))
            }
        }
    }

    /// Keyset pages contain no draft, outgoing MIME or attachment payloads.
    pub fn delivery_attention(&self, after: Option<&DeliveryAttentionCursor>, limit: usize) -> Result<DeliveryAttentionPage> {
        let limit = limit.min(50);
        if limit == 0 { return Ok(DeliveryAttentionPage { entries: Vec::new(), next: None }); }
        let seek = if after.is_some() { "AND (created_at,id)>(?1,?2)" } else { "" };
        let sql = format!("SELECT id,account_id,state,interrupted_from,created_at FROM deliveries INDEXED BY delivery_attention_pending
            WHERE {PENDING}
            {seek} ORDER BY created_at,id LIMIT ?3");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![after.map(|cursor|cursor.created_at),after.map(|cursor|cursor.id.as_str()),limit as i64+1], |row| {
            let interrupted: Option<String> = row.get(3)?;
            Ok(DeliveryAttentionEntry { id: row.get(0)?, account_id: row.get(1)?, state: crate::codec::named(row,2)?,
                interrupted_from: interrupted.map(|_|crate::codec::named(row,3)).transpose()?, created_at: row.get(4)?, file_count:0,file_bytes:0 })
        })?;
        let mut entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let next = if entries.len()>limit {
            entries.truncate(limit);
            entries.last().map(|last| DeliveryAttentionCursor {created_at:last.created_at,id:last.id.clone()})
        } else { None };
        for entry in &mut entries {
            (entry.file_count,entry.file_bytes)=self.conn.query_row(&format!("SELECT count(*),coalesce(sum(length(data)),0) FROM draft_files WHERE {RELEASABLE}"),[&entry.id],|row|Ok((row.get(0)?,row.get(1)?)))?;
        }
        Ok(DeliveryAttentionPage { entries, next })
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn keep(store: &mut Store, id: &str, state: &str, created_at: i64) {
        store.prepare_delivery(id, &draft(id), "unreadable outgoing fixture", "fixture@example.test", created_at).unwrap();
        store.conn.execute("UPDATE deliveries SET state=?2,draft='{' WHERE id=?1", [id,state]).unwrap();
    }

    #[test]
    fn attention_is_metadata_only_and_pages_without_skipping_equal_dates() {
        let mut store=store_with_work();
        assert!(store.delivery_attention(None,50).unwrap().entries.is_empty());
        for i in 0..60 {
            keep(&mut store,&format!("attention-{i:02}"),if i%2==0 {"accepted"} else {"uncertain"},100+i/4);
        }
        for state in ["held","claimed","complete","cancelled"] {
            keep(&mut store,&format!("excluded-{state}"),state,0);
        }
        keep(&mut store,"stopped-hold","held",200);
        assert!(matches!(store.cancel_delivery("stopped-hold"),Err(crate::Error::UnreadableDeliveryDraft)));
        store.conn.execute("UPDATE deliveries SET interrupted_from='held' WHERE id='attention-01'",[]).unwrap();
        let first=store.delivery_attention(None,3).unwrap();
        assert_eq!(first.entries.iter().map(|row|row.id.as_str()).collect::<Vec<_>>(),["attention-00","attention-01","attention-02"]);
        assert_eq!(first.entries[0].state,DeliveryState::Accepted);
        assert_eq!(first.entries[1].interrupted_from,Some(DeliveryState::Held));
        assert!(first.entries.iter().all(|row|row.account_id=="work"));
        assert_eq!(store.delivery_attention(None,usize::MAX).unwrap().entries.len(),50);
        assert!(store.delivery_attention(None,0).unwrap().entries.is_empty());
        let mut seen=first.entries;let mut next=first.next;
        while let Some(cursor)=next {
            let page=store.delivery_attention(Some(&cursor),3).unwrap();
            seen.extend(page.entries);next=page.next;
        }
        assert_eq!(seen.len(),61);
        assert_eq!(seen.iter().map(|row|row.id.as_str()).collect::<std::collections::HashSet<_>>().len(),61);
        assert_eq!(seen.last().unwrap().id,"stopped-hold");
        assert_eq!(seen.last().unwrap().state,DeliveryState::Held);
        let end=DeliveryAttentionCursor {created_at:200,id:"stopped-hold".into()};
        assert_eq!(store.delivery_attention(Some(&end),50).unwrap(),DeliveryAttentionPage {entries:vec![],next:None});
    }
    #[test]
    fn selected_delivery_preview_is_bounded_and_never_hides_retained_corruption() {
        let mut store=store_with_work();
        let original=draft("preview");
        store.prepare_delivery("preview",&original,"outgoing","preview@example.test",0).unwrap();
        assert!(store.delivery_attention_draft("preview").unwrap().is_none(),"an ordinary active hold is not a review item");
        store.conn.execute("UPDATE deliveries SET state='uncertain' WHERE id='preview'",[]).unwrap();
        assert_eq!(store.delivery_attention_draft("preview").unwrap(),Some(original.clone()));
        store.conn.execute("UPDATE deliveries SET draft='{' WHERE id='preview'",[]).unwrap();
        assert!(matches!(store.delivery_attention_draft("preview"),Err(crate::Error::UnreadableDeliveryDraft)));
        assert_eq!(store.delivery_attention(None,50).unwrap().entries.len(),1);
        let mut large=original;large.body="x".repeat(16*1024*1024);
        let json=serde_json::to_string(&large).unwrap();
        store.conn.execute("UPDATE deliveries SET draft=?1 WHERE id='preview'",[&json]).unwrap();
        assert!(matches!(store.delivery_attention_draft("preview"),Err(crate::Error::Invalid(_))));
        assert_eq!(store.conn.query_row("SELECT length(draft) FROM deliveries WHERE id='preview'",[],|r|r.get::<_,i64>(0)).unwrap(),json.len() as i64);
        store.conn.execute("UPDATE deliveries SET resolution='reviewed',resolved_at=1 WHERE id='preview'",[]).unwrap();
        assert!(store.delivery_attention_draft("preview").unwrap().is_none());
        assert!(store.delivery_attention(None,50).unwrap().entries.is_empty());
        assert!(store.delivery_attention_draft("missing").unwrap().is_none());
        store.conn.execute_batch("DROP TABLE deliveries").unwrap();
        assert!(store.delivery_attention_draft("preview").is_err());
        assert!(store.delivery_attention(None,50).is_err());
    }

    #[test]
    fn attention_does_not_scan_unrelated_active_holds() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        let mut store=store_with_work();
        keep(&mut store,"needs-review","uncertain",100);
        store.conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<5000)
            INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at)
            SELECT printf('active-%d',i),printf('active-%d',i),'work','{','{','fixture@example.test','held',0 FROM n").unwrap();
        let steps=Arc::new(AtomicUsize::new(0));let counted=steps.clone();
        store.conn.progress_handler(1,Some(move || { counted.fetch_add(1,Ordering::Relaxed);false })).unwrap();
        let page=store.delivery_attention(None,50).unwrap();
        assert_eq!(page.entries.len(),1);assert_eq!(page.entries[0].id,"needs-review");
        assert!(steps.load(Ordering::Relaxed)<2000,"attention scanned unrelated holds: {} VM steps",steps.load(Ordering::Relaxed));
    }

    #[test]
    fn upgrading_v42_retains_existing_cancellation_intent_in_the_attention_index() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("v42.db");
        let db=rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(&crate::schema::MIGRATIONS[..42].join("\n")).unwrap();
        db.pragma_update(None,"user_version",42).unwrap();
        db.execute_batch("INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at) VALUES('old-undo','old-draft','work','{','{','fixture@example.test','held',0);
            INSERT INTO delivery_cancellations(id) VALUES('old-undo');").unwrap();
        drop(db);
        let mut store=Store::open(path).unwrap();
        assert_eq!(store.conn.query_row("SELECT cancel_intent FROM deliveries WHERE id='old-undo'",[],|row|row.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(store.delivery_attention(None,50).unwrap().entries[0].id,"old-undo");
        assert!(!store.claim_delivery("old-undo").unwrap());
    }

    #[test]
    fn review_copy_is_independent_idempotent_and_never_recreates_a_resolved_copy() {
        let mut store=store_with_work();
        let mut original=draft("copy-source");
        original.attachments.push(store.add_draft_file(&original.id,"notes.txt","text/plain",b"private fixture",0).unwrap());
        store.prepare_delivery("copy-operation",&original,"outgoing","copy@example.test",0).unwrap();
        store.claim_delivery("copy-operation").unwrap();store.uncertain_delivery("copy-operation","no receipt").unwrap();
        let copy=store.copy_delivery_recovery("copy-operation",1).unwrap();
        assert!(copy.available);
        assert!(copy.recovery_id.starts_with("delivery-"));
        let saved=store.local_draft_recovery(&copy.recovery_id).unwrap().unwrap();
        assert_eq!(saved.draft,moss_core::Draft {updated_at:1,..original.clone()});
        assert!(saved.missing_files.is_empty());
        assert_eq!(store.conn.query_row("SELECT data FROM local_draft_recovery_files WHERE recovery_id=?1",[&copy.recovery_id],|r|r.get::<_,Vec<u8>>(0)).unwrap(),b"private fixture");
        assert_eq!(store.copy_delivery_recovery("copy-operation",2).unwrap(),copy);
        assert_eq!(store.local_draft_recovery_index().unwrap().len(),1);
        assert_eq!(store.delivery_state("copy-operation").unwrap(),Some(DeliveryState::Uncertain));
        assert_eq!(store.delivery("copy-operation").unwrap().unwrap().draft,original);
        store.discard_local_draft_recovery(&copy.recovery_id,&saved.version).unwrap();
        assert_eq!(store.copy_delivery_recovery("copy-operation",3).unwrap(),DeliveryReviewCopy {recovery_id:copy.recovery_id,available:false});
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
        assert!(!store.claim_delivery("copy-operation").unwrap());
    }

    #[test]
    fn review_copy_refuses_sent_active_missing_and_damaged_deliveries() {
        for state in ["accepted","held","claimed","complete","cancelled"] {
            let mut store=store_with_work();
            let original=draft("not-copyable");
            store.prepare_delivery("not-copyable",&original,"outgoing","fixture@example.test",0).unwrap();
            store.conn.execute("UPDATE deliveries SET state=?1",[state]).unwrap();
            assert!(store.copy_delivery_recovery("not-copyable",1).is_err(),"{state}");
            assert!(store.local_draft_recovery_index().unwrap().is_empty());
            assert_eq!(store.delivery("not-copyable").unwrap().unwrap().draft,original);
        }
        let mut store=store_with_work();
        assert!(store.copy_delivery_recovery("missing",1).is_err());
        keep(&mut store,"broken-copy","uncertain",0);
        assert!(matches!(store.copy_delivery_recovery("broken-copy",1),Err(crate::Error::UnreadableDeliveryDraft)));
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
        assert_eq!(store.delivery_attention(None,50).unwrap().entries.len(),1);
    }

    #[test]
    fn selected_preview_refuses_a_snapshot_from_another_draft_or_account() {
        for wrong_account in [false,true] {
            let mut store=store_with_work();let original=draft("scoped-copy");
            store.prepare_delivery("scoped-operation",&original,"{}","fixture@example.test",0).unwrap();
            let mut changed=original;
            if wrong_account {changed.from_account_id="different-account".into();} else {changed.id="different-draft".into();}
            store.conn.execute("UPDATE deliveries SET state='uncertain',draft=?1 WHERE id='scoped-operation'",[serde_json::to_string(&changed).unwrap()]).unwrap();
            assert!(matches!(store.delivery_attention_draft("scoped-operation"),Err(crate::Error::UnreadableDeliveryDraft)));
            assert!(store.copy_delivery_recovery("scoped-operation",1).is_err());
            assert!(store.local_draft_recovery_index().unwrap().is_empty());
        }
    }

    #[test]
    fn copy_faults_and_conflicting_recovery_receipts_preserve_the_original() {
        let mut store=store_with_work();let original=draft("receipt-copy");
        store.prepare_delivery("receipt-operation",&original,"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='uncertain'",[]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_copy BEFORE INSERT ON local_draft_recoveries BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.copy_delivery_recovery("receipt-operation",1).is_err());
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
        assert_eq!(store.delivery("receipt-operation").unwrap().unwrap().draft,original);
        store.conn.execute_batch("DROP TRIGGER refuse_copy").unwrap();
        let copy=store.copy_delivery_recovery("receipt-operation",1).unwrap();
        let saved=store.local_draft_recovery(&copy.recovery_id).unwrap().unwrap();
        let changed=moss_core::Draft {body:"different copy".into(),..saved.draft};
        let replacement=store.keep_local_draft_recovery(&copy.recovery_id,Some(&saved.version),&changed,2).unwrap();
        assert!(matches!(store.copy_delivery_recovery("receipt-operation",3),Err(crate::Error::RequestConflict)));
        store.discard_local_draft_recovery(&copy.recovery_id,&replacement.version).unwrap();
        assert!(matches!(store.copy_delivery_recovery("receipt-operation",4),Err(crate::Error::RequestConflict)),"a resolved receipt for different text does not prove this delivery was copied");
        assert_eq!(store.delivery("receipt-operation").unwrap().unwrap().draft,original);
    }

    #[test]
    fn resolving_a_copied_delivery_retains_identity_and_replays_without_reading_payload() {
        let mut store=store_with_work();let original=draft("resolve-copy");
        store.prepare_delivery("resolve-operation",&original,"outgoing","fixture@example.test",0).unwrap();
        store.claim_delivery("resolve-operation").unwrap();store.uncertain_delivery("resolve-operation","no receipt").unwrap();
        let outcome=store.resolve_delivery("resolve-operation",DeliveryResolution::Copied,1).unwrap();
        let DeliveryResolutionOutcome::Resolved {recovery_id:Some(id)}=&outcome else {panic!("missing committed copy")};
        assert_eq!(store.local_draft_recovery(id).unwrap().unwrap().draft.body,original.body);
        assert!(store.delivery_attention(None,50).unwrap().entries.is_empty());
        assert_eq!(store.delivery_state("resolve-operation").unwrap(),Some(DeliveryState::Uncertain));
        assert!(!store.claim_delivery("resolve-operation").unwrap());
        store.conn.execute("UPDATE deliveries SET draft='{',outgoing='{' WHERE id='resolve-operation'",[]).unwrap();
        assert_eq!(store.resolve_delivery("resolve-operation",DeliveryResolution::Copied,2).unwrap(),outcome);
        assert!(matches!(store.resolve_delivery("resolve-operation",DeliveryResolution::Reviewed,3),Err(crate::Error::RequestConflict)));
        assert_eq!(store.local_draft_recovery_index().unwrap().len(),1);
    }

    #[test]
    fn explicit_review_handles_corrupt_unsent_and_accepted_evidence_without_claiming_delivery() {
        for state in ["uncertain","accepted","held"] {
            let mut store=store_with_work();keep(&mut store,"review",state,0);
            if state=="held" {assert!(store.cancel_delivery("review").is_err());}
            assert_eq!(store.resolve_delivery("review",DeliveryResolution::Reviewed,1).unwrap(),DeliveryResolutionOutcome::Resolved {recovery_id:None});
            assert!(store.delivery_attention(None,50).unwrap().entries.is_empty());
            assert!(!store.claim_delivery("review").unwrap());
            assert_eq!(store.delivery_state("review").unwrap(),Some(if state=="accepted" {DeliveryState::Accepted} else {DeliveryState::Uncertain}));
            if state=="held" {assert_eq!(store.conn.query_row("SELECT interrupted_from FROM deliveries WHERE id='review'",[],|r|r.get::<_,String>(0)).unwrap(),"held");}
        }
        for state in ["complete","cancelled"] {
            let mut store=store_with_work();keep(&mut store,"finished",state,0);
            assert_eq!(store.resolve_delivery("finished",DeliveryResolution::Copied,1).unwrap(),DeliveryResolutionOutcome::AlreadyFinished);
            assert!(store.local_draft_recovery_index().unwrap().is_empty());
        }
        let mut store=store_with_work();keep(&mut store,"claimed","claimed",0);
        assert_eq!(store.resolve_delivery("claimed",DeliveryResolution::Reviewed,1).unwrap(),DeliveryResolutionOutcome::Sending);
        keep(&mut store,"active","held",0);
        assert!(store.resolve_delivery("active",DeliveryResolution::Reviewed,1).is_err());
        assert!(store.resolve_delivery("missing",DeliveryResolution::Reviewed,1).is_err());
    }

    #[test]
    fn reviewed_accepted_mail_cannot_resume_local_repair_or_reminders() {
        let mut store=store_with_work();keep(&mut store,"reviewed-accepted","accepted",0);
        store.resolve_delivery("reviewed-accepted",DeliveryResolution::Reviewed,1).unwrap();
        assert!(store.claim_delivery_repairs(2).unwrap().is_empty());
        assert!(!store.apply_delivery_reminder("reviewed-accepted","missing","missing",10,2).unwrap());
        assert_eq!(store.conn.query_row("SELECT reminder_applied FROM deliveries WHERE id='reviewed-accepted'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(!store.complete_delivery("reviewed-accepted").unwrap());
        assert_eq!(store.delivery_state("reviewed-accepted").unwrap(),Some(DeliveryState::Accepted));
    }

    #[test]
    fn review_retires_only_its_schedule_and_records_only_proven_acceptance() {
        for state in ["accepted","uncertain"] {
            let mut store=store_with_work();let d=draft("scheduled-review");
            store.schedule_operation("schedule-review-op",&d,"{}",10,0).unwrap();
            store.prepare_delivery("schedule-review-op",&d,"{}","receipt@example.test",0).unwrap();
            store.conn.execute("UPDATE deliveries SET state=?1 WHERE id='schedule-review-op'",[state]).unwrap();
            store.resolve_delivery("schedule-review-op",DeliveryResolution::Reviewed,1).unwrap();
            assert!(store.scheduled_emails().unwrap().is_empty());
            let receipt:Option<String>=store.conn.query_row("SELECT outcome FROM schedule_outcomes WHERE id='schedule-review-op'",[],|r|r.get(0)).optional().unwrap();
            assert_eq!(receipt,if state=="accepted" {Some("completed".into())} else {None});
        }
    }

    #[test]
    fn failed_schedule_retirement_rolls_back_the_review() {
        for fault in ["ignore_delete","abort_delete","ignore_receipt","abort_receipt"] {
            let mut store=store_with_work();let d=draft("scheduled-review");
            store.schedule_operation("schedule-review-op",&d,"{}",10,0).unwrap();
            store.prepare_delivery("schedule-review-op",&d,"{}","receipt@example.test",0).unwrap();
            store.conn.execute("UPDATE deliveries SET state='accepted' WHERE id='schedule-review-op'",[]).unwrap();
            let target=if fault.ends_with("delete") {"DELETE ON scheduled"} else {"INSERT ON schedule_outcomes"};
            let error=if fault.starts_with("ignore") {"IGNORE"} else {"ABORT,'fixture'"};
            store.conn.execute_batch(&format!("CREATE TRIGGER fail_review BEFORE {target} BEGIN SELECT RAISE({error}); END;")).unwrap();
            assert!(store.resolve_delivery("schedule-review-op",DeliveryResolution::Reviewed,1).is_err(),"{fault}");
            assert!(!store.delivery_resolved("schedule-review-op").unwrap());
            assert_eq!(store.scheduled_emails().unwrap().len(),1);
            store.conn.execute_batch("DROP TRIGGER fail_review").unwrap();
            store.resolve_delivery("schedule-review-op",DeliveryResolution::Reviewed,2).unwrap();
            assert!(store.scheduled_emails().unwrap().is_empty());
        }
    }

    #[test]
    fn review_releases_only_original_files_and_retains_newer_imports_even_after_clock_rollback() {
        for action in [DeliveryResolution::Reviewed,DeliveryResolution::Copied] {
            let mut store=store_with_work();let mut d=draft("file-review");
            let old=store.add_draft_file(&d.id,"old.txt","text/plain",b"original",100).unwrap();
            d.attachments.push(old.clone());
            store.prepare_delivery("file-review-op",&d,"{}","file@example.test",100).unwrap();
            store.conn.execute("UPDATE deliveries SET state='uncertain' WHERE id='file-review-op'",[]).unwrap();
            let new=store.add_draft_file(&d.id,"new.txt","text/plain",b"new import",1).unwrap();
            let visible=serde_json::to_value(store.delivery_attention(None,50).unwrap()).unwrap();
            assert_eq!(visible["entries"][0]["fileCount"],1);
            assert_eq!(visible["entries"][0]["fileBytes"],8);
            let outcome=store.resolve_delivery("file-review-op",action,101).unwrap();
            assert!(store.draft_file_for(&d.id,&old.id).unwrap().is_none());
            assert!(store.draft_file_for(&d.id,&new.id).unwrap().is_some());
            if let DeliveryResolutionOutcome::Resolved { recovery_id:Some(id) }=outcome {
                let copy=store.local_draft_recovery(&id).unwrap().unwrap();
                assert_eq!(copy.draft.attachments.len(),1);
            }
        }
    }

    #[test]
    fn ignored_file_cleanup_rolls_back_review_but_preserves_the_committed_copy() {
        let mut store=store_with_work();let mut d=draft("file-fault");
        let file=store.add_draft_file(&d.id,"old.txt","text/plain",b"original",0).unwrap();d.attachments.push(file.clone());
        store.prepare_delivery("file-fault-op",&d,"{}","file@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='uncertain' WHERE id='file-fault-op'",[]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER ignore_file BEFORE DELETE ON draft_files BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.resolve_delivery("file-fault-op",DeliveryResolution::Copied,1).is_err());
        assert!(!store.delivery_resolved("file-fault-op").unwrap());
        assert_eq!(store.local_draft_recovery_index().unwrap().len(),1);
        assert!(store.draft_file_for(&d.id,&file.id).unwrap().is_some());
        store.conn.execute_batch("DROP TRIGGER ignore_file").unwrap();
        store.resolve_delivery("file-fault-op",DeliveryResolution::Copied,2).unwrap();
        assert!(store.draft_file_for(&d.id,&file.id).unwrap().is_none());
    }

    #[test]
    fn upgrade_file_fence_handles_malformed_attachment_entries_without_losing_legacy_files() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("v42-files.db");
        let db=rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(&crate::schema::MIGRATIONS[..42].join("\n")).unwrap();
        db.pragma_update(None,"user_version",42).unwrap();
        db.execute_batch("INSERT INTO draft_files(id,draft_id,name,content_type,data,added_at) VALUES(1,'legacy','kept','text/plain',X'01',0),(2,'other','other','text/plain',X'02',0),(3,'legacy','new','text/plain',X'03',0);").unwrap();
        let payload=r#"{"attachments":["broken",null,7,{"id":"1"},{"id":"2"}]}"#;
        db.execute("INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at) VALUES('legacy','legacy','work',?1,'{','fixture@example.test','uncertain',0)",[payload]).unwrap();
        drop(db);
        let mut store=Store::open(path).unwrap();
        assert_eq!(store.conn.query_row("SELECT file_ceiling FROM deliveries WHERE id='legacy'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        store.resolve_delivery("legacy",DeliveryResolution::Reviewed,1).unwrap();
        assert!(store.draft_file_for("legacy","1").unwrap().is_none());
        assert!(store.draft_file_for("other","2").unwrap().is_some());
        assert!(store.draft_file_for("legacy","3").unwrap().is_some());
    }

    #[test]
    fn review_preserves_replacement_schedules_and_other_file_owners() {
        for owner in ["draft","schedule"] {
            let mut store=store_with_work();let mut d=draft("shared-review");
            let file=store.add_draft_file(&d.id,"kept.txt","text/plain",b"shared",0).unwrap();d.attachments.push(file.clone());
            store.prepare_delivery("old-review-op",&d,"{}","old@example.test",0).unwrap();
            store.conn.execute("UPDATE deliveries SET state='uncertain' WHERE id='old-review-op'",[]).unwrap();
            // Simulate a later durable owner without violating public save guards.
            match owner {
                "draft" => {store.conn.execute("INSERT INTO drafts(id,account_id,mode,to_json,cc_json,bcc_json,subject,body,body_html,remind_if_no_reply,updated_at) VALUES(?1,'work','new','[]','[]','[]','','','',0,0)",[&d.id]).unwrap();},
                _ => {store.conn.execute("INSERT INTO scheduled(draft_id,account_id,draft,outgoing,send_at,next_try,operation_id) VALUES(?1,'work',?2,'{}',10,10,'replacement-op')",params![d.id,serde_json::to_string(&d).unwrap()]).unwrap();},
            }
            store.resolve_delivery("old-review-op",DeliveryResolution::Reviewed,1).unwrap();
            assert!(store.draft_file_for(&d.id,&file.id).unwrap().is_some(),"{owner}");
            if owner=="schedule" {assert_eq!(store.scheduled_operation(&d.id).unwrap().as_deref(),Some("replacement-op"));}
        }
    }

    #[test]
    fn ignored_review_mark_preserves_original_and_committed_copy_for_retry() {
        let mut store=store_with_work();let d=draft("mark-fault");
        store.prepare_delivery("mark-fault-op",&d,"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='uncertain' WHERE id='mark-fault-op'",[]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER ignore_review BEFORE UPDATE OF resolution ON deliveries BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.resolve_delivery("mark-fault-op",DeliveryResolution::Copied,1).is_err());
        assert!(!store.delivery_resolved("mark-fault-op").unwrap());
        assert_eq!(store.local_draft_recovery_index().unwrap().len(),1);
        store.conn.execute_batch("DROP TRIGGER ignore_review").unwrap();
        store.resolve_delivery("mark-fault-op",DeliveryResolution::Copied,2).unwrap();
        assert!(store.delivery_resolved("mark-fault-op").unwrap());
        assert_eq!(store.local_draft_recovery_index().unwrap().len(),1);
    }

    #[test]
    fn inconsistent_stopped_schedule_requires_schedule_resolution_first() {
        let mut store=store_with_work();let d=draft("stopped-schedule");
        store.schedule_operation("stopped-schedule-op",&d,"{}",10,0).unwrap();
        store.prepare_delivery("stopped-schedule-op",&d,"{}","fixture@example.test",0).unwrap();
        store.conn.execute_batch("INSERT INTO delivery_cancellations(id) VALUES('stopped-schedule-op'); UPDATE deliveries SET cancel_intent=1 WHERE id='stopped-schedule-op';").unwrap();
        assert!(store.resolve_delivery("stopped-schedule-op",DeliveryResolution::Reviewed,1).is_err());
        assert!(store.resolve_delivery("stopped-schedule-op",DeliveryResolution::Copied,1).is_err());
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
        assert!(!store.delivery_resolved("stopped-schedule-op").unwrap());
        assert_eq!(store.scheduled_emails().unwrap().len(),1);
        assert!(!store.claim_delivery("stopped-schedule-op").unwrap());
    }

    #[test]
    fn a_resolved_delivery_never_enters_the_scheduled_retry_queue() {
        let mut store=store_with_work();let d=draft("resolved-queue");
        store.prepare_delivery("resolved-queue-op",&d,"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='accepted' WHERE id='resolved-queue-op'",[]).unwrap();
        store.resolve_delivery("resolved-queue-op",DeliveryResolution::Reviewed,1).unwrap();
        store.conn.execute("INSERT INTO scheduled(draft_id,account_id,draft,outgoing,send_at,next_try,operation_id) VALUES(?1,'work',?2,'{}',0,0,'resolved-queue-op')",params![d.id,serde_json::to_string(&d).unwrap()]).unwrap();
        assert!(store.due_emails(2).unwrap().is_empty());
        assert_eq!(store.next_due().unwrap(),None);
    }

    #[test]
    fn explicit_review_allows_account_retirement_while_preserving_copies_and_receipts() {
        for state in ["accepted","uncertain"] {
            let mut store=store_with_work();let mut d=draft("retired-review");
            d.attachments.push(store.add_draft_file(&d.id,"kept.txt","text/plain",b"independent bytes",0).unwrap());
            store.prepare_delivery("retired-review-op",&d,"{}","fixture@example.test",0).unwrap();
            store.conn.execute("UPDATE deliveries SET state=?1 WHERE id='retired-review-op'",[state]).unwrap();
            assert!(store.remove_account("work").is_err());
            let action=if state=="accepted" {DeliveryResolution::Reviewed} else {DeliveryResolution::Copied};
            let outcome=store.resolve_delivery("retired-review-op",action,1).unwrap();
            assert!(store.remove_account("work").unwrap());
            assert_eq!(store.conn.query_row("SELECT account_retired FROM deliveries WHERE id='retired-review-op'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert_eq!(store.resolve_delivery("retired-review-op",action,2).unwrap(),outcome);
            assert!(!store.claim_delivery("retired-review-op").unwrap());
            if let DeliveryResolutionOutcome::Resolved {recovery_id:Some(id)}=outcome {
                assert_eq!(store.local_draft_recovery(&id).unwrap().unwrap().draft.body,d.body);
                assert_eq!(store.conn.query_row("SELECT data FROM local_draft_recovery_files WHERE recovery_id=?1",[id],|r|r.get::<_,Vec<u8>>(0)).unwrap(),b"independent bytes");
            }
        }
    }

    #[test]
    fn retrying_copy_after_its_discard_cannot_resolve_or_release_the_original() {
        let mut store=store_with_work();let mut d=draft("discarded-copy");
        let file=store.add_draft_file(&d.id,"keep.txt","text/plain",b"original",0).unwrap();d.attachments.push(file.clone());
        store.prepare_delivery("discarded-copy-op",&d,"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='uncertain' WHERE id='discarded-copy-op'",[]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER ignore_review BEFORE UPDATE OF resolution ON deliveries BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.resolve_delivery("discarded-copy-op",DeliveryResolution::Copied,1).is_err());
        let copy=store.local_draft_recovery_index().unwrap().pop().unwrap();
        store.discard_local_draft_recovery(&copy.id,&copy.version).unwrap();
        store.conn.execute_batch("DROP TRIGGER ignore_review").unwrap();
        assert!(matches!(store.resolve_delivery("discarded-copy-op",DeliveryResolution::Copied,2), Err(crate::Error::Invalid(message)) if message.contains("recovery copy was discarded")));
        assert!(!store.delivery_resolved("discarded-copy-op").unwrap());
        assert!(store.draft_file_for(&d.id,&file.id).unwrap().is_some());
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
        store.resolve_delivery("discarded-copy-op",DeliveryResolution::Reviewed,3).unwrap();
        assert!(store.draft_file_for(&d.id,&file.id).unwrap().is_none());
    }

    #[test]
    fn an_adopted_copy_is_a_valid_handoff_when_review_is_retried() {
        let mut store=store_with_work();let d=draft("adopted-review");
        store.prepare_delivery("adopted-review-op",&d,"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='uncertain' WHERE id='adopted-review-op'",[]).unwrap();
        let copy=store.copy_delivery_recovery("adopted-review-op",1).unwrap();
        let saved=store.local_draft_recovery(&copy.recovery_id).unwrap().unwrap();
        store.adopt_local_draft_recovery(&copy.recovery_id,&saved.version,"adopted-standalone","work",&[],2).unwrap();
        assert_eq!(store.resolve_delivery("adopted-review-op",DeliveryResolution::Copied,3).unwrap(),DeliveryResolutionOutcome::Resolved {recovery_id:Some(copy.recovery_id)});
        assert_eq!(store.editable_draft("adopted-standalone").unwrap().unwrap().body,d.body);
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
        // A later deliberate deletion does not undo the completed handoff or resurrect mail.
        assert!(store.delete_draft("adopted-standalone").unwrap());
        assert!(matches!(store.resolve_delivery("adopted-review-op",DeliveryResolution::Copied,4).unwrap(),DeliveryResolutionOutcome::Resolved { .. }));
        assert!(store.editable_draft("adopted-standalone").unwrap().is_none());
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
    }

}
