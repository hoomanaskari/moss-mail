//! The other addresses an account sends from, such as hi@rossstudio.art on an iCloud
//! login (see `moss_core::AccountAddress`).
//!
//! Mail from an added address is yours, like mail from the account's own address. Moss
//! suggests addresses that mail came to (the engine finds them), and blocking one sends
//! mail to it to Blocked, for catch-all addresses that leaked to spammers.

use std::collections::HashSet;

use moss_core::{AccountAddress, AccountSignatures};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::classify::{domain_of, is_public_domain};
use crate::codec::{named, parse_id};
use crate::senders::{block_caught, unblock_lifted};
use crate::{Error, Result, Store};

/// SQL: the lowercase To and Cc addresses of message `m`.
pub(crate) const RECIPIENTS: &str =
    "SELECT lower(json_extract(j.value, '$.email')) FROM json_each(m.to_json) j
    UNION ALL SELECT lower(json_extract(j.value, '$.email')) FROM json_each(m.cc_json) j";

/// SQL: the next place in account `?1`'s list.
const NEXT: &str =
    "(SELECT coalesce(max(position) + 1, 0) FROM account_addresses WHERE account_id = ?1)";

const COLUMNS: &str = "email, name, state, source, seen, blocked";

fn address(r: &Row) -> rusqlite::Result<AccountAddress> {
    Ok(AccountAddress {
        email: r.get(0)?,
        name: r.get(1)?,
        state: named(r, 2)?,
        source: named(r, 3)?,
        seen: r.get(4)?,
        blocked: r.get(5)?,
    })
}

/// An address as typed, trimmed and lowercase, if it is one.
fn checked(email: &str) -> Result<String> {
    let email = email.trim().to_lowercase();
    let odd = |c: char| c.is_whitespace() || "<>,;\"()[]".contains(c);
    let valid = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !email.contains(odd)
    });
    if valid {
        Ok(email)
    } else {
        Err(Error::InvalidAddress(email))
    }
}

/// The account's own address, lowercase.
fn primary(conn: &Connection, account_id: &str) -> Result<String> {
    conn.query_row(
        "SELECT lower(address) FROM accounts WHERE id = ?1",
        [account_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| Error::UnknownAccount(account_id.into()))
}

/// An address an account can have besides its own: a real one, and not its own.
fn other_address(conn: &Connection, account_id: &str, email: &str) -> Result<String> {
    let email = checked(email)?;
    if primary(conn, account_id)? == email {
        return Err(Error::PrimaryAddress(email));
    }
    Ok(email)
}

/// A name for mail from an address: `None` when blank, for the account's own.
fn name_of(name: Option<&str>) -> Option<&str> {
    name.map(str::trim).filter(|n| !n.is_empty())
}

/// Mail already here from `email` is yours: read and done, as mail you send is stored.
fn mark_yours(conn: &Connection, account_id: &str, email: &str) -> Result<()> {
    conn.execute(
        "UPDATE messages SET from_me = 1, read = 1, done = 1
         WHERE account_id = ?1 AND lower(from_email) = ?2 AND from_me = 0",
        params![account_id, email],
    )?;
    Ok(())
}

/// An error when a change found no such address.
fn found(changed: usize, email: String) -> Result<()> {
    if changed == 0 {
        return Err(Error::NotFound {
            kind: "address",
            id: email,
        });
    }
    Ok(())
}

/// Whether mail from `email` is yours because it is an address you added to the account.
pub(crate) fn is_added(conn: &Connection, account_id: &str, email: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM account_addresses
             WHERE account_id = ?1 AND email = ?2 AND state = 'added')",
        params![account_id, email.trim().to_lowercase()],
        |r| r.get(0),
    )?)
}

/// What Moss knows of an account's addresses: its own and every address row, and the
/// domains of the rows. Left out: public domains like icloud.com, and the domain of the
/// account's own address, where other people work too (an organisation's domain).
pub(crate) struct Yours {
    primary: String,
    addresses: Vec<String>,
    domains: Vec<String>,
}

pub(crate) fn yours(conn: &Connection, account_id: &str) -> Result<Yours> {
    let primary = primary(conn, account_id)?;
    let mut stmt = conn
        .prepare("SELECT email FROM account_addresses WHERE account_id = ?1 ORDER BY position")?;
    let rows = stmt.query_map([account_id], |r| r.get::<_, String>(0))?;
    let mut addresses = vec![primary.clone()];
    addresses.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    let own = domain_of(&primary);
    let mut domains: Vec<String> = Vec::new();
    for domain in addresses.iter().skip(1).filter_map(|a| domain_of(a)) {
        if !is_public_domain(&domain) && Some(&domain) != own.as_ref() && !domains.contains(&domain) {
            domains.push(domain);
        }
    }
    Ok(Yours {
        primary,
        addresses,
        domains,
    })
}

impl Yours {
    fn knows(&self, email: &str) -> bool {
        self.addresses.iter().any(|a| a == email)
    }

    fn at_own_domain(&self, email: &str) -> bool {
        domain_of(email).is_some_and(|d| self.domains.contains(&d))
    }

    /// Which of yours mail to `recipients` (lowercase, To then Cc) came to: the first one
    /// Moss knows, else the first at one of your domains.
    pub(crate) fn came_to(&self, recipients: &[String]) -> Option<String> {
        let known = recipients.iter().find(|r| self.knows(r));
        known
            .or_else(|| recipients.iter().find(|r| self.at_own_domain(r)))
            .cloned()
    }
}

/// The To and Cc addresses (lowercase) of each email others sent to this account and you
/// didn't delete, oldest stored first.
fn received(conn: &Connection, account_id: &str) -> Result<Vec<Vec<String>>> {
    let mut stmt = conn.prepare(
        "SELECT id, email FROM (
             SELECT m.id, 0 AS part, j.key AS n, lower(json_extract(j.value, '$.email')) AS email
             FROM messages m, json_each(m.to_json) j
             WHERE m.account_id = ?1 AND m.from_me = 0 AND m.deleted_from IS NULL
             UNION ALL
             SELECT m.id, 1, j.key, lower(json_extract(j.value, '$.email'))
             FROM messages m, json_each(m.cc_json) j
             WHERE m.account_id = ?1 AND m.from_me = 0 AND m.deleted_from IS NULL
         ) ORDER BY id, part, n",
    )?;
    let rows = stmt.query_map([account_id], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut all: Vec<(i64, Vec<String>)> = Vec::new();
    for row in rows {
        let (id, email) = row?;
        match all.last_mut() {
            Some((last, recipients)) if *last == id => recipients.push(email),
            _ => all.push((id, vec![email])),
        }
    }
    Ok(all.into_iter().map(|(_, recipients)| recipients).collect())
}

impl Store {
    /// The addresses Settings shows for an account, in list order: added and suggested
    /// ones, and dismissed ones only while they are blocked.
    pub fn addresses(&self, account_id: &str) -> Result<Vec<AccountAddress>> {
        self.address_rows(account_id, "AND (state <> 'dismissed' OR blocked = 1)")
    }

    /// Every address row of an account, dismissed ones too, in list order.
    pub fn every_address(&self, account_id: &str) -> Result<Vec<AccountAddress>> {
        self.address_rows(account_id, "")
    }

    fn address_rows(&self, account_id: &str, filter: &str) -> Result<Vec<AccountAddress>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM account_addresses WHERE account_id = ?1 {filter} ORDER BY position"
        ))?;
        let rows = stmt.query_map([account_id], address)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One address of an account, whatever its state.
    pub fn address(&self, account_id: &str, email: &str) -> Result<Option<AccountAddress>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM account_addresses WHERE account_id = ?1 AND email = ?2"
                ),
                params![account_id, email.trim().to_lowercase()],
                address,
            )
            .optional()?)
    }

    /// You send from `email` too: it is added (again, if you had removed it or Moss had
    /// suggested it), and the mail here from it becomes yours. `name` replaces its sender
    /// name when given.
    pub fn add_address(&mut self, account_id: &str, email: &str, name: Option<&str>) -> Result<()> {
        let tx = self.conn.transaction()?;
        let email = other_address(&tx, account_id, email)?;
        tx.execute(
            &format!(
                "INSERT INTO account_addresses (account_id, email, name, state, source, position)
                 VALUES (?1, ?2, ?3, 'added', 'you', {NEXT})
                 ON CONFLICT (account_id, email) DO UPDATE SET state = 'added', name = coalesce(?3, name)"
            ),
            params![account_id, email, name_of(name)],
        )?;
        mark_yours(&tx, account_id, &email)?;
        tx.commit()?;
        Ok(())
    }

    /// The sender name for mail from an address; blank for the account's own.
    pub fn update_address(
        &mut self,
        account_id: &str,
        email: &str,
        name: Option<&str>,
    ) -> Result<()> {
        let email = email.trim().to_lowercase();
        let changed = self.conn.execute(
            "UPDATE account_addresses SET name = ?3 WHERE account_id = ?1 AND email = ?2",
            params![account_id, email, name_of(name)],
        )?;
        found(changed, email)
    }

    /// Removes an address you added, or says no to a suggestion. It is kept, dismissed, so
    /// Moss doesn't suggest it again, and its own signatures are forgotten.
    pub fn remove_address(&mut self, account_id: &str, email: &str) -> Result<()> {
        let email = email.trim().to_lowercase();
        let changed = self.conn.execute(
            "UPDATE account_addresses
             SET state = 'dismissed', own_signatures = 0, new_emails = NULL, replies = NULL
             WHERE account_id = ?1 AND email = ?2",
            params![account_id, email],
        )?;
        found(changed, email)
    }

    /// Blocks (or unblocks) mail sent to `email`, an address of this account that leaked
    /// to spammers: it goes to Blocked like mail from a blocked sender. An address Moss had
    /// no row for gets one, dismissed. Unblocking only lifts the block.
    pub fn block_address(&mut self, account_id: &str, email: &str, blocked: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        let email = other_address(&tx, account_id, email)?;
        if blocked {
            tx.execute(
                &format!(
                    "INSERT INTO account_addresses (account_id, email, state, source, blocked, position)
                     VALUES (?1, ?2, 'dismissed', 'mail', 1, {NEXT})
                     ON CONFLICT (account_id, email) DO UPDATE SET blocked = 1"
                ),
                params![account_id, email],
            )?;
            tx.execute(&block_caught(), [account_id])?;
        } else {
            tx.execute(
                "UPDATE account_addresses SET blocked = 0 WHERE account_id = ?1 AND email = ?2",
                params![account_id, email],
            )?;
            tx.execute(&unblock_lifted(), [account_id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Lifts the address blocks a conversation was caught by, so it comes back when its
    /// sender is unblocked too.
    pub fn unblock_addresses_in(&mut self, thread_id: &str) -> Result<()> {
        let id = parse_id(thread_id);
        let tx = self.conn.transaction()?;
        tx.execute(
            &format!(
                "UPDATE account_addresses AS a SET blocked = 0 WHERE a.blocked = 1 AND EXISTS (
                     SELECT 1 FROM messages m WHERE m.thread_id = ?1 AND m.account_id = a.account_id
                         AND a.email IN ({RECIPIENTS}))"
            ),
            [id],
        )?;
        let account: Option<String> = tx
            .query_row("SELECT account_id FROM threads WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(account) = account {
            tx.execute(&unblock_lifted(), [account])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The signatures mail from an address adds; `None` uses its account's again.
    pub fn set_address_signatures(
        &mut self,
        account_id: &str,
        email: &str,
        choice: Option<&AccountSignatures>,
    ) -> Result<()> {
        let email = email.trim().to_lowercase();
        let (own, new_emails, replies) = match choice {
            Some(c) => (true, c.new_emails.as_deref(), c.replies.as_deref()),
            None => (false, None, None),
        };
        let changed = self.conn.execute(
            "UPDATE account_addresses SET own_signatures = ?3, new_emails = ?4, replies = ?5
             WHERE account_id = ?1 AND email = ?2",
            params![account_id, email, own, new_emails, replies],
        )?;
        found(changed, email)
    }

    /// An address from Gmail's "Send mail as" list: added, unless you removed it. A name
    /// you gave it stays.
    pub fn add_gmail_address(
        &mut self,
        account_id: &str,
        email: &str,
        name: Option<&str>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let email = other_address(&tx, account_id, email)?;
        tx.execute(
            &format!(
                "INSERT INTO account_addresses (account_id, email, name, state, source, position)
                 VALUES (?1, ?2, ?3, 'added', 'gmail', {NEXT})
                 ON CONFLICT (account_id, email) DO UPDATE SET state = 'added', name = coalesce(name, ?3),
                     source = CASE source WHEN 'mail' THEN 'gmail' ELSE source END
                 WHERE state <> 'dismissed'"
            ),
            params![account_id, email, name_of(name)],
        )?;
        if is_added(&tx, account_id, &email)? {
            mark_yours(&tx, account_id, &email)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Domains to ask the DNS about: those of mail that came to none of the account's
    /// known addresses, leaving out public ones and its own. Most mail first.
    pub fn address_domains_to_check(&self, account_id: &str) -> Result<Vec<String>> {
        let yours = yours(&self.conn, account_id)?;
        let mut counts: Vec<(String, usize)> = Vec::new();
        for recipients in received(&self.conn, account_id)? {
            if recipients.iter().any(|r| yours.knows(r)) {
                continue;
            }
            let domains: HashSet<String> = recipients.iter().filter_map(|r| domain_of(r)).collect();
            let mut domains: Vec<String> = domains.into_iter().collect();
            domains.sort();
            for domain in domains {
                if is_public_domain(&domain) || yours.domains.contains(&domain) {
                    continue;
                }
                match counts.iter_mut().find(|(d, _)| *d == domain) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((domain, 1)),
                }
            }
        }
        counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        Ok(counts.into_iter().map(|(d, _)| d).collect())
    }

    /// Counts the mail that came to each address of the account, and suggests the ones mail
    /// came to at its own domains (and at `also`, domains its mail service hosts) when it
    /// came to no address Moss knows. Public domains never.
    pub fn scan_addresses(&mut self, account_id: &str, also: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        let yours = yours(&tx, account_id)?;
        let candidate = |email: &str| {
            domain_of(email).is_some_and(|d| {
                !is_public_domain(&d) && (yours.domains.contains(&d) || also.contains(&d))
            })
        };
        let mut seen: Vec<(String, u32)> = Vec::new();
        for recipients in received(&tx, account_id)? {
            let known = recipients.iter().any(|r| yours.knows(r));
            let mut counted = HashSet::new();
            for r in recipients {
                let counts = if yours.knows(&r) {
                    r != yours.primary
                } else {
                    !known && candidate(&r)
                };
                if !counts || !counted.insert(r.clone()) {
                    continue;
                }
                match seen.iter_mut().find(|(e, _)| *e == r) {
                    Some((_, n)) => *n += 1,
                    None => seen.push((r, 1)),
                }
            }
        }
        let count = |email: &str| seen.iter().find(|(e, _)| e == email).map_or(0, |(_, n)| *n);
        for email in yours.addresses.iter().filter(|a| **a != yours.primary) {
            tx.execute(
                "UPDATE account_addresses SET seen = ?3 WHERE account_id = ?1 AND email = ?2 AND seen <> ?3",
                params![account_id, email, count(email)],
            )?;
        }
        for (email, n) in seen.iter().filter(|(e, _)| !yours.knows(e)) {
            tx.execute(
                &format!(
                    "INSERT INTO account_addresses (account_id, email, state, source, seen, position)
                     VALUES (?1, ?2, 'suggested', 'mail', ?3, {NEXT})"
                ),
                params![account_id, email, n],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::{Error, BLOCKED_FOLDER};
    use moss_core::{
        AddressSource, AddressState, MessageSource, SenderStatus, Signature, SplitId, ThreadSummary,
    };

    const HI: &str = "hi@rossstudio.art";
    const LEAK: &str = "bookshop@rossstudio.art";

    /// An iCloud login with a custom domain, like the one this is for.
    fn icloud() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store
            .upsert_account(&account("work", "maya@icloud.com"))
            .unwrap();
        store
    }

    fn row(email: &str, state: AddressState, source: AddressSource) -> AccountAddress {
        AccountAddress {
            email: email.into(),
            name: None,
            state,
            source,
            seen: 0,
            blocked: false,
        }
    }

    fn emails(list: &[AccountAddress]) -> Vec<&str> {
        list.iter().map(|a| a.email.as_str()).collect()
    }

    fn to(store: &mut Store, id: &str, recipients: &str) -> String {
        let raw = mail()
            .id(&format!("{id}@x"))
            .subject(id)
            .to(recipients)
            .build();
        ingest(store, id, &raw).thread_id
    }

    fn summary(store: &Store, thread: &str) -> ThreadSummary {
        store.thread(thread, NOV_1).unwrap().unwrap().summary
    }

    #[test]
    fn a_person_writing_to_any_of_your_addresses_needs_your_reply() {
        use moss_core::Section;
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        // One you send from, and one at your own domain you haven't added (a catch-all).
        let added = to(&mut store, "a", HI);
        let catch_all = to(&mut store, "b", LEAK);
        let elsewhere = to(&mut store, "c", "team@northwind.co");
        assert_eq!(summary(&store, &added).section, Section::Reply);
        assert_eq!(summary(&store, &catch_all).section, Section::Reply);
        assert_eq!(summary(&store, &elsewhere).section, Section::Info);
    }

    fn inbox(store: &Store) -> Vec<String> {
        store
            .list_threads(None, SplitId::Important, NOV_1)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    }

    fn blocked(store: &Store) -> Vec<String> {
        store
            .list_blocked(None, 50, NOV_1)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    }

    /// Mail you sent, as your Sent folder has it.
    fn in_sent(store: &mut Store, provider_id: &str, raw: &[u8]) -> crate::Ingested {
        store.upsert_mailbox(&moss_core::Mailbox { account_id: "work".into(), name: "Sent".into(), role: Some(moss_core::MailboxRole::Sent) }).unwrap();
        store.ingest(raw, &moss_core::MessageSource { mailbox: "Sent".into(), ..source(provider_id) }).unwrap()
    }

    fn from_me(store: &Store, thread: &str) -> Vec<(bool, bool, bool)> {
        let mut stmt = store
            .conn
            .prepare("SELECT from_me, read, done FROM messages WHERE thread_id = ?1 ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([thread], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn added_addresses_are_kept_lowercase_in_the_order_they_came() {
        let mut store = icloud();
        store
            .add_address("work", "  Hi@RossStudio.art ", Some("Ross Studio"))
            .unwrap();
        store
            .add_address("work", "shop@rossstudio.art", None)
            .unwrap();
        let hi = AccountAddress {
            name: Some("Ross Studio".into()),
            ..row(HI, AddressState::Added, AddressSource::You)
        };
        let shop = row(
            "shop@rossstudio.art",
            AddressState::Added,
            AddressSource::You,
        );
        assert_eq!(store.addresses("work").unwrap(), [hi.clone(), shop]);
        assert_eq!(
            store.address("work", "HI@rossstudio.art").unwrap(),
            Some(hi)
        );
        assert_eq!(
            store.address("work", "nobody@rossstudio.art").unwrap(),
            None
        );
        assert!(store.addresses("nobody").unwrap().is_empty());
    }

    #[test]
    fn only_a_real_address_other_than_the_accounts_own_can_be_added() {
        let mut store = icloud();
        for bad in [
            "nope",
            "a@b",
            "@rossstudio.art",
            "a b@rossstudio.art",
            "<a@x.co>",
            "a@b@c.co",
            "a@.co",
            "a@co.",
        ] {
            let err = store.add_address("work", bad, None).unwrap_err();
            assert!(matches!(err, Error::InvalidAddress(_)), "{bad}: {err}");
        }
        assert_eq!(
            store
                .add_address("work", "nope", None)
                .unwrap_err()
                .to_string(),
            "nope isn't an email address"
        );
        let err = store
            .add_address("work", "Maya@iCloud.com", None)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "maya@icloud.com is this account's own address"
        );
        assert!(matches!(
            store.block_address("work", "maya@icloud.com", true),
            Err(Error::PrimaryAddress(_))
        ));
        assert!(matches!(
            store.add_address("nobody", HI, None),
            Err(Error::UnknownAccount(_))
        ));
        assert!(store.addresses("work").unwrap().is_empty());
    }

    #[test]
    fn adding_a_suggested_or_dismissed_address_adds_it_and_keeps_where_it_came_from() {
        let mut store = icloud();
        to(&mut store, "a", HI);
        store
            .scan_addresses("work", &["rossstudio.art".into()])
            .unwrap();
        let suggested = AccountAddress {
            seen: 1,
            ..row(HI, AddressState::Suggested, AddressSource::Mail)
        };
        assert_eq!(store.addresses("work").unwrap(), [suggested.clone()]);
        store.add_address("work", HI, Some("Studio")).unwrap();
        let added = AccountAddress {
            state: AddressState::Added,
            name: Some("Studio".into()),
            ..suggested
        };
        assert_eq!(store.addresses("work").unwrap(), [added.clone()]);
        store.remove_address("work", HI).unwrap();
        assert!(store.addresses("work").unwrap().is_empty());
        assert_eq!(
            store.address("work", HI).unwrap().unwrap().state,
            AddressState::Dismissed
        );
        // Its name stays unless a new one is given.
        store.add_address("work", HI, None).unwrap();
        assert_eq!(store.addresses("work").unwrap(), [added]);
    }

    #[test]
    fn an_address_can_be_renamed_and_an_empty_name_means_the_accounts_own() {
        let mut store = icloud();
        store.add_address("work", HI, Some("Studio")).unwrap();
        store
            .update_address("work", "HI@rossstudio.art", Some(" Ross Studio "))
            .unwrap();
        assert_eq!(
            store.addresses("work").unwrap()[0].name.as_deref(),
            Some("Ross Studio")
        );
        store.update_address("work", HI, Some("  ")).unwrap();
        assert_eq!(store.addresses("work").unwrap()[0].name, None);
        store.update_address("work", HI, Some("Ross")).unwrap();
        store.update_address("work", HI, None).unwrap();
        assert_eq!(store.addresses("work").unwrap()[0].name, None);
        let err = store
            .update_address("work", "x@rossstudio.art", None)
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::NotFound {
                    kind: "address",
                    ..
                }
            ),
            "{err}"
        );
        assert!(matches!(
            store.remove_address("work", "x@rossstudio.art"),
            Err(Error::NotFound { .. })
        ));
    }

    #[test]
    fn mail_from_an_added_address_is_yours_before_and_after_adding_it() {
        let mut store = icloud();
        let raw = mail()
            .id("old@x")
            .from(&format!("Maya <{HI}>"))
            .to("dana@acme.co")
            .build();
        let old = ingest(&mut store, "1", &raw).thread_id;
        assert_eq!(from_me(&store, &old), [(false, false, false)]);
        assert_eq!(inbox(&store), [old.clone()]);
        // Only an address you added: a suggestion isn't yours yet.
        store
            .scan_addresses("work", &["rossstudio.art".into()])
            .unwrap();
        store
            .add_address("work", "shop@rossstudio.art", None)
            .unwrap();
        assert_eq!(from_me(&store, &old), [(false, false, false)]);

        store
            .add_address("work", "Hi@RossStudio.art", None)
            .unwrap();
        assert_eq!(from_me(&store, &old), [(true, true, true)]);
        assert!(inbox(&store).is_empty());
        let raw = mail()
            .id("new@x")
            .from(&format!("Maya <{}>", HI.to_uppercase()))
            .to("dana@acme.co")
            .build();
        let new = in_sent(&mut store, "2", &raw).thread_id;
        assert_eq!(from_me(&store, &new), [(true, true, true)]);
        // Only you wrote, so it shows who it went to.
        assert_eq!(summary(&store, &new).from.email, "dana@acme.co");
        // Mail from an address you removed comes in as anyone's.
        store.remove_address("work", HI).unwrap();
        let raw = mail().id("later@x").from(HI).to("dana@acme.co").build();
        let later = ingest(&mut store, "3", &raw).thread_id;
        assert_eq!(from_me(&store, &later), [(false, false, false)]);
    }

    #[test]
    fn a_conversation_says_which_of_your_addresses_it_came_to() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        store.block_address("work", LEAK, true).unwrap();
        store.block_address("work", LEAK, false).unwrap();
        let own = to(&mut store, "a", "Maya <Maya@iCloud.com>");
        assert_eq!(
            summary(&store, &own).to_address.as_deref(),
            Some("maya@icloud.com")
        );
        // To first, then Cc; a known address wins over another at your domain.
        let raw = mail()
            .id("b@x")
            .subject("b")
            .to("Dana <dana@acme.co>, info@rossstudio.art")
            .header("Cc", "Hi@RossStudio.art")
            .build();
        let cc = ingest(&mut store, "b", &raw).thread_id;
        assert_eq!(summary(&store, &cc).to_address.as_deref(), Some(HI));
        let leak = to(&mut store, "c", "Bookshop@rossstudio.art");
        assert_eq!(summary(&store, &leak).to_address.as_deref(), Some(LEAK));
        // Anyone at one of your domains, when none is an address Moss knows.
        let other = to(&mut store, "d", "dana@acme.co, Info@RossStudio.art");
        assert_eq!(
            summary(&store, &other).to_address.as_deref(),
            Some("info@rossstudio.art")
        );
        // Never at a public domain like icloud.com.
        let public = to(&mut store, "e", "someone@icloud.com");
        assert_eq!(summary(&store, &public).to_address, None);
        let bcc = ingest(
            &mut store,
            "f",
            &mail().id("f@x").subject("f").without("To").build(),
        )
        .thread_id;
        assert_eq!(summary(&store, &bcc).to_address, None);
        // Only you wrote: it came to no one of yours.
        let raw = mail()
            .id("g@x")
            .subject("g")
            .from(HI)
            .to("maya@icloud.com")
            .build();
        let mine = ingest(&mut store, "g", &raw).thread_id;
        assert_eq!(summary(&store, &mine).to_address, None);
        // It is the latest email from someone else that counts.
        let raw = mail()
            .id("h@x")
            .subject("b")
            .reply_to(&["b@x"])
            .to("maya@icloud.com")
            .build();
        assert_eq!(ingest(&mut store, "h", &raw).thread_id, cc);
        assert_eq!(
            summary(&store, &cc).to_address.as_deref(),
            Some("maya@icloud.com")
        );
    }

    #[test]
    fn every_list_says_which_address_mail_came_to() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        let thread = to(&mut store, "a", HI);
        let want = Some(HI.to_string());
        assert_eq!(
            store.list_threads(None, SplitId::Important, NOV_1).unwrap()[0].to_address,
            want
        );
        assert_eq!(
            store.search("budget", None, 10, NOV_1).unwrap()[0]
                .thread
                .to_address,
            want
        );
        store.mark_done(&thread, true, NOV_1).unwrap();
        assert_eq!(
            store.list_done(None, 10, NOV_1).unwrap()[0].to_address,
            want
        );
        store.block_address("work", HI, true).unwrap();
        assert_eq!(
            store.list_blocked(None, 10, NOV_1).unwrap()[0].to_address,
            want
        );
        // The domain of the account's own address isn't a catch-all: colleagues work there.
        let mut work = store_with_work();
        work.add_address("work", "design@northwind.co", None).unwrap();
        let team = to(&mut work, "t", "team@northwind.co");
        assert_eq!(summary(&work, &team).to_address, None);
    }

    #[test]
    fn blocking_an_address_blocks_mail_sent_to_it_now_and_later() {
        let mut store = icloud();
        let first = to(&mut store, "a", LEAK);
        let other = to(&mut store, "b", HI);
        let mine = mail()
            .id("m@x")
            .subject("m")
            .from("maya@icloud.com")
            .to(LEAK)
            .build();
        let sent = in_sent(&mut store, "m", &mine).thread_id;
        store
            .block_address("work", "Bookshop@RossStudio.art", true)
            .unwrap();
        // It had no row: it gets one, dismissed but shown while blocked.
        let leak = AccountAddress {
            blocked: true,
            ..row(LEAK, AddressState::Dismissed, AddressSource::Mail)
        };
        assert_eq!(store.addresses("work").unwrap(), [leak]);
        assert_eq!(blocked(&store), [first.clone()]);
        assert_eq!(inbox(&store), [other.clone()]);
        assert_eq!(
            store.thread(&first, NOV_1).unwrap().unwrap().summary.sender,
            SenderStatus::Blocked
        );
        assert_eq!(from_me(&store, &sent), [(true, true, true)]);
        // Later mail to it, in To or Cc, goes straight to Blocked.
        let raw = mail()
            .id("c@x")
            .subject("c")
            .to("dana@acme.co")
            .header("Cc", LEAK)
            .build();
        let later = ingest(&mut store, "c", &raw).thread_id;
        assert_eq!(blocked(&store), [later.clone(), first.clone()]);
        assert_eq!(store.blocked_in("work", "INBOX").unwrap(), ["a", "c"]);

        // Unblocking only clears the flag: the address stays dismissed.
        store.block_address("work", LEAK, false).unwrap();
        assert!(store.addresses("work").unwrap().is_empty());
        assert_eq!(
            store.address("work", LEAK).unwrap().unwrap().state,
            AddressState::Dismissed
        );
        assert!(blocked(&store).is_empty());
        assert_eq!(inbox(&store), [later, other, first]);
    }

    #[test]
    fn blocking_an_address_you_added_keeps_it_added() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        store.block_address("work", HI, true).unwrap();
        let hi = store.addresses("work").unwrap();
        assert_eq!((hi[0].state, hi[0].blocked), (AddressState::Added, true));
    }

    #[test]
    fn a_sender_block_and_an_address_block_each_hold_on_their_own() {
        let mut store = icloud();
        let raw = mail()
            .id("a@x")
            .from("Jordan <jordan@growthleads.example>")
            .to(LEAK)
            .build();
        let jordan = ingest(&mut store, "a", &raw).thread_id;
        let dana = to(&mut store, "b", LEAK);
        store
            .block_sender("work", "jordan@growthleads.example", false, NOV_1)
            .unwrap();
        store.block_address("work", LEAK, true).unwrap();
        store.block_address("work", LEAK, false).unwrap();
        assert_eq!(blocked(&store), [jordan.clone()]);
        assert_eq!(inbox(&store), [dana.clone()]);
        store.block_address("work", LEAK, true).unwrap();
        store
            .unblock_sender("work", "jordan@growthleads.example", NOV_1)
            .unwrap();
        assert_eq!(blocked(&store), [dana, jordan]);
    }

    #[test]
    fn mail_in_the_blocked_folder_for_a_blocked_address_never_blocks_its_sender() {
        let mut store = icloud();
        store.block_address("work", LEAK, true).unwrap();
        let raw = mail()
            .id("a@x")
            .from("Shop <shop@bigshop.example>")
            .to(LEAK)
            .build();
        let at = MessageSource {
            mailbox: BLOCKED_FOLDER.into(),
            ..source("9")
        };
        let thread = store.ingest(&raw, &at).unwrap().thread_id;
        assert_eq!(blocked(&store), [thread.clone()]);
        assert_eq!(
            store.sender_status("work", "shop@bigshop.example").unwrap(),
            SenderStatus::New
        );
        store.block_address("work", LEAK, false).unwrap();
        assert!(blocked(&store).is_empty());
        assert_eq!(store.unblocked_in("work", BLOCKED_FOLDER).unwrap(), ["9"]);
        // Mail there for no blocked address still blocks its sender.
        let raw = mail()
            .id("b@x")
            .from("Shop <shop@bigshop.example>")
            .to(HI)
            .build();
        let at = MessageSource {
            mailbox: BLOCKED_FOLDER.into(),
            ..source("10")
        };
        store.ingest(&raw, &at).unwrap();
        assert_eq!(
            store.sender_status("work", "shop@bigshop.example").unwrap(),
            SenderStatus::Blocked
        );
    }

    #[test]
    fn unblocking_a_conversation_lifts_the_address_blocks_it_was_caught_by() {
        let mut store = icloud();
        let leak = to(&mut store, "a", LEAK);
        let raw = mail().id("b@x").subject("b").to("x@rossstudio.art").build();
        let other = ingest(&mut store, "b", &raw).thread_id;
        store.block_address("work", LEAK, true).unwrap();
        store
            .block_address("work", "x@rossstudio.art", true)
            .unwrap();
        store.unblock_addresses_in(&leak).unwrap();
        assert_eq!(blocked(&store), [other]);
        assert!(!store.address("work", LEAK).unwrap().unwrap().blocked);
        assert!(
            store
                .address("work", "x@rossstudio.art")
                .unwrap()
                .unwrap()
                .blocked
        );
        store.unblock_addresses_in("999").unwrap();
    }

    #[test]
    fn an_address_can_have_its_own_signatures() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        store
            .add_address("work", "shop@rossstudio.art", None)
            .unwrap();
        for id in ["studio", "short"] {
            store
                .save_signature(&Signature {
                    id: id.into(),
                    name: id.into(),
                    lines: vec![],
                })
                .unwrap();
        }
        let studio = AccountSignatures {
            new_emails: Some("studio".into()),
            replies: Some("short".into()),
        };
        store
            .set_address_signatures("work", "Hi@RossStudio.art", Some(&studio))
            .unwrap();
        let none = AccountSignatures::default();
        store
            .set_address_signatures("work", "shop@rossstudio.art", Some(&none))
            .unwrap();
        let by = store.signatures().unwrap().by_address;
        assert_eq!(by.len(), 2);
        assert_eq!((&by[HI], &by["shop@rossstudio.art"]), (&studio, &none));
        // Deleting a signature leaves the addresses that used it with none.
        store.delete_signature("studio").unwrap();
        let by = store.signatures().unwrap().by_address;
        assert_eq!(
            by[HI],
            AccountSignatures {
                new_emails: None,
                replies: Some("short".into())
            }
        );
        // None: the same as the account again.
        store
            .set_address_signatures("work", "shop@rossstudio.art", None)
            .unwrap();
        assert!(!store
            .signatures()
            .unwrap()
            .by_address
            .contains_key("shop@rossstudio.art"));
        // Removing an address forgets its signatures.
        store.remove_address("work", HI).unwrap();
        assert!(store.signatures().unwrap().by_address.is_empty());
        assert!(matches!(
            store.set_address_signatures("work", "x@rossstudio.art", None),
            Err(Error::NotFound {
                kind: "address",
                ..
            })
        ));
        let gone = AccountSignatures {
            new_emails: Some("gone".into()),
            replies: None,
        };
        assert!(store
            .set_address_signatures("work", "shop@rossstudio.art", Some(&gone))
            .is_err());
    }

    #[test]
    fn domains_worth_asking_about_are_those_of_mail_that_came_to_no_known_address() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        to(&mut store, "a", "x@studio-two.art");
        to(&mut store, "b", "y@studio-two.art");
        to(&mut store, "c", "z@other.co");
        // Came to an address Moss knows, so the others on it are other people.
        to(&mut store, "d", "maya@icloud.com, dana@acme.co");
        to(&mut store, "e", "hi@rossstudio.art, sam@team.co");
        // Public and own domains need no asking.
        to(&mut store, "f", "maya.ross@gmail.com");
        to(&mut store, "g", "info@rossstudio.art");
        let mine = mail()
            .id("m@x")
            .subject("m")
            .from("maya@icloud.com")
            .to("pat@sent.co")
            .build();
        ingest(&mut store, "m", &mine);
        assert_eq!(
            store.address_domains_to_check("work").unwrap(),
            ["studio-two.art", "other.co"]
        );
        assert!(matches!(
            store.address_domains_to_check("nobody"),
            Err(Error::UnknownAccount(_))
        ));
    }

    #[test]
    fn scanning_suggests_addresses_mail_came_to_and_counts_it() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        store.block_address("work", LEAK, true).unwrap();
        to(&mut store, "a", "hi@rossstudio.art");
        let raw = mail()
            .id("b@x")
            .subject("b")
            .to("dana@acme.co")
            .header("Cc", "Hi@RossStudio.art")
            .build();
        ingest(&mut store, "b", &raw);
        to(&mut store, "c", "Shop@RossStudio.art");
        to(&mut store, "d", "shop@rossstudio.art, info@rossstudio.art");
        to(&mut store, "e", LEAK);
        to(&mut store, "f", "me@studio-two.art");
        to(&mut store, "g", "x@not-asked.co");
        // Someone else on mail to an address Moss knows isn't suggested.
        to(
            &mut store,
            "h",
            "hi@rossstudio.art, colleague@rossstudio.art",
        );
        // Mail you sent and mail you deleted don't count.
        let mine = mail()
            .id("m@x")
            .subject("m")
            .from(HI)
            .to("shop@rossstudio.art")
            .build();
        ingest(&mut store, "m", &mine);
        let gone = to(&mut store, "i", "old@rossstudio.art");
        store.delete_thread(&gone).unwrap();
        store.remove_address("work", HI).unwrap();
        store.add_address("work", HI, None).unwrap();

        store
            .scan_addresses("work", &["studio-two.art".into()])
            .unwrap();
        let seen: Vec<(String, AddressState, u32)> = store
            .every_address("work")
            .unwrap()
            .into_iter()
            .map(|a| (a.email, a.state, a.seen))
            .collect();
        let s = |e: &str, state, n| (e.to_string(), state, n);
        assert_eq!(
            seen,
            [
                s(HI, AddressState::Added, 3),
                s(LEAK, AddressState::Dismissed, 1),
                s("shop@rossstudio.art", AddressState::Suggested, 2),
                s("info@rossstudio.art", AddressState::Suggested, 1),
                s("me@studio-two.art", AddressState::Suggested, 1),
            ]
        );
        // Scanning again changes nothing; a dismissed suggestion stays dismissed.
        store.remove_address("work", "info@rossstudio.art").unwrap();
        store
            .scan_addresses("work", &["studio-two.art".into()])
            .unwrap();
        assert_eq!(
            emails(&store.addresses("work").unwrap()),
            [HI, LEAK, "shop@rossstudio.art", "me@studio-two.art"]
        );
        assert_eq!(
            store
                .address("work", "info@rossstudio.art")
                .unwrap()
                .unwrap()
                .state,
            AddressState::Dismissed
        );
        // Never at a public domain, even when asked.
        to(&mut store, "j", "maya.ross@gmail.com");
        store.scan_addresses("work", &["gmail.com".into()]).unwrap();
        assert_eq!(store.address("work", "maya.ross@gmail.com").unwrap(), None);
    }

    #[test]
    fn gmail_send_as_addresses_are_added_but_never_one_you_dismissed() {
        let mut store = icloud();
        store.add_address("work", HI, Some("Mine")).unwrap();
        to(&mut store, "a", "shop@rossstudio.art");
        store.scan_addresses("work", &[]).unwrap();
        store
            .add_address("work", "old@rossstudio.art", None)
            .unwrap();
        store.remove_address("work", "old@rossstudio.art").unwrap();
        for (email, name) in [
            ("Work@Northwind.co", Some("Maya at Northwind")),
            ("shop@rossstudio.art", Some("Shop")),
            (HI, Some("Gmail's name")),
            ("old@rossstudio.art", None),
        ] {
            store.add_gmail_address("work", email, name).unwrap();
        }
        let got: Vec<_> = store
            .every_address("work")
            .unwrap()
            .into_iter()
            .map(|a| (a.email, a.name, a.state, a.source))
            .collect();
        let s = |e: &str, n: Option<&str>, state, source| {
            (e.to_string(), n.map(String::from), state, source)
        };
        assert_eq!(
            got,
            [
                s(HI, Some("Mine"), AddressState::Added, AddressSource::You),
                s(
                    "shop@rossstudio.art",
                    Some("Shop"),
                    AddressState::Added,
                    AddressSource::Gmail
                ),
                s(
                    "old@rossstudio.art",
                    None,
                    AddressState::Dismissed,
                    AddressSource::You
                ),
                s(
                    "work@northwind.co",
                    Some("Maya at Northwind"),
                    AddressState::Added,
                    AddressSource::Gmail
                ),
            ]
        );
        // Mail from them is yours now.
        let raw = mail()
            .id("w@x")
            .subject("w")
            .from("work@northwind.co")
            .to("dana@acme.co")
            .build();
        let w = in_sent(&mut store, "w", &raw).thread_id;
        assert_eq!(from_me(&store, &w), [(true, true, true)]);
        assert!(matches!(
            store.add_gmail_address("work", "maya@icloud.com", None),
            Err(Error::PrimaryAddress(_))
        ));
        assert!(matches!(
            store.add_gmail_address("work", "nope", None),
            Err(Error::InvalidAddress(_))
        ));
    }

    #[test]
    fn removing_an_account_forgets_its_addresses() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        store.remove_account("work").unwrap();
        store
            .upsert_account(&account("work", "maya@icloud.com"))
            .unwrap();
        assert!(store.every_address("work").unwrap().is_empty());
    }

    #[test]
    fn failures_are_errors() {
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        break_schema(
            &store,
            "ALTER TABLE account_addresses RENAME TO old_addresses",
        );
        assert!(store.addresses("work").is_err());
        assert!(store.address("work", HI).is_err());
        assert!(store.add_address("work", HI, None).is_err());
        assert!(store.update_address("work", HI, None).is_err());
        assert!(store.remove_address("work", HI).is_err());
        assert!(store.block_address("work", HI, true).is_err());
        assert!(store.unblock_addresses_in("1").is_err());
        assert!(store.set_address_signatures("work", HI, None).is_err());
        assert!(store.add_gmail_address("work", HI, None).is_err());
        assert!(store.address_domains_to_check("work").is_err());
        assert!(store.scan_addresses("work", &[]).is_err());
        assert!(store.signatures().is_err());
        assert!(store.ingest(&mail().build(), &source("1")).is_err());
        assert!(
            store.list_threads(None, SplitId::Important, NOV_1).is_ok(),
            "an empty list needs no addresses"
        );
        let mut store = icloud();
        to(&mut store, "a", HI);
        break_schema(
            &store,
            "ALTER TABLE account_addresses RENAME TO old_addresses",
        );
        assert!(store.list_threads(None, SplitId::Important, NOV_1).is_err());
        let mut store = icloud();
        ingest(&mut store, "a", &mail().from(HI).build());
        break_schema(&store, "CREATE TRIGGER no_mine BEFORE UPDATE OF from_me ON messages BEGIN SELECT RAISE(FAIL, 'full'); END");
        assert!(store.add_address("work", HI, None).is_err());
        assert!(store.add_gmail_address("work", HI, None).is_err());
        assert_eq!(store.address("work", HI).unwrap(), None, "nothing is kept");
        let mut store = icloud();
        to(&mut store, "a", HI);
        break_schema(&store, "CREATE TRIGGER no_blocks BEFORE UPDATE OF blocked ON messages BEGIN SELECT RAISE(FAIL, 'full'); END");
        assert!(store.block_address("work", HI, true).is_err());
        let mut store = icloud();
        break_schema(&store, "ALTER TABLE accounts RENAME TO old_accounts");
        assert!(store.block_address("work", HI, true).is_err());
        assert!(store.address_domains_to_check("work").is_err());
        assert!(store.scan_addresses("work", &[]).is_err());
        let mut store = icloud();
        to(&mut store, "a", HI);
        break_schema(&store, "CREATE TRIGGER no_new BEFORE INSERT ON account_addresses BEGIN SELECT RAISE(FAIL, 'full'); END");
        assert!(store
            .scan_addresses("work", &["rossstudio.art".into()])
            .is_err());
        let mut store = icloud();
        store.add_address("work", HI, None).unwrap();
        to(&mut store, "a", HI);
        break_schema(&store, "CREATE TRIGGER no_count BEFORE UPDATE OF seen ON account_addresses BEGIN SELECT RAISE(FAIL, 'full'); END");
        assert!(store.scan_addresses("work", &[]).is_err());
        let store = icloud();
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN cc_json TO cc");
        assert!(store.address_domains_to_check("work").is_err());
        let mut store = icloud();
        break_schema(
            &store,
            "ALTER TABLE account_addresses RENAME COLUMN blocked TO b",
        );
        assert!(
            store.ingest(&mail().build(), &source("1")).is_err(),
            "can't tell whether it is blocked"
        );
    }
}
