//! Retire terminal payloads while retaining durable replay identities.
use crate::{Result, Store};
use moss_core::Draft;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

/// Canonical, domain-separated version 1 fingerprints. Future shape changes require
/// an explicit version decision; an unknown compacted hash always fails closed.
pub fn delivery_fingerprint(domain: &str, mut parts: Vec<serde_json::Value>) -> String {
    let canonical:Vec<Vec<u8>>=parts.iter_mut().map(|part| {
        part.sort_all_objects();
        serde_json::to_vec(part).expect("JSON values serialize")
    }).collect();
    fingerprint_bytes(domain,canonical.iter().map(Vec::as_slice))
}

fn fingerprint_bytes<'a>(domain: &str, parts: impl IntoIterator<Item=&'a [u8]>) -> String {
    let mut hash=Sha256::new();
    hash.update(b"moss-delivery-v1\0");
    hash.update((domain.len() as u64).to_be_bytes()); hash.update(domain.as_bytes());
    for bytes in parts {
        hash.update((bytes.len() as u64).to_be_bytes()); hash.update(bytes);
    }
    format!("v1:{:x}",hash.finalize())
}

pub(crate) fn storage_fingerprint(draft: &Draft, outgoing: &str, message_id: &str) -> String {
    let snapshot=Draft {revision:None,..draft.clone()};
    let mut value=serde_json::to_value(snapshot).expect("draft serializes");
    value.sort_all_objects();
    let bytes=serde_json::to_vec(&value).expect("JSON values serialize");
    fingerprint_bytes("storage",[bytes.as_slice(),outgoing.as_bytes(),message_id.as_bytes()])
}

const ELIGIBLE: &str = "(state IN ('complete','cancelled') OR resolution IS NOT NULL)
    AND NOT EXISTS(SELECT 1 FROM scheduled WHERE scheduled.operation_id=deliveries.id)";
// Legacy oversized payloads are retired without guessing a replay fingerprint.
const MAX_BACKFILL_BYTES: i64 = 64 * 1024 * 1024;

impl Store {
    /// Called inside account removal's transaction. These rows need no payload reads.
    pub(crate) fn compact_prehashed_account(&self, account: &str, now: i64) -> Result<()> {
        let pending=format!("account_id=?1 AND storage_hash IS NOT NULL AND logical_hash IS NOT NULL AND compacted_at IS NULL AND {ELIGIBLE}");
        self.conn.execute(&format!("UPDATE deliveries SET draft='',outgoing='',raw=NULL,source=NULL,error=NULL,compacted_at=?2 WHERE {pending}"),params![account,now])?;
        let remains:bool=self.conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM deliveries WHERE account_id=?1 AND storage_hash IS NOT NULL AND logical_hash IS NOT NULL AND {ELIGIBLE}
            AND (compacted_at IS NULL OR draft!='' OR outgoing!='' OR raw IS NOT NULL OR source IS NOT NULL OR error IS NOT NULL))"),[account],|row|row.get(0))?;
        if remains { return Err(crate::Error::Invalid("The account's completed delivery data could not be retired. The account has been kept.".into())); }
        Ok(())
    }

    /// One bounded maintenance unit. The caller supplies the versioned logical hash.
    pub fn compact_delivery_step(&mut self, now: i64, logical: impl FnOnce(&Draft, &str) -> Option<String>) -> Result<bool> {
        self.delivery_transaction(|store| {
            let encoding:String=store.conn.pragma_query_value(None,"encoding",|row|row.get(0))?;
            // SQLite converts TEXT to UTF-8 on read. Conservatively budget that
            // conversion too for old UTF-16 databases; BLOB bytes stay unchanged.
            let divisor=if encoding=="UTF-8" {1} else {2};
            let candidate:Option<(i64,Option<Vec<u8>>,Option<Vec<u8>>,Option<Vec<u8>>)>=store.conn.query_row(
                &format!("SELECT rowid,CASE WHEN (storage_hash IS NULL OR logical_hash IS NULL) AND octet_length(draft)+octet_length(outgoing)<=?1 THEN draft END,
                 CASE WHEN (storage_hash IS NULL OR logical_hash IS NULL) AND octet_length(draft)+octet_length(outgoing)<=?1 THEN outgoing END,
                 CASE WHEN storage_hash IS NULL AND octet_length(message_id)<=?2 THEN message_id END
                 FROM deliveries WHERE compacted_at IS NULL AND {ELIGIBLE} ORDER BY account_retired DESC,created_at,id LIMIT 1"),params![MAX_BACKFILL_BYTES/divisor,4096/divisor],
                |r|Ok((r.get(0)?,r.get_ref(1)?.as_bytes().ok().map(<[u8]>::to_vec),r.get_ref(2)?.as_bytes().ok().map(<[u8]>::to_vec),r.get_ref(3)?.as_bytes().ok().map(<[u8]>::to_vec)))).optional()?;
            let Some((id,draft,outgoing,message_id))=candidate else { return Ok(false); };
            let parsed=draft.as_deref().and_then(|value|serde_json::from_slice::<Draft>(value).ok());
            let outgoing=outgoing.as_deref().and_then(|bytes|std::str::from_utf8(bytes).ok());
            let message_id=message_id.as_deref().and_then(|bytes|std::str::from_utf8(bytes).ok());
            let (strict,request)=match (parsed,outgoing) {
                (Some(draft),Some(outgoing))=>(message_id.map(|message_id|storage_fingerprint(&draft,outgoing,message_id)),logical(&draft,outgoing)),
                _=>(None,None),
            };
            store.conn.execute(&format!("UPDATE deliveries SET storage_hash=coalesce(storage_hash,?2),logical_hash=coalesce(logical_hash,?3),
                draft='',outgoing='',raw=NULL,source=NULL,error=NULL,compacted_at=?4 WHERE rowid=?1 AND compacted_at IS NULL AND {ELIGIBLE}"),params![id,strict,request,now])?;
            let saved:bool=store.conn.query_row("SELECT compacted_at IS NOT NULL AND draft='' AND outgoing='' AND raw IS NULL AND source IS NULL AND error IS NULL FROM deliveries WHERE rowid=?1",[id],|r|r.get(0))?;
            if !saved { return Err(crate::Error::Invalid("The completed delivery data could not be retired.".into())); }
            Ok(true)
        })
    }

    pub fn delivery_replay_hash(&self, id: &str) -> Result<Option<(Option<String>,bool)>> {
        Ok(self.conn.query_row("SELECT logical_hash,compacted_at IS NOT NULL FROM deliveries WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?)
    }

    pub(crate) fn compacted_storage_hash(&self, id: &str) -> Result<Option<Option<String>>> {
        Ok(self.conn.query_row("SELECT storage_hash FROM deliveries WHERE id=?1 AND compacted_at IS NOT NULL",[id],|r|r.get(0)).optional()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::*;
    use crate::DeliveryState;

    #[derive(Default)]
    struct BackfillProbe(std::cell::Cell<usize>);
    impl BackfillProbe {
        fn callback(&self) -> impl FnOnce(&moss_core::Draft,&str)->Option<String> + '_ {
            move |_,_| { self.0.set(self.0.get()+1); None }
        }
    }

    #[test]
    fn storage_fingerprint_v1_has_a_fixed_protocol_vector() {
        assert_eq!(super::storage_fingerprint(&draft("protocol-vector"),"opaque outgoing\nwith bytes","fixture@example.test"),"v1:baf6d264df756eb17463b3e3d7804e0087ee0a21bf178cad8ff61d9f1d500754");
    }

    #[test]
    fn account_removal_atomically_compacts_prehashed_terminal_payloads() {
        let mut store=store_with_work();let d=draft("remove-payload");
        store.prepare_delivery_fingerprinted("remove-payload",&d,"private outgoing","fixture@example.test",0,Some("v1:logical"),|_|Ok(None)).unwrap();
        store.conn.execute("UPDATE deliveries SET state='complete' WHERE id='remove-payload'",[]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_compaction BEFORE UPDATE OF compacted_at ON deliveries BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.accounts().unwrap().len(),1);
        assert_eq!(store.delivery("remove-payload").unwrap().unwrap().outgoing,"private outgoing");
        store.conn.execute_batch("DROP TRIGGER refuse_compaction").unwrap();
        store.conn.execute_batch("CREATE TRIGGER retain_compacted_content AFTER UPDATE OF compacted_at ON deliveries BEGIN UPDATE deliveries SET outgoing='retained' WHERE id=NEW.id; END;").unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.accounts().unwrap().len(),1);
        store.conn.execute_batch("DROP TRIGGER retain_compacted_content").unwrap();
        assert!(store.remove_account("work").unwrap());
        assert_eq!(store.delivery_replay_hash("remove-payload").unwrap(),Some((Some("v1:logical".into()),true)));
        assert!(!store.prepare_delivery("remove-payload",&d,"private outgoing","fixture@example.test",1).unwrap());
        assert!(store.delivery("remove-payload").is_err());
    }

    #[test]
    fn fingerprints_are_canonical_domain_separated_and_unambiguous() {
        use serde_json::json;
        use super::delivery_fingerprint as hash;
        assert_eq!(hash("logical",vec![json!({"b":2,"a":{"d":4,"c":3}})]),hash("logical",vec![json!({"a":{"c":3,"d":4},"b":2})]));
        assert_ne!(hash("storage",vec![json!("ab"),json!("c")]),hash("storage",vec![json!("a"),json!("bc")]));
        assert_ne!(hash("storage",vec![json!("a")]),hash("logical",vec![json!("a")]));
    }

    #[test]
    fn legacy_fingerprints_preserve_valid_utf16_database_text() {
        let conn=rusqlite::Connection::open_in_memory().unwrap();
        conn.pragma_update(None,"encoding","UTF-16").unwrap();
        let mut store=crate::Store::init(conn).unwrap();
        store.upsert_account(&account("work","fixture@example.test")).unwrap();
        let mut d=draft("utf16");d.body="日本語 and 𝄞".into();
        store.prepare_delivery("utf16",&d,"legacy outgoing","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='complete',storage_hash=NULL WHERE id='utf16'",[]).unwrap();
        assert!(store.compact_delivery_step(1,|observed,outgoing| {
            assert_eq!(observed,&d);assert_eq!(outgoing,"legacy outgoing");Some("v1:logical".into())
        }).unwrap());
        assert_eq!(store.delivery_replay_hash("utf16").unwrap(),Some((Some("v1:logical".into()),true)));
        assert!(!store.prepare_delivery("utf16",&d,"legacy outgoing","fixture@example.test",2).unwrap());
    }

    #[test]
    fn maintenance_preserves_unresolved_and_owned_rows_and_skips_rehashing_new_receipts() {
        let probe=BackfillProbe::default();
        let mut store=store_with_work();
        for state in ["held","claimed","accepted","uncertain","complete","cancelled"] {
            let d=draft(state);
            store.prepare_delivery_fingerprinted(state,&d,"private outgoing","fixture@example.test",0,Some("v1:logical"),|_|Ok(None)).unwrap();
            store.conn.execute("UPDATE deliveries SET state=?1 WHERE id=?1",[state]).unwrap();
        }
        for _ in 0..2 { assert!(store.compact_delivery_step(1,probe.callback()).unwrap()); }
        assert!(!store.compact_delivery_step(2,probe.callback()).unwrap());
        for state in ["held","claimed","accepted","uncertain"] {
            assert_eq!(store.delivery(state).unwrap().unwrap().outgoing,"private outgoing");
            assert!(store.conn.execute("UPDATE deliveries SET compacted_at=1 WHERE id=?1",[state]).is_err());
        }
        assert_eq!(store.delivery_replay_hash("complete").unwrap(),Some((Some("v1:logical".into()),true)));
        assert_eq!(store.delivery_replay_hash("missing").unwrap(),None);
        assert!(!store.claim_delivery("complete").unwrap());
        store.resolve_delivery("uncertain",crate::DeliveryResolution::Reviewed,3).unwrap();
        assert!(store.compact_delivery_step(4,probe.callback()).unwrap());
        assert_eq!(store.delivery_state("uncertain").unwrap(),Some(DeliveryState::Uncertain));
        assert!(store.pending_deliveries().unwrap().iter().all(|delivery|delivery.id!="uncertain"));
        assert_eq!(probe.0.get(),0);
    }

    #[test]
    fn damaged_legacy_rows_do_not_starve_maintenance_or_authorize_replay() {
        let probe=BackfillProbe::default();
        let mut store=store_with_work();
        for id in ["damaged","invalid-utf8","valid"] {
            store.prepare_delivery(id,&draft(id),"legacy outgoing","fixture@example.test",0).unwrap();
            store.conn.execute("UPDATE deliveries SET state='complete',storage_hash=NULL WHERE id=?1",[id]).unwrap();
        }
        store.conn.execute("UPDATE deliveries SET draft='{' WHERE id='damaged'",[]).unwrap();
        store.conn.execute("UPDATE deliveries SET draft=x'ff',outgoing=x'fe' WHERE id='invalid-utf8'",[]).unwrap();
        assert!(store.compact_delivery_step(1,probe.callback()).unwrap());
        assert!(store.prepare_delivery("damaged",&draft("damaged"),"legacy outgoing","fixture@example.test",1).is_err());
        assert!(store.compact_delivery_step(1,probe.callback()).unwrap());
        assert!(store.compact_delivery_step(1,probe.callback()).unwrap());
        assert!(!store.compact_delivery_step(1,probe.callback()).unwrap());
        assert_eq!(store.delivery_replay_hash("damaged").unwrap(),Some((None,true)));
        assert_eq!(store.delivery_replay_hash("valid").unwrap(),Some((None,true)));
        assert!(!store.prepare_delivery("valid",&draft("valid"),"legacy outgoing","fixture@example.test",2).unwrap());
        assert_eq!(probe.0.get(),1);
    }

    #[test]
    fn ignored_compaction_rolls_back_without_losing_payload() {
        let mut store=store_with_work();
        store.prepare_delivery("fault",&draft("fault"),"keep","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='complete' WHERE id='fault'",[]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_compaction BEFORE UPDATE OF compacted_at ON deliveries BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.compact_delivery_step(1,|_,_|None).is_err());
        assert_eq!(store.delivery("fault").unwrap().unwrap().outgoing,"keep");
    }

    #[test]
    fn a_schedule_blocks_retirement_until_it_is_gone() {
        let probe=BackfillProbe::default();
        let mut store=store_with_work();
        let d=draft("scheduled");
        store.schedule_operation("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",&d,"{}",100,0).unwrap();
        store.prepare_delivery("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",&d,"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='complete' WHERE id='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",[]).unwrap();
        assert!(!store.compact_delivery_step(1,probe.callback()).unwrap());
        store.conn.execute("DELETE FROM scheduled WHERE operation_id='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",[]).unwrap();
        assert!(store.compact_delivery_step(1,|_,_|None).unwrap());
        assert_eq!(probe.0.get(),0);
    }

    #[test]
    fn oversized_legacy_payloads_are_retired_without_loading_or_guessing_their_hash() {
        let probe=BackfillProbe::default();
        let mut store=store_with_work();
        store.prepare_delivery("oversized",&draft("oversized"),"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='complete',storage_hash=NULL,draft=CAST(zeroblob(?1) AS TEXT) WHERE id='oversized'",[super::MAX_BACKFILL_BYTES+1]).unwrap();
        assert!(store.compact_delivery_step(1,probe.callback()).unwrap());
        assert_eq!(store.delivery_replay_hash("oversized").unwrap(),Some((None,true)));
        assert!(store.prepare_delivery("oversized",&draft("oversized"),"{}","fixture@example.test",2).is_err());
        assert_eq!(probe.0.get(),0);
    }

    #[test]
    fn maintenance_does_not_scan_unrelated_active_deliveries() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        let mut store=store_with_work();
        store.prepare_delivery("terminal",&draft("terminal"),"{}","fixture@example.test",0).unwrap();
        store.conn.execute("UPDATE deliveries SET state='complete' WHERE id='terminal'",[]).unwrap();
        store.conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<5000)
            INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at)
            SELECT printf('active-%d',i),printf('active-%d',i),'work','{','{','fixture@example.test','held',0 FROM n").unwrap();
        let steps=Arc::new(AtomicUsize::new(0));let counted=steps.clone();
        store.conn.progress_handler(1,Some(move || {counted.fetch_add(1,Ordering::Relaxed);false})).unwrap();
        assert!(store.compact_delivery_step(1,|_,_|None).unwrap());
        assert!(steps.load(Ordering::Relaxed)<2000,"scanned unrelated rows: {}",steps.load(Ordering::Relaxed));
    }

    #[test]
    fn terminal_payloads_are_removed_but_the_receipt_and_replay_identity_survive() {
        let probe=BackfillProbe::default();
        let mut store=store_with_work(); let d=draft("compact-draft");
        store.prepare_delivery("compact-op",&d,"private outgoing","fixture@example.test",0).unwrap();
        store.claim_delivery("compact-op").unwrap();
        store.accept_delivery("compact-op",b"private raw","private source").unwrap();
        store.complete_delivery_with_receipt("compact-op",None).unwrap();
        assert!(store.compact_delivery_step(1,|_,_|Some("v1:logical".into())).unwrap());
        let payload:(String,String,Option<Vec<u8>>,Option<String>)=store.conn.query_row("SELECT draft,outgoing,raw,source FROM deliveries WHERE id='compact-op'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(payload,(String::new(),String::new(),None,None));
        assert_eq!(store.delivery_state("compact-op").unwrap(),Some(DeliveryState::Complete));
        assert!(!store.prepare_delivery("compact-op",&d,"private outgoing","fixture@example.test",2).unwrap());
        let mut changed=d.clone();changed.body="changed".into();
        assert!(store.prepare_delivery("compact-op",&changed,"private outgoing","fixture@example.test",2).is_err());
        assert!(store.delivery("compact-op").is_err());
        assert!(!store.compact_delivery_step(2,probe.callback()).unwrap());
        assert_eq!(probe.0.get(),0);
    }
}
