//! New, accepted and blocked senders, per account, like Spark's Gatekeeper.
//!
//! Blocking marks the sender's mail `blocked`: it leaves the inbox and the Done list and
//! shows in the Blocked list. The engine moves it to the account's [`BLOCKED_FOLDER`] on
//! the server, so other apps hide it too. Mail found in that folder (put there by Spark,
//! say) blocks its sender here as well. Unblocking is accepting: the mail comes back.
//!
//! Mail sent to an address you blocked (see `addresses.rs`) is blocked the same way.

use std::collections::HashMap;

use moss_core::{SenderRule, SenderStatus, ThreadSummary};
use rusqlite::{params, Connection, OptionalExtension};

use crate::addresses::RECIPIENTS;
use crate::classify::{self, domain_of, is_public_domain, same_address, SenderFacts};
use crate::{Error, Result, Store};

/// The server folder blocked mail goes to. Spark uses the same name, so mail it blocked
/// shows up here too.
pub const BLOCKED_FOLDER: &str = "Blocked";

/// SQL: whether message `m` is from a sender blocked in its account, by address or by the
/// domain after its @. Each rule that could catch it is looked up by its key, so blocking
/// costs the same however many rules there are. An address with more than one @ (no real
/// mail has one) is also caught by a rule on whatever follows any of them, as it always was.
fn from_blocked() -> String {
    const SENDER: &str = "lower(m.from_email)";
    // Where the last @ is: everything after it trimmed away.
    let last_at = format!("length(rtrim({SENDER}, replace({SENDER}, '@', '')))");
    format!(
        "(EXISTS (SELECT 1 FROM sender_rules r WHERE r.account_id = m.account_id AND r.rule = 'block'
             AND r.sender IN ({SENDER}, substr({SENDER}, {last_at} + 1)))
          OR (instr({SENDER}, '@') < {last_at} AND EXISTS (SELECT 1 FROM sender_rules r
             WHERE r.account_id = m.account_id AND r.rule = 'block'
               AND substr({SENDER}, -length(r.sender) - 1) = '@' || r.sender)))"
    )
}

/// SQL: whether message `m` was sent (To or Cc) to an address of its account you blocked.
fn to_blocked() -> String {
    format!(
        "EXISTS (SELECT 1 FROM account_addresses a WHERE a.account_id = m.account_id AND a.blocked = 1
             AND a.email IN ({RECIPIENTS}))"
    )
}

/// SQL: whether a block catches message `m`: one on its sender, or on an address it was
/// sent to.
fn blocked_by_rule() -> String {
    format!("({} OR {})", from_blocked(), to_blocked())
}

/// SQL: brings back mail in account `?1` that no block catches any more.
pub(crate) fn unblock_lifted() -> String {
    format!(
        "UPDATE messages AS m SET blocked = 0, done = 0, done_at = NULL
         WHERE m.account_id = ?1 AND m.blocked = 1 AND NOT {}",
        blocked_by_rule()
    )
}

/// SQL: blocks the mail from the lowercase address `?2` in account `?1`, found by index.
const BLOCK_THEIRS: &str = "UPDATE messages AS m SET blocked = 1
    WHERE m.account_id = ?1 AND lower(m.from_email) = ?2 AND m.from_me = 0 AND m.blocked = 0";

/// SQL: blocks the mail others sent in account `?1` that a block catches.
pub(crate) fn block_caught() -> String {
    format!(
        "UPDATE messages AS m SET blocked = 1 WHERE m.account_id = ?1 AND m.from_me = 0 AND m.blocked = 0 AND {}",
        blocked_by_rule()
    )
}

/// A row of `sender_rules`: account, sender, rule, created_at.
fn sender_rule(r: &rusqlite::Row) -> rusqlite::Result<SenderRule> {
    let sender: String = r.get(1)?;
    Ok(SenderRule {
        account_id: r.get(0)?,
        whole_domain: !sender.contains('@'),
        sender,
        blocked: r.get::<_, String>(2)? == "block",
        created_at: r.get(3)?,
    })
}

/// The domain of `email`, for a rule about everyone there. Public ones like gmail.com can't be.
fn whole(email: &str) -> Result<String> {
    match domain_of(email) {
        Some(domain) if !is_public_domain(&domain) => Ok(domain),
        other => Err(Error::PublicDomain(other.unwrap_or_else(|| email.to_string()))),
    }
}

fn rule(conn: &Connection, account_id: &str, sender: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT rule FROM sender_rules WHERE account_id = ?1 AND sender = ?2",
            params![account_id, sender],
            |r| r.get(0),
        )
        .optional()?)
}

/// Whether mail from `email` is blocked in this account, by address or by domain.
fn is_blocked(conn: &Connection, account_id: &str, email: &str) -> Result<bool> {
    let email = email.trim().to_lowercase();
    let by_domain = match domain_of(&email) {
        Some(domain) => rule(conn, account_id, &domain)?,
        None => None,
    };
    Ok(rule(conn, account_id, &email)?.as_deref() == Some("block")
        || by_domain.as_deref() == Some("block"))
}

/// Whether you accepted `email` in this account, by address or by domain.
pub(crate) fn is_accepted(conn: &Connection, account_id: &str, email: &str) -> Result<bool> {
    let email = email.trim().to_lowercase();
    let by_domain = match domain_of(&email) {
        Some(domain) => rule(conn, account_id, &domain)?,
        None => None,
    };
    Ok(rule(conn, account_id, &email)?.as_deref() == Some("accept")
        || by_domain.as_deref() == Some("accept"))
}

/// Blocks new mail as it arrives: from a blocked sender, sent to a blocked address, or
/// found in the Blocked folder (which blocks its sender, unless you accepted them or their
/// domain, or the mail is there for its blocked address). `from` is the message's sender.
pub(crate) fn on_ingest(
    conn: &Connection,
    account_id: &str,
    message: i64,
    from: &str,
    from_me: bool,
    mailbox: &str,
    now: i64,
) -> Result<()> {
    if from_me {
        return Ok(());
    }
    let email = from.trim().to_lowercase();
    let to_address: bool = conn.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM messages m WHERE m.id = ?1 AND {})", to_blocked()),
        [message],
        |r| r.get(0),
    )?;
    // Already blocked by their domain needs no rule of its own, so the domain's rule decides;
    // nor does mail blocked for the address it was sent to.
    if mailbox.eq_ignore_ascii_case(BLOCKED_FOLDER)
        && !to_address
        && rule(conn, account_id, &email)?.is_none()
        && !is_accepted(conn, account_id, &email)?
        && !is_blocked(conn, account_id, &email)?
    {
        conn.execute(
            "INSERT INTO sender_rules (account_id, sender, rule, created_at) VALUES (?1, ?2, 'block', ?3)",
            params![account_id, email, now],
        )?;
        // Blocked elsewhere means blocked: their mail already here goes too. Only theirs is new
        // to block; what other blocks catch was blocked when they came.
        conn.execute(BLOCK_THEIRS, params![account_id, email])?;
    }
    if to_address || is_blocked(conn, account_id, &email)? {
        conn.execute("UPDATE messages SET blocked = 1 WHERE id = ?1", [message])?;
    }
    Ok(())
}

/// SQL: whether you sent mail in account `?1` to the lowercase address `?2` (To, Cc or Bcc).
const YOU_WROTE: &str = "SELECT EXISTS (SELECT 1 FROM sent_to WHERE account_id = ?1 AND email = ?2)";
/// SQL: whether `?2` wrote in a conversation you started, in account `?1`.
const IN_YOUR_THREAD: &str = "SELECT EXISTS (SELECT 1 FROM messages m
     WHERE m.account_id = ?1 AND lower(m.from_email) = ?2
       AND (SELECT f.from_me FROM messages f WHERE f.thread_id = m.thread_id
            ORDER BY f.sent_at, f.id LIMIT 1) = 1)";

/// What decides who senders are in one account, read once, so a whole list of threads can
/// say who each sender is without asking the database again for every row. Whether you know
/// someone from your mail is looked up the first time they're asked about, by index.
pub(crate) struct Senders {
    account_id: String,
    address: String,
    org_domain: Option<String>,
    /// Rules by address or domain: `block` or `accept`.
    rules: HashMap<String, String>,
    /// Lowercase addresses asked about, and whether you wrote to them or they wrote in a
    /// conversation you started.
    familiar: HashMap<String, bool>,
}

impl Senders {
    pub(crate) fn load(conn: &Connection, account_id: &str) -> Result<Self> {
        let (address, org_domain) = conn.query_row(
            "SELECT address, org_domain FROM accounts WHERE id = ?1",
            [account_id],
            |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?)),
        )?;
        let mut stmt = conn.prepare_cached("SELECT sender, rule FROM sender_rules WHERE account_id = ?1")?;
        let rules = stmt
            .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Senders {
            account_id: account_id.into(),
            address,
            org_domain: org_domain.map(|d| d.to_lowercase()),
            rules,
            familiar: HashMap::new(),
        })
    }

    /// Who mail from `email` is from, to you: see [`classify::sender_status`].
    pub(crate) fn status(&mut self, conn: &Connection, email: &str) -> Result<SenderStatus> {
        let email = email.trim().to_lowercase();
        let domain = domain_of(&email);
        // A rule on the address, or on everyone at its domain.
        let ruled = |rule: &str| {
            [Some(&email), domain.as_ref()]
                .into_iter()
                .flatten()
                .any(|who| self.rules.get(who).is_some_and(|r| r == rule))
        };
        let mut facts = SenderFacts {
            blocked: ruled("block"),
            accepted: ruled("accept"),
            your_domain: self.org_domain.is_some() && domain == self.org_domain,
            is_you: same_address(&email, &self.address),
            ..SenderFacts::default()
        };
        // Only someone the rules call new is looked up in your mail. Writing to them and their
        // writing in a conversation you started make them known alike, so one answer is kept.
        if classify::sender_status(&facts) == SenderStatus::New {
            facts.you_wrote = match self.familiar.get(&email) {
                Some(known) => *known,
                None => {
                    let asks = |sql: &str| -> Result<bool> {
                        Ok(conn.prepare_cached(sql)?.query_row(params![self.account_id, email], |r| r.get(0))?)
                    };
                    let known = asks(YOU_WROTE)? || asks(IN_YOUR_THREAD)?;
                    self.familiar.insert(email.clone(), known);
                    known
                }
            };
        }
        Ok(classify::sender_status(&facts))
    }
}

impl Store {
    /// Whether to ask "Block this sender?" about mail from `email` in this account.
    #[cfg(test)]
    pub fn sender_status(&self, account_id: &str, email: &str) -> Result<SenderStatus> {
        Senders::load(&self.conn, account_id)?.status(&self.conn, email)
    }

    /// Blocks `email` (or, with `whole_domain`, everyone at its domain) in this account:
    /// their mail, old and new, goes to Blocked. Public domains like gmail.com can't be
    /// blocked as a whole.
    pub fn block_sender(
        &mut self,
        account_id: &str,
        email: &str,
        whole_domain: bool,
        now: i64,
    ) -> Result<()> {
        let email = email.trim().to_lowercase();
        let sender = if whole_domain { whole(&email)? } else { email };
        let tx = self.conn.transaction()?;
        if whole_domain {
            // "Everyone at" means everyone, including people you had accepted.
            tx.execute(
                "DELETE FROM sender_rules WHERE account_id = ?1 AND substr(sender, -length(?2) - 1) = '@' || ?2",
                params![account_id, sender],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO sender_rules (account_id, sender, rule, created_at) VALUES (?1, ?2, 'block', ?3)",
            params![account_id, sender, now],
        )?;
        tx.execute(&block_caught(), [account_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Accept: never ask about `email` (or, with `whole_domain`, anyone at its domain) again
    /// in this account, and show their images. Someone you blocked by address stays blocked;
    /// a block on the domain itself is lifted. Public domains like gmail.com can't be accepted
    /// as a whole.
    pub fn accept_sender(
        &mut self,
        account_id: &str,
        email: &str,
        whole_domain: bool,
        now: i64,
    ) -> Result<()> {
        let email = email.trim().to_lowercase();
        let sender = if whole_domain { whole(&email)? } else { email };
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO sender_rules (account_id, sender, rule, created_at) VALUES (?1, ?2, 'accept', ?3)",
            params![account_id, sender, now],
        )?;
        if whole_domain {
            tx.execute(&unblock_lifted(), [account_id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Unblocks `email`, and the domain block that caught them if there is one, and accepts
    /// them. Mail no longer blocked comes back to the inbox.
    pub fn unblock_sender(&mut self, account_id: &str, email: &str, now: i64) -> Result<()> {
        let email = email.trim().to_lowercase();
        let tx = self.conn.transaction()?;
        if let Some(domain) = domain_of(&email) {
            tx.execute(
                "DELETE FROM sender_rules WHERE account_id = ?1 AND sender = ?2 AND rule = 'block'",
                params![account_id, domain],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO sender_rules (account_id, sender, rule, created_at) VALUES (?1, ?2, 'accept', ?3)",
            params![account_id, email, now],
        )?;
        tx.execute(&unblock_lifted(), [account_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Everyone you blocked or accepted, by address or domain, in every account, newest first.
    pub fn sender_rules(&self) -> Result<Vec<SenderRule>> {
        let mut stmt = self.conn.prepare(
            "SELECT account_id, sender, rule, created_at FROM sender_rules
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], sender_rule)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Forgets what you decided about `sender` (an address or a domain) in this account, so
    /// Moss asks about them again. Mail a forgotten block kept out comes back to the inbox.
    pub fn forget_sender_rule(&mut self, account_id: &str, sender: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM sender_rules WHERE account_id = ?1 AND sender = ?2",
            params![account_id, sender.trim().to_lowercase()],
        )?;
        tx.execute(&unblock_lifted(), [account_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Blocked threads, newest first, for one account or all of them.
    pub fn list_blocked(
        &self,
        account: Option<&str>,
        limit: usize,
        now: i64,
    ) -> Result<Vec<ThreadSummary>> {
        self.list_where(crate::query::BLOCKED, account, limit, now)
    }

    /// Server ids of blocked mail still in `mailbox` (the inbox), to move to [`BLOCKED_FOLDER`].
    pub fn blocked_in(&self, account_id: &str, mailbox: &str) -> Result<Vec<String>> {
        self.located(account_id, mailbox, true)
    }

    /// Server ids of mail in `mailbox` (the Blocked folder) that is no longer blocked, to move
    /// back to the inbox.
    pub fn unblocked_in(&self, account_id: &str, mailbox: &str) -> Result<Vec<String>> {
        self.located(account_id, mailbox, false)
    }

    fn located(&self, account_id: &str, mailbox: &str, blocked: bool) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.provider_id FROM message_locations l
             JOIN mailboxes b ON b.id = l.mailbox_id JOIN messages m ON m.id = l.message_id
             WHERE b.account_id = ?1 AND b.name = ?2 AND m.blocked = ?3 AND m.from_me = 0
             ORDER BY m.sent_at, m.id",
        )?;
        let rows = stmt.query_map(params![account_id, mailbox, blocked], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{Mailbox, MailboxRole, MessageSource, SplitId};

    const JORDAN: &str = "Jordan Blake <jordan@growthleads.example>";

    fn from(store: &mut Store, id: &str, sender: &str) -> String {
        ingest(store, id, &mail().id(&format!("{id}@x")).from(sender).subject(id).build()).thread_id
    }

    /// Every list a thread can be in, as rows: each split, Done and Blocked.
    fn rows(store: &Store) -> Vec<ThreadSummary> {
        use SplitId::*;
        let mut rows: Vec<ThreadSummary> = [Important, Other, Calendar, Newsletters, Receipts]
            .into_iter()
            .flat_map(|split| store.list_threads(None, split, NOV_1).unwrap())
            .collect();
        rows.extend(store.list_done(None, 50, NOV_1).unwrap());
        rows.extend(store.list_blocked(None, 50, NOV_1).unwrap());
        rows
    }

    /// Who the sender is, as the open thread says. Its row in the list says the same, so
    /// the "Block this sender?" bar is decided before you open it and doesn't come or go.
    fn status(store: &Store, thread: &str) -> SenderStatus {
        let open = store.thread(thread, NOV_1).unwrap().unwrap().summary.sender;
        let listed: Vec<SenderStatus> =
            rows(store).into_iter().filter(|t| t.id == thread).map(|t| t.sender).collect();
        assert_eq!(listed, [open], "the list and the open thread disagree");
        open
    }

    fn inbox(store: &Store) -> Vec<String> {
        let list = store.list_threads(None, SplitId::Important, NOV_1).unwrap();
        list.into_iter().map(|t| t.id).collect()
    }

    fn blocked(store: &Store, account: Option<&str>) -> Vec<String> {
        store.list_blocked(account, 50, NOV_1).unwrap().into_iter().map(|t| t.id).collect()
    }

    fn with_sent(store: &mut Store) {
        let sent = Mailbox {
            account_id: "work".into(),
            name: "Sent".into(),
            role: Some(MailboxRole::Sent),
        };
        store.upsert_mailbox(&sent).unwrap();
    }

    fn sent(store: &mut Store, id: &str, mail: Mail) -> String {
        let raw = mail.id(id).from("Maya Ross <maya@northwind.co>").build();
        store
            .ingest(&raw, &MessageSource { mailbox: "Sent".into(), ..source(id) })
            .unwrap()
            .thread_id
    }

    fn in_blocked_folder(store: &mut Store, id: &str, raw: &[u8]) -> String {
        let at = MessageSource { mailbox: BLOCKED_FOLDER.into(), ..source(id) };
        store.ingest(raw, &at).unwrap().thread_id
    }

    #[test]
    fn every_row_of_a_list_says_who_its_sender_is_in_its_own_account() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let work = from(&mut store, "a", JORDAN);
        let stranger = from(&mut store, "b", "Alex <alex@seo-boost.example>");
        let raw = mail().id("p@x").from(JORDAN).subject("p").build();
        let at = MessageSource { account_id: "personal".into(), ..source("p") };
        let personal = store.ingest(&raw, &at).unwrap().thread_id;
        store.accept_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        let listed = |store: &Store, id: &str| rows(store).into_iter().find(|t| t.id == id).unwrap().sender;
        assert_eq!(listed(&store, &work), SenderStatus::Known);
        assert_eq!(listed(&store, &personal), SenderStatus::New, "accepted in work only");
        assert_eq!(listed(&store, &stranger), SenderStatus::New);
        // Search results say it too.
        let hits = store.search("seo-boost", None, 10, NOV_1).unwrap();
        assert_eq!(hits.iter().map(|h| h.thread.sender).collect::<Vec<_>>(), [SenderStatus::New]);
        store.block_sender("work", "alex@seo-boost.example", false, NOV_1).unwrap();
        assert_eq!(listed(&store, &stranger), SenderStatus::Blocked);
    }

    #[test]
    fn whether_someone_is_familiar_is_searched_by_index_not_read_from_all_your_mail() {
        let store = store_with_work();
        let you_wrote = plan(&store, YOU_WROTE);
        assert!(you_wrote.contains("SEARCH sent_to USING COVERING INDEX sent_to_by_email (account_id=? AND email=?)"), "{you_wrote}");
        let in_thread = plan(&store, IN_YOUR_THREAD);
        assert!(in_thread.contains("SEARCH m USING INDEX messages_by_sender (account_id=? AND <expr>=?)"), "{in_thread}");
        assert!(!scans(&you_wrote, "m") && !scans(&in_thread, "messages"), "{in_thread}");
    }

    #[test]
    fn a_block_looks_up_each_sender_s_rules_instead_of_trying_every_rule() {
        let store = store_with_work();
        for sql in [block_caught(), unblock_lifted()] {
            let plan = plan(&store, &sql);
            assert!(plan.contains("SEARCH r USING INDEX sqlite_autoindex_sender_rules_1 (account_id=? AND sender=?)"), "{plan}");
        }
    }

    #[test]
    fn mail_found_in_the_blocked_folder_blocks_only_that_sender_s_mail_by_index() {
        let store = store_with_work();
        let plan = plan(&store, BLOCK_THEIRS);
        assert!(plan.contains("SEARCH m USING INDEX messages_by_sender (account_id=? AND <expr>=?)"), "{plan}");
    }

    #[test]
    fn an_address_with_two_at_signs_is_caught_by_a_rule_on_what_follows_either() {
        let mut store = store_with_work();
        let thread = from(&mut store, "a", "Odd <odd@x@growthleads.example>");
        let other = from(&mut store, "b", "Pat <pat@other.example>");
        store.conn.execute("UPDATE messages SET from_email = 'odd@x@growthleads.example' WHERE thread_id = ?1", [&thread]).unwrap();
        store.block_sender("work", "x@growthleads.example", false, NOV_1).unwrap();
        assert_eq!(blocked(&store, None), [thread.clone()], "a rule on the address after the first @");
        store.forget_sender_rule("work", "x@growthleads.example").unwrap();
        store.block_sender("work", "growthleads.example", false, NOV_1).unwrap();
        assert_eq!(blocked(&store, None), [thread], "a rule on the domain after the last @");
        assert_eq!(inbox(&store), [other]);
    }

    #[test]
    fn a_stranger_is_a_new_sender() {
        let mut store = store_with_work();
        let thread = from(&mut store, "a", JORDAN);
        assert_eq!(status(&store, &thread), SenderStatus::New);
        assert_eq!(
            store.sender_status("work", "jordan@growthleads.example").unwrap(),
            SenderStatus::New
        );
    }

    #[test]
    fn someone_you_wrote_to_is_known_whatever_the_case() {
        let mut store = store_with_work();
        with_sent(&mut store);
        sent(&mut store, "s@x", mail().to("Jordan <Jordan@GrowthLeads.example>"));
        let thread = from(&mut store, "a", JORDAN);
        assert_eq!(status(&store, &thread), SenderStatus::Known);
    }

    #[test]
    fn someone_in_a_conversation_you_started_is_known() {
        let mut store = store_with_work();
        with_sent(&mut store);
        let started = sent(&mut store, "s@x", mail().to("daniel@northwind.co"));
        let raw = mail().id("r@x").from(JORDAN).reply_to(&["s@x"]).build();
        let reply = ingest(&mut store, "r", &raw).thread_id;
        assert_eq!(reply, started);
        assert_eq!(status(&store, &reply), SenderStatus::Known);
        // A conversation they started and you only read doesn't count.
        let other = from(&mut store, "b", "Alex <alex@seo-boost.example>");
        let raw = mail().id("c@x").from("Pat <pat@agency.io>").reply_to(&["b@x"]).build();
        assert_eq!(ingest(&mut store, "c", &raw).thread_id, other);
        assert_eq!(status(&store, &other), SenderStatus::New);
    }

    #[test]
    fn colleagues_at_the_company_domain_are_known() {
        let mut store = store_with_work();
        let thread = from(&mut store, "a", "Daniel <daniel@northwind.co>");
        assert_eq!(status(&store, &thread), SenderStatus::New);
        let work = moss_core::Account {
            org_domain: Some("Northwind.co".into()),
            ..account("work", "maya@northwind.co")
        };
        store.upsert_account(&work).unwrap();
        assert_eq!(status(&store, &thread), SenderStatus::Known);
        assert_eq!(store.sender_status("work", "x@sub.northwind.co").unwrap(), SenderStatus::New);
    }

    #[test]
    fn your_own_mail_is_never_from_a_new_sender() {
        let mut store = store_with_work();
        with_sent(&mut store);
        let thread = sent(&mut store, "s@x", mail().to("jordan@growthleads.example"));
        // Mail only you sent is in no list.
        assert!(rows(&store).is_empty());
        assert_eq!(store.thread(&thread, NOV_1).unwrap().unwrap().summary.sender, SenderStatus::Known);
        assert_eq!(store.sender_status("work", "Maya@Northwind.co").unwrap(), SenderStatus::Known);
    }

    #[test]
    fn accepting_a_sender_stops_the_question() {
        let mut store = store_with_work();
        let thread = from(&mut store, "a", JORDAN);
        store.accept_sender("work", "Jordan@growthleads.example", false, NOV_1).unwrap();
        assert_eq!(status(&store, &thread), SenderStatus::Known);
        assert_eq!(inbox(&store), [thread]);
    }

    #[test]
    fn accepting_a_domain_accepts_everyone_there_but_only_there() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let jordan = from(&mut store, "a", JORDAN);
        let sam = from(&mut store, "b", "Sam <sam@growthleads.example>");
        let lookalike = from(&mut store, "c", "Pat <pat@notgrowthleads.example>");
        store.accept_sender("work", "Jordan@GrowthLeads.example", true, NOV_1).unwrap();
        assert_eq!(status(&store, &jordan), SenderStatus::Known);
        assert_eq!(status(&store, &sam), SenderStatus::Known);
        assert_eq!(store.sender_status("work", "new@growthleads.example").unwrap(), SenderStatus::Known);
        assert_eq!(status(&store, &lookalike), SenderStatus::New);
        assert_eq!(
            store.sender_status("personal", "new@growthleads.example").unwrap(),
            SenderStatus::New
        );
    }

    #[test]
    fn someone_you_blocked_stays_blocked_when_you_accept_their_domain() {
        let mut store = store_with_work();
        let jordan = from(&mut store, "a", JORDAN);
        let sam = from(&mut store, "b", "Sam <sam@growthleads.example>");
        store.block_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        store.accept_sender("work", "sam@growthleads.example", true, NOV_1).unwrap();
        assert_eq!(status(&store, &jordan), SenderStatus::Blocked);
        assert_eq!(blocked(&store, None), [jordan]);
        assert_eq!(inbox(&store), [sam]);
    }

    #[test]
    fn accepting_a_blocked_domain_brings_its_mail_back_and_blocking_it_again_wins() {
        let mut store = store_with_work();
        let jordan = from(&mut store, "a", JORDAN);
        store.block_sender("work", "jordan@growthleads.example", true, NOV_1).unwrap();
        store.accept_sender("work", "jordan@growthleads.example", true, NOV_1).unwrap();
        assert!(blocked(&store, None).is_empty());
        assert_eq!(inbox(&store), [jordan.clone()]);
        store.block_sender("work", "jordan@growthleads.example", true, NOV_1).unwrap();
        assert_eq!(status(&store, &jordan), SenderStatus::Blocked);
        assert_eq!(blocked(&store, None), [jordan]);
    }

    #[test]
    fn lists_who_you_blocked_or_accepted_newest_first_in_every_account() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        assert!(store.sender_rules().unwrap().is_empty());
        store.block_sender("work", "Jordan@growthleads.example", false, 1).unwrap();
        store.accept_sender("personal", "no-reply@accounts.google.com", true, 2).unwrap();
        store.accept_sender("work", "alex@studio-build.example.test", false, 2).unwrap();
        let rules: Vec<_> = store
            .sender_rules()
            .unwrap()
            .into_iter()
            .map(|r| (r.account_id, r.sender, r.whole_domain, r.blocked, r.created_at))
            .collect();
        assert_eq!(
            rules,
            [
                ("work".into(), "alex@studio-build.example.test".into(), false, false, 2),
                ("personal".into(), "accounts.google.com".into(), true, false, 2),
                ("work".into(), "jordan@growthleads.example".into(), false, true, 1),
            ]
        );
    }

    #[test]
    fn forgetting_a_block_brings_their_mail_back_and_asks_again() {
        let mut store = store_with_work();
        let jordan = from(&mut store, "a", JORDAN);
        let alex = from(&mut store, "b", "Alex <alex@seo-boost.example>");
        store.block_sender("work", "jordan@growthleads.example", true, NOV_1).unwrap();
        store.block_sender("work", "alex@seo-boost.example", false, NOV_1).unwrap();
        store.forget_sender_rule("work", "GrowthLeads.example").unwrap();
        assert_eq!(blocked(&store, None), [alex]);
        assert_eq!(inbox(&store), [jordan.clone()]);
        assert_eq!(status(&store, &jordan), SenderStatus::New);
        assert_eq!(store.sender_rules().unwrap().len(), 1);
    }

    #[test]
    fn forgetting_an_accept_asks_again_and_hides_their_images() {
        let mut store = store_with_work();
        let jordan = from(&mut store, "a", JORDAN);
        store.accept_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        store.forget_sender_rule("work", " jordan@growthleads.example ").unwrap();
        assert_eq!(status(&store, &jordan), SenderStatus::New);
        assert_eq!(
            store.image_trust("work", "jordan@growthleads.example").unwrap(),
            moss_core::ImageTrust::Ask
        );
        // Forgetting what isn't there changes nothing.
        store.forget_sender_rule("work", "nobody@nowhere.io").unwrap();
        assert!(store.sender_rules().unwrap().is_empty());
    }

    #[test]
    fn blocking_moves_their_mail_from_the_inbox_to_blocked_now_and_later() {
        let mut store = store_with_work();
        let first = from(&mut store, "a", JORDAN);
        let other = from(&mut store, "b", "Alex <alex@seo-boost.example>");
        store.block_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        assert_eq!(status(&store, &first), SenderStatus::Blocked);
        assert_eq!(inbox(&store), [other.as_str()]);
        assert_eq!(blocked(&store, None), [first.as_str()]);
        let later = from(&mut store, "c", "JORDAN@growthleads.example");
        assert_eq!(blocked(&store, Some("work")), [later, first]);
        assert_eq!(inbox(&store), [other]);
        assert_eq!(store.split_counts(None, NOV_1).unwrap().important, 1);
    }

    #[test]
    fn blocked_mail_stays_out_of_the_done_list() {
        let mut store = store_with_work();
        let thread = from(&mut store, "a", JORDAN);
        store.block_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        store.mark_done(&thread, true, NOV_1).unwrap();
        assert!(store.list_done(None, 10, NOV_1).unwrap().is_empty());
        assert_eq!(blocked(&store, None), [thread]);
    }

    #[test]
    fn blocking_a_domain_blocks_everyone_there_but_only_there() {
        let mut store = store_with_work();
        store.accept_sender("work", "sam@growthleads.example", false, NOV_1).unwrap();
        let jordan = from(&mut store, "a", JORDAN);
        let sam = from(&mut store, "b", "Sam <sam@growthleads.example>");
        let lookalike = from(&mut store, "c", "Pat <pat@notgrowthleads.example>");
        store.block_sender("work", "jordan@GrowthLeads.example", true, NOV_1).unwrap();
        assert_eq!(blocked(&store, None), [sam.clone(), jordan]);
        assert_eq!(status(&store, &sam), SenderStatus::Blocked);
        assert_eq!(inbox(&store), [lookalike]);
        assert_eq!(
            store.sender_status("work", "new@growthleads.example").unwrap(),
            SenderStatus::Blocked
        );
    }

    #[test]
    fn a_public_domain_is_never_blocked_as_a_whole() {
        let mut store = store_with_work();
        let thread = from(&mut store, "a", "Pat <pat@gmail.com>");
        let err = store.block_sender("work", "pat@gmail.com", true, NOV_1).unwrap_err();
        assert_eq!(
            err.to_string(),
            "gmail.com is a public mail service, so it can't be blocked or accepted as a whole"
        );
        assert!(matches!(
            store.accept_sender("work", "pat@gmail.com", true, NOV_1),
            Err(Error::PublicDomain(_))
        ));
        assert_eq!(status(&store, &thread), SenderStatus::New);
        assert!(matches!(
            store.block_sender("work", "nobody", true, NOV_1),
            Err(Error::PublicDomain(_))
        ));
        assert_eq!(inbox(&store), [thread]);
    }

    #[test]
    fn unblocking_brings_their_mail_back_and_accepts_them() {
        let mut store = store_with_work();
        let jordan = from(&mut store, "a", JORDAN);
        let sam = from(&mut store, "b", "Sam <sam@growthleads.example>");
        store.block_sender("work", "jordan@growthleads.example", true, NOV_1).unwrap();
        store.mark_done(&jordan, true, NOV_1).unwrap();
        // Unblocking one person lifts the domain block they were caught by.
        store.unblock_sender("work", "jordan@growthleads.example", NOV_1).unwrap();
        assert!(blocked(&store, None).is_empty());
        assert_eq!(inbox(&store), [sam.clone(), jordan.clone()]);
        assert_eq!(status(&store, &jordan), SenderStatus::Known);
        assert_eq!(status(&store, &sam), SenderStatus::New);
    }

    #[test]
    fn unblocking_one_address_leaves_other_blocks_alone() {
        let mut store = store_with_work();
        let jordan = from(&mut store, "a", JORDAN);
        let alex = from(&mut store, "b", "Alex <alex@seo-boost.example>");
        store.block_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        store.block_sender("work", "alex@seo-boost.example", false, NOV_1).unwrap();
        store.unblock_sender("work", "jordan@growthleads.example", NOV_1).unwrap();
        assert_eq!(blocked(&store, None), [alex]);
        assert_eq!(inbox(&store), [jordan]);
    }

    #[test]
    fn each_account_has_its_own_blocked_list() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let work = from(&mut store, "a", JORDAN);
        let raw = mail().id("p@x").from(JORDAN).to("maya.ross@fastmail.com").build();
        let personal = store
            .ingest(&raw, &MessageSource { account_id: "personal".into(), ..source("p") })
            .unwrap()
            .thread_id;
        store.block_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        assert_eq!(blocked(&store, None), [work.as_str()]);
        assert!(blocked(&store, Some("personal")).is_empty());
        assert_eq!(status(&store, &personal), SenderStatus::New);
        assert_eq!(inbox(&store), [personal]);
    }

    #[test]
    fn mail_in_the_blocked_folder_blocks_its_sender() {
        // Spark (or Moss on another Mac) blocked them and moved their mail there.
        let mut store = store_with_work();
        let earlier = from(&mut store, "e", JORDAN);
        let old = in_blocked_folder(&mut store, "1", &mail().id("a@x").from(JORDAN).build());
        assert_eq!(blocked(&store, None), [old.clone(), earlier.clone()]);
        let new = from(&mut store, "b", JORDAN);
        assert_eq!(blocked(&store, None), [new, old, earlier]);
        assert!(inbox(&store).is_empty());
    }

    #[test]
    fn mail_moved_to_the_blocked_folder_elsewhere_is_blocked_here() {
        let mut store = store_with_work();
        let thread = from(&mut store, "a", JORDAN);
        in_blocked_folder(&mut store, "9", &mail().id("a@x").from(JORDAN).subject("a").build());
        assert_eq!(blocked(&store, None), [thread]);
        // Seeing it there again changes nothing.
        in_blocked_folder(&mut store, "9", &mail().id("a@x").from(JORDAN).subject("a").build());
        assert_eq!(blocked(&store, None).len(), 1);
    }

    #[test]
    fn mail_in_the_blocked_folder_from_a_blocked_domain_comes_back_with_the_domain() {
        // Moss moved it there for the domain block; finding it there adds no block of its own.
        let mut store = store_with_work();
        store.block_sender("work", "jordan@growthleads.example", true, NOV_1).unwrap();
        let sam = in_blocked_folder(&mut store, "1", &mail().id("s@x").from("Sam <sam@growthleads.example>").build());
        assert_eq!(blocked(&store, None), [sam.clone()]);
        store.accept_sender("work", "jordan@growthleads.example", true, NOV_1).unwrap();
        assert!(blocked(&store, None).is_empty());
        assert_eq!(status(&store, &sam), SenderStatus::Known);
    }

    #[test]
    fn the_blocked_folder_never_blocks_someone_you_accepted_or_yourself() {
        let mut store = store_with_work();
        store.accept_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        store.accept_sender("work", "sam@friendly.co", true, NOV_1).unwrap();
        in_blocked_folder(&mut store, "1", &mail().id("a@x").from(JORDAN).build());
        in_blocked_folder(&mut store, "3", &mail().id("c@x").from("Lu <lu@friendly.co>").build());
        let mine = mail().id("m@x").from("Maya <maya@northwind.co>").to("jordan@growthleads.example");
        in_blocked_folder(&mut store, "2", &mine.build());
        assert!(blocked(&store, None).is_empty());
        assert_eq!(store.sender_status("work", "maya@northwind.co").unwrap(), SenderStatus::Known);
    }

    #[test]
    fn a_failure_while_blocking_from_the_blocked_folder_stores_nothing() {
        let mut store = store_with_work();
        let raw = mail().id("a@x").from(JORDAN).build();
        break_schema(&store, "CREATE TRIGGER no_rules BEFORE INSERT ON sender_rules BEGIN SELECT RAISE(FAIL, 'full'); END");
        let at = MessageSource { mailbox: BLOCKED_FOLDER.into(), ..source("1") };
        assert!(store.ingest(&raw, &at).is_err());
        let mut store = store_with_work();
        break_schema(&store, "CREATE TRIGGER no_blocks BEFORE UPDATE OF blocked ON messages BEGIN SELECT RAISE(FAIL, 'full'); END");
        assert!(store.ingest(&raw, &at).is_err());
        assert!(blocked(&store, None).is_empty());
    }

    #[test]
    fn an_address_without_a_domain_is_handled() {
        let mut store = store_with_work();
        assert_eq!(store.sender_status("work", "nobody").unwrap(), SenderStatus::New);
        store.unblock_sender("work", "nobody", NOV_1).unwrap();
        assert_eq!(store.sender_status("work", "nobody").unwrap(), SenderStatus::Known);
    }

    #[test]
    fn says_which_mail_the_server_still_has_in_the_wrong_folder() {
        let mut store = store_with_work();
        from(&mut store, "1:1", JORDAN);
        from(&mut store, "1:2", "Alex <alex@seo-boost.example>");
        in_blocked_folder(&mut store, "5:1", &mail().id("z@x").from("Zed <zed@spam.biz>").build());
        store.block_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        assert_eq!(store.blocked_in("work", "INBOX").unwrap(), ["1:1"]);
        assert!(store.unblocked_in("work", BLOCKED_FOLDER).unwrap().is_empty());
        store.unblock_sender("work", "zed@spam.biz", NOV_1).unwrap();
        assert_eq!(store.unblocked_in("work", BLOCKED_FOLDER).unwrap(), ["5:1"]);
        assert!(store.blocked_in("work", "Nowhere").unwrap().is_empty());
    }

    #[test]
    fn blocking_needs_a_known_account_and_a_working_database() {
        let mut store = store_with_work();
        assert!(store.block_sender("nobody", "jordan@growthleads.example", false, NOV_1).is_err());
        assert!(store.accept_sender("nobody", "jordan@growthleads.example", false, NOV_1).is_err());
        let thread = from(&mut store, "a", JORDAN);
        break_schema(&store, "ALTER TABLE accounts RENAME TO old_accounts");
        assert!(store.sender_status("work", "jordan@growthleads.example").is_err());
        assert!(store.thread(&thread, NOV_1).is_err());
        let mut store = store_with_work();
        break_schema(&store, "DROP TABLE sender_rules");
        assert!(store.sender_status("work", "x@y.z").is_err());
        assert!(store.block_sender("work", "x@y.z", false, NOV_1).is_err());
        assert!(store.unblock_sender("work", "x@y.z", NOV_1).is_err());
        assert!(store.accept_sender("work", "x@y.z", true, NOV_1).is_err());
        assert!(store.sender_rules().is_err());
        assert!(store.forget_sender_rule("work", "x@y.z").is_err());
        assert!(store.blocked_in("work", "INBOX").is_ok());
        let mut store = store_with_work();
        let thread = from(&mut store, "a", JORDAN);
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN blocked TO b");
        assert!(store.thread(&thread, NOV_1).is_err());
        let store = store_with_work();
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN from_me TO mine");
        assert!(store.sender_status("work", "x@y.z").is_err());
        let store = store_with_work();
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN from_email TO sender");
        assert!(store.sender_status("work", "x@y.z").is_err());
        let store = store_with_work();
        break_schema(&store, "ALTER TABLE message_locations RENAME TO old_locations");
        assert!(store.blocked_in("work", "INBOX").is_err());
        assert!(store.unblocked_in("work", BLOCKED_FOLDER).is_err());
        assert!(store.list_blocked(None, 1, NOV_1).is_ok());
    }
}
