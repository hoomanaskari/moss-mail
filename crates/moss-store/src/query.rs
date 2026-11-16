//! Reading threads back for the UI, in the shapes and order `src/state/store.ts` uses.
//!
//! A thread's state comes from its messages: it is done when every message is done, unread
//! when any is, snoozed ("later") while every open message is snoozed, and its split and
//! section are those of its latest message. A new message therefore brings a done or
//! snoozed thread back, and once you have replied (even from another app) it no longer
//! needs your reply.

use std::collections::HashMap;

use moss_core::{
    Attachment, Forward, ImageTrust, Message, Person, Section, SenderStatus, SplitCounts, SplitId, Thread,
    ThreadSummary,
};
use base64::Engine as _;
use rusqlite::{named_params, Row};

use crate::codec::{json, name, named, now_ms, parse_id};
use crate::parse::{cid_matches, shown_in};
use crate::senders::Senders;
use crate::decisions::{answer_sql, Answer, DEADLINE, EXPECTS_REPLY, SPLIT};
use crate::{addresses, bodies, text, Result, Store};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// The columns of a thread row, from `base_sql`'s facts about it. Parameters: `:now`.
const SUMMARY_COLUMNS: &str = "SELECT id, account_id,
                -- Where you moved it wins over Moss's sorting; a star means Important (moves.rs).
                CASE WHEN starred THEN 'important' ELSE coalesce(moved_to, base_split) END AS split,
                CASE WHEN snoozed > :now THEN 'later' WHEN starred THEN 'reply'
                     -- Outside Important, nothing waits on your reply, as the rules have it.
                     WHEN moved_to IS NOT NULL THEN 'info' ELSE base_section END AS section,
                from_name, from_email, to_json, subject, snippet, last_message_at,
                CASE WHEN snoozed > :now THEN snoozed END, unread, open = 0, labels, message_count, has_attachments,
                from_is_me, CASE WHEN open = 0 THEN done_at END AS done_at, CASE WHEN NOT starred THEN ai_sure END,
                ai_summary, fm_to, fm_cc, ai_summary_why, blocked,
                -- Moved by you: why the model put it elsewhere no longer counts.
                CASE WHEN NOT starred AND moved_to IS NULL THEN split_why END AS split_why, due, waiting, you_wrote_last,
                starred OR moved_to IS NOT NULL AS moved_by_you";

/// Facts about each thread matching `filter` (a condition on `t`, the threads table), for
/// [`SUMMARY_COLUMNS`]. Parameters: `:now`, plus whatever `filter` uses.
fn base_sql(filter: &str) -> String {
    format!(
        "SELECT t.id, t.account_id, t.subject, t.last_message_at, t.moved_to,
                    lm.split AS base_split, lm.snippet, lm.to_json, lm.from_me AS you_wrote_last,
                    -- Your email waited for an answer long enough: it needs you again.
                    CASE WHEN lm.from_me = 1 AND (SELECT CAST(value AS INTEGER) FROM ai_decisions
                        WHERE message_id = lm.id AND question = 'expectsReply' AND acted = 1) <= :now
                        THEN 'reply' ELSE lm.section END AS base_section,
                    EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND flagged = 1) AS starred,
                    coalesce(fm.from_name, lm.from_name) AS from_name,
                    coalesce(fm.from_email, lm.from_email) AS from_email,
                    fm.id IS NULL AS from_is_me, fm.to_json AS fm_to, fm.cc_json AS fm_cc,
                    (SELECT min(coalesce(snoozed_until, 0)) FROM messages WHERE thread_id = t.id AND done = 0) AS snoozed,
                    EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND read = 0) AS unread,
                    EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND done = 0) AS open,
                    (SELECT count(*) FROM messages WHERE thread_id = t.id) AS message_count,
                    (SELECT max(done_at) FROM messages WHERE thread_id = t.id) AS done_at,
                    (SELECT CAST(round(100 * max(yes, 1 - yes)) AS INTEGER) FROM ai_decisions
                        WHERE message_id = lm.id AND question = 'needsReply' AND source = 'model' AND acted = 1) AS ai_sure,
                    {split_why} AS split_why, {due} AS due, {waiting} AS waiting,
                    (SELECT text FROM summaries WHERE thread_id = t.id) AS ai_summary,
                    (SELECT why FROM summaries WHERE thread_id = t.id) AS ai_summary_why,
                    (SELECT json_group_array(label) FROM (SELECT DISTINCT l.label FROM message_labels l
                        JOIN messages m ON m.id = l.message_id WHERE m.thread_id = t.id ORDER BY l.label)) AS labels,
                    EXISTS (SELECT 1 FROM attachments a JOIN messages m ON m.id = a.message_id
                        WHERE m.thread_id = t.id AND a.inline = 0) AS has_attachments,
                    EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND blocked = 1) AS blocked
             FROM threads t
             JOIN messages lm ON lm.id =
                 (SELECT id FROM messages WHERE thread_id = t.id ORDER BY sent_at DESC, id DESC LIMIT 1)
             LEFT JOIN messages fm ON fm.id =
                 (SELECT id FROM messages WHERE thread_id = t.id AND from_me = 0 ORDER BY sent_at DESC, id DESC LIMIT 1)
             WHERE {filter}",
        split_why = answer_sql(SPLIT, "lm.id", "source = 'model'"),
        // The deadline someone else set you, even once you have answered.
        due = answer_sql(DEADLINE, "fm.id", "1"),
        waiting = answer_sql(EXPECTS_REPLY, "lm.id", "1"),
    )
}

/// Thread rows for the threads matching `filter` (a condition on `t`, the threads table).
/// Parameters: `:now`, plus whatever `filter` uses.
fn summary_sql(filter: &str) -> String {
    format!("{SUMMARY_COLUMNS}\n         FROM (\n{}\n         )", base_sql(filter))
}

/// [`summary_sql`] for a list that shows every row it finds: each thread's facts are worked
/// out once and kept, instead of again for each column that uses them.
fn summary_sql_once(filter: &str) -> String {
    format!("WITH base AS MATERIALIZED (\n{}\n) {SUMMARY_COLUMNS} FROM base", base_sql(filter))
}

/// Threads in the list: not done, not deleted, not blocked, in an account (or all of them).
const LISTED: &str = "(:account IS NULL OR t.account_id = :account)
    AND EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND done = 0 AND deleted_from IS NULL)
    AND NOT EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND blocked = 1)";

/// Threads in the Blocked list: any message from a blocked sender.
pub(crate) const BLOCKED: &str = "(:account IS NULL OR t.account_id = :account)
    AND EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND blocked = 1)";

/// Threads in the Done list: every message done, and at least one you received and didn't
/// delete (mail you only sent is stored as done, but you never handled it).
const DONE: &str = "(:account IS NULL OR t.account_id = :account)
    AND NOT EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND done = 0)
    AND EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND from_me = 0 AND deleted_from IS NULL)
    AND NOT EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND blocked = 1)";

/// A thread row, with the To and Cc addresses (lowercase) of its latest email from someone
/// else, to tell which of your addresses it came to.
fn summary(r: &Row) -> rusqlite::Result<(ThreadSummary, Vec<String>)> {
    let mut recipients = Vec::new();
    for idx in [20, 21] {
        if r.get_ref(idx)?.as_str_or_null()?.is_some() {
            let people: Vec<Person> = json(r, idx)?;
            recipients.extend(people.into_iter().map(|p| p.email.trim().to_lowercase()));
        }
    }
    let from_name: String = r.get(4)?;
    let from_email: String = r.get(5)?;
    let to: Vec<Person> = json(r, 6)?;
    let thread = ThreadSummary {
        id: r.get::<_, i64>(0)?.to_string(),
        account_id: r.get(1)?,
        split: named(r, 2)?,
        section: named(r, 3)?,
        // Only you have written: show who it went to.
        from: match (r.get::<_, bool>(16)?, to.into_iter().next()) {
            (true, Some(first)) => first,
            _ => Person { name: from_name, email: from_email },
        },
        subject: r.get(7)?,
        summary: r.get(8)?,
        last_message_at: r.get(9)?,
        snoozed_until: r.get(10)?,
        unread: r.get(11)?,
        done: r.get(12)?,
        done_at: r.get(17)?,
        labels: json(r, 13)?,
        message_count: r.get(14)?,
        has_attachments: r.get(15)?,
        ai_sure: r.get(18)?,
        ai_summary: r.get(19)?,
        ai_summary_why: r.get::<_, Option<String>>(22)?.as_deref().and_then(moss_core::SummaryWhy::from_name),
        to_address: None,
        // Mail blocked for the address it came to has no rule on its sender: it is blocked
        // all the same. Anyone else is worked out by `Store::summaries`.
        sender: if r.get(23)? { SenderStatus::Blocked } else { SenderStatus::Known },
        split_why: answer(r, 24)?.map(Answer::why),
        due: answer(r, 25)?.and_then(Answer::due),
        waiting: answer(r, 26)?.and_then(Answer::waiting),
        you_wrote_last: r.get(27)?,
        moved_by_you: r.get(28)?,
    };
    Ok((thread, recipients))
}

/// A decision the row has, from its JSON object.
fn answer(r: &Row, idx: usize) -> rusqlite::Result<Option<Answer>> {
    match r.get_ref(idx)?.as_str_or_null()? {
        Some(_) => json(r, idx),
        None => Ok(None),
    }
}

/// SQL: the Done list, most recently done first, up to `:limit` threads. Which threads is
/// settled first, so only those are worked out in full.
fn done_sql() -> String {
    let page = format!(
        "t.id IN (SELECT id FROM (SELECT t.id, t.last_message_at,
             (SELECT max(done_at) FROM messages WHERE thread_id = t.id) AS done_at FROM threads t WHERE {DONE})
         ORDER BY done_at IS NULL, done_at DESC, last_message_at DESC, id DESC LIMIT :limit)"
    );
    format!("SELECT * FROM ({}) ORDER BY done_at IS NULL, done_at DESC, last_message_at DESC, id DESC", summary_sql(&page))
}

/// SQL: up to `:limit` threads matching `filter`, newest first, worked out in full only once
/// they're known to be on the page.
fn where_sql(filter: &str) -> String {
    let page = format!("t.id IN (SELECT t.id FROM threads t WHERE {filter} ORDER BY t.last_message_at DESC, t.id DESC LIMIT :limit)");
    format!("SELECT * FROM ({}) ORDER BY last_message_at DESC, id DESC", summary_sql(&page))
}

/// What needs a reply first, then what's for your information, then snoozed. In Important,
/// what you moved there comes first in each. Like `visibleThreads` in the UI.
const SECTION_ORDER: &str = "CASE section WHEN 'reply' THEN 0 WHEN 'info' THEN 1 ELSE 2 END,
    CASE WHEN split = 'important' THEN moved_by_you ELSE 0 END DESC";

/// What thread rows need to know about each account, read once for many rows.
#[derive(Default)]
pub(crate) struct Accounts {
    yours: HashMap<String, addresses::Yours>,
    senders: HashMap<String, Senders>,
}

impl Store {
    /// Thread rows as the UI gets them: each says which of your addresses it came to, and
    /// who its sender is to you.
    fn summaries(
        &self,
        rows: impl Iterator<Item = rusqlite::Result<(ThreadSummary, Vec<String>)>>,
        accounts: &mut Accounts,
    ) -> Result<Vec<ThreadSummary>> {
        let mut out = Vec::new();
        for row in rows {
            let (mut thread, recipients) = row?;
            let account = &thread.account_id;
            if !recipients.is_empty() {
                if !accounts.yours.contains_key(account) {
                    accounts.yours.insert(account.clone(), addresses::yours(&self.conn, account)?);
                }
                thread.to_address = accounts.yours[account].came_to(&recipients);
            }
            if thread.sender != SenderStatus::Blocked {
                if !accounts.senders.contains_key(account) {
                    accounts.senders.insert(account.clone(), Senders::load(&self.conn, account)?);
                }
                let senders = accounts.senders.get_mut(account).expect("loaded above");
                thread.sender = senders.status(&self.conn, &thread.from.email)?;
            }
            out.push(thread);
        }
        Ok(out)
    }

    /// The thread list for an account (`None` for all accounts) and split: what needs a
    /// reply first, then what's for your information, then snoozed; newest first in each.
    /// Like `visibleThreads` in the UI.
    pub fn list_threads(
        &self,
        account: Option<&str>,
        split: SplitId,
        now: i64,
    ) -> Result<Vec<ThreadSummary>> {
        let sql = format!(
            "SELECT * FROM ({}) WHERE split = :split ORDER BY {SECTION_ORDER}, last_message_at DESC, id DESC",
            summary_sql(LISTED)
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params = named_params! {":account": account, ":split": name(&split), ":now": now};
        let rows = stmt.query_map(params, summary)?;
        self.summaries(rows, &mut Accounts::default())
    }

    /// Every open thread, for an account (`None` for all of them): split by split in the
    /// order of the tabs, each as [`Self::list_threads`] lists it. One pass over the mail
    /// instead of one per split.
    pub fn list_open(&self, account: Option<&str>, now: i64) -> Result<Vec<ThreadSummary>> {
        let sql = format!(
            "SELECT * FROM ({}) WHERE split IN ('important', 'other', 'calendar', 'newsletters', 'receipts')
             ORDER BY CASE split WHEN 'important' THEN 0 WHEN 'other' THEN 1 WHEN 'calendar' THEN 2
                          WHEN 'newsletters' THEN 3 ELSE 4 END,
                      {SECTION_ORDER}, last_message_at DESC, id DESC",
            summary_sql_once(LISTED)
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(named_params! {":account": account, ":now": now}, summary)?;
        self.summaries(rows, &mut Accounts::default())
    }

    /// Done threads, most recently done first; those done before Moss kept the time come
    /// last, newest first.
    pub fn list_done(
        &self,
        account: Option<&str>,
        limit: usize,
        now: i64,
    ) -> Result<Vec<ThreadSummary>> {
        let mut stmt = self.conn.prepare(&done_sql())?;
        let params = named_params! {":account": account, ":limit": limit as i64, ":now": now};
        let rows = stmt.query_map(params, summary)?;
        self.summaries(rows, &mut Accounts::default())
    }

    /// Threads matching `filter` (which uses `:account`), newest first.
    pub(crate) fn list_where(
        &self,
        filter: &str,
        account: Option<&str>,
        limit: usize,
        now: i64,
    ) -> Result<Vec<ThreadSummary>> {
        let mut stmt = self.conn.prepare(&where_sql(filter))?;
        let params = named_params! {":account": account, ":limit": limit as i64, ":now": now};
        let rows = stmt.query_map(params, summary)?;
        self.summaries(rows, &mut Accounts::default())
    }

    /// Tab counts like `splitCountsFor`: every thread waiting in each tab, For your info
    /// included, leaving out those reminded for later.
    pub fn split_counts(&self, account: Option<&str>, now: i64) -> Result<SplitCounts> {
        let sql = format!(
            "SELECT split, section, count(*) FROM ({}) GROUP BY split, section",
            summary_sql(LISTED)
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(named_params! {":account": account, ":now": now}, |r| {
            Ok((named::<SplitId>(r, 0)?, named::<Section>(r, 1)?, r.get::<_, u32>(2)?))
        })?;
        let mut counts = SplitCounts::default();
        for row in rows {
            let (split, section, n) = row?;
            // Reminded mail waits nowhere until its time comes.
            if section != Section::Later {
                *counts.get_mut(split) += n;
            }
        }
        Ok(counts)
    }

    pub(crate) fn thread_summary(&self, id: i64, now: i64) -> Result<Option<ThreadSummary>> {
        self.thread_summary_in(id, now, &mut Accounts::default())
    }

    /// [`Self::thread_summary`] for one of many threads, reading each account once.
    pub(crate) fn thread_summary_in(&self, id: i64, now: i64, accounts: &mut Accounts) -> Result<Option<ThreadSummary>> {
        let mut stmt = self.conn.prepare_cached(&summary_sql("t.id = :id"))?;
        let rows = stmt.query_map(named_params! {":id": id, ":now": now}, summary)?;
        Ok(self.summaries(rows, accounts)?.pop())
    }

    /// A thread with its messages, oldest first. Done threads can still be opened (from search).
    pub fn thread(&self, id: &str, now: i64) -> Result<Option<Thread>> {
        let id = parse_id(id);
        let Some(summary) = self.thread_summary(id, now)? else {
            return Ok(None);
        };
        let messages = self.thread_messages(id, &summary.account_id)?;
        Ok(Some(Thread { summary, messages }))
    }

    fn thread_messages(&self, id: i64, account: &str) -> Result<Vec<Message>> {
        // Each message's files, with their Content-ID.
        let mut attachments: HashMap<i64, Vec<(Attachment, Option<String>)>> = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT a.message_id, a.position, a.name, a.bytes, a.content_type, a.inline, a.content_id
             FROM attachments a JOIN messages m ON m.id = a.message_id WHERE m.thread_id = ?1 ORDER BY a.position",
        )?;
        let rows = stmt.query_map([id], |r| {
            let bytes: i64 = r.get(3)?;
            let attachment = Attachment {
                index: r.get(1)?,
                name: r.get(2)?,
                size: moss_core::format_size(bytes as u64),
                bytes: bytes as u64,
                content_type: r.get(4)?,
                inline: r.get(5)?,
                text: None,
            };
            Ok((r.get::<_, i64>(0)?, (attachment, r.get(6)?)))
        })?;
        for row in rows {
            let (message_id, attachment) = row?;
            attachments.entry(message_id).or_default().push(attachment);
        }
        // Inline images kept here, to show in place of the HTML's cid: links.
        let mut images: HashMap<i64, Vec<(String, String)>> = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT a.message_id, a.content_id, a.content_type, d.data FROM attachment_data d
             JOIN attachments a ON a.message_id = d.message_id AND a.position = d.position
             JOIN messages m ON m.id = a.message_id WHERE m.thread_id = ?1 AND a.content_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([id], |r| {
            let media_type: String = r.get(2)?;
            let data: Vec<u8> = r.get(3)?;
            let url = format!("data:{media_type};base64,{}", B64.encode(data));
            Ok((r.get::<_, i64>(0)?, (format!("cid:{}", r.get::<_, String>(1)?), url)))
        })?;
        for row in rows {
            let (message_id, image) = row?;
            images.entry(message_id).or_default().push(image);
        }
        // Forwards sent from Moss, by the message they forwarded.
        let mut forwards: HashMap<i64, Vec<Forward>> = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT f.message_id, s.thread_id, s.to_json, s.cc_json, s.bcc_json, s.sent_at
             FROM forwards f JOIN messages m ON m.id = f.message_id JOIN messages s ON s.id = f.sent_id
             WHERE m.thread_id = ?1 ORDER BY s.sent_at, s.id",
        )?;
        let rows = stmt.query_map([id], |r| {
            let to: Vec<Person> = [json::<Vec<Person>>(r, 2)?, json(r, 3)?, json(r, 4)?].concat();
            Ok((r.get::<_, i64>(0)?, Forward { thread_id: r.get::<_, i64>(1)?.to_string(), to, sent_at: r.get(5)? }))
        })?;
        for row in rows {
            let (message_id, forward) = row?;
            forwards.entry(message_id).or_default().push(forward);
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id, from_name, from_email, to_json, cc_json, bcc_json, subject, sent_at, snippet, read, from_me,
                    b.method, b.text, b.html, m.forwarded, m.sender_verified
             FROM messages m LEFT JOIN bodies b ON b.message_id = m.id WHERE thread_id = ?1 ORDER BY sent_at, m.id",
        )?;
        let rows = stmt.query_map([id], |r| {
            let message_id: i64 = r.get(0)?;
            let method: Option<String> = r.get(11)?;
            let unpack = |idx: usize| -> rusqlite::Result<Option<String>> {
                let packed: Option<Vec<u8>> = r.get(idx)?;
                Ok(method.as_deref().zip(packed).and_then(|(method, packed)| bodies::unpack(method, &packed)))
            };
            let text = unpack(12)?;
            let html = text.as_ref().and(unpack(13)?);
            let preview: String = r.get(8)?;
            let mut files = attachments.remove(&message_id).unwrap_or_default();
            // Mail stored before Moss looked at the HTML lists the images it shows as files.
            for (file, cid) in &mut files {
                file.inline |= html.as_deref().zip(cid.as_deref()).is_some_and(|(h, c)| shown_in(h, c));
            }
            let body_html = html.map(|mut html| {
                for (cid, url) in images.get(&message_id).into_iter().flatten() {
                    // All declared IDs matter, including images that are not cached. A
                    // shorter cached ID must never consume a longer missing one's prefix.
                    let content_id = &cid[4..];
                    let not_longer = |(start, _): &(usize, &str)| !files.iter().filter_map(|(_, id)| id.as_deref()).any(|id|
                        id.len() > content_id.len() && id.starts_with(content_id) && html[start + 4..].starts_with(id));
                    let matches = cid_matches(&html, cid).filter(&not_longer).count();
                    if matches == 0 { continue; }
                    let mut replaced = String::with_capacity(html.len() - matches * cid.len() + matches * url.len());
                    let mut end = 0;
                    for (start, matched) in cid_matches(&html, cid).filter(not_longer) {
                        replaced.push_str(&html[end..start]);
                        replaced.push_str(url);
                        end = start + matched.len();
                    }
                    replaced.push_str(&html[end..]);
                    html = replaced;
                }
                html
            });
            // An image the HTML still links to by Content-ID isn't on this device yet.
            let images_missing = body_html.as_deref().is_some_and(|h| {
                files.iter().any(|(f, cid)| f.inline && cid.as_deref().is_some_and(|c| shown_in(h, c)))
            });
            Ok(Message {
                id: message_id.to_string(),
                thread_id: id.to_string(),
                from: Person { name: r.get(1)?, email: r.get(2)? },
                to: json(r, 3)?,
                cc: json(r, 4)?,
                bcc: json(r, 5)?,
                subject: r.get(6)?,
                sent_at: r.get(7)?,
                // Without the body, the preview stands in until it's fetched.
                paragraphs: match &text {
                    Some(text) => text::paragraphs(&text::strip_quoted(text)),
                    None => text::paragraphs(&preview),
                },
                headers_only: text.is_none(),
                body_text: text.unwrap_or_default(),
                images_missing,
                body_html,
                attachments: files.into_iter().map(|(file, _)| file).collect(),
                read: r.get(9)?,
                image_trust: if r.get(10)? { ImageTrust::Yours } else { ImageTrust::Ask },
                sender_unverified: !r.get::<_, bool>(15)?,
                forwards: forwards.remove(&message_id).unwrap_or_default(),
                forwarded: r.get(14)?,
            })
        })?;
        let mut messages: Vec<Message> = rows.collect::<rusqlite::Result<_>>()?;
        // Reading a thread counts as using its bodies, so cache that's read stays longest.
        self.conn.execute(
            "UPDATE bodies SET used_at = ?2 WHERE message_id IN (SELECT m.id FROM messages m WHERE thread_id = ?1)",
            rusqlite::params![id, now_ms()],
        )?;
        for m in messages.iter_mut().filter(|m| m.image_trust == ImageTrust::Ask) {
            m.image_trust = self.image_trust(account, &m.from.email)?;
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::{Error, MessageSource};

    /// Mail you sent, as your Sent folder has it.
    fn in_sent(store: &mut Store, provider_id: &str, raw: &[u8]) -> crate::Ingested {
        store.upsert_mailbox(&moss_core::Mailbox { account_id: "work".into(), name: "Sent".into(), role: Some(moss_core::MailboxRole::Sent) }).unwrap();
        store.ingest(raw, &moss_core::MessageSource { mailbox: "Sent".into(), ..source(provider_id) }).unwrap()
    }

    /// A thread from `from`, `minutes` after 09:12 on 1 November.
    fn add(store: &mut Store, id: &str, mail: Mail, minutes: i64) -> String {
        let raw = mail.id(&format!("{id}@x")).at(NOV_1 + minutes * 60_000).build();
        ingest(store, id, &raw).thread_id
    }

    fn ids(list: &[ThreadSummary]) -> Vec<&str> {
        list.iter().map(|t| t.id.as_str()).collect()
    }

    fn inbox(store: &Store, split: SplitId) -> Vec<ThreadSummary> {
        store.list_threads(None, split, NOV_1 + DAY).unwrap()
    }

    #[test]
    fn unread_summary_stops_after_finding_an_unread_message() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "a", &mail().build());
        store.conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<2000)
            INSERT INTO messages(account_id,thread_id,message_id,from_name,from_email,to_json,cc_json,bcc_json,subject,sent_at,snippet,read,from_me,participants,received_at,split,section)
            SELECT account_id,thread_id,printf('sibling%d@example.test',i),from_name,from_email,to_json,cc_json,bcc_json,subject,sent_at,snippet,0,0,participants,received_at,split,section FROM messages,n WHERE messages.id=1;").unwrap();
        let full_sql = summary_sql("t.id=:id");
        let mut full = store.conn.prepare(&full_sql).unwrap();
        let row = full.query_row(named_params!{":id":first.thread_id, ":now":NOV_1}, summary).unwrap().0;
        assert!(row.unread);
        assert!(!row.done);
        assert_eq!(row.message_count, 2001);
        let full_steps = full.get_status(rusqlite::StatementStatus::VmStep);
        eprintln!("full summary VM steps: {full_steps}");
        assert!(full_steps < 120_000, "boolean states must not count every match: {full_steps}");
        let sql = format!("SELECT unread FROM ({full_sql})");
        let mut statement = store.conn.prepare(&sql).unwrap();
        assert!(statement.query_row(named_params!{":id":first.thread_id}, |r|r.get::<_,bool>(0)).unwrap());
        let steps = statement.get_status(rusqlite::StatementStatus::VmStep);
        eprintln!("unread projection VM steps: {steps}");
        assert!(steps < 300, "finding unread mail should stop at its first match, used {steps} VM steps");
        store.conn.execute("UPDATE messages SET read=1", []).unwrap();
        assert!(!statement.query_row(named_params!{":id":first.thread_id}, |r|r.get::<_,bool>(0)).unwrap());
    }

    /// How many steps SQLite takes to run `sql` with these parameters, reading every row.
    fn steps(store: &Store, sql: &str, params: &[(&str, &dyn rusqlite::ToSql)]) -> i32 {
        let mut stmt = store.conn.prepare(sql).unwrap();
        let rows = stmt.query_map(params, summary).unwrap().count();
        assert!(rows > 0);
        stmt.get_status(rusqlite::StatementStatus::VmStep)
    }

    #[test]
    fn a_short_done_or_blocked_list_works_out_only_the_threads_it_shows() {
        let mut store = store_with_work();
        for n in 0..120 {
            let thread = add(&mut store, &n.to_string(), mail().subject(&format!("S{n}")), n);
            store.mark_done(&thread, true, NOV_1 + n).unwrap();
        }
        for sql in [done_sql(), where_sql("(:account IS NULL OR t.account_id = :account)")] {
            let at = |limit: i64| steps(&store, &sql, named_params! {":account": None::<String>, ":limit": limit, ":now": NOV_1});
            let (one, all) = (at(1), at(120));
            assert!(one * 5 < all, "one thread took {one} steps, all of them {all}");
        }
    }

    #[test]
    fn the_blocked_list_of_every_account_reads_the_newest_threads_first_by_index() {
        let store = store_with_work();
        let plan = plan(&store, &where_sql(BLOCKED));
        let newest_first = plan.lines().any(|step| step.starts_with("SCAN t USING") && step.ends_with("INDEX threads_by_last_message"));
        assert!(newest_first && plan.contains("SEARCH t USING INTEGER PRIMARY KEY"), "{plan}");
    }

    #[test]
    fn every_open_thread_comes_in_one_list_split_by_split_as_each_tab_lists_them() {
        use SplitId::*;
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let subjects = ["Q4", "Weekly", "Your receipt", "Lunch", "Invite", "FYI", "Old", "Later"];
        let mut threads = Vec::new();
        for (n, subject) in subjects.iter().enumerate() {
            let mail = match n {
                1 => mail().subject(subject).header("List-Id", "<news>"),
                5 => mail().subject(subject).to("team@northwind.co"),
                _ => mail().subject(subject),
            };
            threads.push(add(&mut store, &n.to_string(), mail, n as i64));
        }
        let raw = mail().id("p@x").subject("Personal").to("maya.ross@fastmail.com").build();
        store.ingest(&raw, &MessageSource { account_id: "personal".into(), ..source("p") }).unwrap();
        store.move_thread(&threads[3], Some(Other), NOV_1).unwrap();
        store.move_thread(&threads[4], Some(Important), NOV_1).unwrap();
        store.set_split(&threads[0], Calendar, NOV_1).unwrap();
        store.mark_done(&threads[6], true, NOV_1).unwrap();
        store.snooze(&threads[7], Some(NOV_1 + 2 * DAY)).unwrap();
        for account in [None, Some("work"), Some("personal")] {
            let each: Vec<ThreadSummary> = [Important, Other, Calendar, Newsletters, Receipts]
                .into_iter()
                .flat_map(|split| store.list_threads(account, split, NOV_1 + DAY).unwrap())
                .collect();
            assert_eq!(store.list_open(account, NOV_1 + DAY).unwrap(), each);
        }
        assert_eq!(store.list_open(None, NOV_1 + DAY).unwrap().len(), 8);
    }

    #[test]
    fn lists_reply_then_info_then_later_newest_first() {
        let mut store = store_with_work();
        let old_reply = add(&mut store, "a", mail().subject("A"), 0);
        let info = add(&mut store, "b", mail().subject("B").to("team@northwind.co"), 30);
        let new_reply = add(&mut store, "c", mail().subject("C"), 10);
        let later = add(&mut store, "d", mail().subject("D"), 60);
        store.snooze(&later, Some(NOV_1 + 2 * DAY)).unwrap();
        let list = inbox(&store, SplitId::Important);
        assert_eq!(ids(&list), [new_reply.as_str(), &old_reply, &info, &later]);
        assert_eq!(list[3].section, Section::Later);
        assert_eq!(list[3].snoozed_until, Some(NOV_1 + 2 * DAY));
        assert_eq!(list[2].section, Section::Info);
        assert_eq!(list[0].snoozed_until, None);
    }

    #[test]
    fn filters_by_split_and_account_and_hides_done() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let important = add(&mut store, "a", mail(), 0);
        let news = add(&mut store, "b", mail().subject("Weekly").header("List-Id", "<news>"), 1);
        let done = add(&mut store, "c", mail().subject("Old"), 2);
        store.mark_done(&done, true, NOV_1).unwrap();
        let raw = mail().id("p@x").to("maya.ross@fastmail.com").build();
        let personal = store
            .ingest(&raw, &MessageSource { account_id: "personal".into(), ..source("p") })
            .unwrap()
            .thread_id;

        assert_eq!(ids(&inbox(&store, SplitId::Important)), [personal.as_str(), &important]);
        assert_eq!(ids(&inbox(&store, SplitId::Newsletters)), [news.as_str()]);
        assert!(inbox(&store, SplitId::Receipts).is_empty());
        let work = store.list_threads(Some("work"), SplitId::Important, NOV_1).unwrap();
        assert_eq!(ids(&work), [important.as_str()]);
        assert_eq!(work[0].account_id, "work");
    }

    #[test]
    fn counts_tabs_like_the_ui() {
        let mut store = store_with_work();
        add(&mut store, "a", mail(), 0);
        add(&mut store, "b", mail().subject("FYI").to("team@northwind.co"), 1);
        let later = add(&mut store, "c", mail().subject("Later"), 2);
        store.snooze(&later, Some(NOV_1 + 2 * DAY)).unwrap();
        add(&mut store, "d", mail().subject("Weekly").header("List-Id", "<news>"), 3);
        let snoozed_news =
            add(&mut store, "e", mail().subject("Monthly").header("List-Id", "<news>"), 4);
        store.snooze(&snoozed_news, Some(NOV_1 + 2 * DAY)).unwrap();
        add(&mut store, "f", mail().subject("Your receipt"), 5);
        let counts = store.split_counts(None, NOV_1 + DAY).unwrap();
        // Reminded mail isn't waiting in any tab until its time comes; then it's back in its own.
        assert_eq!(
            counts,
            SplitCounts { important: 2, other: 0, calendar: 0, newsletters: 1, receipts: 1 }
        );
        let back = store.split_counts(None, NOV_1 + 3 * DAY).unwrap();
        assert_eq!((back.important, back.newsletters), (3, 2));
        assert_eq!(store.split_counts(Some("personal"), NOV_1).unwrap(), SplitCounts::default());
    }

    #[test]
    fn snoozes_end_on_time_or_when_a_new_message_arrives() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        store.snooze(&thread, Some(NOV_1 + DAY)).unwrap();
        let at = |store: &Store, now| {
            store.list_threads(None, SplitId::Important, now).unwrap()[0].section
        };
        assert_eq!(at(&store, NOV_1 + DAY - 1), Section::Later);
        assert_eq!(at(&store, NOV_1 + DAY), Section::Reply);
        store.snooze(&thread, Some(NOV_1 + 3 * DAY)).unwrap();
        add(&mut store, "b", mail().reply_to(&["a@x"]), 5);
        assert_eq!(at(&store, NOV_1 + DAY), Section::Reply);
        store.snooze(&thread, None).unwrap();
        assert_eq!(at(&store, NOV_1), Section::Reply);
    }

    fn done_list(store: &Store) -> Vec<ThreadSummary> {
        store.list_done(None, 10, NOV_1).unwrap()
    }

    #[test]
    fn deleting_takes_a_thread_out_of_the_list_until_it_is_restored() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        let other = add(&mut store, "b", mail().subject("Lunch").id("l@x"), 1);
        store.delete_thread(&thread).unwrap();
        assert_eq!(ids(&inbox(&store, SplitId::Important)), [other.as_str()]);
        assert_eq!(store.split_counts(None, NOV_1).unwrap().important, 1);
        assert!(done_list(&store).is_empty(), "deleted isn't done");

        // The server took it out of the Inbox meanwhile, which counts as done.
        store.mark_done(&thread, true, NOV_1).unwrap();
        assert_eq!(store.restore_thread(&thread).unwrap(), Some(true));
        let list = inbox(&store, SplitId::Important);
        assert_eq!(ids(&list), [other.as_str(), &thread]);
        assert!(!list[1].done);
        assert_eq!(store.restore_thread(&thread).unwrap(), None, "nothing left to restore");
    }

    #[test]
    fn a_thread_deleted_from_the_done_list_goes_back_there() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        store.mark_done(&thread, true, NOV_1 + 5).unwrap();
        store.delete_thread(&thread).unwrap();
        assert!(done_list(&store).is_empty());
        assert!(inbox(&store, SplitId::Important).is_empty());
        assert_eq!(store.restore_thread(&thread).unwrap(), Some(false));
        let done = done_list(&store);
        assert_eq!(ids(&done), [thread.as_str()]);
        assert_eq!(done[0].done_at, Some(NOV_1 + 5));
    }

    #[test]
    fn a_new_message_brings_a_deleted_thread_back() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        store.delete_thread(&thread).unwrap();
        add(&mut store, "b", mail().reply_to(&["a@x"]), 5);
        assert_eq!(ids(&inbox(&store, SplitId::Important)), [thread.as_str()]);
    }

    #[test]
    fn deleting_an_unknown_thread_is_an_error() {
        let mut store = store_with_work();
        assert!(matches!(store.delete_thread("99"), Err(Error::NotFound { .. })));
        assert!(matches!(store.restore_thread("99"), Err(Error::NotFound { .. })));
    }

    #[test]
    fn a_new_message_brings_a_done_thread_back() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        store.mark_done(&thread, true, NOV_1).unwrap();
        assert!(inbox(&store, SplitId::Important).is_empty());
        add(&mut store, "b", mail().reply_to(&["a@x"]), 5);
        let list = inbox(&store, SplitId::Important);
        assert_eq!(ids(&list), [thread.as_str()]);
        assert!(!list[0].done);
    }

    #[test]
    fn lists_done_threads_most_recently_done_first() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let open = add(&mut store, "a", mail().subject("Open"), 0);
        let early = add(&mut store, "b", mail().subject("Early"), 1);
        let late = add(&mut store, "c", mail().subject("Late"), 2);
        // Done before Moss kept the time: last, newest first.
        let older = add(&mut store, "d", mail().subject("Older"), 3);
        let newer = add(&mut store, "e", mail().subject("Newer"), 4);
        let raw = mail().id("p@x").to("maya.ross@fastmail.com").build();
        let personal = store
            .ingest(&raw, &MessageSource { account_id: "personal".into(), ..source("p") })
            .unwrap()
            .thread_id;
        store.mark_done(&late, true, NOV_1 + 30).unwrap();
        store.mark_done(&early, true, NOV_1 + 10).unwrap();
        store.mark_done(&personal, true, NOV_1 + 20).unwrap();
        for id in [&older, &newer] {
            store.mark_done(id, true, NOV_1).unwrap();
        }
        store
            .conn
            .execute("UPDATE messages SET done_at = NULL WHERE subject IN ('Older', 'Newer')", [])
            .unwrap();

        let all = store.list_done(None, 10, NOV_1).unwrap();
        assert_eq!(ids(&all), [late.as_str(), &personal, &early, &newer, &older]);
        assert!(all.iter().all(|t| t.done));
        assert_eq!(all[0].done_at, Some(NOV_1 + 30));
        assert_eq!(all[3].done_at, None);
        assert_eq!(ids(&store.list_done(None, 2, NOV_1).unwrap()), [late.as_str(), &personal]);
        let work = store.list_done(Some("work"), 10, NOV_1).unwrap();
        assert_eq!(ids(&work), [late.as_str(), &early, &newer, &older]);
        assert!(!ids(&all).contains(&open.as_str()));
        assert_eq!(inbox(&store, SplitId::Important)[0].done_at, None);
    }

    #[test]
    fn mail_you_only_sent_is_not_in_the_done_list() {
        let mut store = store_with_work();
        let sent =
            add(&mut store, "a", mail().from("maya@northwind.co").to("daniel@northwind.co"), 0);
        store.mark_done(&sent, true, NOV_1).unwrap();
        assert!(store.list_done(None, 10, NOV_1).unwrap().is_empty());
    }

    #[test]
    fn a_thread_is_done_when_its_latest_message_was() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        store.mark_done(&thread, true, NOV_1 + 10).unwrap();
        add(&mut store, "b", mail().reply_to(&["a@x"]), 5);
        assert!(store.list_done(None, 10, NOV_1).unwrap().is_empty());
        assert_eq!(inbox(&store, SplitId::Important)[0].done_at, None);
        store.mark_done(&thread, true, NOV_1 + 20).unwrap();
        assert_eq!(store.list_done(None, 10, NOV_1).unwrap()[0].done_at, Some(NOV_1 + 20));
    }

    #[test]
    fn your_reply_does_not_bring_a_thread_back_or_take_its_name() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        store.mark_done(&thread, true, NOV_1).unwrap();
        let reply = mail().from("maya@northwind.co").to("daniel@northwind.co").reply_to(&["a@x"]);
        in_sent(&mut store, "b", &reply.id("b@x").at(NOV_1 + 5 * 60_000).build());
        assert!(inbox(&store, SplitId::Important).is_empty());
        let summary = store.thread(&thread, NOV_1).unwrap().unwrap().summary;
        assert_eq!(summary.from.name, "Daniel Okafor");
        assert_eq!(summary.message_count, 2);
    }

    #[test]
    fn once_you_have_replied_it_no_longer_needs_your_reply() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        add(
            &mut store,
            "b",
            mail()
                .from("maya@northwind.co")
                .to("daniel@northwind.co")
                .reply_to(&["a@x"])
                .body("Signed."),
            5,
        );
        let list = inbox(&store, SplitId::Important);
        assert_eq!(ids(&list), [thread.as_str()]);
        assert_eq!(list[0].section, Section::Info);
        assert_eq!(list[0].summary, "Signed.");
        assert_eq!(list[0].from.name, "Daniel Okafor");
    }

    #[test]
    fn a_thread_only_you_wrote_shows_who_it_went_to() {
        let mut store = store_with_work();
        let sent = add(
            &mut store,
            "a",
            mail().from("maya@northwind.co").to("Lena Vogt <lena@northwind.co>"),
            0,
        );
        assert_eq!(store.thread(&sent, NOV_1).unwrap().unwrap().summary.from.name, "Lena Vogt");
        let nobody = add(&mut store, "b", mail().from("maya@northwind.co").without("To"), 0);
        assert_eq!(
            store.thread(&nobody, NOV_1).unwrap().unwrap().summary.from.email,
            "maya@northwind.co"
        );
    }

    #[test]
    fn a_thread_comes_with_its_messages_oldest_first() {
        let mut store = store_with_work();
        let body = "Hi Maya,\n\nCan you sign off?\n\nOn Sat, Lena wrote:\n> numbers attached";
        let first = mail().body(body).attach("Q4.pdf", "application/pdf", "PDFDATA").inline(
            "logo.png",
            "image/png",
            "PNG",
        );
        let thread = add(&mut store, "a", first, 0);
        add(
            &mut store,
            "b",
            mail()
                .from("Lena Vogt <lena@northwind.co>")
                .header("Cc", "jonas@northwind.co")
                .reply_to(&["a@x"])
                .body("Done."),
            30,
        );
        let got = store.thread(&thread, NOV_1).unwrap().unwrap();
        assert_eq!(got.summary.from.name, "Lena Vogt");
        assert_eq!(got.summary.summary, "Done.");
        assert!(got.summary.has_attachments && got.summary.unread);
        assert_eq!(got.messages.len(), 2);
        let m = &got.messages[0];
        assert_eq!(m.paragraphs, ["Hi Maya,", "Can you sign off?"]);
        assert!(m.body_text.contains("numbers attached"));
        assert_eq!(m.to[0].email, "maya@northwind.co");
        assert_eq!(m.thread_id, thread);
        assert_eq!(m.attachments.len(), 2);
        assert_eq!(
            (m.attachments[0].name.as_str(), m.attachments[0].size.as_str()),
            ("Q4.pdf", "7 B")
        );
        assert!(m.attachments[1].inline);
        assert_eq!(got.messages[1].cc[0].email, "jonas@northwind.co");
        assert!(got.messages[1].attachments.is_empty());
        assert!(!got.messages[1].read);
    }

    #[test]
    fn inline_images_alone_are_not_attachments() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail().inline("logo.png", "image/png", "PNG"), 0);
        assert!(!store.thread(&thread, NOV_1).unwrap().unwrap().summary.has_attachments);
    }

    #[test]
    fn unknown_threads_are_none() {
        let store = store_with_work();
        assert_eq!(store.thread("42", NOV_1).unwrap(), None);
        assert_eq!(store.thread("not-a-number", NOV_1).unwrap(), None);
    }

    #[test]
    fn a_corrupt_row_is_an_error_not_a_panic() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        store.conn.execute("UPDATE messages SET split = 'sideways'", []).unwrap();
        assert!(matches!(store.thread(&thread, NOV_1), Err(Error::Sqlite(_))));
    }

    #[test]
    fn a_broken_schema_is_an_error_when_reading_a_thread() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN snippet TO preview");
        assert!(store.thread(&thread, NOV_1).is_err());
        // On its own, so the thread's files are what fails.
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        break_schema(&store, "ALTER TABLE attachments RENAME COLUMN position TO n");
        assert!(store.thread(&thread, NOV_1).is_err());
        // And so that noting the bodies were read is what fails.
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail(), 0);
        break_schema(&store, "CREATE TRIGGER no_use BEFORE UPDATE ON bodies BEGIN SELECT RAISE(ABORT, 'read only'); END");
        assert!(store.thread(&thread, NOV_1).is_err());
    }
}
