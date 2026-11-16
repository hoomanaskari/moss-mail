//! Forwards: which messages you forwarded, so opening one later says so.

use rusqlite::{params, OptionalExtension};

use crate::codec::parse_id;
use crate::{Location, Result, Store};

impl Store {
    /// Commit the local forward and its server updates together, once per sent copy.
    /// Targets and queue ownership come from the original message, not the sending account.
    /// Returns the account with newly queued work. Empty courtesy intent still records the forward.
    pub fn record_forward_with_changes(&mut self, message_id: &str, sent_id: &str, now: i64,
        build: impl FnOnce(&Self, &str, Vec<Location>) -> Result<Vec<String>>) -> Result<Option<String>> {
        self.delivery_transaction(|store| {
            let account: Option<String> = store.conn.query_row("SELECT account_id FROM messages WHERE id=?1",
                [parse_id(message_id)], |row| row.get(0)).optional()?;
            let Some(account) = account else { return Ok(None); };
            let recorded: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM forwards WHERE message_id=?1 AND sent_id=?2)",
                [parse_id(message_id), parse_id(sent_id)], |row| row.get(0))?;
            if recorded { return Ok(None); }
            if store.conn.execute("INSERT INTO forwards(message_id,sent_id) VALUES(?1,?2)",
                [parse_id(message_id), parse_id(sent_id)])? != 1 {
                return Err(crate::Error::Invalid("The local forward receipt was not saved.".into()));
            }
            if store.conn.execute("UPDATE messages SET forwarded=1 WHERE id=?1", [parse_id(message_id)])? != 1 {
                return Err(crate::Error::Invalid("The local forwarded mark was not saved.".into()));
            }
            let changes = build(store, &account, store.message_locations(message_id)?)?;
            store.queue_batch_in_transaction(&account, crate::BatchKind::Forwarded, &changes, now)?;
            Ok((!changes.is_empty()).then_some(account))
        })
    }

    /// Forwarded was set or cleared in another mail app. Returns whether the message is stored.
    pub fn set_forwarded(
        &mut self,
        account_id: &str,
        mailbox: &str,
        provider_id: &str,
        forwarded: bool,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE messages SET forwarded = ?4 WHERE id IN (SELECT l.message_id FROM message_locations l
             JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE b.account_id = ?1 AND b.name = ?2 AND l.provider_id = ?3)",
            params![account_id, mailbox, provider_id, forwarded],
        )? > 0)
    }

    /// Where the server keeps one message, so a change to it alone can be sent there.
    pub fn message_locations(&self, message_id: &str) -> Result<Vec<Location>> {
        let mut stmt = self.conn.prepare(
            "SELECT b.name, l.provider_id FROM message_locations l JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE l.message_id = ?1 ORDER BY b.name, l.provider_id",
        )?;
        let rows = stmt.query_map([parse_id(message_id)], |r| {
            Ok(Location { mailbox: r.get(0)?, provider_id: r.get(1)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{Forward, MessageSource, Person};

    fn person(name: &str, email: &str) -> Person {
        Person { name: name.into(), email: email.into() }
    }

    fn single_forward_change(_: &Store, _: &str, _: Vec<Location>) -> Result<Vec<String>> {
        Ok(vec!["forwarded".into()])
    }

    /// A forward of mail with no server copy to mark.
    fn no_changes(_: &Store, _: &str, _: Vec<crate::Location>) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn sent_forward(store: &mut Store) -> crate::Ingested {
        let raw = mail()
            .id("fwd@northwind.co")
            .from("Maya Ross <maya@northwind.co>")
            .to("Rebecca Vale <rebecca@vale.example>")
            .header("Cc", "Jonas Weber <jonas@northwind.co>")
            .subject("Fwd: Q4 budget")
            .at(NOV_1 + 60_000)
            .build();
        store.ingest(&raw, &MessageSource { mailbox: "Sent".into(), ..source("2:1") }).unwrap()
    }

    #[test]
    fn a_forward_from_moss_shows_on_the_message_it_forwarded() {
        let mut store = store_with_work();
        let original = ingest(&mut store, "1:7", &mail().build());
        let sent = sent_forward(&mut store);
        store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, no_changes).unwrap();
        // Saving it twice, as a retried send might, still lists it once.
        store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, no_changes).unwrap();

        let thread = store.thread(&original.thread_id, NOV_1).unwrap().unwrap();
        let m = &thread.messages[0];
        assert_eq!(
            m.forwards,
            [Forward {
                thread_id: sent.thread_id.clone(),
                to: vec![person("Rebecca Vale", "rebecca@vale.example"), person("Jonas Weber", "jonas@northwind.co")],
                sent_at: NOV_1 + 60_000,
            }]
        );
        assert!(m.forwarded);
        // The forward itself wasn't forwarded.
        let fwd = store.thread(&sent.thread_id, NOV_1).unwrap().unwrap();
        assert!(fwd.messages[0].forwards.is_empty() && !fwd.messages[0].forwarded);
    }

    #[test]
    fn forward_intent_uses_the_original_account_and_one_batch() {
        let mut store = store_with_work();
        store.upsert_account(&account("other", "sender@example.test")).unwrap();
        let original = ingest(&mut store, "1:7", &mail().build());
        let sent = store.ingest(&mail().id("sent@example.test").build(), &MessageSource { account_id: "other".into(), mailbox: "Sent".into(), ..source("2:1") }).unwrap();
        let changes = vec!["first".into(), "second".into()];
        assert!(store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, |_, _, _| Ok(changes.clone())).unwrap().is_some());
        assert!(store.pending_changes("other").unwrap().is_empty());
        let pending = store.pending_changes("work").unwrap();
        assert_eq!(pending.len(), 2);
        let batch = store.batch_for_change("work", pending[0].0).unwrap().unwrap();
        assert_eq!(batch.kind, crate::BatchKind::Forwarded);
        assert_eq!(batch.parts.len(), 2);
        assert_eq!(store.batch_for_change("work", pending[1].0).unwrap().unwrap(), batch);
    }

    #[test]
    fn separate_sends_get_separate_forward_batches() {
        let mut store = store_with_work();
        let original = ingest(&mut store, "1:7", &mail().build());
        let first = sent_forward(&mut store);
        let second = store.ingest(&mail().id("second@example.test").build(), &MessageSource { mailbox:"Sent".into(), ..source("2:2") }).unwrap();
        for sent in [&first, &second] {
            assert_eq!(store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, |_, account, locations| {
                assert_eq!(account, "work");
                assert_eq!(locations.len(), 1);
                Ok(vec!["forwarded".into()])
            }).unwrap().as_deref(), Some("work"));
        }
        let pending = store.pending_changes("work").unwrap();
        assert_eq!(pending.len(), 2);
        assert_ne!(store.batch_for_change("work", pending[0].0).unwrap().unwrap().id, store.batch_for_change("work", pending[1].0).unwrap().unwrap().id);
        assert_eq!(store.conn.query_row("SELECT count(*) FROM forwards", [], |r|r.get::<_,i64>(0)).unwrap(), 2);
    }

    #[test]
    fn forward_repair_and_server_updates_commit_together_once() {
        let mut store = store_with_work();
        let original = ingest(&mut store, "1:7", &mail().build());
        let sent = sent_forward(&mut store);
        let changes = vec!["forwarded-inbox".into(), "forwarded-archive".into()];
        let build = |_: &Store, _: &str, _: Vec<Location>| Ok(changes.clone());
        store.conn.execute_batch("CREATE TRIGGER refuse_second_forward BEFORE INSERT ON outbox WHEN NEW.change = 'forwarded-archive' BEGIN SELECT RAISE(ABORT, 'full'); END;").unwrap();
        assert!(store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, build).is_err());
        assert!(store.pending_changes("work").unwrap().is_empty());
        let message = &store.thread(&original.thread_id, NOV_1).unwrap().unwrap().messages[0];
        assert!(!message.forwarded);
        assert!(message.forwards.is_empty());
        store.conn.execute_batch("DROP TRIGGER refuse_second_forward").unwrap();
        assert!(store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, build).unwrap().is_some());
        assert!(!store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, build).unwrap().is_some());
        let pending = store.pending_changes("work").unwrap();
        assert_eq!(pending.iter().map(|(_, text)| text.clone()).collect::<Vec<_>>(), changes);
        for (id, _) in pending { store.drop_change(id).unwrap(); }
        assert!(!store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, build).unwrap().is_some());
        assert!(store.pending_changes("work").unwrap().is_empty(), "completed server work must not be recreated");
    }

    #[test]
    fn repair_skips_an_original_that_was_removed_after_sending() {
        let mut store = store_with_work();
        let original = ingest(&mut store, "1:7", &mail().build());
        let sent = sent_forward(&mut store);
        store.conn.execute("DELETE FROM messages WHERE id = ?1", [&original.message_id]).unwrap();
        assert!(!store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, single_forward_change).unwrap().is_some());
        assert!(store.pending_changes("work").unwrap().is_empty());
        assert!(store.thread(&sent.thread_id, NOV_1).unwrap().is_some(), "the accepted sent copy remains intact");
    }

    #[test]
    fn failed_forward_records_never_queue_server_changes() {
        for trigger in [
            "CREATE TRIGGER refuse_forward BEFORE INSERT ON forwards BEGIN SELECT RAISE(ABORT, 'full'); END;",
            "CREATE TRIGGER refuse_forward BEFORE UPDATE OF forwarded ON messages BEGIN SELECT RAISE(ABORT, 'full'); END;",
            "CREATE TRIGGER refuse_forward BEFORE UPDATE OF forwarded ON messages BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER refuse_forward BEFORE INSERT ON forwards BEGIN SELECT RAISE(IGNORE); END;",
        ] {
            let mut store = store_with_work();
            let original = ingest(&mut store, "1:7", &mail().build());
            let sent = sent_forward(&mut store);
            store.conn.execute_batch(trigger).unwrap();
            assert!(store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, single_forward_change).is_err());
            assert!(store.pending_changes("work").unwrap().is_empty());
            let message = &store.thread(&original.thread_id, NOV_1).unwrap().unwrap().messages[0];
            assert!(!message.forwarded);
            assert!(message.forwards.is_empty());
            store.conn.execute_batch("DROP TRIGGER refuse_forward").unwrap();
            assert_eq!(store.record_forward_with_changes(&original.message_id, &sent.message_id, NOV_1, single_forward_change).unwrap().as_deref(), Some("work"));
            assert_eq!(store.pending_changes("work").unwrap().len(), 1);
            assert!(store.thread(&original.thread_id, NOV_1).unwrap().unwrap().messages[0].forwarded);
        }
    }

    #[test]
    fn a_forward_made_in_another_app_comes_from_the_server_flag() {
        let mut store = store_with_work();
        let id = store
            .ingest(&mail().build(), &MessageSource { forwarded: true, ..source("1:7") })
            .unwrap()
            .thread_id;
        let m = || store.thread(&id, NOV_1).unwrap().unwrap().messages[0].clone();
        assert!(m().forwarded && m().forwards.is_empty());

        assert!(store.set_forwarded("work", "INBOX", "1:7", false).unwrap());
        assert!(!store.thread(&id, NOV_1).unwrap().unwrap().messages[0].forwarded);
        assert!(store.set_forwarded("work", "INBOX", "1:7", true).unwrap());
        assert!(store.thread(&id, NOV_1).unwrap().unwrap().messages[0].forwarded);
        assert!(!store.set_forwarded("work", "INBOX", "nope", true).unwrap());
    }

    #[test]
    fn a_message_is_found_where_the_server_keeps_it() {
        let mut store = store_with_work();
        let original = ingest(&mut store, "1:7", &mail().build());
        store
            .ingest(&mail().build(), &MessageSource { mailbox: "[Gmail]/All Mail".into(), ..source("1:90") })
            .unwrap();
        // Another message in the same thread isn't included.
        ingest(&mut store, "1:8", &mail().id("r@x").reply_to(&["q4@northwind.co"]).build());
        let pairs: Vec<_> = store
            .message_locations(&original.message_id)
            .unwrap()
            .into_iter()
            .map(|l| (l.mailbox, l.provider_id))
            .collect();
        assert_eq!(
            pairs,
            [("INBOX".to_string(), "1:7".to_string()), ("[Gmail]/All Mail".into(), "1:90".into())]
        );
        assert!(store.message_locations("999").unwrap().is_empty());
    }

    #[test]
    fn a_broken_schema_is_an_error() {
        let mut store = store_with_work();
        let original = ingest(&mut store, "1:7", &mail().build());
        let mut other = store_with_work();
        let theirs = ingest(&mut other, "1:7", &mail().build());

        // Reading a thread fails on its forwards, then on the forwarded mark.
        break_schema(&store, "ALTER TABLE forwards RENAME TO old_forwards");
        assert!(store.thread(&original.thread_id, NOV_1).is_err());
        break_schema(&other, "ALTER TABLE messages RENAME COLUMN forwarded TO fwd");
        assert!(other.thread(&theirs.thread_id, NOV_1).is_err());

        break_schema(&store, "ALTER TABLE message_locations RENAME TO old_locations");
        assert!(store.message_locations(&original.message_id).is_err());
    }

    #[test]
    fn an_unreadable_location_is_an_error_instead_of_a_partial_result() {
        for sql in ["UPDATE message_locations SET provider_id = x'80'", "UPDATE mailboxes SET name = x'80'"] {
            let mut store = store_with_work();
            let original = ingest(&mut store, "1:7", &mail().build());
            store.conn.execute(sql, []).unwrap();
            assert!(store.message_locations(&original.message_id).is_err());
        }
    }

    #[test]
    fn a_failed_write_is_an_error() {
        let mut store = store_with_work();
        let original = ingest(&mut store, "1:7", &mail().build());
        read_only(&store);
        assert!(store.record_forward_with_changes(&original.message_id, &original.message_id, NOV_1, no_changes).is_err());
        assert!(store.set_forwarded("work", "INBOX", "1:7", true).is_err());
    }
}
