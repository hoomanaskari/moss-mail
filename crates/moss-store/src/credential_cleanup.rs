//! Durable references to retired credentials. Secret values never enter this table.
use rusqlite::params;
use crate::{Error, Result, Store};

const CLEANUP_CURSOR: &str = "credentialCleanupCursor";
const CLEANUP_BATCH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialCleanup {
    pub service: String,
    pub account: String,
}

impl Store {
    /// Keep a retryable cleanup intent for a sign-in that could not be attached.
    pub fn retire_credential(&mut self, reference: &CredentialCleanup) -> Result<()> {
        self.delivery_transaction(|store| store.queue_credential_cleanup(reference))
    }

    pub(crate) fn queue_credential_cleanup(&self, reference: &CredentialCleanup) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO credential_cleanup(service,account) VALUES(?1,?2)", params![reference.service,reference.account])?;
        if !self.has_credential_cleanup(reference)? {
            return Err(Error::Invalid("The credential cleanup could not be saved. The account has been kept.".into()));
        }
        Ok(())
    }

    fn has_credential_cleanup(&self, reference: &CredentialCleanup) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM credential_cleanup WHERE service=?1 AND account=?2)", params![reference.service,reference.account], |r|r.get(0))?)
    }

    /// Inspect the first bounded page. Retry workers use the durable round-robin selector.
    #[cfg(test)]
    pub fn credential_cleanups(&self) -> Result<Vec<CredentialCleanup>> {
        let mut statement = self.conn.prepare("SELECT service,account FROM credential_cleanup ORDER BY service,account LIMIT 128")?;
        let rows = statement.query_map([], |r|Ok(CredentialCleanup {service:r.get(0)?,account:r.get(1)?}))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Persist a bounded round-robin scan position before attempting any Keychain work.
    /// Rowids are scan positions only; exact references remain deletion authority.
    pub fn next_credential_cleanup_batch(&mut self) -> Result<Vec<CredentialCleanup>> {
        self.delivery_transaction(|store| {
            // This is progress only, not authorization. A malformed position can
            // restart the scan; the replacement still has to commit before work.
            let cursor = store.setting(CLEANUP_CURSOR)?
                .and_then(|value| value.parse::<i64>().ok()).unwrap_or(0);
            let mut batch = store.cleanup_page(cursor, CLEANUP_BATCH, false)?;
            if batch.len() < CLEANUP_BATCH {
                batch.extend(store.cleanup_page(cursor, CLEANUP_BATCH - batch.len(), true)?);
            }
            if let Some((position, _)) = batch.last() {
                let value = position.to_string();
                store.conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![CLEANUP_CURSOR,value])?;
                if store.setting(CLEANUP_CURSOR)?.as_deref() != Some(value.as_str()) {
                    return Err(Error::Invalid("The credential cleanup position could not be saved.".into()));
                }
            }
            Ok(batch.into_iter().map(|(_, reference)| reference).collect())
        })
    }

    fn cleanup_page(&self, cursor: i64, limit: usize, wrap: bool) -> Result<Vec<(i64, CredentialCleanup)>> {
        let sql = if wrap {
            "SELECT rowid,service,account FROM credential_cleanup WHERE rowid<=?1 ORDER BY rowid LIMIT ?2"
        } else {
            "SELECT rowid,service,account FROM credential_cleanup WHERE rowid>?1 ORDER BY rowid LIMIT ?2"
        };
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement.query_map(params![cursor,limit as i64], |row| Ok((row.get(0)?, CredentialCleanup {service:row.get(1)?,account:row.get(2)?})))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn credential_cleanup_pending(&self) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM credential_cleanup)", [], |r|r.get(0))?)
    }

    /// Called only after the exact Keychain item was deleted successfully.
    pub fn finish_credential_cleanup(&mut self, reference: &CredentialCleanup) -> Result<()> {
        self.delivery_transaction(|store| {
            store.conn.execute("DELETE FROM credential_cleanup WHERE service=?1 AND account=?2", params![reference.service,reference.account])?;
            if store.has_credential_cleanup(reference)? {
                return Err(Error::Invalid("Credential cleanup is still pending.".into()));
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{account, store_with_work};
    fn reference() -> CredentialCleanup { CredentialCleanup {service:"app.moss.mail".into(),account:"fixture-generation".into()} }

    #[test]
    fn removal_and_cleanup_intent_survive_reopen_without_secret_values() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("mail.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "a@example.com")).unwrap();
        store.set_setting("connection:work", Some("connection metadata")).unwrap();
        assert!(store.remove_account_with_cleanup("work", Some(&reference())).unwrap());
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(store.accounts().unwrap().is_empty());
        assert_eq!(store.setting("connection:work").unwrap(), None);
        assert_eq!(store.credential_cleanups().unwrap(), [reference()]);
        assert!(store.credential_cleanup_pending().unwrap());
        store.finish_credential_cleanup(&reference()).unwrap();
        store.finish_credential_cleanup(&reference()).unwrap();
        assert!(!store.credential_cleanup_pending().unwrap());
        assert!(!store.remove_account_with_cleanup("work", Some(&reference())).unwrap());
        assert!(store.credential_cleanups().unwrap().is_empty());
    }

    #[test]
    fn refused_or_ignored_cleanup_intent_keeps_the_whole_account() {
        for action in ["RAISE(IGNORE)", "RAISE(ABORT,'fixture')"] {
            let mut store = store_with_work();
            store.set_setting("connection:work", Some("metadata")).unwrap();
            store.conn.execute_batch(&format!("CREATE TRIGGER deny_cleanup BEFORE INSERT ON credential_cleanup BEGIN SELECT {action}; END;")).unwrap();
            assert!(store.remove_account_with_cleanup("work", Some(&reference())).is_err());
            assert_eq!(store.accounts().unwrap().len(), 1);
            assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("metadata"));
            assert!(!store.credential_cleanup_pending().unwrap());
        }
    }

    #[test]
    fn failed_account_delete_rolls_back_the_cleanup_intent_too() {
        let mut store = store_with_work();
        store.conn.execute_batch("CREATE TRIGGER deny_account BEFORE DELETE ON accounts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(store.remove_account_with_cleanup("work", Some(&reference())).is_err());
        assert!(!store.credential_cleanup_pending().unwrap());
        assert_eq!(store.accounts().unwrap().len(), 1);
    }

    #[test]
    fn acknowledgment_is_exact_and_ignored_deletes_remain_pending() {
        let mut store = store_with_work();
        store.queue_credential_cleanup(&reference()).unwrap();
        store.queue_credential_cleanup(&reference()).unwrap();
        let other = CredentialCleanup {account:"other".into(), ..reference()};
        store.queue_credential_cleanup(&other).unwrap();
        store.conn.execute_batch("CREATE TRIGGER deny_cleanup BEFORE DELETE ON credential_cleanup BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.finish_credential_cleanup(&reference()).is_err());
        assert_eq!(store.credential_cleanups().unwrap().len(), 2);
        store.conn.execute_batch("DROP TRIGGER deny_cleanup").unwrap();
        store.finish_credential_cleanup(&reference()).unwrap();
        assert_eq!(store.credential_cleanups().unwrap(), [other]);
    }

    #[test]
    fn cleanup_passes_are_bounded_and_references_cannot_be_rewritten() {
        let mut store = store_with_work();
        for i in 0..130 { store.queue_credential_cleanup(&CredentialCleanup {account:format!("fixture-{i:03}"), ..reference()}).unwrap(); }
        let batch = store.credential_cleanups().unwrap();
        assert_eq!(batch.len(),128);
        assert!(store.conn.execute("UPDATE credential_cleanup SET account='replacement'", []).is_err());
        for key in batch {store.finish_credential_cleanup(&key).unwrap();}
        assert_eq!(store.credential_cleanups().unwrap().len(),2);
        assert!(store.queue_credential_cleanup(&CredentialCleanup {account:"".into(), ..reference()}).is_err());
    }
    fn queue_many(store: &Store, count: usize) {
        for i in 0..count {
            store.queue_credential_cleanup(&CredentialCleanup {account:format!("fixture-{i:03}"), ..reference()}).unwrap();
        }
    }

    #[test]
    fn retry_position_survives_reopen_and_wraps_without_repeating_a_reference_in_a_batch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let mut store = Store::open(&path).unwrap();
        queue_many(&store, 130);
        let first = store.next_credential_cleanup_batch().unwrap();
        assert_eq!(first.len(), 128);
        assert_eq!(first[0].account, "fixture-000");
        assert_eq!(first[127].account, "fixture-127");
        assert_eq!(store.setting(CLEANUP_CURSOR).unwrap().as_deref(), Some("128"));
        drop(store);
        let mut store = Store::open(&path).unwrap();
        let second = store.next_credential_cleanup_batch().unwrap();
        assert_eq!(second.len(), 128);
        assert_eq!(second[0].account, "fixture-128");
        assert_eq!(second[1].account, "fixture-129");
        assert_eq!(second[2].account, "fixture-000");
        let unique: std::collections::HashSet<_> = second.iter().map(|r|(&r.service,&r.account)).collect();
        assert_eq!(unique.len(), second.len());
        assert_eq!(store.setting(CLEANUP_CURSOR).unwrap().as_deref(), Some("126"));
        assert_eq!(store.conn.query_row("SELECT count(*) FROM credential_cleanup", [], |r|r.get::<_,i64>(0)).unwrap(), 130);
    }

    #[test]
    fn an_empty_queue_and_reused_rowids_do_not_lose_new_references() {
        let mut store = store_with_work();
        assert!(store.next_credential_cleanup_batch().unwrap().is_empty());
        assert_eq!(store.setting(CLEANUP_CURSOR).unwrap(), None);
        store.queue_credential_cleanup(&reference()).unwrap();
        let old_id: i64 = store.conn.query_row("SELECT rowid FROM credential_cleanup", [], |r|r.get(0)).unwrap();
        assert_eq!(store.next_credential_cleanup_batch().unwrap(), [reference()]);
        store.finish_credential_cleanup(&reference()).unwrap();
        assert!(store.next_credential_cleanup_batch().unwrap().is_empty());
        let replacement = CredentialCleanup {account:"replacement-generation".into(), ..reference()};
        store.queue_credential_cleanup(&replacement).unwrap();
        let new_id: i64 = store.conn.query_row("SELECT rowid FROM credential_cleanup", [], |r|r.get(0)).unwrap();
        assert_eq!(old_id, new_id, "exercise actual SQLite rowid reuse");
        store.finish_credential_cleanup(&reference()).unwrap();
        assert_eq!(store.next_credential_cleanup_batch().unwrap(), [replacement]);
        assert!(store.credential_cleanup_pending().unwrap());
    }

    #[test]
    fn malformed_scan_positions_restart_without_discarding_intent_and_extremes_wrap() {
        let mut store = store_with_work();
        store.queue_credential_cleanup(&reference()).unwrap();
        for value in ["unreadable", "1.5", "9223372036854775808"] {
            store.set_setting(CLEANUP_CURSOR, Some(value)).unwrap();
            assert_eq!(store.next_credential_cleanup_batch().unwrap(), [reference()]);
            assert_eq!(store.setting(CLEANUP_CURSOR).unwrap().as_deref(), Some("1"));
            assert_eq!(store.credential_cleanups().unwrap(), [reference()]);
        }
        for value in [i64::MIN.to_string(), i64::MAX.to_string()] {
            store.set_setting(CLEANUP_CURSOR, Some(&value)).unwrap();
            assert_eq!(store.next_credential_cleanup_batch().unwrap(), [reference()]);
        }
    }

    #[test]
    fn ignored_or_refused_cursor_writes_do_not_release_a_batch() {
        for update in [false, true] {
            for action in ["RAISE(IGNORE)", "RAISE(ABORT,'fixture')"] {
                let mut store = store_with_work();
                queue_many(&store, 129);
                if update { assert_eq!(store.next_credential_cleanup_batch().unwrap().len(), 128); }
                let before = store.setting(CLEANUP_CURSOR).unwrap();
                let event = if update {"UPDATE"} else {"INSERT"};
                store.conn.execute_batch(&format!("CREATE TRIGGER refuse_cursor BEFORE {event} ON settings WHEN NEW.key='credentialCleanupCursor' BEGIN SELECT {action}; END;")).unwrap();
                assert!(store.next_credential_cleanup_batch().is_err());
                assert_eq!(store.setting(CLEANUP_CURSOR).unwrap(), before);
                assert_eq!(store.conn.query_row("SELECT count(*) FROM credential_cleanup", [], |r|r.get::<_,i64>(0)).unwrap(), 129);
            }
        }
    }

    #[test]
    fn cursor_is_committed_with_full_durability() {
        let mut store = store_with_work();
        store.queue_credential_cleanup(&reference()).unwrap();
        store.conn.execute_batch("CREATE TABLE observed_cursor_mode(sync INTEGER, flush INTEGER); CREATE TRIGGER observe_cursor AFTER INSERT ON settings WHEN NEW.key='credentialCleanupCursor' BEGIN INSERT INTO observed_cursor_mode SELECT (SELECT synchronous FROM pragma_synchronous),(SELECT fullfsync FROM pragma_fullfsync); END;").unwrap();
        assert_eq!(store.next_credential_cleanup_batch().unwrap(), [reference()]);
        let observed = store.conn.query_row("SELECT sync,flush FROM observed_cursor_mode", [], |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap();
        assert_eq!(observed, (2,1));
    }

}
