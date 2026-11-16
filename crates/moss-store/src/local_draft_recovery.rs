//! Private, non-sendable copies of local edits that could not replace a draft.
//! They are independent of account deletion and never appear in ordinary draft sync.
use crate::{Error, Result, Store};
use moss_core::Draft;
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_OPEN_RECOVERIES: i64 = 128;
const MAX_TOTAL_BYTES: i64 = 512 * 1024 * 1024;
const MAX_FILE_BYTES: i64 = 50 * 1024 * 1024;
const MAX_FILES: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDraftRecovery {
    pub id: String,
    pub version: String,
    pub draft: Draft,
    pub missing_files: Vec<String>,
    pub updated_at: i64,
}

/// Cheap metadata remains readable even if the private snapshot is damaged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDraftRecoveryEntry {
    pub id: String,
    pub version: String,
    pub original_id: String,
    pub updated_at: i64,
}

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn digest(snapshot: &str) -> String {
    format!("{:x}", Sha256::digest(snapshot.as_bytes()))
}

impl Store {
    /// A terminal recovery receipt proves the exact snapshot was copied before review.
    pub(crate) fn local_recovery_matches_draft(&self, id: &str, draft: &Draft) -> Result<bool> {
        let mut normalized = draft.clone();normalized.updated_at = 0;
        let version = digest(&serde_json::to_string(&normalized)?);
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM local_draft_recoveries WHERE id=?1 AND original_id=?2 AND base_revision IS ?3 AND version=?4)",params![id,draft.id,draft.revision,version],|row|row.get(0))?)
    }

    /// A retry with identical content is read-only. Changed content needs its predecessor.
    /// The original editable row and any outgoing owner are never changed.
    pub fn keep_local_draft_recovery(
        &mut self,
        id: &str,
        previous: Option<&str>,
        draft: &Draft,
        now: i64,
    ) -> Result<LocalDraftRecovery> {
        self.keep_local_recovery_with_budget(id, previous, draft, now, MAX_TOTAL_BYTES)
    }

    fn keep_local_recovery_with_budget(
        &mut self,
        id: &str,
        previous: Option<&str>,
        draft: &Draft,
        now: i64,
        total_limit: i64,
    ) -> Result<LocalDraftRecovery> {
        if id.is_empty()
            || id.len() > 128
            || id.chars().any(char::is_control)
            || draft.id.is_empty()
            || draft.id.len() > 4096
        {
            return Err(invalid("The local recovery identity is invalid."));
        }
        let identities: HashSet<_> = draft.attachments.iter().map(|file| &file.id).collect();
        if draft.attachments.len() > MAX_FILES || identities.len() != draft.attachments.len() {
            return Err(invalid(
                "The recovery attachment list is too large or contains duplicate identities.",
            ));
        }
        let wanted: Vec<String> = draft
            .attachments
            .iter()
            .filter(|file| file.forwarded.is_none())
            .filter(|file| {
                file.id
                    .parse::<i64>()
                    .is_ok_and(|id| id > 0 && id.to_string() == file.id)
            })
            .map(|file| file.id.clone())
            .collect();
        let mut normalized = draft.clone();
        normalized.updated_at = 0;
        let snapshot = serde_json::to_string(&normalized)?;
        if snapshot.len() > MAX_SNAPSHOT_BYTES {
            return Err(invalid(
                "This draft exceeds the local recovery text limit. Keep it open until it can be saved.",
            ));
        }
        let version = digest(&snapshot);
        self.delivery_transaction(|store| {
            let prior: Option<(String, Option<String>, String, String, i64)> = store.conn.query_row(
                "SELECT original_id,base_revision,version,state,snapshot_bytes FROM local_draft_recoveries WHERE id=?1",
                [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
            ).optional()?;
            if let Some((original,base,current,state,_)) = &prior {
                if original != &draft.id || base != &draft.revision { return Err(Error::RequestConflict); }
                if state != "open" {
                    let adopted_id=store.conn.query_row("SELECT adopted_id FROM local_draft_recoveries WHERE id=?1",[id],|row|row.get(0))?;
                    return Err(Error::LocalDraftRecoveryResolved { adopted_id });
                }
                let existing = store.local_draft_recovery(id)?.ok_or_else(|| invalid("The local recovery could not be read."))?;
                if current == &version { return Ok(existing); }
                if previous != Some(current.as_str()) { return Err(Error::RequestConflict); }
            } else if previous.is_some() { return Err(Error::RequestConflict); }
            // Free dropped copies in the same transaction before checking available capacity.
            let sql = format!("DELETE FROM local_draft_recovery_files WHERE recovery_id=? AND source_id NOT IN ({})", vec!["?";wanted.len()].join(","));
            store.conn.execute(&sql, params_from_iter(std::iter::once(id).chain(wanted.iter().map(String::as_str))))?;
            let existing = store.local_recovery_file_ids(id)?;
            if existing.iter().any(|file| !wanted.contains(file)) { return Err(invalid("Removed recovery files could not be released.")); }
            let (count,bytes): (i64,i64) = store.conn.query_row(
                "SELECT count(*),coalesce(sum(snapshot_bytes),0) + (SELECT coalesce(sum(length(data)),0) FROM local_draft_recovery_files) FROM local_draft_recoveries WHERE state='open'",
                [], |row| Ok((row.get(0)?,row.get(1)?)),
            )?;
            let old_bytes = prior.as_ref().map_or(0, |entry| entry.4);
            if (prior.is_none() && count >= MAX_OPEN_RECOVERIES) || bytes - old_bytes + snapshot.len() as i64 > total_limit {
                return Err(invalid("Recovery storage is full. Keep this draft open until space is available."));
            }
            let changed = store.conn.execute(
                "INSERT INTO local_draft_recoveries(id,original_id,base_revision,version,snapshot,snapshot_bytes,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(id) DO UPDATE SET version=excluded.version,snapshot=excluded.snapshot,snapshot_bytes=excluded.snapshot_bytes,updated_at=excluded.updated_at",
                params![id,draft.id,draft.revision,version,snapshot,snapshot.len() as i64,now],
            )?;
            if changed != 1 { return Err(invalid("The local recovery was not saved.")); }
            let mut total_bytes = bytes - old_bytes + snapshot.len() as i64;
            let mut file_bytes: i64 = store.conn.query_row("SELECT coalesce(sum(length(data)),0) FROM local_draft_recovery_files WHERE recovery_id=?1",[id],|row|row.get(0))?;
            for file in wanted.iter().filter(|file| !existing.contains(*file)) {
                let size: Option<i64> = store.conn.query_row("SELECT length(data) FROM draft_files WHERE draft_id=?1 AND id=?2",params![draft.id,file],|row|row.get(0)).optional()?;
                let Some(size) = size else { continue; };
                // A capacity failure must not acknowledge a copy whose available files were lost.
                if size > MAX_FILE_BYTES-file_bytes || size > total_limit-total_bytes {
                    return Err(invalid("Recovery attachment storage is full. Keep this draft open until space is available."));
                }
                let (name,content_type,hash):(String,String,String)=store.conn.query_row(
                    "SELECT name,content_type,data FROM draft_files WHERE draft_id=?1 AND id=?2 AND length(data)<=?3",
                    params![draft.id,file,MAX_FILE_BYTES],|row|Ok((row.get(0)?,row.get(1)?,format!("{:x}",Sha256::digest(row.get_ref(2)?.as_blob()?)))),
                )?;
                let metadata=draft.attachments.iter().find(|entry| &entry.id==file).expect("wanted IDs come from attachments");
                if metadata.name != name || metadata.content_type != content_type || metadata.bytes != size as u64 {
                    return Err(invalid("Recovery attachment metadata does not match its stored file."));
                }
                let copied = store.conn.execute("INSERT INTO local_draft_recovery_files(recovery_id,source_id,name,content_type,bytes,sha256,data) SELECT ?1,?2,name,content_type,length(data),?4,data FROM draft_files WHERE draft_id=?3 AND id=?2",params![id,file,draft.id,hash])?;
                if copied != 1 { return Err(invalid("A recovery attachment was not saved.")); }
                let verified: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM local_draft_recovery_files r JOIN draft_files f ON f.id=r.source_id AND f.draft_id=?3 WHERE r.recovery_id=?1 AND r.source_id=?2 AND r.data=f.data)",params![id,file,draft.id],|row|row.get(0))?;
                if !verified { return Err(invalid("A recovery attachment could not be verified.")); }
                file_bytes += size;
                total_bytes += size;
            }
            let saved = store.local_draft_recovery(id)?.ok_or_else(|| invalid("The local recovery was not saved."))?;
            if saved.version != version { return Err(invalid("The local recovery could not be verified.")); }
            Ok(saved)
        })
    }

    pub fn local_draft_recovery_index(&self) -> Result<Vec<LocalDraftRecoveryEntry>> {
        let mut stmt=self.conn.prepare("SELECT id,version,original_id,updated_at FROM local_draft_recoveries WHERE state='open' ORDER BY updated_at DESC,id LIMIT ?1")?;
        let rows = stmt
            .query_map([MAX_OPEN_RECOVERIES + 1], |row| {
                Ok(LocalDraftRecoveryEntry {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    original_id: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > MAX_OPEN_RECOVERIES as usize {
            return Err(invalid("The recovery index exceeds its limit."));
        }
        Ok(rows)
    }

    #[cfg(test)]
    fn local_draft_recovery_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .local_draft_recovery_index()?
            .into_iter()
            .map(|row| row.id)
            .collect())
    }

    /// Explicit review creates standalone mail. None on replay means the new draft
    /// was subsequently removed or handed off; it must never be recreated.
    pub fn adopt_local_draft_recovery(
        &mut self,
        id: &str,
        version: &str,
        new_id: &str,
        account: &str,
        missing: &[String],
        now: i64,
    ) -> Result<Option<Draft>> {
        if new_id.is_empty() || new_id.len() > 128 || new_id.chars().any(char::is_control) {
            return Err(invalid("The new draft identity is invalid."));
        }
        let mut acknowledged = missing.to_vec();
        acknowledged.sort();
        if acknowledged.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::RequestConflict);
        }
        let request = digest(&serde_json::to_string(&(new_id, account, &acknowledged))?);
        self.delivery_transaction(|store| {
            let row:Option<(String,String,Option<String>,Option<String>)>=store.conn.query_row(
                "SELECT version,state,adopted_id,adoption_request FROM local_draft_recoveries WHERE id=?1",[id],
                |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).optional()?;
            let Some((current,state,target,prior_request))=row else {return Err(Error::RequestConflict);};
            if current!=version {return Err(Error::RequestConflict);}
            if state=="adopted" {
                if target.as_deref()!=Some(new_id) || prior_request.as_deref()!=Some(&request) {return Err(Error::RequestConflict);}
                return store.editable_draft(new_id);
            }
            if state!="open" {return Err(Error::RequestConflict);}
            let recovery=store.local_draft_recovery(id)?.ok_or(Error::RequestConflict)?;
            let mut expected_missing=recovery.missing_files.clone();expected_missing.sort();
            if acknowledged!=expected_missing || new_id==recovery.draft.id {return Err(Error::RequestConflict);}
            let exists:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)",[account],|row|row.get(0))?;
            if !exists {return Err(Error::UnknownAccount(account.into()));}
            store.require_draft_revision(new_id,None)?;
            let staged:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM draft_files WHERE draft_id=?1)
                    OR EXISTS(SELECT 1 FROM local_draft_recoveries WHERE original_id=?1 OR adopted_id=?1)
                    OR EXISTS(SELECT 1 FROM deliveries WHERE draft_id=?1)
                    OR EXISTS(SELECT 1 FROM scheduled WHERE draft_id=?1)
                    OR EXISTS(SELECT 1 FROM schedule_outcomes WHERE draft_id=?1)",[new_id],|row|row.get(0))?;
            if staged {return Err(Error::DraftConflict);}
            let mut adopted=recovery.draft;
            adopted.id=new_id.into();adopted.revision=None;adopted.from_account_id=account.into();
            adopted.mode=moss_core::ComposeMode::New;adopted.thread_id=None;
            adopted.from_address=None;adopted.signature_id=None;adopted.signature_chosen=false;adopted.updated_at=now;
            adopted.attachments.retain(|file|!acknowledged.contains(&file.id));
            for file in &mut adopted.attachments {
                let copied=store.conn.execute("INSERT INTO draft_files(draft_id,name,content_type,data,added_at) SELECT ?1,name,content_type,data,?2 FROM local_draft_recovery_files WHERE recovery_id=?3 AND source_id=?4",params![new_id,now,id,file.id])?;
                if copied!=1 {return Err(invalid("A recovered attachment could not be adopted."));}
                let new_file=store.conn.last_insert_rowid().to_string();
                let verified:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM draft_files d JOIN local_draft_recovery_files r ON r.recovery_id=?1 AND r.source_id=?2 WHERE d.id=?3 AND d.draft_id=?4 AND d.name=r.name AND d.content_type=r.content_type AND d.data=r.data)",params![id,file.id,new_file,new_id],|row|row.get(0))?;
                if !verified {return Err(invalid("A recovered attachment could not be verified."));}
                file.id=new_file;
            }
            store.save_draft_in_transaction(&adopted)?;
            let saved=store.editable_draft(new_id)?.ok_or(Error::DraftConflict)?;
            adopted.revision=saved.revision.clone();
            if adopted!=saved {return Err(invalid("The adopted draft could not be verified."));}
            store.conn.execute("DELETE FROM local_draft_recovery_files WHERE recovery_id=?1",[id])?;
            if !store.local_recovery_file_ids(id)?.is_empty() {return Err(invalid("Recovery attachments could not be released."));}
            let changed=store.conn.execute("UPDATE local_draft_recoveries SET state='adopted',adopted_id=?1,adoption_request=?2,snapshot=NULL,snapshot_bytes=0 WHERE id=?3 AND version=?4 AND state='open'",params![new_id,request,id,version])?;
            let verified:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM local_draft_recoveries WHERE id=?1 AND version=?2 AND state='adopted' AND adopted_id=?3 AND adoption_request=?4 AND snapshot IS NULL AND snapshot_bytes=0)",params![id,version,new_id,request],|row|row.get(0))?;
            if changed!=1 || !verified {return Err(invalid("Recovery adoption could not be retained."));}
            Ok(Some(saved))
        })
    }

    /// Remove live content references but retain identity against delayed saves.
    /// This is logical deletion; existing backups and SQLite pages can retain bytes.
    pub fn discard_local_draft_recovery(&mut self, id: &str, version: &str) -> Result<()> {
        self.delivery_transaction(|store| {
            let row: Option<(String,String)> = store.conn.query_row(
                "SELECT version,state FROM local_draft_recoveries WHERE id=?1",[id],
                |row|Ok((row.get(0)?,row.get(1)?)),
            ).optional()?;
            let Some((current,state))=row else { return Err(Error::RequestConflict); };
            if current != version { return Err(Error::RequestConflict); }
            if state == "discarded" { return Ok(()); }
            if state != "open" { return Err(Error::RequestConflict); }
            store.conn.execute("DELETE FROM local_draft_recovery_files WHERE recovery_id=?1",[id])?;
            if !store.local_recovery_file_ids(id)?.is_empty() {
                return Err(invalid("Recovery attachments could not be removed."));
            }
            let changed=store.conn.execute("UPDATE local_draft_recoveries SET state='discarded',snapshot=NULL,snapshot_bytes=0 WHERE id=?1 AND version=?2 AND state='open'",params![id,version])?;
            let verified: bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM local_draft_recoveries WHERE id=?1 AND version=?2 AND state='discarded' AND snapshot IS NULL AND snapshot_bytes=0)",params![id,version],|row|row.get(0))?;
            if changed != 1 || !verified { return Err(invalid("The local recovery could not be discarded.")); }
            Ok(())
        })
    }

    /// Reads one bounded, verified snapshot. Terminal identities return no editable text.
    pub fn local_draft_recovery(&self, id: &str) -> Result<Option<LocalDraftRecovery>> {
        let row: Option<(String,Option<String>,i64,String,Option<String>)> = self.conn.query_row(
            "SELECT version,CASE WHEN length(CAST(snapshot AS BLOB)) <= ?2 AND snapshot_bytes=length(CAST(snapshot AS BLOB)) THEN snapshot END,updated_at,original_id,base_revision FROM local_draft_recoveries WHERE id=?1 AND state='open'",
            params![id,MAX_SNAPSHOT_BYTES as i64], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).optional()?;
        let Some((version, snapshot, updated_at, original, base)) = row else {
            return Ok(None);
        };
        let snapshot =
            snapshot.ok_or_else(|| invalid("The stored local recovery exceeds its text limit."))?;
        if digest(&snapshot) != version {
            return Err(invalid("The stored local recovery could not be verified."));
        }
        let mut draft: Draft = serde_json::from_str(&snapshot)?;
        if draft.id != original || draft.revision != base {
            return Err(invalid(
                "The stored local recovery identity could not be verified.",
            ));
        }
        draft.updated_at = updated_at;
        let files = self.verified_local_recovery_files(id, &draft)?;
        let missing_files = draft
            .attachments
            .iter()
            .filter(|file| file.forwarded.is_some() || !files.contains(&file.id))
            .map(|file| file.id.clone())
            .collect();
        Ok(Some(LocalDraftRecovery {
            id: id.into(),
            version,
            draft,
            missing_files,
            updated_at,
        }))
    }

    fn verified_local_recovery_files(&self, id: &str, draft: &Draft) -> Result<HashSet<String>> {
        let ids = self.local_recovery_file_ids(id)?;
        let mut total = 0i64;
        for id_file in &ids {
            let metadata = draft
                .attachments
                .iter()
                .find(|file| &file.id == id_file && file.forwarded.is_none())
                .ok_or_else(|| invalid("The recovery contains an unexpected attachment."))?;
            let (name,content_type,size,hash):(String,String,i64,String)=self.conn.query_row(
                "SELECT name,content_type,bytes,sha256 FROM local_draft_recovery_files WHERE recovery_id=?1 AND source_id=?2",
                params![id,id_file],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            )?;
            if size < 0
                || size > MAX_FILE_BYTES - total
                || metadata.name != name
                || metadata.content_type != content_type
                || metadata.bytes != size as u64
            {
                return Err(invalid(
                    "The stored recovery attachment metadata could not be verified.",
                ));
            }
            let actual:Option<String>=self.conn.query_row(
                "SELECT CASE WHEN length(data)=?3 THEN data END FROM local_draft_recovery_files WHERE recovery_id=?1 AND source_id=?2",
                params![id,id_file,size],|row|match row.get_ref(0)? {
                    rusqlite::types::ValueRef::Blob(bytes)=>Ok(Some(format!("{:x}",Sha256::digest(bytes)))),
                    _=>Ok(None),
                },
            )?;
            if actual.as_deref() != Some(hash.as_str()) {
                return Err(invalid(
                    "The stored recovery attachment bytes could not be verified.",
                ));
            }
            total += size;
        }
        Ok(ids)
    }

    fn local_recovery_file_ids(&self, id: &str) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id FROM local_draft_recovery_files WHERE recovery_id=?1 LIMIT ?2",
        )?;
        let files: HashSet<String> = stmt
            .query_map(params![id, MAX_FILES as i64 + 1], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if files.len() > MAX_FILES {
            return Err(invalid(
                "The stored recovery attachment list exceeds its limit.",
            ));
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn local_conflicting_text_survives_restart_and_account_removal_without_changing_the_winner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let mut store = Store::open(&path).unwrap();
        store
            .upsert_account(&account("work", "work@example.test"))
            .unwrap();
        let original = draft("original-local-draft");
        store.save_draft(&original).unwrap();
        let mut local = store.editable_draft(&original.id).unwrap().unwrap();
        let mut winner = local.clone();
        winner.body = "Winning remote text".into();
        let (winner, _) = store
            .edit_draft(&winner, &[], NOV_1, |_| Ok(vec![]))
            .unwrap();
        local.body = "My unsaved local work".into();
        let saved = store
            .keep_local_draft_recovery("recovery-one", None, &local, NOV_1)
            .unwrap();
        assert_eq!(saved.draft.body, "My unsaved local work");
        assert_eq!(saved.draft.id, original.id);
        assert_eq!(store.editable_drafts().unwrap(), vec![winner.clone()]);
        assert!(saved.missing_files.is_empty());
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            store.local_draft_recovery("recovery-one").unwrap(),
            Some(saved.clone())
        );
        assert_eq!(store.editable_drafts().unwrap(), vec![winner]);
        store.remove_account("work").unwrap();
        assert!(store.editable_drafts().unwrap().is_empty());
        assert_eq!(
            store.local_draft_recovery("recovery-one").unwrap(),
            Some(saved)
        );
        assert_eq!(store.local_draft_recovery("unknown").unwrap(), None);
    }

    #[test]
    fn recovery_copies_owned_saved_and_staged_files_and_records_unavailable_metadata() {
        let mut store = store_with_work();
        let mut local = draft("owned-recovery-files");
        let saved_file = store
            .add_draft_file(&local.id, "saved.txt", "text/plain", b"saved bytes", NOV_1)
            .unwrap();
        local.attachments.push(saved_file.clone());
        store.save_draft(&local).unwrap();
        let staged_file = store
            .add_draft_file(
                &local.id,
                "staged.txt",
                "text/plain",
                b"staged bytes",
                NOV_1,
            )
            .unwrap();
        let foreign = store
            .add_draft_file(
                "another-draft",
                "foreign.txt",
                "text/plain",
                b"private other bytes",
                NOV_1,
            )
            .unwrap();
        let forwarded = moss_core::DraftFile {
            id: "forwarded:1".into(),
            name: "original.pdf".into(),
            bytes: 8,
            content_type: "application/pdf".into(),
            forwarded: Some(moss_core::FileRef {
                message_id: "999".into(),
                index: 0,
            }),
        };
        local
            .attachments
            .extend([staged_file.clone(), foreign.clone(), forwarded.clone()]);
        let recovery = store
            .keep_local_draft_recovery("file-copy", None, &local, NOV_1)
            .unwrap();
        assert_eq!(
            recovery.missing_files,
            vec![foreign.id.clone(), forwarded.id.clone()]
        );
        assert_eq!(recovery.draft.attachments, local.attachments);
        for (file, bytes) in [
            (&saved_file, b"saved bytes".as_slice()),
            (&staged_file, b"staged bytes".as_slice()),
        ] {
            let copied:Vec<u8> = store.conn.query_row("SELECT data FROM local_draft_recovery_files WHERE recovery_id='file-copy' AND source_id=?1",[&file.id],|row|row.get(0)).unwrap();
            assert_eq!(copied, bytes);
        }
        let count: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM local_draft_recovery_files",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        // A later writer removes the original bytes. The recovery still owns its copies.
        store
            .conn
            .execute("DELETE FROM draft_files WHERE draft_id=?1", [&local.id])
            .unwrap();
        local.body = "Keep typing after the conflict".into();
        let updated = store
            .keep_local_draft_recovery("file-copy", Some(&recovery.version), &local, NOV_1 + 1)
            .unwrap();
        assert_eq!(updated.missing_files, recovery.missing_files);
        local.attachments.retain(|file| file.id == staged_file.id);
        let trimmed = store
            .keep_local_draft_recovery("file-copy", Some(&updated.version), &local, NOV_1 + 2)
            .unwrap();
        assert!(trimmed.missing_files.is_empty());
        let remaining: Vec<String> = store
            .conn
            .prepare("SELECT source_id FROM local_draft_recovery_files")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(remaining, vec![staged_file.id]);
        assert_eq!(store.draft_file_usage("another-draft").unwrap(), (1, 19));
    }

    #[test]
    fn identical_recovery_retries_are_read_only_and_changed_content_requires_its_predecessor() {
        let mut store = store_with_work();
        let mut local = draft("recovery-replay");
        local.revision = Some("base".into());
        let first = store
            .keep_local_draft_recovery("replay", None, &local, NOV_1)
            .unwrap();
        store.conn.execute_batch("CREATE TRIGGER no_replay_write BEFORE UPDATE ON local_draft_recoveries BEGIN SELECT RAISE(ABORT,'unexpected write'); END;").unwrap();
        local.updated_at += 1000;
        assert_eq!(
            store
                .keep_local_draft_recovery("replay", None, &local, NOV_1 + 1000)
                .unwrap(),
            first
        );
        store
            .conn
            .execute_batch("DROP TRIGGER no_replay_write")
            .unwrap();
        local.body = "Newer local text".into();
        assert!(matches!(
            store.keep_local_draft_recovery("replay", None, &local, NOV_1),
            Err(Error::RequestConflict)
        ));
        let second = store
            .keep_local_draft_recovery("replay", Some(&first.version), &local, NOV_1 + 1)
            .unwrap();
        assert_ne!(second.version, first.version);
        local.body = "Stale update".into();
        assert!(matches!(
            store.keep_local_draft_recovery("replay", Some(&first.version), &local, NOV_1),
            Err(Error::RequestConflict)
        ));
        assert_eq!(
            store.local_draft_recovery("replay").unwrap(),
            Some(second.clone())
        );
        local.id = "another-original".into();
        assert!(matches!(
            store.keep_local_draft_recovery("replay", Some(&second.version), &local, NOV_1),
            Err(Error::RequestConflict)
        ));
        local.id = "recovery-replay".into();
        local.revision = Some("other-base".into());
        assert!(matches!(
            store.keep_local_draft_recovery("replay", Some(&second.version), &local, NOV_1),
            Err(Error::RequestConflict)
        ));
        assert!(matches!(
            store.keep_local_draft_recovery("unknown", Some(&first.version), &local, NOV_1),
            Err(Error::RequestConflict)
        ));
    }

    #[test]
    fn discard_requires_current_version_releases_content_and_keeps_a_replay_tombstone() {
        let mut store = store_with_work();
        let mut local = draft("discard-recovery");
        local.attachments.push(
            store
                .add_draft_file(&local.id, "a.txt", "text/plain", b"private", NOV_1)
                .unwrap(),
        );
        let saved = store
            .keep_local_draft_recovery("discard", None, &local, NOV_1)
            .unwrap();
        assert!(matches!(
            store.discard_local_draft_recovery("discard", "stale"),
            Err(Error::RequestConflict)
        ));
        assert_eq!(
            store.local_draft_recovery("discard").unwrap(),
            Some(saved.clone())
        );
        store
            .discard_local_draft_recovery("discard", &saved.version)
            .unwrap();
        assert_eq!(store.local_draft_recovery("discard").unwrap(), None);
        let row:(String,Option<String>,i64,i64)=store.conn.query_row("SELECT state,snapshot,snapshot_bytes,(SELECT count(*) FROM local_draft_recovery_files) FROM local_draft_recoveries WHERE id='discard'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(row, ("discarded".into(), None, 0, 0));
        store.conn.execute_batch("CREATE TRIGGER no_discard_replay BEFORE UPDATE ON local_draft_recoveries BEGIN SELECT RAISE(ABORT,'unexpected write'); END").unwrap();
        store
            .discard_local_draft_recovery("discard", &saved.version)
            .unwrap();
        assert!(matches!(
            store.discard_local_draft_recovery("discard", "stale"),
            Err(Error::RequestConflict)
        ));
        assert!(
            store
                .discard_local_draft_recovery("unknown", &saved.version)
                .is_err()
        );
        assert!(matches!(
            store.keep_local_draft_recovery("discard", None, &local, NOV_1),
            Err(Error::LocalDraftRecoveryResolved { adopted_id: None })
        ));
        assert_eq!(store.draft_file_usage(&local.id).unwrap(), (1, 7));
    }

    #[test]
    fn recovery_index_lists_only_open_entries_in_stable_newest_order() {
        let mut store = store_with_work();
        let local = draft("index-source");
        assert!(store.local_draft_recovery_ids().unwrap().is_empty());
        for (id, time) in [("old", 1), ("z-new", 3), ("a-new", 3), ("removed", 4)] {
            store
                .keep_local_draft_recovery(id, None, &local, time)
                .unwrap();
        }
        let removed = store.local_draft_recovery("removed").unwrap().unwrap();
        store
            .discard_local_draft_recovery("removed", &removed.version)
            .unwrap();
        assert_eq!(
            store.local_draft_recovery_ids().unwrap(),
            vec!["a-new", "z-new", "old"]
        );
    }

    #[test]
    fn failed_discard_rolls_back_text_and_file_removal() {
        for trigger in [
            "CREATE TRIGGER fail BEFORE DELETE ON local_draft_recovery_files BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail BEFORE UPDATE ON local_draft_recoveries BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail BEFORE UPDATE ON local_draft_recoveries BEGIN SELECT RAISE(ABORT,'disk fault'); END",
            "CREATE TRIGGER fail AFTER UPDATE ON local_draft_recoveries BEGIN UPDATE local_draft_recoveries SET version='corrupt' WHERE id=NEW.id; END",
        ] {
            let mut store = store_with_work();
            let mut local = draft("discard-fault");
            local.attachments.push(
                store
                    .add_draft_file(&local.id, "a.txt", "text/plain", b"keep", NOV_1)
                    .unwrap(),
            );
            let saved = store
                .keep_local_draft_recovery("fault", None, &local, NOV_1)
                .unwrap();
            store.conn.execute_batch(trigger).unwrap();
            assert!(
                store
                    .discard_local_draft_recovery("fault", &saved.version)
                    .is_err(),
                "{trigger}"
            );
            assert_eq!(store.local_draft_recovery("fault").unwrap(), Some(saved));
            let bytes: Vec<u8> = store
                .conn
                .query_row("SELECT data FROM local_draft_recovery_files", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(bytes, b"keep");
        }
    }

    #[test]
    fn invalid_identity_or_duplicate_files_never_creates_a_recovery() {
        let mut store = store_with_work();
        let mut local = draft("valid-source");
        for id in ["".to_string(), "x".repeat(129), "bad\nidentity".into()] {
            assert!(
                store
                    .keep_local_draft_recovery(&id, None, &local, 0)
                    .is_err()
            );
        }
        for id in ["".to_string(), "x".repeat(4097)] {
            local.id = id;
            assert!(
                store
                    .keep_local_draft_recovery("valid", None, &local, 0)
                    .is_err()
            );
        }
        local.id = "valid-source".into();
        let file = store
            .add_draft_file(&local.id, "a.txt", "text/plain", b"a", 0)
            .unwrap();
        local.attachments = vec![file.clone(), file.clone()];
        assert!(
            store
                .keep_local_draft_recovery("valid", None, &local, 0)
                .is_err()
        );
        local.attachments = (0..=MAX_FILES)
            .map(|id| {
                let mut f = file.clone();
                f.id = id.to_string();
                f
            })
            .collect();
        assert!(
            store
                .keep_local_draft_recovery("valid", None, &local, 0)
                .is_err()
        );
        assert!(store.local_draft_recovery_ids().unwrap().is_empty());
    }

    #[test]
    fn attachment_metadata_must_match_owned_bytes_and_corruption_is_rejected() {
        for field in ["name", "type", "bytes"] {
            let mut store = store_with_work();
            let mut local = draft("metadata-source");
            let mut file = store
                .add_draft_file(&local.id, "true.txt", "text/plain", b"true", 0)
                .unwrap();
            match field {
                "name" => file.name = "forged.txt".into(),
                "type" => file.content_type = "image/png".into(),
                _ => file.bytes = 999,
            }
            local.attachments.push(file);
            assert!(
                store
                    .keep_local_draft_recovery("metadata", None, &local, 0)
                    .is_err(),
                "{field}"
            );
            assert!(store.local_draft_recovery_ids().unwrap().is_empty());
        }
        let mut store = store_with_work();
        let mut local = draft("corrupt-source");
        local.attachments.push(
            store
                .add_draft_file(&local.id, "true.txt", "text/plain", b"true", 0)
                .unwrap(),
        );
        store
            .keep_local_draft_recovery("corrupt", None, &local, 0)
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE local_draft_recovery_files SET data=?1",
                [b"fake".as_slice()],
            )
            .unwrap();
        assert!(store.local_draft_recovery("corrupt").is_err());
    }

    #[test]
    fn attachment_capacity_failure_rolls_back_instead_of_claiming_a_saved_copy() {
        let mut store = store_with_work();
        let mut local = draft("capacity-source");
        let file = store
            .add_draft_file(&local.id, "large.txt", "text/plain", b"", 0)
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE draft_files SET data=zeroblob(?1) WHERE id=?2",
                params![MAX_FILE_BYTES + 1, file.id],
            )
            .unwrap();
        let mut file = file;
        file.bytes = (MAX_FILE_BYTES + 1) as u64;
        local.attachments.push(file);
        assert!(
            store
                .keep_local_draft_recovery("capacity", None, &local, 0)
                .is_err()
        );
        assert!(store.local_draft_recovery_ids().unwrap().is_empty());
        assert_eq!(store.local_recovery_file_ids("capacity").unwrap().len(), 0);
    }

    #[test]
    fn adopted_tombstone_reports_target_without_resurrecting_text() {
        let mut store = store_with_work();
        let local = draft("terminal-source");
        let saved = store
            .keep_local_draft_recovery("terminal", None, &local, 0)
            .unwrap();
        store.conn.execute("UPDATE local_draft_recoveries SET state='adopted',adopted_id='new-target',adoption_request='fixture',snapshot=NULL,snapshot_bytes=0",[]).unwrap();
        assert!(
            matches!(store.keep_local_draft_recovery("terminal",None,&local,0),Err(Error::LocalDraftRecoveryResolved{adopted_id:Some(id)}) if id=="new-target")
        );
        assert!(matches!(
            store.discard_local_draft_recovery("terminal", &saved.version),
            Err(Error::RequestConflict)
        ));
        assert!(store.local_draft_recovery_ids().unwrap().is_empty());
    }

    #[test]
    fn record_and_text_limits_preserve_existing_copies() {
        let mut store = store_with_work();
        let mut local = draft("limit-source");
        for i in 0..MAX_OPEN_RECOVERIES {
            store
                .keep_local_draft_recovery(&format!("r-{i}"), None, &local, i)
                .unwrap();
        }
        assert!(
            store
                .keep_local_draft_recovery("overflow", None, &local, 0)
                .is_err()
        );
        assert_eq!(
            store.local_draft_recovery_ids().unwrap().len(),
            MAX_OPEN_RECOVERIES as usize
        );
        let first = store.local_draft_recovery("r-0").unwrap().unwrap();
        local.body = "Still editable at record capacity".into();
        let changed = store
            .keep_local_draft_recovery("r-0", Some(&first.version), &local, 1)
            .unwrap();
        local.body = "x".repeat(MAX_SNAPSHOT_BYTES);
        assert!(
            store
                .keep_local_draft_recovery("r-0", Some(&changed.version), &local, 2)
                .is_err()
        );
        assert_eq!(store.local_draft_recovery("r-0").unwrap(), Some(changed));
        // A damaged or externally modified database cannot make the list unbounded.
        store.conn.execute("INSERT INTO local_draft_recoveries SELECT 'excess',original_id,base_revision,version,snapshot,snapshot_bytes,state,adopted_id,adoption_request,updated_at FROM local_draft_recoveries WHERE id='r-1'",[]).unwrap();
        assert!(store.local_draft_recovery_ids().is_err());
    }

    #[test]
    fn schema_rejects_terminal_file_ownership_and_duplicate_adoptions() {
        let mut store = store_with_work();
        let mut local = draft("schema-source");
        local.attachments.push(
            store
                .add_draft_file(&local.id, "a.txt", "text/plain", b"a", 0)
                .unwrap(),
        );
        store
            .keep_local_draft_recovery("open", None, &local, 0)
            .unwrap();
        assert!(store.conn.execute("UPDATE local_draft_recoveries SET state='discarded',snapshot=NULL,snapshot_bytes=0 WHERE id='open'",[]).is_err());
        let empty = draft("empty-source");
        for id in ["terminal-a", "terminal-b"] {
            store
                .keep_local_draft_recovery(id, None, &empty, 0)
                .unwrap();
        }
        store.conn.execute("UPDATE local_draft_recoveries SET state='adopted',adopted_id='one-target',adoption_request='fixture',snapshot=NULL,snapshot_bytes=0 WHERE id='terminal-a'",[]).unwrap();
        assert!(store.conn.execute("UPDATE local_draft_recoveries SET state='adopted',adopted_id='one-target',adoption_request='fixture',snapshot=NULL,snapshot_bytes=0 WHERE id='terminal-b'",[]).is_err());
        assert!(store.conn.execute("UPDATE local_draft_recovery_files SET recovery_id='terminal-a' WHERE recovery_id='open'",[]).is_err());
        assert!(store.conn.execute("INSERT INTO local_draft_recovery_files SELECT 'terminal-a',source_id,name,content_type,bytes,sha256,data FROM local_draft_recovery_files WHERE recovery_id='open'",[]).is_err());
        assert_eq!(
            store
                .local_draft_recovery("open")
                .unwrap()
                .unwrap()
                .draft
                .attachments,
            local.attachments
        );
    }

    #[test]
    fn corrupt_recovery_can_be_discarded_using_bounded_index_metadata_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut store = Store::open(&path).unwrap();
        let local = draft("original-index");
        let saved = store
            .keep_local_draft_recovery("damaged", None, &local, 42)
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE local_draft_recoveries SET snapshot='invalid',snapshot_bytes=7",
                [],
            )
            .unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(store.local_draft_recovery("damaged").is_err());
        let index = store.local_draft_recovery_index().unwrap();
        assert_eq!(
            index,
            vec![LocalDraftRecoveryEntry {
                id: "damaged".into(),
                version: saved.version.clone(),
                original_id: local.id,
                updated_at: 42
            }]
        );
        store
            .discard_local_draft_recovery(&index[0].id, &index[0].version)
            .unwrap();
        assert!(store.local_draft_recovery_index().unwrap().is_empty());
    }

    #[test]
    fn adoption_creates_a_fresh_detached_draft_and_replay_never_recreates_it() {
        let mut store = store_with_work();
        let mut local = draft("adopt-source");
        local.mode = moss_core::ComposeMode::Reply;
        local.thread_id = Some("999".into());
        local.from_address = Some("old@example.test".into());
        local.signature_id = Some("old-signature".into());
        local.signature_chosen = true;
        local.attachments.push(
            store
                .add_draft_file(&local.id, "owned.txt", "text/plain", b"owned", 0)
                .unwrap(),
        );
        let saved = store
            .keep_local_draft_recovery("adopt", None, &local, 0)
            .unwrap();
        let adopted = store
            .adopt_local_draft_recovery("adopt", &saved.version, "fresh-draft", "work", &[], 10)
            .unwrap()
            .unwrap();
        assert_eq!(adopted.id, "fresh-draft");
        assert_eq!(adopted.from_account_id, "work");
        assert_eq!(adopted.mode, moss_core::ComposeMode::New);
        assert_eq!(adopted.thread_id, None);
        assert_eq!(adopted.from_address, None);
        assert_eq!(adopted.signature_id, None);
        assert!(!adopted.signature_chosen);
        assert_eq!(adopted.body, local.body);
        assert_eq!(adopted.to, local.to);
        assert!(adopted.revision.is_some());
        assert_ne!(adopted.attachments[0].id, local.attachments[0].id);
        assert_eq!(
            store
                .draft_file_for(&adopted.id, &adopted.attachments[0].id)
                .unwrap()
                .unwrap()
                .2,
            b"owned"
        );
        assert!(store.local_draft_recovery("adopt").unwrap().is_none());
        assert!(store.local_recovery_file_ids("adopt").unwrap().is_empty());
        assert_eq!(
            store
                .adopt_local_draft_recovery("adopt", &saved.version, "fresh-draft", "work", &[], 11)
                .unwrap(),
            Some(adopted.clone())
        );
        let mut newer = adopted.clone();
        newer.body = "Edited after adoption".into();
        let (newer, _) = store.edit_draft(&newer, &[], 12, |_| Ok(vec![])).unwrap();
        assert_eq!(
            store
                .adopt_local_draft_recovery("adopt", &saved.version, "fresh-draft", "work", &[], 13)
                .unwrap(),
            Some(newer.clone())
        );
        let copy = crate::ServerCopy { message_id: "adopted-copy@example.test".into(), provider_id: Some("server-copy".into()) };
        assert!(store.set_server_copy(&newer.id, Some(&copy), newer.revision.as_deref()).unwrap());
        store
            .discard_editable_draft(&newer.id, newer.revision.as_deref(), false, 14, |observed| {
                assert_eq!(observed, &copy);
                Ok(vec!["remove-adopted-copy".into()])
            })
            .unwrap();
        assert!(store.pending_changes("work").unwrap().iter().any(|(_, change)| change == "remove-adopted-copy"));
        assert_eq!(
            store
                .adopt_local_draft_recovery("adopt", &saved.version, "fresh-draft", "work", &[], 15)
                .unwrap(),
            None
        );
        assert!(
            store
                .adopt_local_draft_recovery(
                    "adopt",
                    &saved.version,
                    "different-id",
                    "work",
                    &[],
                    15
                )
                .is_err()
        );
        assert!(
            store
                .adopt_local_draft_recovery(
                    "adopt",
                    &saved.version,
                    "fresh-draft",
                    "other",
                    &[],
                    15
                )
                .is_err()
        );
    }

    #[test]
    fn adoption_requires_exact_missing_review_current_version_account_and_unused_identity() {
        let mut store = store_with_work();
        let mut local = draft("review-source");
        let foreign = store
            .add_draft_file("someone-else", "missing.txt", "text/plain", b"other", 0)
            .unwrap();
        local.attachments.push(foreign.clone());
        let saved = store
            .keep_local_draft_recovery("review", None, &local, 0)
            .unwrap();
        let missing = vec![foreign.id.clone()];
        for list in [
            vec![],
            vec!["wrong".into()],
            vec![foreign.id.clone(), foreign.id.clone()],
            vec![foreign.id.clone(), "extra".into()],
        ] {
            assert!(
                store
                    .adopt_local_draft_recovery(
                        "review",
                        &saved.version,
                        "new-review",
                        "work",
                        &list,
                        1
                    )
                    .is_err()
            );
        }
        assert!(
            store
                .adopt_local_draft_recovery("review", "stale", "new-review", "work", &missing, 1)
                .is_err()
        );
        assert!(
            store
                .adopt_local_draft_recovery(
                    "unknown",
                    &saved.version,
                    "new-review",
                    "work",
                    &missing,
                    1
                )
                .is_err()
        );
        assert!(matches!(
            store.adopt_local_draft_recovery(
                "review",
                &saved.version,
                "new-review",
                "absent",
                &missing,
                1
            ),
            Err(Error::UnknownAccount(_))
        ));
        for id in [
            "".into(),
            "bad\nidentity".into(),
            "x".repeat(129),
            local.id.clone(),
        ] {
            assert!(
                store
                    .adopt_local_draft_recovery("review", &saved.version, &id, "work", &missing, 1)
                    .is_err()
            );
        }
        store.save_draft(&draft("existing")).unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "review",
                    &saved.version,
                    "existing",
                    "work",
                    &missing,
                    1
                )
                .is_err()
        );
        store.delete_draft("existing").unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "review",
                    &saved.version,
                    "existing",
                    "work",
                    &missing,
                    1
                )
                .is_err()
        );
        store
            .add_draft_file("staging", "a.txt", "text/plain", b"a", 0)
            .unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "review",
                    &saved.version,
                    "staging",
                    "work",
                    &missing,
                    1
                )
                .is_err()
        );
        assert_eq!(
            store.local_draft_recovery("review").unwrap(),
            Some(saved.clone())
        );
        store
            .upsert_account(&account("new-account", "new@example.test"))
            .unwrap();
        let adopted = store
            .adopt_local_draft_recovery(
                "review",
                &saved.version,
                "new-review",
                "new-account",
                &missing,
                1,
            )
            .unwrap()
            .unwrap();
        assert!(adopted.attachments.is_empty());
        assert_eq!(adopted.from_account_id, "new-account");
        assert_eq!(store.draft_file_usage("someone-else").unwrap(), (1, 5));
        assert!(
            store
                .adopt_local_draft_recovery(
                    "review",
                    &saved.version,
                    "new-review",
                    "new-account",
                    &[],
                    1
                )
                .is_err()
        );
    }

    #[test]
    fn adoption_faults_roll_back_new_identity_files_and_tombstone() {
        for trigger in [
            "CREATE TRIGGER fail BEFORE INSERT ON draft_files BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail AFTER INSERT ON draft_files BEGIN UPDATE draft_files SET data=X'66616B65' WHERE id=NEW.id; END",
            "CREATE TRIGGER fail BEFORE INSERT ON drafts BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail AFTER INSERT ON drafts BEGIN UPDATE drafts SET body='corrupted' WHERE id=NEW.id; END",
            "CREATE TRIGGER fail BEFORE DELETE ON local_draft_recovery_files BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail BEFORE UPDATE ON local_draft_recoveries BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail AFTER UPDATE ON local_draft_recoveries BEGIN UPDATE local_draft_recoveries SET adoption_request='corrupted' WHERE id=NEW.id; END",
        ] {
            let mut store = store_with_work();
            let mut local = draft("fault-source");
            local.attachments.push(
                store
                    .add_draft_file(&local.id, "a.txt", "text/plain", b"safe", 0)
                    .unwrap(),
            );
            let saved = store
                .keep_local_draft_recovery("fault-adopt", None, &local, 0)
                .unwrap();
            store.conn.execute_batch(trigger).unwrap();
            assert!(
                store
                    .adopt_local_draft_recovery(
                        "fault-adopt",
                        &saved.version,
                        "fault-target",
                        "work",
                        &[],
                        1
                    )
                    .is_err(),
                "{trigger}"
            );
            assert_eq!(
                store.local_draft_recovery("fault-adopt").unwrap(),
                Some(saved)
            );
            assert!(store.editable_draft("fault-target").unwrap().is_none());
            assert_eq!(store.draft_file_usage("fault-target").unwrap(), (0, 0));
            let remembered: bool = store
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM draft_identities WHERE id='fault-target')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(!remembered, "{trigger}");
        }
    }

    #[test]
    fn adoption_refuses_unsaved_recovery_and_reserved_agent_identities() {
        let mut store = store_with_work();
        let local = draft("adopt-collision");
        let saved = store
            .keep_local_draft_recovery("collision", None, &local, 0)
            .unwrap();
        store
            .keep_local_draft_recovery("other-recovery", None, &draft("other-unsaved"), 0)
            .unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "collision",
                    &saved.version,
                    "other-unsaved",
                    "work",
                    &[],
                    1
                )
                .is_err()
        );
        assert_eq!(
            store.local_draft_recovery("collision").unwrap(),
            Some(saved)
        );
    }

    #[test]
    fn adoption_receipt_is_required_only_for_adopted_rows() {
        let mut store = store_with_work();
        let local = draft("receipt-source");
        store
            .keep_local_draft_recovery("receipt", None, &local, 0)
            .unwrap();
        assert!(store.conn.execute("UPDATE local_draft_recoveries SET state='adopted',adopted_id='target',snapshot=NULL,snapshot_bytes=0",[]).is_err());
        assert!(
            store
                .conn
                .execute(
                    "UPDATE local_draft_recoveries SET adoption_request='unexpected'",
                    []
                )
                .is_err()
        );
        assert!(store.conn.execute("UPDATE local_draft_recoveries SET state='discarded',adoption_request='unexpected',snapshot=NULL,snapshot_bytes=0",[]).is_err());
    }

    #[test]
    fn failed_keep_never_leaves_partial_text_or_files() {
        for trigger in [
            "CREATE TRIGGER fail BEFORE INSERT ON local_draft_recoveries BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail BEFORE INSERT ON local_draft_recoveries BEGIN SELECT RAISE(ABORT,'disk fault'); END",
            "CREATE TRIGGER fail AFTER INSERT ON local_draft_recoveries BEGIN UPDATE local_draft_recoveries SET version='corrupt' WHERE id=NEW.id; END",
            "CREATE TRIGGER fail BEFORE INSERT ON local_draft_recovery_files BEGIN SELECT RAISE(IGNORE); END",
            "CREATE TRIGGER fail AFTER INSERT ON local_draft_recovery_files BEGIN UPDATE local_draft_recovery_files SET data=X'66616B65' WHERE recovery_id=NEW.recovery_id; END",
        ] {
            let mut store = store_with_work();
            let mut local = draft("keep-fault-source");
            local.attachments.push(
                store
                    .add_draft_file(&local.id, "a.txt", "text/plain", b"safe", 0)
                    .unwrap(),
            );
            store.conn.execute_batch(trigger).unwrap();
            assert!(
                store
                    .keep_local_draft_recovery("keep-fault", None, &local, 0)
                    .is_err(),
                "{trigger}"
            );
            assert!(store.local_draft_recovery_index().unwrap().is_empty());
            assert!(
                store
                    .local_recovery_file_ids("keep-fault")
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(store.draft_file_usage(&local.id).unwrap(), (1, 4));
        }
    }

    #[test]
    fn corruption_is_bounded_and_never_adopted_but_remains_discardable() {
        for corruption in [
            "UPDATE local_draft_recoveries SET original_id='other'",
            "UPDATE local_draft_recoveries SET base_revision='other'",
            "UPDATE local_draft_recoveries SET snapshot_bytes=1",
            "UPDATE local_draft_recovery_files SET name='other.txt'",
            "UPDATE local_draft_recovery_files SET content_type='other/type'",
            "UPDATE local_draft_recovery_files SET bytes=-1",
            "UPDATE local_draft_recovery_files SET bytes=52428801",
            "UPDATE local_draft_recovery_files SET data='safe'",
            "UPDATE local_draft_recovery_files SET data=X'73686F7274'",
            "UPDATE local_draft_recovery_files SET source_id='unexpected'",
            "UPDATE local_draft_recovery_files SET name=X'FF'",
        ] {
            let mut store = store_with_work();
            let mut local = draft("corrupt-adopt-source");
            local.attachments.push(
                store
                    .add_draft_file(&local.id, "a.txt", "text/plain", b"safe", 0)
                    .unwrap(),
            );
            let saved = store
                .keep_local_draft_recovery("corrupt-adopt", None, &local, 0)
                .unwrap();
            store
                .conn
                .execute_batch("PRAGMA ignore_check_constraints=ON")
                .unwrap();
            store.conn.execute_batch(corruption).unwrap();
            assert!(
                store.local_draft_recovery("corrupt-adopt").is_err(),
                "{corruption}"
            );
            assert!(
                store
                    .adopt_local_draft_recovery(
                        "corrupt-adopt",
                        &saved.version,
                        "corrupt-target",
                        "work",
                        &[],
                        1
                    )
                    .is_err(),
                "{corruption}"
            );
            assert!(store.editable_draft("corrupt-target").unwrap().is_none());
            assert_eq!(store.draft_file_usage("corrupt-target").unwrap(), (0, 0));
            let row = &store.local_draft_recovery_index().unwrap()[0];
            store
                .discard_local_draft_recovery(&row.id, &row.version)
                .unwrap();
            assert!(store.local_draft_recovery_index().unwrap().is_empty());
        }
    }

    #[test]
    fn ignored_file_cleanup_preserves_previous_snapshot() {
        let mut store = store_with_work();
        let mut local = draft("cleanup-source");
        local.attachments.push(
            store
                .add_draft_file(&local.id, "a.txt", "text/plain", b"safe", 0)
                .unwrap(),
        );
        let saved = store
            .keep_local_draft_recovery("cleanup", None, &local, 0)
            .unwrap();
        store.conn.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON local_draft_recovery_files BEGIN SELECT RAISE(IGNORE); END").unwrap();
        local.attachments.clear();
        assert!(
            store
                .keep_local_draft_recovery("cleanup", Some(&saved.version), &local, 1)
                .is_err()
        );
        assert_eq!(store.local_draft_recovery("cleanup").unwrap(), Some(saved));
    }

    #[test]
    fn adoption_survives_restart_and_tracks_handoff_without_resurrecting_mail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adopt.db");
        let mut store = Store::open(&path).unwrap();
        store
            .upsert_account(&account("work", "work@example.test"))
            .unwrap();
        let local = draft("restart-source");
        let saved = store
            .keep_local_draft_recovery("restart", None, &local, 0)
            .unwrap();
        let adopted = store
            .adopt_local_draft_recovery("restart", &saved.version, "restart-target", "work", &[], 1)
            .unwrap()
            .unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .adopt_local_draft_recovery(
                    "restart",
                    &saved.version,
                    "restart-target",
                    "work",
                    &[],
                    2
                )
                .unwrap(),
            Some(adopted.clone())
        );
        let synced = store.synced_draft(&adopted.id).unwrap().unwrap();
        assert!(!synced.pushed);
        assert!(synced.copy.is_none());
        assert!(
            store
                .prepare_delivery(
                    "recovered-send",
                    &adopted,
                    "{}",
                    "recovered@example.test",
                    2
                )
                .unwrap()
        );
        assert!(
            store
                .adopt_local_draft_recovery(
                    "restart",
                    &saved.version,
                    "restart-target",
                    "work",
                    &[],
                    3
                )
                .unwrap()
                .is_none()
        );
        assert!(store.cancel_delivery("recovered-send").unwrap());
        let restored = store
            .adopt_local_draft_recovery("restart", &saved.version, "restart-target", "work", &[], 4)
            .unwrap()
            .unwrap();
        assert_eq!(restored.body, local.body);
        store
            .schedule_operation("recovered-schedule", &restored, "{}", 100, 5)
            .unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "restart",
                    &saved.version,
                    "restart-target",
                    "work",
                    &[],
                    6
                )
                .unwrap()
                .is_none()
        );
        store
            .cancel_schedule_operation("recovered-schedule", &restored.id, 7)
            .unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "restart",
                    &saved.version,
                    "restart-target",
                    "work",
                    &[],
                    8
                )
                .unwrap()
                .is_some()
        );
        assert!(
            matches!(store.keep_local_draft_recovery("restart",None,&local,9),Err(Error::LocalDraftRecoveryResolved{adopted_id:Some(id)}) if id=="restart-target")
        );
    }

    #[test]
    fn mixed_attachment_adoption_preserves_the_winning_draft_and_requires_fresh_review() {
        let mut store = store_with_work();
        let mut local = draft("mixed-original");
        let first = store
            .add_draft_file(&local.id, "a.txt", "text/plain", b"a", 0)
            .unwrap();
        let last = store
            .add_draft_file(&local.id, "b.txt", "text/plain", b"b", 0)
            .unwrap();
        let missing = moss_core::DraftFile {
            id: "absent".into(),
            name: "gone.txt".into(),
            bytes: 1,
            content_type: "text/plain".into(),
            forwarded: None,
        };
        let forwarded = moss_core::DraftFile {
            id: "forwarded".into(),
            name: "old.txt".into(),
            bytes: 1,
            content_type: "text/plain".into(),
            forwarded: Some(moss_core::FileRef {
                message_id: "1".into(),
                index: 0,
            }),
        };
        local.attachments = vec![first.clone(), missing, forwarded, last.clone()];
        let mut winner = local.clone();
        winner.attachments = vec![first, last];
        winner.body = "Winning text".into();
        store.save_draft(&winner).unwrap();
        let winner = store.editable_draft(&winner.id).unwrap().unwrap();
        let saved = store
            .keep_local_draft_recovery("mixed", None, &local, 0)
            .unwrap();
        local.body = "More local text".into();
        let updated = store
            .keep_local_draft_recovery("mixed", Some(&saved.version), &local, 1)
            .unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "mixed",
                    &saved.version,
                    "mixed-target",
                    "work",
                    &saved.missing_files,
                    2
                )
                .is_err()
        );
        assert!(
            store
                .adopt_local_draft_recovery(
                    "mixed",
                    &updated.version,
                    "mixed-target",
                    "work",
                    &["absent".into()],
                    2
                )
                .is_err()
        );
        let mut reversed = updated.missing_files.clone();
        reversed.reverse();
        let adopted = store
            .adopt_local_draft_recovery(
                "mixed",
                &updated.version,
                "mixed-target",
                "work",
                &reversed,
                2,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            adopted
                .attachments
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt", "b.txt"]
        );
        assert_eq!(
            store.editable_draft(&winner.id).unwrap(),
            Some(winner.clone())
        );
        assert_eq!(store.draft_file_usage(&winner.id).unwrap(), (2, 2));
        assert_eq!(
            store
                .adopt_local_draft_recovery(
                    "mixed",
                    &updated.version,
                    "mixed-target",
                    "work",
                    &updated.missing_files,
                    3
                )
                .unwrap(),
            Some(adopted.clone())
        );
        let mut trimmed = adopted;
        trimmed.attachments.clear();
        store.edit_draft(&trimmed, &[], 4, |_| Ok(vec![])).unwrap();
        assert_eq!(store.draft_file_usage(&trimmed.id).unwrap(), (0, 0));
        store.remove_account("work").unwrap();
        assert!(
            store
                .adopt_local_draft_recovery(
                    "mixed",
                    &updated.version,
                    "mixed-target",
                    "work",
                    &reversed,
                    5
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unreadable_quota_source_metadata_or_copied_bytes_fail_closed() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        for stage in 0..3 {
            let mut store = store_with_work();
            let mut local = draft("read-fault-source");
            local.attachments.push(
                store
                    .add_draft_file(&local.id, "a.txt", "text/plain", b"safe", 0)
                    .unwrap(),
            );
            let saved = if stage == 2 {
                Some(
                    store
                        .keep_local_draft_recovery("read-fault", None, &local, 0)
                        .unwrap(),
                )
            } else {
                None
            };
            let denied=std::sync::atomic::AtomicBool::new(false);
            store
                .conn
                .authorizer(Some(move |context: AuthContext<'_>| {
                    let deny = match context.action {
                        AuthAction::Function { function_name } => {
                            stage == 0 && function_name == "sum" && !denied.swap(true,std::sync::atomic::Ordering::SeqCst)
                        }
                        AuthAction::Read {
                            table_name,
                            column_name,
                        } => {
                            (stage == 1 && table_name == "draft_files" && column_name == "name")
                                || (stage == 2
                                    && table_name == "local_draft_recovery_files"
                                    && column_name == "data")
                        }
                        _ => false,
                    };
                    if deny {
                        Authorization::Deny
                    } else {
                        Authorization::Allow
                    }
                }))
                .unwrap();
            if stage == 2 {
                assert!(matches!(
                    store.local_draft_recovery("read-fault"),
                    Err(Error::Sqlite(_))
                ));
            } else {
                assert!(matches!(
                    store.keep_local_draft_recovery("read-fault", None, &local, 0),
                    Err(Error::Sqlite(_))
                ));
            }
            store
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            assert_eq!(store.local_draft_recovery("read-fault").unwrap(), saved);
            assert_eq!(store.draft_file_usage(&local.id).unwrap(), (1, 4));
        }
    }

    #[test]
    fn aggregate_budget_failure_rolls_back_dropped_files_and_released_space_can_be_reused() {
        let mut store = store_with_work();
        let mut local = draft("budget-source");
        local.attachments.push(
            store
                .add_draft_file(&local.id, "first.txt", "text/plain", &vec![1; 4096], 0)
                .unwrap(),
        );
        let first = store
            .keep_local_recovery_with_budget("budget-first", None, &local, 0, 8192)
            .unwrap();
        local.attachments.clear();
        local.body = "x".repeat(8192);
        assert!(
            store
                .keep_local_recovery_with_budget(
                    "budget-first",
                    Some(&first.version),
                    &local,
                    1,
                    8192
                )
                .is_err()
        );
        assert_eq!(
            store.local_draft_recovery("budget-first").unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            store.local_recovery_file_ids("budget-first").unwrap().len(),
            1
        );
        let mut second = draft("budget-second-source");
        second.attachments.push(
            store
                .add_draft_file(&second.id, "second.txt", "text/plain", &vec![2; 6144], 0)
                .unwrap(),
        );
        assert!(
            store
                .keep_local_recovery_with_budget("budget-second", None, &second, 1, 8192)
                .is_err()
        );
        assert!(
            store
                .local_draft_recovery("budget-second")
                .unwrap()
                .is_none()
        );
        local.body.clear();
        store
            .keep_local_recovery_with_budget("budget-first", Some(&first.version), &local, 2, 8192)
            .unwrap();
        assert!(
            store
                .local_recovery_file_ids("budget-first")
                .unwrap()
                .is_empty()
        );
        let kept = store
            .keep_local_recovery_with_budget("budget-second", None, &second, 3, 8192)
            .unwrap();
        assert!(kept.missing_files.is_empty());
        assert_eq!(store.local_draft_recovery_index().unwrap().len(), 2);
    }
    #[test]
    fn corrupt_recovery_file_index_is_bounded_and_fails_closed() {
        let mut store=store_with_work();
        let local=draft("index-limit");
        store.keep_local_draft_recovery("index-limit",None,&local,NOV_1).unwrap();
        for i in 0..=MAX_FILES {
            store.conn.execute("INSERT INTO local_draft_recovery_files(recovery_id,source_id,name,content_type,bytes,sha256,data) VALUES('index-limit',?1,'empty','text/plain',0,'unused',x'')",[i.to_string()]).unwrap();
        }
        assert!(store.local_recovery_file_ids("index-limit").unwrap_err().to_string().contains("exceeds its limit"));
        assert!(store.local_draft_recovery("index-limit").is_err());
        store.conn.execute_batch("DROP TABLE local_draft_recovery_files").unwrap();
        assert!(store.local_recovery_file_ids("index-limit").is_err());
    }

}
