//! Storing raw messages: parse, dedupe, thread, index. All in one transaction, so a crash
//! never leaves a message half stored or missing from search.

use std::collections::BTreeSet;

use moss_core::{MailboxRole, MessageSource};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;

use crate::bodies::{self, Body, Offline};
use crate::codec::{name, now_ms, to_json};
use crate::parse::{parse, Parsed};
use crate::{addresses, authres, classify, content, details, people, senders, text, Error, Result, Store};

/// Characters of preview kept for each message.
pub const PREVIEW: usize = 150;

/// Replies that only match on subject and people must be this close in time.
const SUBJECT_MATCH_WINDOW: i64 = 30 * 86_400_000;

/// What [`Store::ingest`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ingested {
    pub message_id: String,
    pub thread_id: String,
    /// False when the message was stored already (from this mailbox or another).
    pub created: bool,
}

impl Store {
    /// Stores a raw RFC 822 message. Safe to call again with the same message: it is stored
    /// once per account, however many mailboxes it turns up in, and local state such as
    /// read or done is never reset.
    pub fn ingest(&mut self, raw: &[u8], source: &MessageSource) -> Result<Ingested> {
        let offline = self.offline()?;
        let tx = self.conn.transaction()?;
        let (address, provider): (String, String) = tx
            .query_row(
                "SELECT address, provider FROM accounts WHERE id = ?1",
                [&source.account_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::UnknownAccount(source.account_id.clone()))?;
        let server_keeps = content::server_keeps(&provider);
        tx.execute(
            "INSERT OR IGNORE INTO mailboxes (account_id, name) VALUES (?1, ?2)",
            params![source.account_id, source.mailbox],
        )?;
        let (mailbox_id, role): (i64, Option<String>) = tx.query_row(
            "SELECT id, role FROM mailboxes WHERE account_id = ?1 AND name = ?2",
            params![source.account_id, source.mailbox],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let located: Option<i64> = tx
            .query_row(
                "SELECT message_id FROM message_locations WHERE mailbox_id = ?1 AND provider_id = ?2",
                params![mailbox_id, source.provider_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = located {
            return existing(&tx, id);
        }

        let mut parsed = parse(raw).ok_or(Error::NotEmail)?;
        if source.headers_only {
            // Whatever a header block parses to, there's no body yet.
            parsed.body_text.clear();
            parsed.body_html = None;
            parsed.attachments.clear();
        }
        let known: Option<i64> = tx
            .query_row(
                "SELECT id FROM messages WHERE account_id = ?1 AND message_id = ?2",
                params![source.account_id, parsed.message_id],
                |r| r.get(0),
            )
            .optional()?;
        // A copy that is yours: in a Sent folder, or the one Moss keeps of what it sent. Nobody
        // else can put mail there, so whoever its From line names, it comes from you.
        let own_copy = role.as_deref() == Some(name(&MailboxRole::Sent).as_str()) || !content::on_server(&source.provider_id);
        if let Some(id) = known {
            if own_copy {
                tx.execute("UPDATE messages SET sender_verified = 1 WHERE id = ?1", [id])?;
            }
            let inbox = role.as_deref() == Some(name(&MailboxRole::Inbox).as_str());
            // New in the Inbox: put back from Gmail, a phone or Moss's own Not done or undo of
            // a delete, so open again.
            if locate(&tx, id, mailbox_id, &source.provider_id)? && inbox {
                tx.execute(
                    "UPDATE messages SET done = 0, done_at = NULL, deleted_from = NULL WHERE id = ?1",
                    [id],
                )?;
            }
            let account = &source.account_id;
            let mine = parsed.from.email.eq_ignore_ascii_case(&address)
                || addresses::is_added(&tx, account, &parsed.from.email)?;
            senders::on_ingest(&tx, account, id, &parsed.from.email, mine, &source.mailbox, now_ms())?;
            if !source.headers_only {
                fill(&tx, id, &parsed, offline_for(&tx, id, offline)?.unwrap_or_default())?;
            }
            // Mail sent from here: once the server has its copy, this device needn't.
            if server_keeps && content::on_server(&source.provider_id) {
                tx.execute("DELETE FROM message_raw WHERE message_id = ?1", [id])?;
            }
            let out = existing(&tx, id)?;
            tx.commit()?;
            return Ok(out);
        }

        let role = |r: MailboxRole| role.as_deref() == Some(name(&r).as_str());
        // From the account's own address, another you added, or filed in Sent.
        let from_me = parsed.from.email.eq_ignore_ascii_case(&address)
            || role(MailboxRole::Sent)
            || addresses::is_added(&tx, &source.account_id, &parsed.from.email)?;
        let sent_at = parsed.sent_at.or(source.received_at).unwrap_or_else(now_ms);
        let received_at = source.received_at.unwrap_or(sent_at);
        let thread_id = find_thread(&tx, &source.account_id, &parsed, sent_at)?;
        let body_stripped = parsed.body_stripped();
        // Written to any of your addresses, or one at your own domain (a catch-all), is to you.
        let yours = addresses::yours(&tx, &source.account_id)?;
        let mine = |email: &str| {
            classify::same_address(email, &address)
                || yours.came_to(&[email.trim().to_lowercase()]).is_some()
        };
        let (split, section) = classify::classify(&parsed, mine, from_me);
        // Whether it comes from whoever its From line names; only then does trusting that
        // address show its images. Decided now: the headers that say so aren't kept.
        let confirmed = own_copy
            || (parsed.single_from
                && authres::sender_confirmed(&provider, &address, &parsed.authentication_results, &parsed.from.email));
        tx.execute(
            "INSERT INTO messages (account_id, thread_id, message_id, subject, from_name, from_email, to_json, cc_json,
                 bcc_json, participants, sent_at, received_at, snippet, from_me, split, section, read, done, flagged,
                 headers_only, forwarded, sender_verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                source.account_id,
                thread_id,
                parsed.message_id,
                parsed.subject,
                parsed.from.name,
                parsed.from.email,
                to_json(&parsed.to),
                to_json(&parsed.cc),
                to_json(&parsed.bcc),
                parsed.participants_text(),
                sent_at,
                received_at,
                text::snippet(&body_stripped, PREVIEW),
                from_me,
                name(&split),
                name(&section),
                // Your own copy is read and Done. Mail elsewhere that only says it's from you
                // arrives like any other: anyone can write your address on the From line.
                own_copy || source.seen,
                // Only in All Mail or Archive means archived: done, out of the list.
                own_copy
                    || [MailboxRole::Trash, MailboxRole::Junk, MailboxRole::All, MailboxRole::Archive]
                        .into_iter()
                        .any(role),
                source.flagged,
                source.headers_only,
                source.forwarded,
                confirmed,
            ],
        )?;
        let id = tx.last_insert_rowid();
        for r in &parsed.refs {
            tx.execute(
                "INSERT INTO message_refs (message_id, ref) VALUES (?1, ?2)",
                params![id, r],
            )?;
        }
        for label in &source.labels {
            tx.execute(
                "INSERT OR IGNORE INTO message_labels (message_id, label) VALUES (?1, ?2)",
                params![id, label],
            )?;
        }
        add_attachments(&tx, id, &parsed)?;
        index(&tx, id, &parsed, &body_stripped)?;
        people::count(&tx, &source.account_id, &parsed, from_me, sent_at)?;
        if !source.headers_only {
            if offline.keeps_text(sent_at, now_ms()) {
                keep_body(&tx, id, &parsed, true)?;
            }
            if offline.keeps_files() {
                keep_images(&tx, id, &parsed, true)?;
            }
        }
        // The server is the archive; keep the original only where no server has it.
        if !server_keeps || !content::on_server(&source.provider_id) {
            tx.execute("INSERT INTO message_raw (message_id, raw) VALUES (?1, ?2)", params![id, raw])?;
        }
        locate(&tx, id, mailbox_id, &source.provider_id)?;
        let account = &source.account_id;
        senders::on_ingest(&tx, account, id, &parsed.from.email, from_me, &source.mailbox, now_ms())?;
        tx.execute(
            "UPDATE threads SET last_message_at = (SELECT max(sent_at) FROM messages WHERE thread_id = ?1) WHERE id = ?1",
            [thread_id],
        )?;
        tx.commit()?;
        Ok(Ingested { message_id: id.to_string(), thread_id: thread_id.to_string(), created: true })
    }
}

impl Store {
    /// Puts a body fetched from the server into a message: one stored with headers only
    /// (which gets its preview, index and details now), or one whose body isn't on this
    /// device. The offline setting decides whether it's kept for offline use; otherwise
    /// it's cache. Returns false when the body was here already.
    pub fn fill_in(&mut self, message_id: &str, raw: &[u8]) -> Result<bool> {
        let parsed = parse(raw).ok_or(Error::NotEmail)?;
        let id = crate::codec::parse_id(message_id);
        let offline = self.offline()?;
        let tx = self.conn.transaction()?;
        let Some(policy) = offline_for(&tx, id, offline)? else {
            return Ok(false);
        };
        let filled = fill(&tx, id, &parsed, policy)?;
        let here: bool =
            tx.query_row("SELECT EXISTS (SELECT 1 FROM bodies WHERE message_id = ?1)", [id], |r| r.get(0))?;
        let cached = !here && {
            keep_body(&tx, id, &parsed, false)?;
            true
        };
        // The images it shows, so it opens at once next time.
        if !policy.1 {
            keep_images(&tx, id, &parsed, false)?;
        }
        tx.commit()?;
        Ok(filled || cached)
    }
}

/// The offline setting as it applies to message `id`: whether its text is kept, and
/// whether its files are. `None` when the message isn't stored.
fn offline_for(tx: &Transaction, id: i64, offline: Offline) -> Result<Option<(bool, bool)>> {
    let sent_at: Option<i64> =
        tx.query_row("SELECT sent_at FROM messages WHERE id = ?1", [id], |r| r.get(0)).optional()?;
    Ok(sent_at.map(|at| (offline.keeps_text(at, now_ms()), offline.keeps_files())))
}

/// Fills in a message stored with headers only from its whole copy: preview, index,
/// details, attachments, and its body if the offline setting keeps it. Returns false
/// when it had more than headers already.
fn fill(tx: &Transaction, id: i64, parsed: &Parsed, (text, files): (bool, bool)) -> Result<bool> {
    let body_stripped = parsed.body_stripped();
    let changed = tx.execute(
        "UPDATE messages SET snippet = ?2, headers_only = 0 WHERE id = ?1 AND headers_only = 1",
        params![id, text::snippet(&body_stripped, PREVIEW)],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    add_attachments(tx, id, parsed)?;
    index(tx, id, parsed, &body_stripped)?;
    if text {
        keep_body(tx, id, parsed, true)?;
    }
    if files {
        keep_images(tx, id, parsed, true)?;
    }
    Ok(true)
}

/// Keeps a message's body as offline mail or as cache.
fn keep_body(tx: &Transaction, id: i64, parsed: &Parsed, offline: bool) -> Result<()> {
    let body = Body { text: parsed.body_text.clone(), html: parsed.body_html.clone() };
    bodies::keep(tx, id, &body, offline)
}

/// Keeps the small inline images that came with a message, as offline mail or as cache.
/// Other files come from the server when opened.
fn keep_images(tx: &Transaction, id: i64, parsed: &Parsed, offline: bool) -> Result<()> {
    for (position, a) in parsed.attachments.iter().enumerate() {
        if let Some(data) = &a.kept {
            bodies::keep_file(tx, id, position as i64, data, offline)?;
        }
    }
    Ok(())
}

/// The words of a message, for keyword search, and the details rules find in it. Only
/// which words are where is kept, not the text.
fn index(tx: &Transaction, id: i64, parsed: &Parsed, body_stripped: &str) -> Result<()> {
    tx.execute("DELETE FROM message_search WHERE rowid = ?1", [id])?;
    tx.execute(
        "INSERT INTO message_search (rowid, subject, participants, body) VALUES (?1, ?2, ?3, ?4)",
        params![id, parsed.subject, parsed.participants_text(), body_stripped],
    )?;
    details::find(tx, id, &parsed.subject, body_stripped)
}

fn add_attachments(tx: &Transaction, id: i64, parsed: &Parsed) -> Result<()> {
    for (position, a) in parsed.attachments.iter().enumerate() {
        tx.execute(
            "INSERT OR IGNORE INTO attachments (message_id, position, name, content_type, bytes, content_id, inline, part)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, position as i64, a.name, a.content_type, a.bytes as i64, a.content_id, a.inline, a.part],
        )?;
    }
    Ok(())
}

fn existing(tx: &Transaction, id: i64) -> Result<Ingested> {
    let thread_id: i64 =
        tx.query_row("SELECT thread_id FROM messages WHERE id = ?1", [id], |r| r.get(0))?;
    Ok(Ingested { message_id: id.to_string(), thread_id: thread_id.to_string(), created: false })
}

/// Notes where a message is on the server. Whether that place is new.
fn locate(tx: &Transaction, id: i64, mailbox_id: i64, provider_id: &str) -> Result<bool> {
    Ok(tx.execute(
        "INSERT OR IGNORE INTO message_locations (message_id, mailbox_id, provider_id) VALUES (?1, ?2, ?3)",
        params![id, mailbox_id, provider_id],
    )? > 0)
}

/// The thread a new message joins, made if need be. Headers come first: any stored message
/// it references, or that references it (when a reply arrived before its parent). Threads
/// it links together are merged. Failing that, a reply joins a recent thread with the same
/// subject and people.
fn find_thread(tx: &Transaction, account_id: &str, parsed: &Parsed, sent_at: i64) -> Result<i64> {
    let mut linked = BTreeSet::new();
    for r in &parsed.refs {
        let mut stmt = tx.prepare_cached(
            "SELECT thread_id FROM messages WHERE account_id = ?1 AND message_id = ?2",
        )?;
        linked.extend(
            stmt.query_map(params![account_id, r], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    let mut stmt = tx.prepare_cached(
        "SELECT m.thread_id FROM message_refs r JOIN messages m ON m.id = r.message_id
         WHERE r.ref = ?2 AND m.account_id = ?1",
    )?;
    linked.extend(
        stmt.query_map(params![account_id, parsed.message_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );

    if let Some(&keep) = linked.first() {
        for other in linked.iter().skip(1) {
            tx.execute("UPDATE messages SET thread_id = ?1 WHERE thread_id = ?2", [keep, *other])?;
            tx.execute("UPDATE drafts SET thread_id = ?1 WHERE thread_id = ?2", [keep, *other])?;
            // Completed send receipts follow the surviving conversation in this same commit.
            tx.execute("UPDATE deliveries SET thread_id=?1 WHERE thread_id=?2 AND account_id=?3 AND account_retired=0", params![keep.to_string(),other.to_string(),account_id])?;
            let stale: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM deliveries WHERE thread_id=?1 AND account_id=?2 AND account_retired=0)", params![other.to_string(),account_id], |row| row.get(0))?;
            if stale { return Err(Error::Invalid("A send receipt could not follow the merged conversation.".into())); }

            // Preserve the existing order, not the merge time, of the latest manual choice.
            tx.execute(
                "INSERT OR REPLACE INTO snooze_edits (id, thread_id)
                 SELECT max(id), ?1 FROM snooze_edits WHERE thread_id IN (?1, ?2) HAVING count(*) > 0",
                [keep, *other],
            )?;
            tx.execute("DELETE FROM threads WHERE id = ?1", [*other])?;
        }
        return Ok(keep);
    }

    let subject_key = text::subject_key(&parsed.subject);
    let participants_key = parsed.participants_key();
    let is_reply = !parsed.refs.is_empty() || text::base_subject(&parsed.subject) != parsed.subject;
    if is_reply && !subject_key.is_empty() {
        let matched: Option<i64> = tx
            .query_row(
                "SELECT id FROM threads WHERE account_id = ?1 AND subject_key = ?2 AND participants_key = ?3
                 AND abs(last_message_at - ?4) <= ?5 ORDER BY last_message_at DESC LIMIT 1",
                params![account_id, subject_key, participants_key, sent_at, SUBJECT_MATCH_WINDOW],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = matched {
            return Ok(id);
        }
    }
    tx.execute(
        "INSERT INTO threads (account_id, subject, subject_key, participants_key, last_message_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            account_id,
            text::base_subject(&parsed.subject),
            subject_key,
            participants_key,
            sent_at
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{Mailbox, Section};

    fn count(store: &Store, sql: &str) -> u32 {
        store.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn in_mailbox(store: &mut Store, mailbox: &str, provider_id: &str, raw: &[u8]) -> Ingested {
        let source = MessageSource { mailbox: mailbox.into(), ..source(provider_id) };
        store.ingest(raw, &source).unwrap()
    }

    #[test]
    fn stores_a_message_once_however_often_it_arrives() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1", &mail().build());
        assert!(first.created);
        let again = ingest(&mut store, "1", &mail().build());
        assert_eq!(again, Ingested { created: false, ..first.clone() });
        let elsewhere = in_mailbox(&mut store, "[Gmail]/All Mail", "900", &mail().build());
        assert_eq!(elsewhere, Ingested { created: false, ..first });
        assert_eq!(count(&store, "SELECT count(*) FROM messages"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM message_locations"), 2);
        assert_eq!(count(&store, "SELECT count(*) FROM message_search"), 1);
        assert!(store.contains("work", "[Gmail]/All Mail", "900").unwrap());
    }

    #[test]
    fn keeps_local_state_when_a_message_arrives_again() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1", &mail().build());
        store.mark_done(&first.thread_id, true, NOV_1).unwrap();
        in_mailbox(&mut store, "Archive", "7", &mail().build());
        let thread = store.thread(&first.thread_id, NOV_1).unwrap().unwrap();
        assert!(thread.summary.done);
    }

    #[test]
    fn refuses_unknown_accounts_and_non_email_without_storing_anything() {
        let mut store = store_with_work();
        let stranger = MessageSource { account_id: "nobody".into(), ..source("1") };
        let err = store.ingest(&mail().build(), &stranger).unwrap_err();
        assert!(matches!(err, Error::UnknownAccount(ref id) if id == "nobody"));
        assert_eq!(err.to_string(), "no account with id nobody");
        let err = store.ingest(b"", &source("2")).unwrap_err();
        assert!(matches!(err, Error::NotEmail));
        assert_eq!(count(&store, "SELECT count(*) FROM mailboxes"), 0);
        assert_eq!(count(&store, "SELECT count(*) FROM messages"), 0);
    }

    #[test]
    fn records_who_what_and_when() {
        let mut store = store_with_work();
        let raw = mail().attach("Q4.pdf", "application/pdf", "PDFDATA").build();
        let got = store
            .ingest(
                &raw,
                &MessageSource {
                    received_at: Some(NOV_1 + 5_000),
                    flagged: true,
                    labels: vec!["Finance".into()],
                    ..source("1")
                },
            )
            .unwrap();
        let (sent, received, flagged, read, done, split, section): (i64, i64, bool, bool, bool, String, String) = store
            .conn
            .query_row(
                "SELECT sent_at, received_at, flagged, read, done, split, section FROM messages WHERE id = ?1",
                [got.message_id.parse::<i64>().unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .unwrap();
        assert_eq!((sent, received), (NOV_1, NOV_1 + 5_000));
        assert!(flagged && !read && !done);
        assert_eq!((split.as_str(), section.as_str()), ("important", "reply"));
        assert_eq!(count(&store, "SELECT count(*) FROM attachments"), 1);
        // The server has the original.
        assert_eq!(store.raw_message(&got.message_id).unwrap(), None);
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.summary.labels, ["Finance"]);
        assert_eq!(thread.summary.subject, "Q4 budget");
        assert_eq!(thread.summary.last_message_at, NOV_1);
    }

    #[test]
    fn dates_fall_back_to_received_then_now() {
        let mut store = store_with_work();
        let undated = mail().without("Date");
        let got = store
            .ingest(
                &undated.clone().id("a@x").build(),
                &MessageSource { received_at: Some(42), ..source("1") },
            )
            .unwrap();
        assert_eq!(store.thread(&got.thread_id, 0).unwrap().unwrap().messages[0].sent_at, 42);
        let got = ingest(&mut store, "2", &undated.id("b@x").build());
        let sent_at = store.thread(&got.thread_id, 0).unwrap().unwrap().messages[0].sent_at;
        assert!(sent_at > NOV_1 - 400 * DAY);
    }

    fn with_folders(store: &mut Store) {
        for (name, role) in [("INBOX", MailboxRole::Inbox), ("Sent", MailboxRole::Sent)] {
            store.upsert_mailbox(&Mailbox { account_id: "work".into(), name: name.into(), role: Some(role) }).unwrap();
        }
    }

    #[test]
    fn your_own_copies_are_read_done_and_informational() {
        let mut store = store_with_work();
        with_folders(&mut store);
        // Your Sent folder: sent from another mail app, from any address of yours.
        for (id, from) in [("sent@x", "Maya Ross <Maya@Northwind.co>"), ("alias@x", "Maya <maya@alias.co>")] {
            let got = in_mailbox(&mut store, "Sent", id, &mail().id(id).from(from).to("daniel@northwind.co").build());
            let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
            assert!(thread.summary.done && !thread.summary.unread, "{from}");
            assert_eq!(thread.summary.section, Section::Info);
        }
        // The copy Moss keeps of what it sent.
        let kept = mail().id("local@x").from("Maya Ross <maya@northwind.co>").subject("Sent from Moss").build();
        let got = ingest(&mut store, "local:<local@x>", &kept);
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(thread.summary.done && !thread.summary.unread);
    }

    #[test]
    fn mail_in_the_inbox_that_says_it_is_from_you_arrives_like_any_other() {
        let mut store = store_with_work();
        with_folders(&mut store);
        // Anyone can write your address on the From line: it is not hidden from you.
        let forged = mail().from("Maya Ross <maya@northwind.co>").to("maya@northwind.co").subject("Your account").build();
        let got = ingest(&mut store, "1", &forged);
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(!thread.summary.done && thread.summary.unread);
        let seen = mail().id("seen@x").from("Maya Ross <maya@northwind.co>").subject("Seen").build();
        let got = store.ingest(&seen, &MessageSource { seen: true, ..source("2") }).unwrap();
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(!thread.summary.done && !thread.summary.unread, "read as the server says");
    }

    #[test]
    fn an_email_you_send_to_yourself_or_a_list_you_are_on_shows_in_the_inbox() {
        for inbox_first in [true, false] {
            let mut store = store_with_work();
            with_folders(&mut store);
            let to_list = mail().id("list@x").from("Maya Ross <maya@northwind.co>").to("team@northwind.co").subject("Team lunch").build();
            let copies = [("INBOX", "7"), ("Sent", "8")];
            let order: Vec<_> = if inbox_first { copies.to_vec() } else { copies.iter().rev().copied().collect() };
            let mut thread = String::new();
            for (mailbox, id) in order {
                thread = in_mailbox(&mut store, mailbox, id, &to_list).thread_id;
            }
            let summary = store.thread(&thread, NOV_1).unwrap().unwrap().summary;
            assert!(!summary.done, "inbox first: {inbox_first}");
        }
    }

    #[test]
    fn provider_flags_and_folders_set_the_starting_state() {
        let mut store = store_with_work();
        let seen =
            store.ingest(&mail().build(), &MessageSource { seen: true, ..source("1") }).unwrap();
        assert!(!store.thread(&seen.thread_id, NOV_1).unwrap().unwrap().summary.unread);
        for (name, role) in [("Trash", MailboxRole::Trash), ("Spam", MailboxRole::Junk)] {
            store
                .upsert_mailbox(&Mailbox {
                    account_id: "work".into(),
                    name: name.into(),
                    role: Some(role),
                })
                .unwrap();
            let got = in_mailbox(&mut store, name, "1", &mail().id(&format!("{name}@x")).build());
            assert!(store.thread(&got.thread_id, NOV_1).unwrap().unwrap().summary.done, "{name}");
        }
    }

    #[test]
    fn archived_mail_arrives_done_but_inbox_mail_stays_open() {
        let mut store = store_with_work();
        for (name, role) in
            [("[Gmail]/All Mail", MailboxRole::All), ("Archive", MailboxRole::Archive)]
        {
            store
                .upsert_mailbox(&Mailbox {
                    account_id: "work".into(),
                    name: name.into(),
                    role: Some(role),
                })
                .unwrap();
            let got = in_mailbox(&mut store, name, "1", &mail().id(&format!("{name}@x")).build());
            assert!(store.thread(&got.thread_id, NOV_1).unwrap().unwrap().summary.done, "{name}");
        }
        // Already here from the Inbox: seeing it in All Mail too leaves it open.
        let open = ingest(&mut store, "1", &mail().build());
        in_mailbox(&mut store, "[Gmail]/All Mail", "2", &mail().build());
        assert!(!store.thread(&open.thread_id, NOV_1).unwrap().unwrap().summary.done);
    }

    #[test]
    fn mail_put_back_in_the_inbox_elsewhere_brings_its_thread_back() {
        let mut store = store_with_work();
        store
            .upsert_mailbox(&Mailbox { account_id: "work".into(), name: "INBOX".into(), role: Some(MailboxRole::Inbox) })
            .unwrap();
        let got = ingest(&mut store, "1", &mail().build());
        store.mark_done(&got.thread_id, true, NOV_1).unwrap();
        // The same place again (a later sync): still done.
        ingest(&mut store, "1", &mail().build());
        assert!(store.thread(&got.thread_id, NOV_1).unwrap().unwrap().summary.done);
        // Back in the Inbox under a new id, from Gmail, a phone or Moss's own Not done: open again.
        ingest(&mut store, "2", &mail().build());
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(!thread.summary.done);
        assert_eq!(thread.summary.done_at, None);
        // Deleted, then restored from the Bin in Gmail or undone in Moss: open again.
        store.delete_thread(&got.thread_id).unwrap();
        ingest(&mut store, "3", &mail().build());
        let listed = store.list_threads(None, thread.summary.split, NOV_1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(store.restore_thread(&got.thread_id).unwrap(), None, "nothing left deleted");
        // A new place in another folder doesn't count.
        store.mark_done(&got.thread_id, true, NOV_1).unwrap();
        in_mailbox(&mut store, "Receipts", "9", &mail().build());
        assert!(store.thread(&got.thread_id, NOV_1).unwrap().unwrap().summary.done);
    }

    #[test]
    fn replies_join_their_thread_by_headers() {
        let mut store = store_with_work();
        let parent = ingest(&mut store, "1", &mail().build());
        let reply = mail()
            .id("r1@x")
            .subject("Different subject")
            .reply_to(&["q4@northwind.co"])
            .at(NOV_1 + DAY);
        let got = ingest(&mut store, "2", &reply.build());
        assert_eq!(got.thread_id, parent.thread_id);
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.summary.message_count, 2);
        assert_eq!(thread.summary.last_message_at, NOV_1 + DAY);
        assert_eq!(thread.summary.subject, "Q4 budget");
    }

    #[test]
    fn a_parent_that_arrives_late_joins_its_replies() {
        let mut store = store_with_work();
        let reply = ingest(
            &mut store,
            "2",
            &mail().id("r1@x").subject("Re: Q4 budget").reply_to(&["q4@northwind.co"]).build(),
        );
        let parent = ingest(&mut store, "1", &mail().build());
        assert_eq!(parent.thread_id, reply.thread_id);
    }

    #[test]
    fn a_message_that_links_two_threads_merges_them() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1", &mail().id("a@x").subject("Alpha").build());
        let b = ingest(&mut store, "2", &mail().id("b@x").subject("Beta").build());
        assert_ne!(a.thread_id, b.thread_id);
        let mut draft = draft("d1");
        draft.thread_id = Some(b.thread_id.clone());
        store.save_draft(&draft).unwrap();
        let both = ingest(
            &mut store,
            "3",
            &mail().id("c@x").subject("Re: Alpha").header("References", "<a@x> <b@x>").build(),
        );
        assert_eq!(both.thread_id, a.thread_id);
        assert_eq!(count(&store, "SELECT count(*) FROM threads"), 1);
        assert_eq!(store.thread(&a.thread_id, NOV_1).unwrap().unwrap().messages.len(), 3);
        assert_eq!(store.draft("d1").unwrap().unwrap().thread_id, Some(a.thread_id));
    }

    #[test]
    fn thread_merges_keep_completed_delivery_receipts_attached_atomically() {
        for mode in ["none", "abort", "ignore", "retired"] {
            let mut store=store_with_work();
            let a=ingest(&mut store,"receipt-a",&mail().id("receipt-a@example.test").subject("Alpha").build());
            let b=ingest(&mut store,"receipt-b",&mail().id("receipt-b@example.test").subject("Beta").build());
            let local=draft("receipt-draft");
            store.prepare_delivery("receipt-operation",&local,"{}","<receipt-b@example.test>",NOV_1).unwrap();
            store.claim_delivery("receipt-operation").unwrap();
            store.accept_delivery("receipt-operation",b"accepted","{}").unwrap();
            store.complete_delivery_with_receipt("receipt-operation",Some(&b.thread_id)).unwrap();
            if mode == "retired" { store.conn.execute("UPDATE deliveries SET account_retired=1 WHERE id='receipt-operation'", []).unwrap(); }
            if mode != "none" {
                let refusal=if mode=="abort" || mode=="retired" {"RAISE(ABORT,'receipt write failed')"} else {"RAISE(IGNORE)"};
                store.conn.execute_batch(&format!("CREATE TRIGGER refuse_receipt_merge BEFORE UPDATE OF thread_id ON deliveries BEGIN SELECT {refusal}; END")).unwrap();
            }
            let raw=mail().id("receipt-link@example.test").subject("Re: Alpha").header("References","<receipt-a@example.test> <receipt-b@example.test>").build();
            let result=store.ingest(&raw,&source("receipt-link"));
            if mode == "abort" || mode == "ignore" {
                assert!(result.is_err());
                assert_eq!(count(&store,"SELECT count(*) FROM threads"),2);
                assert_eq!(store.thread(&a.thread_id,NOV_1).unwrap().unwrap().messages.len(),1);
                assert_eq!(store.thread(&b.thread_id,NOV_1).unwrap().unwrap().messages.len(),1);
                assert_eq!(store.delivery("receipt-operation").unwrap().unwrap().thread_id,Some(b.thread_id));
            } else {
                assert_eq!(result.unwrap().thread_id,a.thread_id);
                assert_eq!(store.delivery("receipt-operation").unwrap().unwrap().thread_id,Some(if mode == "retired" { b.thread_id } else { a.thread_id }));
            }
        }
    }

    #[test]
    fn replies_without_headers_join_by_subject_and_people() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1", &mail().build());
        let back = mail()
            .id("r@x")
            .subject("RE: q4  budget")
            .from("maya@northwind.co")
            .to("Daniel <daniel@northwind.co>");
        assert_eq!(
            ingest(&mut store, "2", &back.at(NOV_1 + DAY).build()).thread_id,
            first.thread_id
        );

        let other_people = mail().id("o@x").subject("Re: Q4 budget").from("lena@northwind.co");
        assert_ne!(ingest(&mut store, "3", &other_people.build()).thread_id, first.thread_id);
        let not_a_reply = mail().id("n@x");
        assert_ne!(ingest(&mut store, "4", &not_a_reply.build()).thread_id, first.thread_id);
        let much_later = mail().id("l@x").subject("Re: Q4 budget").at(NOV_1 + 40 * DAY);
        assert_ne!(ingest(&mut store, "5", &much_later.build()).thread_id, first.thread_id);
        let unknown_parent = mail().id("u@x").reply_to(&["missing@x"]).at(NOV_1 + 2 * DAY);
        assert_eq!(ingest(&mut store, "6", &unknown_parent.build()).thread_id, first.thread_id);
        let empty = mail().id("e@x").subject("Re:");
        let e1 = ingest(&mut store, "7", &empty.build()).thread_id;
        assert_ne!(ingest(&mut store, "8", &empty.id("e2@x").build()).thread_id, e1);
    }

    #[test]
    fn threads_belong_to_one_account() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let work = ingest(&mut store, "1", &mail().build());
        let personal = store
            .ingest(
                &mail().build(),
                &MessageSource { account_id: "personal".into(), ..source("1") },
            )
            .unwrap();
        assert!(personal.created);
        assert_ne!(personal.thread_id, work.thread_id);
        let reply = mail().id("r@x").reply_to(&["q4@northwind.co"]).build();
        let got = store
            .ingest(&reply, &MessageSource { account_id: "personal".into(), ..source("2") })
            .unwrap();
        assert_eq!(got.thread_id, personal.thread_id);
    }

    #[test]
    fn a_parent_never_joins_replies_in_another_account() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let reply = mail().id("r@x").reply_to(&["q4@northwind.co"]).build();
        let personal = store
            .ingest(&reply, &MessageSource { account_id: "personal".into(), ..source("1") })
            .unwrap();
        let work = ingest(&mut store, "1", &mail().build());
        assert_ne!(work.thread_id, personal.thread_id);
    }

    #[test]
    fn search_indexes_the_new_text_only() {
        let mut store = store_with_work();
        ingest(
            &mut store,
            "1",
            &mail().body("Signed off.\n\nOn Sun, Daniel wrote:\n> pineapple").build(),
        );
        assert_eq!(store.search("signed", None, 10, NOV_1).unwrap().len(), 1);
        assert!(store.search("pineapple", None, 10, NOV_1).unwrap().is_empty());
    }

    /// The header block only, as IMAP `BODY[HEADER]` returns it.
    fn headers_of(raw: &[u8]) -> Vec<u8> {
        let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        raw[..end].to_vec()
    }

    fn headers_only(mailbox: &str, provider_id: &str) -> MessageSource {
        MessageSource { mailbox: mailbox.into(), headers_only: true, ..source(provider_id) }
    }

    fn your_reply() -> Mail {
        mail()
            .id("r@x")
            .from("Maya Ross <maya@northwind.co>")
            .to("daniel@northwind.co")
            .reply_to(&["q4@northwind.co"])
            .at(NOV_1 + DAY)
            .body("Signed, pineapple.")
            .attach("Q4-signed.pdf", "application/pdf", "PDF")
    }

    #[test]
    fn headers_only_mail_threads_and_counts_as_your_reply_until_its_body_comes() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1", &mail().build());
        let sent = store.ingest(&headers_of(&your_reply().build()), &headers_only("Sent", "5:1")).unwrap();
        assert_eq!(sent.thread_id, first.thread_id);
        let thread = store.thread(&first.thread_id, NOV_1).unwrap().unwrap();
        assert_ne!(thread.summary.section, Section::Reply);
        let reply = &thread.messages[1];
        assert!(reply.headers_only && reply.body_text.is_empty() && reply.attachments.is_empty());
        assert!(!thread.messages[0].headers_only);
        assert!(store.search("pineapple", None, 10, NOV_1).unwrap().is_empty());

        assert!(store.fill_in(&sent.message_id, &your_reply().build()).unwrap());
        let thread = store.thread(&first.thread_id, NOV_1).unwrap().unwrap();
        let reply = &thread.messages[1];
        assert!(!reply.headers_only);
        assert_eq!(reply.body_text, "Signed, pineapple.");
        assert_eq!(reply.attachments[0].name, "Q4-signed.pdf");
        assert_eq!(store.attachment_source(&sent.message_id, 0).unwrap().unwrap().part, "2");
        assert_eq!(store.search("pineapple", None, 10, NOV_1).unwrap().len(), 1);
        // Already whole, or not stored at all: nothing to do.
        assert!(!store.fill_in(&sent.message_id, &your_reply().build()).unwrap());
        assert!(!store.fill_in("999", &your_reply().build()).unwrap());
        assert!(matches!(store.fill_in(&sent.message_id, b""), Err(Error::NotEmail)));
    }

    #[test]
    fn a_whole_copy_from_another_folder_fills_in_a_headers_only_message() {
        let mut store = store_with_work();
        let sent = store.ingest(&headers_of(&your_reply().build()), &headers_only("Sent", "5:1")).unwrap();
        // Another headers-only copy changes nothing.
        store.ingest(&headers_of(&your_reply().build()), &headers_only("Label", "6:1")).unwrap();
        let thread = store.thread(&sent.thread_id, NOV_1).unwrap().unwrap();
        assert!(thread.messages[0].headers_only);
        in_mailbox(&mut store, "[Gmail]/All Mail", "9:1", &your_reply().build());
        let thread = store.thread(&sent.thread_id, NOV_1).unwrap().unwrap();
        assert!(!thread.messages[0].headers_only);
        assert_eq!(thread.messages[0].body_text, "Signed, pineapple.");
    }

    #[test]
    fn a_failed_ingest_leaves_nothing_behind() {
        let mut store = store_with_work();
        break_schema(&store, "CREATE TRIGGER full BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'disk full'); END");
        let err = store.ingest(&mail().build(), &source("1")).unwrap_err();
        assert!(err.to_string().contains("disk full"));
        assert_eq!(count(&store, "SELECT count(*) FROM mailboxes"), 0);
        assert_eq!(count(&store, "SELECT count(*) FROM threads"), 0);
        assert!(store.search("budget", None, 10, NOV_1).unwrap().is_empty());
    }

    #[test]
    fn a_broken_mailbox_table_is_an_error() {
        let mut store = store_with_work();
        break_schema(&store, "ALTER TABLE mailboxes RENAME COLUMN role TO kind");
        assert!(store.ingest(&mail().build(), &source("1")).is_err());
    }
}
