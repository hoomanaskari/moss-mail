//! What sync and the server-side actions need from the store: where each message sits on the
//! server, changes made in other mail apps, the headers a reply links to, and small settings.

use moss_core::MailboxRole;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::codec::{name, parse_id};
use crate::{Result, Store};

/// Where the server keeps one of a thread's messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub mailbox: String,
    pub provider_id: String,
}

/// The headers a reply needs so other mail apps put it in the same conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplyHeaders {
    /// Message-ID of the latest message, without <>: the reply's In-Reply-To.
    pub in_reply_to: Option<String>,
    /// That message's References plus its own id, oldest first.
    pub references: Vec<String>,
}

impl Store {
    /// A small app setting, such as an account's connection details (never a secret).
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Saves a setting, or removes it with `None`.
    pub fn set_setting(&mut self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = ?2",
                params![key, value],
            )?,
            None => self.conn.execute("DELETE FROM settings WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    /// Every server location of a thread's messages, so done and read can be sent to the server.
    pub fn thread_locations(&self, thread_id: &str) -> Result<Vec<Location>> {
        let mut stmt = self.conn.prepare(
            "SELECT b.name, l.provider_id FROM message_locations l
             JOIN mailboxes b ON b.id = l.mailbox_id JOIN messages m ON m.id = l.message_id
             WHERE m.thread_id = ?1 ORDER BY b.name, l.provider_id",
        )?;
        let rows = stmt.query_map([parse_id(thread_id)], |r| {
            Ok(Location { mailbox: r.get(0)?, provider_id: r.get(1)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The provider ids stored for a mailbox, to compare with what the server still has.
    pub fn provider_ids(&self, account_id: &str, mailbox: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.provider_id FROM message_locations l JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE b.account_id = ?1 AND b.name = ?2 ORDER BY l.provider_id",
        )?;
        let rows = stmt.query_map(params![account_id, mailbox], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// How many messages are stored as in `mailbox`, without reading their ids.
    pub fn mailbox_count(&self, account_id: &str, mailbox: &str) -> Result<u32> {
        Ok(self.conn.prepare_cached(
            "SELECT count(*) FROM message_locations l JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE b.account_id = ?1 AND b.name = ?2",
        )?.query_row(params![account_id, mailbox], |r| r.get(0))?)
    }

    /// Read or unread was changed in another mail app. Returns whether the message is stored.
    pub fn set_seen(
        &mut self,
        account_id: &str,
        mailbox: &str,
        provider_id: &str,
        seen: bool,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE messages SET read = ?4 WHERE id IN (SELECT l.message_id FROM message_locations l
             JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE b.account_id = ?1 AND b.name = ?2 AND l.provider_id = ?3)",
            params![account_id, mailbox, provider_id, seen],
        )? > 0)
    }

    /// Messages that left a mailbox on the server (archived, moved or deleted elsewhere).
    /// Leaving the inbox counts as done, the way it does in other mail apps, as of `now` unless
    /// it was done here already. Returns how many locations were removed.
    pub fn remove_from_mailbox(
        &mut self,
        account_id: &str,
        mailbox: &str,
        provider_ids: &[String],
        now: i64,
    ) -> Result<usize> {
        self.drop_locations(account_id, mailbox, provider_ids, Some(now))
    }

    /// Forgets where these messages were, without marking anything done: for Moss's own
    /// archive, which Undo may already have taken back here.
    pub fn forget_locations(
        &mut self,
        account_id: &str,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<usize> {
        self.drop_locations(account_id, mailbox, provider_ids, None)
    }

    /// Removes locations; with `done_at`, mail that left the inbox is done.
    fn drop_locations(
        &mut self,
        account_id: &str,
        mailbox: &str,
        provider_ids: &[String],
        done_at: Option<i64>,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let found: Option<(i64, Option<String>)> = tx
            .query_row(
                "SELECT id, role FROM mailboxes WHERE account_id = ?1 AND name = ?2",
                params![account_id, mailbox],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((mailbox_id, role)) = found else {
            return Ok(0);
        };
        let inbox = role.as_deref() == Some(name(&MailboxRole::Inbox).as_str());
        let mut removed = 0;
        for id in provider_ids {
            if let (true, Some(now)) = (inbox, done_at) {
                tx.execute(
                    "UPDATE messages SET done_at = CASE WHEN done = 1 THEN coalesce(done_at, ?3) ELSE ?3 END,
                         done = 1, snoozed_until = NULL
                     WHERE id = (SELECT message_id FROM message_locations WHERE mailbox_id = ?1 AND provider_id = ?2)",
                    params![mailbox_id, id, now],
                )?;
            }
            removed += tx.execute(
                "DELETE FROM message_locations WHERE mailbox_id = ?1 AND provider_id = ?2",
                params![mailbox_id, id],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }

    /// The Message-IDs of a thread's messages, oldest first, to find them on the server once
    /// their stored locations are gone. Messages without one are left out.
    pub fn message_ids(&self, thread_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT message_id FROM messages WHERE thread_id = ?1 AND message_id NOT LIKE 'sha256:%'
             ORDER BY sent_at, id",
        )?;
        let rows = stmt.query_map([parse_id(thread_id)], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// In-Reply-To and References for a reply to the thread's latest message.
    pub fn reply_headers(&self, thread_id: &str) -> Result<Option<ReplyHeaders>> {
        let latest: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, message_id FROM messages WHERE thread_id = ?1
                 ORDER BY sent_at DESC, id DESC LIMIT 1",
                [parse_id(thread_id)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, message_id)) = latest else {
            return Ok(None);
        };
        let mut stmt =
            self.conn.prepare_cached("SELECT ref FROM message_refs WHERE message_id = ?1 ORDER BY rowid")?;
        let mut references =
            stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        let in_reply_to = (!message_id.starts_with("sha256:")).then_some(message_id);
        references.extend(in_reply_to.clone());
        Ok(Some(ReplyHeaders { in_reply_to, references }))
    }

    /// The name you sign with, from the newest mail you sent from this account's address.
    pub fn sender_name_seen(&self, account_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT m.from_name FROM messages m JOIN accounts a ON a.id = m.account_id
                 WHERE m.account_id = ?1 AND m.from_me = 1 AND lower(m.from_email) = lower(a.address)
                   AND m.from_name <> m.from_email AND m.from_name <> ''
                 ORDER BY m.sent_at DESC LIMIT 1",
                [account_id],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{Mailbox, MessageSource};

    fn inbox_role(store: &mut Store) {
        store
            .upsert_mailbox(&Mailbox {
                account_id: "work".into(),
                name: "INBOX".into(),
                role: Some(MailboxRole::Inbox),
            })
            .unwrap();
    }

    fn at(mailbox: &str, provider_id: &str) -> MessageSource {
        MessageSource { mailbox: mailbox.into(), ..source(provider_id) }
    }

    #[test]
    fn settings_are_saved_replaced_and_removed() {
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(store.setting("a").unwrap(), None);
        store.set_setting("a", Some("1")).unwrap();
        store.set_setting("a", Some("2")).unwrap();
        assert_eq!(store.setting("a").unwrap().as_deref(), Some("2"));
        store.set_setting("a", None).unwrap();
        assert_eq!(store.setting("a").unwrap(), None);
    }

    #[test]
    fn a_thread_knows_where_its_messages_are_on_the_server() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1:7", &mail().build());
        store.ingest(&mail().build(), &at("[Gmail]/All Mail", "1:90")).unwrap();
        store
            .ingest(&mail().id("r@x").reply_to(&["q4@northwind.co"]).build(), &at("INBOX", "1:8"))
            .unwrap();
        let locations = store.thread_locations(&first.thread_id).unwrap();
        let pairs: Vec<_> =
            locations.iter().map(|l| (l.mailbox.as_str(), l.provider_id.as_str())).collect();
        assert_eq!(pairs, [("INBOX", "1:7"), ("INBOX", "1:8"), ("[Gmail]/All Mail", "1:90")]);
        assert_eq!(store.provider_ids("work", "INBOX").unwrap(), ["1:7", "1:8"]);
        assert_eq!(store.mailbox_count("work", "INBOX").unwrap(), 2);
        assert_eq!(store.mailbox_count("work", "[Gmail]/All Mail").unwrap(), 1);
        assert!(store.provider_ids("work", "Nope").unwrap().is_empty());
        assert_eq!(store.mailbox_count("work", "Nope").unwrap(), 0);
        assert_eq!(store.mailbox_count("home", "INBOX").unwrap(), 0);
        assert!(store.thread_locations("999").unwrap().is_empty());
    }

    #[test]
    fn read_changes_from_other_apps_are_kept() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1:7", &mail().build()).thread_id;
        assert!(store.thread(&id, NOV_1).unwrap().unwrap().summary.unread);
        assert!(store.set_seen("work", "INBOX", "1:7", true).unwrap());
        assert!(!store.thread(&id, NOV_1).unwrap().unwrap().summary.unread);
        assert!(store.set_seen("work", "INBOX", "1:7", false).unwrap());
        assert!(store.thread(&id, NOV_1).unwrap().unwrap().summary.unread);
        assert!(!store.set_seen("work", "INBOX", "nope", true).unwrap());
    }

    #[test]
    fn leaving_the_inbox_elsewhere_marks_mail_done() {
        let mut store = store_with_work();
        inbox_role(&mut store);
        let id = ingest(&mut store, "1:7", &mail().build()).thread_id;
        store.snooze(&id, Some(NOV_1 + DAY)).unwrap();
        let gone = ["1:7".to_string(), "1:99".into()];
        assert_eq!(store.remove_from_mailbox("work", "INBOX", &gone, NOV_1 + 5).unwrap(), 1);
        let summary = store.thread(&id, NOV_1).unwrap().unwrap().summary;
        assert!(summary.done);
        assert_eq!(summary.done_at, Some(NOV_1 + 5));
        assert_eq!(summary.snoozed_until, None);
        assert!(store.provider_ids("work", "INBOX").unwrap().is_empty());
    }

    #[test]
    fn mail_done_here_keeps_its_time_when_it_leaves_the_inbox() {
        let mut store = store_with_work();
        inbox_role(&mut store);
        let id = ingest(&mut store, "1:7", &mail().build()).thread_id;
        ingest(&mut store, "1:8", &mail().id("r@x").reply_to(&["q4@northwind.co"]).build());
        store.mark_done(&id, true, NOV_1 + 5).unwrap();
        // Done before Moss kept the time: now is the best guess.
        store
            .conn
            .execute("UPDATE messages SET done_at = NULL WHERE message_id = 'r@x'", [])
            .unwrap();
        store
            .remove_from_mailbox("work", "INBOX", &["1:7".into(), "1:8".into()], NOV_1 + 9)
            .unwrap();
        let times: Vec<i64> = store
            .conn
            .prepare("SELECT done_at FROM messages ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(times, [NOV_1 + 5, NOV_1 + 9]);
    }

    #[test]
    fn leaving_another_folder_only_forgets_the_location() {
        let mut store = store_with_work();
        let id = store.ingest(&mail().build(), &at("Projects", "1:7")).unwrap().thread_id;
        assert_eq!(
            store.remove_from_mailbox("work", "Projects", &["1:7".into()], NOV_1).unwrap(),
            1
        );
        assert!(!store.thread(&id, NOV_1).unwrap().unwrap().summary.done);
        assert_eq!(
            store.remove_from_mailbox("work", "Unknown", &["1:7".into()], NOV_1).unwrap(),
            0
        );
    }

    #[test]
    fn archiving_from_here_forgets_the_inbox_copy_without_marking_it_done() {
        // Undo can bring a thread back before the archive it undoes has finished.
        let mut store = store_with_work();
        inbox_role(&mut store);
        let id = ingest(&mut store, "1:7", &mail().build()).thread_id;
        assert_eq!(
            store.forget_locations("work", "INBOX", &["1:7".into(), "1:99".into()]).unwrap(),
            1
        );
        assert!(!store.thread(&id, NOV_1).unwrap().unwrap().summary.done);
        assert!(store.provider_ids("work", "INBOX").unwrap().is_empty());
        assert_eq!(store.forget_locations("work", "Unknown", &["1:7".into()]).unwrap(), 0);
    }

    #[test]
    fn a_thread_lists_the_message_ids_the_server_can_find() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        ingest(&mut store, "2", &mail().id("r@x").reply_to(&["q4@northwind.co"]).build());
        let bare =
            mail().without("Message-ID").subject("Re: Q4 budget").header("In-Reply-To", "<r@x>");
        ingest(&mut store, "3", &bare.build());
        assert_eq!(store.message_ids(&id).unwrap(), ["q4@northwind.co", "r@x"]);
        assert!(store.message_ids("999").unwrap().is_empty());
    }

    #[test]
    fn a_broken_database_is_an_error_not_a_panic() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1:7", &mail().build()).thread_id;
        break_schema(&store, "ALTER TABLE message_locations RENAME TO old_locations");
        assert!(store.thread_locations(&id).is_err());
        assert!(store.provider_ids("work", "INBOX").is_err());
        assert!(store.mailbox_count("work", "INBOX").is_err());
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN message_id TO mid");
        assert!(store.message_ids(&id).is_err());
        read_only(&store);
        assert!(store.set_setting("a", Some("1")).is_err());
        assert!(store.set_seen("work", "INBOX", "1:7", true).is_err());
    }

    #[test]
    fn a_reply_links_to_the_latest_message_and_its_references() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        assert_eq!(
            store.reply_headers(&id).unwrap().unwrap(),
            ReplyHeaders {
                in_reply_to: Some("q4@northwind.co".into()),
                references: vec!["q4@northwind.co".into()]
            }
        );
        let second = mail().id("b@x").reply_to(&["q4@northwind.co"]).at(NOV_1 + DAY).build();
        ingest(&mut store, "2", &second);
        assert_eq!(
            store.reply_headers(&id).unwrap().unwrap(),
            ReplyHeaders {
                in_reply_to: Some("b@x".into()),
                references: vec!["q4@northwind.co".into(), "b@x".into()]
            }
        );
        assert_eq!(store.reply_headers("999").unwrap(), None);
    }

    #[test]
    fn a_message_without_an_id_gives_no_in_reply_to() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().without("Message-ID").build()).thread_id;
        let headers = store.reply_headers(&id).unwrap().unwrap();
        assert_eq!(headers, ReplyHeaders { in_reply_to: None, references: vec![] });
    }

    #[test]
    fn the_sender_name_comes_from_mail_you_sent() {
        let mut store = store_with_work();
        assert_eq!(store.sender_name_seen("work").unwrap(), None);
        // Bare address: no name to learn.
        let bare = mail().id("s1@x").from("maya@northwind.co").to("daniel@northwind.co");
        ingest(&mut store, "1", &bare.build());
        assert_eq!(store.sender_name_seen("work").unwrap(), None);
        let named = mail().id("s2@x").from("Maya R. <Maya@northwind.co>").to("daniel@northwind.co");
        ingest(&mut store, "2", &named.build());
        assert_eq!(store.sender_name_seen("work").unwrap().as_deref(), Some("Maya R."));
    }
}
