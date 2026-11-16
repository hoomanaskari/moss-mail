//! Gmail conversations (X-GM-THRID). Gmail keeps an archived message only in All Mail, so
//! syncing the Inbox and Sent leaves gaps in a conversation: replies you archived are
//! missing. Sync asks Gmail which conversation each stored message is in, queues those
//! conversations here, and fetches their other messages from All Mail, newest first.
//!
//! `messages.conversation_id` is NULL until asked and 0 when the server had no answer, so
//! each message is asked about once.

use rusqlite::{params, OptionalExtension};

use crate::parse::parse;
use crate::{Result, Store};

impl Store {
    /// Messages in `mailbox` whose conversation isn't known yet, newest first.
    pub fn unlinked(&self, account_id: &str, mailbox: &str, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.provider_id FROM message_locations l
             JOIN mailboxes b ON b.id = l.mailbox_id JOIN messages m ON m.id = l.message_id
             WHERE b.account_id = ?1 AND b.name = ?2 AND m.conversation_id IS NULL
               AND l.provider_id NOT LIKE 'local:%'
             ORDER BY m.sent_at DESC, m.id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, mailbox, limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Records the conversation of each message asked about (`found`); the ones the server
    /// didn't answer for are not asked again. A message newly linked to a conversation
    /// queues it for filling, so a new reply also brings in what was archived around it.
    pub fn link_conversations(
        &mut self,
        account_id: &str,
        mailbox: &str,
        asked: &[String],
        found: &[(String, u64)],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let link = "UPDATE messages SET conversation_id = ?3 WHERE conversation_id IS NULL AND id =
            (SELECT l.message_id FROM message_locations l JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE b.account_id = ?1 AND b.name = ?2 AND l.provider_id = ?4)";
        for (provider_id, conversation) in found {
            // The same 64 bits: SQLite integers are signed.
            let conversation = *conversation as i64;
            if tx.execute(link, params![account_id, mailbox, conversation, provider_id])? > 0 {
                tx.execute(
                    "INSERT INTO conversations (account_id, id, filled) VALUES (?1, ?2, 0)
                     ON CONFLICT (account_id, id) DO UPDATE SET filled = 0",
                    params![account_id, conversation],
                )?;
            }
        }
        for provider_id in asked {
            tx.execute(link, params![account_id, mailbox, 0, provider_id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Conversations whose other messages are still to fetch, the most recent first.
    pub fn conversations_to_fill(&self, account_id: &str, limit: usize) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id FROM conversations c WHERE c.account_id = ?1 AND c.filled = 0
             ORDER BY (SELECT max(m.sent_at) FROM messages m
                       WHERE m.account_id = c.account_id AND m.conversation_id = c.id) DESC, c.id
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], |r| r.get::<_, i64>(0))?;
        Ok(rows.map(|r| r.map(|id| id as u64)).collect::<rusqlite::Result<_>>()?)
    }

    /// These conversations are complete here, until a new message joins one of them.
    pub fn mark_filled(&mut self, account_id: &str, conversations: &[u64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in conversations {
            tx.execute(
                "UPDATE conversations SET filled = 1 WHERE account_id = ?1 AND id = ?2",
                params![account_id, *id as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether a message is stored already, judged from its headers (its Message-ID), so
    /// sync can skip downloading it again.
    pub fn knows(&self, account_id: &str, headers: &[u8]) -> Result<bool> {
        let Some(parsed) = parse(headers) else {
            return Ok(false);
        };
        let found = self
            .conn
            .query_row(
                "SELECT 1 FROM messages WHERE account_id = ?1 AND message_id = ?2",
                params![account_id, parsed.message_id],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::MessageSource;

    fn at(store: &mut Store, mailbox: &str, provider_id: &str, raw: &[u8]) {
        let source = MessageSource { mailbox: mailbox.into(), ..source(provider_id) };
        store.ingest(raw, &source).unwrap();
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn messages_without_a_conversation_are_listed_newest_first() {
        let mut store = store_with_work();
        at(&mut store, "INBOX", "1:1", &mail().id("a@x").at(NOV_1).build());
        at(&mut store, "INBOX", "1:3", &mail().id("c@x").at(NOV_1 + 2 * DAY).build());
        at(&mut store, "INBOX", "1:2", &mail().id("b@x").at(NOV_1 + DAY).build());
        at(&mut store, "INBOX", "local:<d@x>", &mail().id("d@x").at(NOV_1 + 3 * DAY).build());
        at(&mut store, "Sent", "1:9", &mail().id("s@x").build());
        assert_eq!(store.unlinked("work", "INBOX", 2).unwrap(), ["1:3", "1:2"]);
        assert_eq!(store.unlinked("work", "Sent", 10).unwrap(), ["1:9"]);
        assert!(store.unlinked("work", "Nope", 10).unwrap().is_empty());
        assert!(store.unlinked("other", "INBOX", 10).unwrap().is_empty());
    }

    #[test]
    fn linking_queues_each_conversation_until_it_is_filled() {
        let mut store = store_with_work();
        at(&mut store, "INBOX", "1:1", &mail().id("a@x").build());
        at(&mut store, "INBOX", "1:2", &mail().id("b@x").build());
        at(&mut store, "INBOX", "1:3", &mail().id("c@x").build());
        store
            .link_conversations(
                "work",
                "INBOX",
                &ids(&["1:1", "1:2", "1:3"]),
                &[("1:1".into(), 77), ("1:2".into(), 77)],
            )
            .unwrap();
        // Asked once: the one Gmail didn't answer for isn't asked again.
        assert!(store.unlinked("work", "INBOX", 10).unwrap().is_empty());
        assert_eq!(store.conversations_to_fill("work", 10).unwrap(), [77]);
        store.mark_filled("work", &[77]).unwrap();
        assert!(store.conversations_to_fill("work", 10).unwrap().is_empty());

        // Seeing the same messages again changes nothing.
        store.link_conversations("work", "INBOX", &ids(&["1:1"]), &[("1:1".into(), 77)]).unwrap();
        assert!(store.conversations_to_fill("work", 10).unwrap().is_empty());

        // A new message in the conversation queues it again.
        at(&mut store, "INBOX", "1:4", &mail().id("d@x").build());
        store.link_conversations("work", "INBOX", &ids(&["1:4"]), &[("1:4".into(), 77)]).unwrap();
        assert_eq!(store.conversations_to_fill("work", 10).unwrap(), [77]);
        assert!(store.conversations_to_fill("other", 10).unwrap().is_empty());
    }

    #[test]
    fn the_most_recent_conversations_fill_first() {
        let mut store = store_with_work();
        at(&mut store, "INBOX", "1:1", &mail().id("old@x").subject("Old").at(NOV_1).build());
        at(&mut store, "INBOX", "1:2", &mail().id("new@x").subject("New").at(NOV_1 + DAY).build());
        at(
            &mut store,
            "INBOX",
            "1:3",
            &mail().id("mid@x").subject("Mid").at(NOV_1 + DAY / 2).build(),
        );
        let found = [("1:1".into(), 1), ("1:2".into(), u64::MAX - 1), ("1:3".into(), 3)];
        store.link_conversations("work", "INBOX", &ids(&["1:1", "1:2", "1:3"]), &found).unwrap();
        // Gmail's ids are 64-bit and come back as they went in.
        assert_eq!(store.conversations_to_fill("work", 10).unwrap(), [u64::MAX - 1, 3, 1]);
        assert_eq!(store.conversations_to_fill("work", 1).unwrap(), [u64::MAX - 1]);
    }

    #[test]
    fn a_message_is_known_by_its_message_id() {
        let mut store = store_with_work();
        at(&mut store, "INBOX", "1:1", &mail().build());
        assert!(store.knows("work", b"Message-ID: <q4@northwind.co>\r\n\r\n").unwrap());
        assert!(!store.knows("work", b"Message-ID: <other@northwind.co>\r\n\r\n").unwrap());
        assert!(!store.knows("other", b"Message-ID: <q4@northwind.co>\r\n\r\n").unwrap());
        // Nothing to go on: fetch it, and let ingest sort it out.
        assert!(!store.knows("work", b"").unwrap());
    }

    #[test]
    fn failed_writes_are_errors() {
        let mut store = store_with_work();
        at(&mut store, "INBOX", "1:1", &mail().build());
        read_only(&store);
        assert!(store
            .link_conversations("work", "INBOX", &ids(&["1:1"]), &[("1:1".into(), 7)])
            .is_err());
        assert!(store.link_conversations("work", "INBOX", &ids(&["1:1"]), &[]).is_err());
        assert!(store.mark_filled("work", &[7]).is_err());
    }

    #[test]
    fn a_broken_schema_is_an_error() {
        let store = store_with_work();
        break_schema(&store, "DROP TABLE conversations; DROP TABLE message_locations");
        assert!(store.unlinked("work", "INBOX", 1).is_err());
        assert!(store.conversations_to_fill("work", 1).is_err());
        break_schema(&store, "DROP TABLE messages");
        assert!(store.knows("work", b"Message-ID: <a@x>\r\n\r\n").is_err());
    }
}
