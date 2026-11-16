//! Your signatures, which one each account (or address) adds, and the order you last used them in.

use std::collections::BTreeMap;

use moss_core::{AccountSignatures, Signature, SignatureSettings, SignatureUse};
use rusqlite::params;

use crate::codec::{json, to_json};
use crate::{Result, Store};

const LET_MOSS_CHOOSE: &str = "signatures:let_moss_choose";

impl Store {
    /// Every signature in list order, each account's (and address's) choice, and the recently
    /// used order.
    pub fn signatures(&self) -> Result<SignatureSettings> {
        let mut stmt =
            self.conn.prepare("SELECT id, name, lines_json FROM signatures ORDER BY position")?;
        let signatures = stmt
            .query_map([], |r| {
                Ok(Signature { id: r.get(0)?, name: r.get(1)?, lines: json(r, 2)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM signatures ORDER BY used IS NULL, used DESC, position")?;
        let recent = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut stmt =
            self.conn.prepare("SELECT account_id, new_emails, replies FROM account_signatures")?;
        let by_account: BTreeMap<String, AccountSignatures> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, AccountSignatures { new_emails: r.get(1)?, replies: r.get(2)? }))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT email, new_emails, replies FROM account_addresses WHERE own_signatures = 1",
        )?;
        let by_address: BTreeMap<String, AccountSignatures> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, AccountSignatures { new_emails: r.get(1)?, replies: r.get(2)? }))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let let_moss_choose = self.setting(LET_MOSS_CHOOSE)?.as_deref() != Some("off");
        Ok(SignatureSettings { signatures, by_account, by_address, let_moss_choose, recent })
    }

    /// Saves a signature's name and lines; a new one goes to the end of the list.
    pub fn save_signature(&mut self, s: &Signature) -> Result<()> {
        self.conn.execute(
            "INSERT INTO signatures (id, name, lines_json, position)
             VALUES (?1, ?2, ?3, (SELECT coalesce(max(position), 0) + 1 FROM signatures))
             ON CONFLICT (id) DO UPDATE SET name = ?2, lines_json = ?3",
            params![s.id, s.name, to_json(&s.lines)],
        )?;
        Ok(())
    }

    /// Deletes a signature; accounts that used it get none.
    pub fn delete_signature(&mut self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM signatures WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The signature an account adds to new emails or replies; `None` for none.
    pub fn set_signature_choice(
        &mut self,
        account_id: &str,
        kind: SignatureUse,
        signature_id: Option<&str>,
    ) -> Result<()> {
        let column = match kind {
            SignatureUse::NewEmails => "new_emails",
            SignatureUse::Replies => "replies",
        };
        self.conn.execute(
            &format!(
                "INSERT INTO account_signatures (account_id, {column}) VALUES (?1, ?2)
                 ON CONFLICT (account_id) DO UPDATE SET {column} = ?2"
            ),
            params![account_id, signature_id],
        )?;
        Ok(())
    }

    pub fn set_let_moss_choose(&mut self, on: bool) -> Result<()> {
        self.set_setting(LET_MOSS_CHOOSE, Some(if on { "on" } else { "off" }))
    }

    /// Moves a signature to the front of the recently used order.
    pub fn mark_signature_used(&mut self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE signatures SET used = (SELECT coalesce(max(used), 0) + 1 FROM signatures)
             WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use moss_core::{AccountSignatures, Signature, SignatureUse};

    use crate::test_support::*;
    use crate::Store;

    fn sig(id: &str, lines: &[&str]) -> Signature {
        Signature {
            id: id.into(),
            name: id.into(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    fn ids(store: &Store) -> Vec<String> {
        store.signatures().unwrap().signatures.into_iter().map(|s| s.id).collect()
    }

    #[test]
    fn a_new_database_has_no_signatures_and_lets_moss_choose() {
        let s = Store::open_in_memory().unwrap().signatures().unwrap();
        assert!(s.signatures.is_empty());
        assert!(s.by_account.is_empty());
        assert!(s.by_address.is_empty());
        assert!(s.recent.is_empty());
        assert!(s.let_moss_choose);
    }

    #[test]
    fn signatures_are_saved_in_order_and_edited_in_place() {
        let mut store = Store::open_in_memory().unwrap();
        store.save_signature(&sig("work", &["Maya Ross", "northwind.co"])).unwrap();
        store.save_signature(&sig("short", &["Maya"])).unwrap();
        let edited = Signature {
            name: "Work (full)".into(),
            ..sig("work", &["Maya Ross", "", "Design lead"])
        };
        store.save_signature(&edited).unwrap();
        let saved = store.signatures().unwrap().signatures;
        assert_eq!(saved, vec![edited, sig("short", &["Maya"])]);
    }

    #[test]
    fn they_survive_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        store.save_signature(&sig("work", &["Maya Ross"])).unwrap();
        store.set_signature_choice("work", SignatureUse::NewEmails, Some("work")).unwrap();
        store.set_let_moss_choose(false).unwrap();
        store.mark_signature_used("work").unwrap();
        let before = store.signatures().unwrap();
        drop(store);
        assert_eq!(Store::open(&path).unwrap().signatures().unwrap(), before);
    }

    #[test]
    fn each_account_chooses_for_new_emails_and_replies_apart() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya@fastmail.com")).unwrap();
        store.save_signature(&sig("full", &["Maya Ross"])).unwrap();
        store.save_signature(&sig("short", &["Maya"])).unwrap();
        store.set_signature_choice("work", SignatureUse::NewEmails, Some("full")).unwrap();
        store.set_signature_choice("work", SignatureUse::Replies, Some("short")).unwrap();
        store.set_signature_choice("personal", SignatureUse::Replies, Some("short")).unwrap();
        store.set_signature_choice("personal", SignatureUse::Replies, None).unwrap();
        let by = store.signatures().unwrap().by_account;
        assert_eq!(
            by["work"],
            AccountSignatures { new_emails: Some("full".into()), replies: Some("short".into()) }
        );
        assert_eq!(by["personal"], AccountSignatures::default());
    }

    #[test]
    fn deleting_a_signature_leaves_its_accounts_with_none() {
        let mut store = store_with_work();
        store.save_signature(&sig("full", &["Maya Ross"])).unwrap();
        store.save_signature(&sig("short", &["Maya"])).unwrap();
        store.set_signature_choice("work", SignatureUse::NewEmails, Some("full")).unwrap();
        store.set_signature_choice("work", SignatureUse::Replies, Some("short")).unwrap();
        store.mark_signature_used("full").unwrap();
        store.delete_signature("full").unwrap();
        store.delete_signature("never-there").unwrap();
        let s = store.signatures().unwrap();
        assert_eq!(ids(&store), ["short"]);
        assert_eq!(
            s.by_account["work"],
            AccountSignatures { new_emails: None, replies: Some("short".into()) }
        );
        assert_eq!(s.recent, ["short"]);
    }

    #[test]
    fn removing_an_account_forgets_its_choice() {
        let mut store = store_with_work();
        store.save_signature(&sig("full", &["Maya Ross"])).unwrap();
        store.set_signature_choice("work", SignatureUse::NewEmails, Some("full")).unwrap();
        store.remove_account("work").unwrap();
        let s = store.signatures().unwrap();
        assert!(s.by_account.is_empty());
        assert_eq!(ids(&store), ["full"]);
    }

    #[test]
    fn a_choice_must_name_a_real_account_and_signature() {
        let mut store = store_with_work();
        store.save_signature(&sig("full", &["Maya Ross"])).unwrap();
        assert!(store
            .set_signature_choice("nobody", SignatureUse::NewEmails, Some("full"))
            .is_err());
        assert!(store.set_signature_choice("work", SignatureUse::NewEmails, Some("gone")).is_err());
    }

    #[test]
    fn let_moss_choose_can_be_turned_off_and_on() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_let_moss_choose(false).unwrap();
        assert!(!store.signatures().unwrap().let_moss_choose);
        store.set_let_moss_choose(true).unwrap();
        assert!(store.signatures().unwrap().let_moss_choose);
    }

    #[test]
    fn recent_puts_the_last_used_first_then_the_rest_in_list_order() {
        let mut store = Store::open_in_memory().unwrap();
        for id in ["a", "b", "c", "d"] {
            store.save_signature(&sig(id, &[])).unwrap();
        }
        assert_eq!(store.signatures().unwrap().recent, ["a", "b", "c", "d"]);
        store.mark_signature_used("c").unwrap();
        store.mark_signature_used("b").unwrap();
        store.mark_signature_used("c").unwrap();
        store.mark_signature_used("never-there").unwrap();
        assert_eq!(store.signatures().unwrap().recent, ["c", "b", "a", "d"]);
        // Editing doesn't count as using.
        store.save_signature(&sig("b", &["B"])).unwrap();
        assert_eq!(store.signatures().unwrap().recent, ["c", "b", "a", "d"]);
    }

    #[test]
    fn failures_are_errors() {
        let mut store = store_with_work();
        break_schema(&store, "DROP TABLE account_signatures");
        assert!(store.signatures().is_err());
        assert!(store.set_signature_choice("work", SignatureUse::Replies, None).is_err());
        let mut store = store_with_work();
        break_schema(&store, "DROP TABLE settings");
        assert!(store.signatures().is_err());
        assert!(store.set_let_moss_choose(true).is_err());
        let mut store = store_with_work();
        break_schema(&store, "DROP TABLE account_signatures; DROP TABLE signatures");
        assert!(store.signatures().is_err());
        assert!(store.save_signature(&sig("a", &[])).is_err());
        assert!(store.delete_signature("a").is_err());
        assert!(store.mark_signature_used("a").is_err());
    }
}
