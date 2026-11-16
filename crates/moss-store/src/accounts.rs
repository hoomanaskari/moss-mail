//! Accounts, mailboxes and sync checkpoints.

use moss_core::{Account, Mailbox, SyncCheckpoint};
use rusqlite::{params, OptionalExtension};

use crate::codec::{name, named, now_ms};
use crate::{Result, Store};

impl Store {
    /// Adds an account, or updates it in place. New accounts go to the end of the switcher.
    pub fn upsert_account(&mut self, account: &Account) -> Result<()> {
        self.write_account(account)
    }

    fn write_account(&self, account: &Account) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (id, name, letter, address, sender_name, color, provider, org, org_domain, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, (SELECT coalesce(max(position) + 1, 0) FROM accounts))
             ON CONFLICT (id) DO UPDATE SET name = ?2, letter = ?3, address = ?4, sender_name = ?5, color = ?6,
                 provider = ?7, org = ?8, org_domain = ?9",
            params![
                account.id,
                account.name,
                account.letter,
                account.address,
                account.sender_name,
                name(&account.color),
                name(&account.provider),
                account.org,
                account.org_domain,
            ],
        )?;
        Ok(())
    }

    /// Save the account and its exact connection metadata as one durable replacement.
    #[cfg(test)]
    pub fn save_account_connection(&mut self, account: &Account, connection: &str) -> Result<()> {
        self.save_account_connection_retiring(account, connection, None)
    }

    /// Retire the replaced reference in the same commit as the new connection.
    pub fn save_account_connection_retiring(&mut self, account: &Account, connection: &str, retired: Option<&crate::CredentialCleanup>) -> Result<()> {
        self.delivery_transaction(|store| {
            if let Some(retired) = retired { store.queue_credential_cleanup(retired)?; }
            store.write_account(account)?;
            if store.accounts()?.iter().find(|saved| saved.id == account.id) != Some(account) {
                return Err(crate::Error::Invalid("The account could not be saved. Its previous connection has been kept.".into()));
            }
            let key = format!("connection:{}", account.id);
            store.conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,connection])?;
            if store.setting(&key)?.as_deref() != Some(connection) {
                return Err(crate::Error::Invalid("The connection could not be saved. The account has not changed.".into()));
            }
            Ok(())
        })
    }

    /// Accounts in switcher order.
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, letter, address, sender_name, color, provider, org, org_domain
             FROM accounts ORDER BY position",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                name: r.get(1)?,
                letter: r.get(2)?,
                address: r.get(3)?,
                sender_name: r.get(4)?,
                color: named(r, 5)?,
                provider: named(r, 6)?,
                org: r.get(7)?,
                org_domain: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Puts accounts in this order in the switcher. Accounts not listed keep their place after them.
    pub fn reorder_accounts(&mut self, ids: &[&str]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE accounts SET position = position + ?1", [ids.len() as i64])?;
        for (position, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE accounts SET position = ?1 WHERE id = ?2",
                params![position as i64, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Refuse destructive cascades until their evidence has an explicit retirement path.
    /// This reads metadata only. The engine calls it before touching workers or credentials.
    pub fn ensure_account_removable(&self, id: &str) -> Result<()> {
        if self.account_removal_check(id)? > 0 {
            return Err(crate::Error::Invalid("This account has pending server changes. Resolve them before removing the account.".into()));
        }
        Ok(())
    }

    /// How many server changes would be lost by removing this account, when nothing else stops
    /// it. Unsent, uncertain or scheduled mail is an error saying which, as before.
    pub fn account_removal_check(&self, id: &str) -> Result<usize> {
        for (sql, message) in [
            ("SELECT EXISTS(SELECT 1 FROM deliveries WHERE account_id=?1 AND resolution IS NULL AND state NOT IN ('complete','cancelled'))", "This account has an unresolved delivery. Review its outcome before removing the account."),
            ("SELECT EXISTS(SELECT 1 FROM scheduled WHERE account_id=?1)", "This account has scheduled mail. Cancel or send it before removing the account."),
        ] {
            if self.conn.query_row(sql, [id], |r| r.get::<_,bool>(0))? {
                return Err(crate::Error::Invalid(message.into()));
            }
        }
        let waiting: i64 = self.conn.query_row("SELECT count(*) FROM outbox WHERE account_id=?1", [id], |r| r.get(0))?;
        Ok(waiting as usize)
    }

    /// Removes unprotected local account data and its connection setting atomically.
    pub fn remove_account(&mut self, id: &str) -> Result<bool> {
        self.remove_account_with_cleanup(id, None)
    }

    /// The caller captures the exact connection reference while holding account ownership.
    pub fn remove_account_with_cleanup(&mut self, id: &str, cleanup: Option<&crate::CredentialCleanup>) -> Result<bool> {
        self.remove_account_inner(id, cleanup, None)
    }

    /// Removes the account although `changes` server changes haven't reached the server: you
    /// were told they will be lost. They go in the same commit. A different number means
    /// something changed since you were asked, and nothing is removed.
    pub fn remove_account_discarding_changes(&mut self, id: &str, cleanup: Option<&crate::CredentialCleanup>, changes: usize) -> Result<bool> {
        self.remove_account_inner(id, cleanup, Some(changes))
    }

    fn remove_account_inner(&mut self, id: &str, cleanup: Option<&crate::CredentialCleanup>, discard: Option<usize>) -> Result<bool> {
        self.delivery_transaction(|store| {
            match discard {
                None => store.ensure_account_removable(id)?,
                Some(changes) => store.discard_account_changes(id, changes)?,
            }
            let exists: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)", [id], |r| r.get(0))?;
            if !exists { return Ok(false); }
            store.conn.execute("UPDATE deliveries SET account_retired=1 WHERE account_id=?1 AND account_retired=0", [id])?;
            let unretired: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE account_id=?1 AND account_retired=0)", [id], |row| row.get(0))?;
            if unretired { return Err(crate::Error::Invalid("The account's delivery receipts could not be retired. The account has been kept.".into())); }
            store.compact_prehashed_account(id,crate::codec::now_ms())?;
            if let Some(cleanup) = cleanup { store.queue_credential_cleanup(cleanup)?; }
            let key = format!("connection:{id}");
            store.conn.execute("DELETE FROM settings WHERE key=?1", [&key])?;
            let remains: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key=?1)", [&key], |r| r.get(0))?;
            if remains { return Err(crate::Error::Invalid("The account's connection could not be removed. The account has been kept.".into())); }
            if store.conn.execute("DELETE FROM accounts WHERE id=?1", [id])? != 1 {
                return Err(crate::Error::Invalid("The account was not removed. Its saved connection has been kept.".into()));
            }
            Ok(true)
        })
    }

    /// Inside removal's transaction: the exact number of changes you were told about go, each
    /// part recorded as discarded. One on its way to the server now stops the removal.
    fn discard_account_changes(&self, id: &str, changes: usize) -> Result<()> {
        if self.account_removal_check(id)? != changes {
            return Err(crate::Error::Invalid("The changes waiting for the server changed. Check them again before removing the account.".into()));
        }
        let going: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE account_id=?1 AND state='claimed')", [id], |r| r.get(0))?;
        if going { return Err(crate::Error::Invalid("A change is reaching the server now. Try removing the account again in a moment.".into())); }
        self.conn.execute("UPDATE outbox_batch_parts SET outcome='discarded' WHERE outcome='pending' AND token IN (SELECT token FROM outbox WHERE account_id=?1)", [id])?;
        if self.conn.execute("DELETE FROM outbox WHERE account_id=?1", [id])? != changes {
            return Err(crate::Error::Invalid("The waiting server changes could not be removed. The account has been kept.".into()));
        }
        Ok(())
    }

    /// Adds a mailbox or updates its role.
    pub fn upsert_mailbox(&mut self, mailbox: &Mailbox) -> Result<()> {
        self.conn.execute(
            "INSERT INTO mailboxes (account_id, name, role) VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id, name) DO UPDATE SET role = ?3",
            params![mailbox.account_id, mailbox.name, mailbox.role.map(|r| name(&r))],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn mailboxes(&self, account_id: &str) -> Result<Vec<Mailbox>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, role FROM mailboxes WHERE account_id = ?1 ORDER BY name")?;
        let rows = stmt.query_map([account_id], |r| {
            let role: Option<String> = r.get(1)?;
            Ok(Mailbox {
                account_id: account_id.into(),
                name: r.get(0)?,
                role: role.map(|_| named(r, 1)).transpose()?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Where sync got to. `mailbox` is `None` for account-wide cursors, like Resend's.
    pub fn checkpoint(
        &self,
        account_id: &str,
        mailbox: Option<&str>,
    ) -> Result<Option<SyncCheckpoint>> {
        Ok(self
            .conn
            .query_row(
                "SELECT uid_validity, uid_next, highest_modseq, cursor FROM sync_checkpoints
                 WHERE account_id = ?1 AND mailbox = ?2",
                params![account_id, mailbox.unwrap_or_default()],
                |r| {
                    Ok(SyncCheckpoint {
                        uid_validity: r.get(0)?,
                        uid_next: r.get(1)?,
                        highest_modseq: r.get::<_, Option<i64>>(2)?.map(|m| m as u64),
                        cursor: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn save_checkpoint(
        &mut self,
        account_id: &str,
        mailbox: Option<&str>,
        checkpoint: &SyncCheckpoint,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_checkpoints (account_id, mailbox, uid_validity, uid_next, highest_modseq, cursor, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (account_id, mailbox) DO UPDATE SET uid_validity = ?3, uid_next = ?4, highest_modseq = ?5,
                 cursor = ?6, updated_at = ?7",
            params![
                account_id,
                mailbox.unwrap_or_default(),
                checkpoint.uid_validity,
                checkpoint.uid_next,
                checkpoint.highest_modseq.map(|m| m as i64),
                checkpoint.cursor,
                now_ms(),
            ],
        )?;
        Ok(())
    }

    /// Whether a provider message is stored already, so sync can skip fetching it.
    pub fn contains(&self, account_id: &str, mailbox: &str, provider_id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM message_locations l JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE b.account_id = ?1 AND b.name = ?2 AND l.provider_id = ?3)",
            params![account_id, mailbox, provider_id],
            |r| r.get(0),
        )?)
    }

    /// Forgets the provider ids stored for a mailbox, e.g. after IMAP UIDVALIDITY changes.
    /// The messages stay: Moss keeps its own copy, and re-ingesting them links them again.
    pub fn forget_mailbox_ids(&mut self, account_id: &str, mailbox: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM message_locations WHERE mailbox_id IN
             (SELECT id FROM mailboxes WHERE account_id = ?1 AND name = ?2)",
            params![account_id, mailbox],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{AccountColor, MailboxRole};

    #[test]
    fn connection_replacement_rolls_back_ignored_and_refused_writes() {
        for existing in [false, true] {
            for table in ["accounts", "settings"] {
                for action in ["RAISE(IGNORE)", "RAISE(ABORT,'fixture')"] {
                    let mut store = Store::open_in_memory().unwrap();
                    let old = account("work", "a@example.com");
                    if existing { store.save_account_connection(&old, "old metadata").unwrap(); }
                    let event = if existing { "UPDATE" } else { "INSERT" };
                    store.conn.execute_batch(&format!("CREATE TRIGGER refuse_save BEFORE {event} ON {table} BEGIN SELECT {action}; END;")).unwrap();
                    let mut replacement = old.clone(); replacement.name = "Replacement".into();
                    assert!(store.save_account_connection(&replacement, "new metadata").is_err());
                    assert_eq!(store.accounts().unwrap(), if existing {vec![old]} else {vec![]});
                    assert_eq!(store.setting("connection:work").unwrap().as_deref(), if existing {Some("old metadata")} else {None});
                }
            }
        }
    }

    #[test]
    fn connection_replacement_survives_reopen_and_preserves_switcher_order() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("mail.db");
        let mut store = Store::open(&path).unwrap();
        let mut first = account("first", "a@example.com");
        let second = account("second", "b@example.com");
        store.save_account_connection(&first, "old metadata").unwrap();
        store.save_account_connection(&second, "other metadata").unwrap();
        first.name = "Replacement".into();
        store.save_account_connection(&first, "new metadata").unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.accounts().unwrap(), [first, second]);
        assert_eq!(store.setting("connection:first").unwrap().as_deref(), Some("new metadata"));
        assert_eq!(store.setting("connection:second").unwrap().as_deref(), Some("other metadata"));
    }

    #[test]
    fn connection_replacement_observes_full_durability_for_both_records() {
        let mut store = Store::open_in_memory().unwrap();
        store.conn.execute_batch("CREATE TABLE observed_save(sync INTEGER, flush INTEGER); CREATE TRIGGER observe_account AFTER INSERT ON accounts BEGIN INSERT INTO observed_save SELECT (SELECT synchronous FROM pragma_synchronous),(SELECT fullfsync FROM pragma_fullfsync); END; CREATE TRIGGER observe_connection AFTER INSERT ON settings BEGIN INSERT INTO observed_save SELECT (SELECT synchronous FROM pragma_synchronous),(SELECT fullfsync FROM pragma_fullfsync); END;").unwrap();
        store.save_account_connection(&account("work", "a@example.com"), "metadata").unwrap();
        let mut statement = store.conn.prepare("SELECT sync,flush FROM observed_save").unwrap();
        let modes = statement.query_map([], |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(modes, [(2,1),(2,1)]);
    }

    #[test]
    fn accounts_keep_switcher_order_and_update_in_place() {
        let mut store = Store::open_in_memory().unwrap();
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let mut work = account("work", "maya@northwind.co");
        work.color = AccountColor::Green;
        work.org = Some("Northwind".into());
        store.upsert_account(&work).unwrap();
        let all = store.accounts().unwrap();
        assert_eq!(all.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["work", "personal"]);
        assert_eq!(all[0], work);
    }

    #[test]
    fn accounts_can_be_reordered() {
        let mut store = Store::open_in_memory().unwrap();
        for id in ["work", "personal", "studio"] {
            store.upsert_account(&account(id, &format!("{id}@x.co"))).unwrap();
        }
        store.reorder_accounts(&["studio", "work"]).unwrap();
        let order: Vec<String> = store.accounts().unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(order, ["studio", "work", "personal"]);
        store.upsert_account(&account("lab", "lab@x.co")).unwrap();
        assert_eq!(store.accounts().unwrap().last().unwrap().id, "lab");
    }

    #[test]
    fn terminal_delivery_receipts_survive_account_removal_and_recreation() {
        for state in ["complete", "cancelled"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("moss.db");
            let mut store = Store::open(&path).unwrap();
            store.upsert_account(&account("work", "sender@example.test")).unwrap();
            let draft = draft("terminal-draft");
            assert!(store.prepare_delivery("terminal-op", &draft, "{}", "terminal@example.test", 1).unwrap());
            store.conn.execute("UPDATE deliveries SET state=?1 WHERE id='terminal-op'", [state]).unwrap();
            let receipt = store.delivery("terminal-op").unwrap().unwrap();
            assert!(store.remove_account("work").unwrap(), "terminal mail must not prevent retiring a sign-in");
            drop(store);
            let mut store = Store::open(&path).unwrap();
            assert!(store.accounts().unwrap().is_empty());
            assert_eq!(store.delivery("terminal-op").unwrap().unwrap(), receipt);
            assert!(!store.prepare_delivery("terminal-op", &draft, "{}", "terminal@example.test", 2).unwrap());
            assert!(!store.claim_delivery("terminal-op").unwrap());
            assert!(store.prepare_delivery("new-op", &crate::test_support::draft("new-draft"), "{}", "new@example.test", 2).is_err(), "new delivery needs an existing account");
            store.upsert_account(&account("work", "another@example.test")).unwrap();
            ingest(&mut store, "reused-message", &mail().id("terminal@example.test").from("another@example.test").build());
            assert!(store.delivery_receipt_thread("terminal-op").unwrap().is_none(), "an old receipt must not resolve into a replacement account's mail");
            assert!(!store.prepare_delivery("terminal-op", &draft, "{}", "terminal@example.test", 3).unwrap());
            let changed = moss_core::Draft { body: "Different content".into(), ..draft };
            assert!(store.prepare_delivery("terminal-op", &changed, "{}", "terminal@example.test", 3).is_err());
            assert_eq!(store.delivery("terminal-op").unwrap().unwrap(), receipt);
            assert!(store.conn.prepare("PRAGMA foreign_key_check").unwrap().query([]).unwrap().next().unwrap().is_none());
        }
    }

    #[test]
    fn terminal_receipt_retirement_and_account_removal_are_one_commit() {
        for failure in ["IGNORE", "ABORT, 'fixture'"] {
            let mut store = store_with_work();
            store.prepare_delivery("op", &draft("kept"), "{}", "kept@example.test", 1).unwrap();
            store.conn.execute("UPDATE deliveries SET state='complete' WHERE id='op'", []).unwrap();
            store.set_setting("connection:work", Some("kept connection")).unwrap();
            store.conn.execute_batch(&format!("CREATE TRIGGER refuse_receipt_retirement BEFORE UPDATE OF account_retired ON deliveries BEGIN SELECT RAISE({failure}); END;")).unwrap();
            assert!(store.remove_account("work").is_err());
            assert_eq!(store.accounts().unwrap().len(), 1);
            assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("kept connection"));
            assert_eq!(store.conn.query_row("SELECT account_retired FROM deliveries WHERE id='op'", [], |row| row.get::<_,i64>(0)).unwrap(), 0);
            store.conn.execute_batch("DROP TRIGGER refuse_receipt_retirement;
                CREATE TRIGGER refuse_account_removal BEFORE DELETE ON accounts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
            assert!(store.remove_account("work").is_err());
            assert_eq!(store.conn.query_row("SELECT account_retired FROM deliveries WHERE id='op'", [], |row| row.get::<_,i64>(0)).unwrap(), 0, "later failure must undo receipt retirement");
            assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("kept connection"));
            store.conn.execute_batch("DROP TRIGGER refuse_account_removal").unwrap();
            assert!(store.remove_account("work").unwrap());
            assert_eq!(store.conn.query_row("SELECT account_retired FROM deliveries WHERE id='op'", [], |row| row.get::<_,i64>(0)).unwrap(), 1);
        }
    }

    #[test]
    fn unresolved_mail_prevents_retiring_even_terminal_receipts_on_the_same_account() {
        for state in ["held", "claimed", "accepted", "uncertain"] {
            let mut store = store_with_work();
            store.prepare_delivery("terminal", &draft("finished"), "{}", "finished@example.test", 1).unwrap();
            store.prepare_delivery("pending", &draft("unfinished"), "{}", "unfinished@example.test", 2).unwrap();
            store.conn.execute("UPDATE deliveries SET state='complete' WHERE id='terminal'", []).unwrap();
            store.conn.execute("UPDATE deliveries SET state=?1 WHERE id='pending'", [state]).unwrap();
            assert!(store.remove_account("work").is_err(), "{state}");
            assert_eq!(store.accounts().unwrap().len(), 1);
            assert_eq!(store.conn.query_row("SELECT count(*) FROM deliveries WHERE account_retired=0", [], |row| row.get::<_,i64>(0)).unwrap(), 2);
        }
    }

    #[test]
    fn removal_preserves_all_delivery_evidence_and_pending_work() {
        for state in ["held", "claimed", "accepted", "uncertain"] {
            let mut store = store_with_work();
            store.conn.execute("INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at) VALUES('op','draft','work','{}','{}','mail',?1,0)", [state]).unwrap();
            assert!(store.remove_account("work").is_err(), "{state} evidence must survive until explicit retirement exists");
            assert_eq!(store.accounts().unwrap().len(), 1);
            store.upsert_account(&account("other", "other@example.test")).unwrap();
            assert!(store.remove_account("other").unwrap(), "protection is account scoped");
            assert_eq!(store.conn.query_row("SELECT state FROM deliveries WHERE id='op'", [], |r| r.get::<_,String>(0)).unwrap(), state);
        }
        let mut store = store_with_work();
        store.conn.execute("INSERT INTO scheduled(draft_id,account_id,draft,outgoing,send_at,next_try,tries) VALUES('draft','work','{}','{}',1,1,0)", []).unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.conn.query_row("SELECT count(*) FROM scheduled", [], |r|r.get::<_,i64>(0)).unwrap(),1);
        let mut store = store_with_work();
        let action = store.queue_change("work", "retained", 1).unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.pending_changes("work").unwrap(), [(action,"retained".into())]);
    }

    #[test]
    fn account_and_connection_removal_commit_together_and_check_ignored_deletes() {
        let mut store = store_with_work();
        store.set_setting("connection:work", Some("reference-only")).unwrap();
        store.conn.execute_batch("CREATE TRIGGER ignore_removal BEFORE DELETE ON accounts BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("reference-only"));
        assert_eq!(store.accounts().unwrap().len(), 1);
        store.conn.execute_batch("DROP TRIGGER ignore_removal; CREATE TRIGGER keep_connection BEFORE DELETE ON settings BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.remove_account("work").is_err());
        assert_eq!(store.accounts().unwrap().len(), 1);
        assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("reference-only"));
        store.conn.execute_batch("DROP TRIGGER keep_connection").unwrap();
        assert!(store.remove_account("work").unwrap());
        assert_eq!(store.setting("connection:work").unwrap(), None);
    }

    #[test]
    fn an_account_held_back_only_by_server_changes_is_removed_with_them_when_you_say_so() {
        let mut store = store_with_work();
        store.upsert_account(&account("other", "other@example.test")).unwrap();
        assert_eq!(store.account_removal_check("work").unwrap(), 0);
        let first = store.queue_change("work", "first", 1).unwrap();
        store.block_change("work", first, 0, crate::ChangeFailure::Refused).unwrap();
        store.delivery_transaction(|s| s.queue_batch_in_transaction("work", crate::BatchKind::Read, &["a".into(), "b".into()], 2)).unwrap();
        let kept = store.queue_change("other", "kept", 3).unwrap();
        assert_eq!(store.account_removal_check("work").unwrap(), 3);
        assert!(store.remove_account("work").is_err(), "not without saying so");
        let error = store.remove_account_discarding_changes("work", None, 2).unwrap_err();
        assert_eq!(error.to_string(), "The changes waiting for the server changed. Check them again before removing the account.");
        assert_eq!(store.pending_changes("work").unwrap().len(), 3);
        assert!(store.remove_account_discarding_changes("work", None, 3).unwrap());
        assert_eq!(store.accounts().unwrap().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["other"]);
        assert!(store.pending_changes("work").unwrap().is_empty());
        assert_eq!(store.pending_changes("other").unwrap(), [(kept, "kept".into())]);
        for table in ["outbox_batches", "outbox_batch_parts"] {
            let left: i64 = store.conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap();
            assert_eq!(left, 1, "{table}: only the other account's");
        }
        assert!(!store.remove_account_discarding_changes("work", None, 0).unwrap(), "gone already");
    }

    #[test]
    fn discarding_server_changes_never_removes_unsent_mail_or_a_change_on_its_way() {
        let mut store = store_with_work();
        store.queue_change("work", "waiting", 1).unwrap();
        store.prepare_delivery("op", &draft("unsent"), "{}", "unsent@example.test", 1).unwrap();
        let error = store.account_removal_check("work").unwrap_err();
        assert_eq!(error.to_string(), "This account has an unresolved delivery. Review its outcome before removing the account.");
        assert!(store.remove_account_discarding_changes("work", None, 1).is_err());
        let mut store = store_with_work();
        store.queue_change("work", "waiting", 1).unwrap();
        store.schedule_email(&draft("later"), "{}", NOV_1, NOV_1).unwrap();
        assert!(store.account_removal_check("work").is_err());
        assert!(store.remove_account_discarding_changes("work", None, 1).is_err());
        assert_eq!(store.scheduled_emails().unwrap().len(), 1);
        let mut store = store_with_work();
        let going = store.queue_change("work", "going", 1).unwrap();
        store.claim_change("work", going, 0).unwrap();
        let error = store.remove_account_discarding_changes("work", None, 1).unwrap_err();
        assert_eq!(error.to_string(), "A change is reaching the server now. Try removing the account again in a moment.");
        assert_eq!(store.pending_changes("work").unwrap().len(), 1);
        assert_eq!(store.accounts().unwrap().len(), 1);
        // A delete the database ignores keeps everything.
        let mut store = store_with_work();
        store.queue_change("work", "waiting", 1).unwrap();
        store.conn.execute_batch("CREATE TRIGGER keep_change BEFORE DELETE ON outbox BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.remove_account_discarding_changes("work", None, 1).is_err());
        assert_eq!(store.pending_changes("work").unwrap().len(), 1);
        assert_eq!(store.accounts().unwrap().len(), 1);
    }

    #[test]
    fn removing_an_account_removes_its_mail() {
        let mut store = store_with_work();
        ingest(&mut store, "1", &mail().build());
        assert!(store.remove_account("work").unwrap());
        assert!(!store.remove_account("work").unwrap());
        let messages: u32 =
            store.conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(messages, 0);
        assert!(store.search("budget", None, 10, 0).unwrap().is_empty());
    }

    #[test]
    fn mailboxes_have_optional_roles() {
        let mut store = store_with_work();
        store
            .upsert_mailbox(&Mailbox {
                account_id: "work".into(),
                name: "INBOX".into(),
                role: None,
            })
            .unwrap();
        let sent = Mailbox {
            account_id: "work".into(),
            name: "Sent".into(),
            role: Some(MailboxRole::Sent),
        };
        store.upsert_mailbox(&sent).unwrap();
        let inbox = Mailbox {
            account_id: "work".into(),
            name: "INBOX".into(),
            role: Some(MailboxRole::Inbox),
        };
        store.upsert_mailbox(&inbox).unwrap();
        assert_eq!(store.mailboxes("work").unwrap(), vec![inbox, sent]);
        store
            .upsert_mailbox(&Mailbox {
                account_id: "work".into(),
                name: "Notes".into(),
                role: None,
            })
            .unwrap();
        assert_eq!(store.mailboxes("work").unwrap()[1].role, None);
    }

    #[test]
    fn checkpoints_are_per_mailbox_or_account_wide() {
        let mut store = store_with_work();
        assert_eq!(store.checkpoint("work", Some("INBOX")).unwrap(), None);
        let imap = SyncCheckpoint {
            uid_validity: Some(7),
            uid_next: Some(120),
            highest_modseq: Some(u64::from(u32::MAX) + 5),
            cursor: None,
        };
        store.save_checkpoint("work", Some("INBOX"), &imap).unwrap();
        let resend = SyncCheckpoint { cursor: Some("em_123".into()), ..Default::default() };
        store.save_checkpoint("work", None, &resend).unwrap();
        assert_eq!(store.checkpoint("work", Some("INBOX")).unwrap(), Some(imap));
        assert_eq!(store.checkpoint("work", None).unwrap(), Some(resend));
        let later = SyncCheckpoint { uid_validity: Some(8), ..Default::default() };
        store.save_checkpoint("work", Some("INBOX"), &later).unwrap();
        assert_eq!(store.checkpoint("work", Some("INBOX")).unwrap(), Some(later));
    }

    #[test]
    fn knows_which_provider_messages_it_has_and_can_forget_their_ids() {
        let mut store = store_with_work();
        ingest(&mut store, "42", &mail().build());
        assert!(store.contains("work", "INBOX", "42").unwrap());
        assert!(!store.contains("work", "INBOX", "43").unwrap());
        assert_eq!(store.forget_mailbox_ids("work", "INBOX").unwrap(), 1);
        assert!(!store.contains("work", "INBOX", "42").unwrap());
        // The message itself is kept, and comes back under its new id.
        let again = ingest(&mut store, "1042", &mail().build());
        assert!(!again.created);
        assert!(store.contains("work", "INBOX", "1042").unwrap());
    }

    #[test]
    fn database_failures_are_errors() {
        let mut store = store_with_work();
        ingest(&mut store, "1", &mail().build());
        break_schema(&store, "ALTER TABLE message_locations RENAME COLUMN provider_id TO uid");
        assert!(store.contains("work", "INBOX", "1").is_err());
        break_schema(&store, "ALTER TABLE accounts RENAME COLUMN letter TO initial");
        assert!(store.accounts().is_err());
        read_only(&store);
        assert!(store.upsert_account(&account("lab", "lab@x.co")).is_err());
        assert!(store.reorder_accounts(&["work"]).is_err());
        assert!(store
            .upsert_mailbox(&Mailbox { account_id: "work".into(), name: "X".into(), role: None })
            .is_err());
        assert!(store.save_checkpoint("work", None, &SyncCheckpoint::default()).is_err());
        assert!(store.forget_mailbox_ids("work", "INBOX").is_err());
        assert!(store.remove_account("work").is_err());
    }
}
