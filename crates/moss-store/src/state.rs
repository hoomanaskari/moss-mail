//! Changing per-message state from the UI. Each change applies to the messages of a thread.

use rusqlite::params;

use crate::codec::parse_id;
use crate::{Error, Result, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadChange { Done(bool), Read(bool), Delete, Restore }

/// Server targets captured before local deletion metadata changes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThreadTargets {
    pub trash: Vec<String>,
    pub untrash_inbox: Vec<String>,
    pub untrash_archive: Vec<String>,
}

#[derive(Default)]
struct CapturedDeletion { count: u64, any_inbox: bool, targets: ThreadTargets }

impl Store {
    /// Commit one local thread action and all its server intent together.
    /// Delete and Restore receive targets captured before their metadata is changed.
    pub fn change_thread_with_changes(&mut self, thread_id: &str, expected_account: &str,
        action: ThreadChange, now: i64,
        build: impl FnOnce(&Self, ThreadTargets) -> Result<Vec<String>>) -> Result<Option<String>> {
        self.delivery_transaction(|store| {
            let id = store.thread_pk(thread_id)?;
            let account: String = store.conn.query_row("SELECT account_id FROM threads WHERE id=?1", [id], |row| row.get(0))?;
            if account != expected_account { return Err(Error::Invalid("This conversation belongs to a different account. Refresh it before changing it.".into())); }
            let captured = match action {
                ThreadChange::Delete => store.capture_deletion(id, false)?,
                ThreadChange::Restore => store.capture_deletion(id, true)?,
                _ => CapturedDeletion { count: store.conn.query_row("SELECT count(*) FROM messages WHERE thread_id=?1", [id], |row| row.get::<_,i64>(0))? as u64, ..Default::default() },
            };
            if captured.count == 0 { return Ok(None); }
            match action {
                ThreadChange::Done(done) => store.mark_done(thread_id, done, now)?,
                ThreadChange::Read(read) => store.mark_read(thread_id, read)?,
                ThreadChange::Delete => store.delete_thread(thread_id)?,
                ThreadChange::Restore => store.apply_restoration(id, captured.count)?,
            }
            // Each action ends with its message UPDATE. No callback or other write can
            // intervene before checking the affected rows on this locked connection.
            if store.conn.changes() != captured.count { return Err(Error::Invalid("The conversation change was not saved completely.".into())); }
            let kind = match action {
                ThreadChange::Done(true)=>crate::BatchKind::Done,ThreadChange::Done(false)=>crate::BatchKind::Undone,
                ThreadChange::Read(_)=>crate::BatchKind::Read,ThreadChange::Delete=>crate::BatchKind::Delete,ThreadChange::Restore=>crate::BatchKind::Restore,
            };
            store.queue_batch_in_transaction(&account, kind, &build(store, captured.targets)?, now)?;
            Ok(Some(account))
        })
    }

    fn capture_deletion(&self, thread: i64, restore: bool) -> Result<CapturedDeletion> {
        let mut statement = self.conn.prepare(
            "SELECT message_id,deleted_from FROM messages WHERE thread_id=?1
             AND ((?2 AND deleted_from IS NOT NULL) OR (NOT ?2 AND deleted_from IS NULL)) ORDER BY sent_at,id")?;
        let mut rows = statement.query(params![thread,restore])?;
        let mut captured = CapturedDeletion::default();
        while let Some(row) = rows.next()? {
            captured.count += 1;
            let message_id: String = row.get(0)?;
            // Match message_ids(): absent RFC Message-IDs use local sha256 placeholders.
            let has_id = !message_id.as_bytes().get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"sha256:"));
            if restore {
                let from: String = row.get(1)?;
                let inbox = match from.as_str() {
                    "inbox" => true,
                    "done" => false,
                    _ => return Err(Error::Invalid("This conversation has an unknown saved deletion destination.".into())),
                };
                captured.any_inbox |= inbox;
                if has_id {
                    if inbox { captured.targets.untrash_inbox.push(message_id); }
                    else { captured.targets.untrash_archive.push(message_id); }
                }
            } else if has_id { captured.targets.trash.push(message_id); }
        }
        Ok(captured)
    }

    fn apply_restoration(&self, thread: i64, expected: u64) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE messages SET deleted_from=NULL,
             done=CASE WHEN deleted_from='inbox' THEN 0 ELSE done END,
             done_at=CASE WHEN deleted_from='inbox' THEN NULL ELSE done_at END
             WHERE thread_id=?1 AND deleted_from IS NOT NULL", [thread])?;
        if changed as u64 != expected { return Err(Error::Invalid("The conversation restore was not saved completely.".into())); }
        Ok(())
    }

    /// The database id of a thread, or `NotFound`.
    pub(crate) fn thread_pk(&self, thread_id: &str) -> Result<i64> {
        let id = parse_id(thread_id);
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM threads WHERE id = ?1)",
            [id],
            |r| r.get(0),
        )?;
        if exists {
            Ok(id)
        } else {
            Err(Error::NotFound { kind: "thread", id: thread_id.into() })
        }
    }

    /// Done takes a thread out of the list (and clears any snooze), noting `now` (ms) for the
    /// Done list; not done puts it back.
    pub fn mark_done(&self, thread_id: &str, done: bool, now: i64) -> Result<()> {
        let id = self.thread_pk(thread_id)?;
        self.conn.execute(
            "UPDATE messages SET done = ?2, snoozed_until = CASE WHEN ?2 THEN NULL ELSE snoozed_until END,
                 done_at = CASE WHEN ?2 THEN ?3 END
             WHERE thread_id = ?1",
            params![id, done, now],
        )?;
        Ok(())
    }

    /// Delete takes a thread out of every list, remembering whether it was open or done.
    pub fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let id = self.thread_pk(thread_id)?;
        self.conn.execute(
            "UPDATE messages SET deleted_from = CASE
                 WHEN EXISTS (SELECT 1 FROM messages WHERE thread_id = ?1 AND done = 0 AND deleted_from IS NULL) THEN 'inbox'
                 ELSE 'done' END
             WHERE thread_id = ?1 AND deleted_from IS NULL",
            [id],
        )?;
        Ok(())
    }

    /// Restore each deleted message to its saved destination; leave undeleted mail alone.
    /// Returns whether any restored row belonged in the Inbox, or None for no deleted rows.
    pub fn restore_thread(&mut self, thread_id: &str) -> Result<Option<bool>> {
        self.delivery_transaction(|store| {
            let id = store.thread_pk(thread_id)?;
            let captured = store.capture_deletion(id, true)?;
            if captured.count == 0 { return Ok(None); }
            store.apply_restoration(id, captured.count)?;
            Ok(Some(captured.any_inbox))
        })
    }

    pub fn mark_read(&self, thread_id: &str, read: bool) -> Result<()> {
        let id = self.thread_pk(thread_id)?;
        self.conn
            .execute("UPDATE messages SET read = ?2 WHERE thread_id = ?1", params![id, read])?;
        Ok(())
    }

    /// Snoozes the open messages until `until` (ms since the epoch); `None` wakes them now.
    /// A new message on the thread brings it back early.
    pub fn snooze(&mut self, thread_id: &str, until: Option<i64>) -> Result<()> {
        let id = self.thread_pk(thread_id)?;
        let tx = self.conn.transaction()?;
        tx.execute("INSERT OR REPLACE INTO snooze_edits (thread_id) VALUES (?1)", [id])?;
        tx.execute(
            "UPDATE messages SET snoozed_until = ?2 WHERE thread_id = ?1 AND done = 0",
            params![id, until],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{Section, SplitId};

    #[test]
    fn thread_intent_is_account_checked_durable_and_rolls_back_builder_failure() {
        let mut store=store_with_work();
        let id=ingest(&mut store,"1",&mail().build()).thread_id;
        let before=summary(&store,&id);
        let calls=std::cell::Cell::new(0);
        let build=|s: &Store,_| {
            calls.set(calls.get()+1);
            assert_eq!(s.conn.pragma_query_value(None,"synchronous",|r|r.get::<_,i64>(0)).unwrap(),2);
            assert!(s.conn.pragma_query_value(None,"fullfsync",|r|r.get::<_,bool>(0)).unwrap());
            Ok(vec!["first".into(),"second".into()])
        };
        assert!(store.change_thread_with_changes(&id,"other",ThreadChange::Done(true),NOV_1,&build).is_err());
        assert_eq!(calls.get(),0);
        assert_eq!(store.change_thread_with_changes(&id,"work",ThreadChange::Restore,NOV_1,&build).unwrap(),None);
        assert_eq!(calls.get(),0);
        assert!(store.change_thread_with_changes(&id,"work",ThreadChange::Done(true),NOV_1,|_,_|Err(Error::Invalid("fixture".into()))).is_err());
        assert_eq!(summary(&store,&id),before);
        assert!(store.pending_changes("work").unwrap().is_empty());
        assert_eq!(store.change_thread_with_changes(&id,"work",ThreadChange::Done(true),NOV_1,&build).unwrap(),Some("work".into()));
        assert_eq!(calls.get(),1);
        assert!(summary(&store,&id).done);
        assert_eq!(store.pending_changes("work").unwrap().iter().map(|(_,s)|s.as_str()).collect::<Vec<_>>(),["first","second"]);
    }

    #[test]
    fn a_later_intent_insert_failure_rolls_back_the_whole_thread_action() {
        for action in ["IGNORE","ABORT, 'fixture'"] {
            let mut store=store_with_work();
            let id=ingest(&mut store,"1",&mail().build()).thread_id;
            let before=summary(&store,&id);
            store.conn.execute_batch(&format!("CREATE TRIGGER fail_second_intent BEFORE INSERT ON outbox WHEN NEW.change='second' BEGIN SELECT RAISE({action}); END;")).unwrap();
            let build=|_: &Store,_|Ok(vec!["first".into(),"second".into()]);
            assert!(store.change_thread_with_changes(&id,"work",ThreadChange::Done(true),NOV_1,build).is_err());
            assert_eq!(summary(&store,&id),before);
            assert!(store.pending_changes("work").unwrap().is_empty());
            store.conn.execute_batch("DROP TRIGGER fail_second_intent").unwrap();
            store.change_thread_with_changes(&id,"work",ThreadChange::Done(true),NOV_1,build).unwrap();
            assert_eq!(store.pending_changes("work").unwrap().len(),2);
        }
    }

    #[test]
    fn delete_destination_ignores_already_deleted_messages() {
        let mut store=store_with_work();
        let id=ingest(&mut store,"1",&mail().id("first@x").build()).thread_id;
        ingest(&mut store,"2",&mail().id("second@x").reply_to(&["first@x"]).build());
        store.conn.execute_batch("UPDATE messages SET deleted_from='inbox',done=0 WHERE message_id='first@x'; UPDATE messages SET done=1,done_at=123 WHERE message_id='second@x';").unwrap();
        store.delete_thread(&id).unwrap();
        let from:String=store.conn.query_row("SELECT deleted_from FROM messages WHERE message_id='second@x'",[],|r|r.get(0)).unwrap();
        assert_eq!(from,"done");
    }

    #[test]
    fn restore_rejects_unknown_destinations_and_rolls_back_partial_updates() {
        let mut store=store_with_work();
        let id=ingest(&mut store,"1",&mail().id("first@x").build()).thread_id;
        ingest(&mut store,"2",&mail().id("second@x").reply_to(&["first@x"]).build());
        store.conn.execute_batch("UPDATE messages SET deleted_from='inbox',done=1,done_at=123; PRAGMA ignore_check_constraints=ON; UPDATE messages SET deleted_from='future' WHERE message_id='second@x';").unwrap();
        assert!(matches!(store.restore_thread(&id),Err(Error::Invalid(why)) if why=="This conversation has an unknown saved deletion destination."));
        assert_eq!(store.conn.query_row("SELECT count(*) FROM messages WHERE deleted_from IS NOT NULL AND done=1",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        store.conn.execute_batch("UPDATE messages SET deleted_from='done' WHERE message_id='second@x'; PRAGMA ignore_check_constraints=OFF; CREATE TRIGGER ignore_one_restore BEFORE UPDATE ON messages WHEN OLD.message_id='second@x' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.restore_thread(&id).is_err());
        assert_eq!(store.conn.query_row("SELECT count(*) FROM messages WHERE deleted_from IS NOT NULL AND done=1 AND done_at=123",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        store.conn.execute_batch("DROP TRIGGER ignore_one_restore").unwrap();
        assert_eq!(store.restore_thread(&id).unwrap(),Some(true));
        assert_eq!(store.conn.query_row("SELECT count(*) FROM messages WHERE deleted_from IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(store.conn.query_row("SELECT done_at FROM messages WHERE message_id='second@x'",[],|r|r.get::<_,i64>(0)).unwrap(),123);
    }

    #[test]
    fn restore_builder_failure_preserves_the_captured_destinations() {
        let mut store=store_with_work();
        let id=ingest(&mut store,"1",&mail().id("first@x").build()).thread_id;
        ingest(&mut store,"2",&mail().id("second@x").reply_to(&["first@x"]).build());
        store.conn.execute_batch("UPDATE messages SET deleted_from=CASE message_id WHEN 'first@x' THEN 'inbox' ELSE 'done' END,done=1,done_at=123;").unwrap();
        assert!(store.change_thread_with_changes(&id,"work",ThreadChange::Restore,NOV_1,|_,targets| {
            assert_eq!(targets,ThreadTargets {trash:vec![],untrash_inbox:vec!["first@x".into()],untrash_archive:vec!["second@x".into()]});
            Err(Error::Invalid("fixture".into()))
        }).is_err());
        assert_eq!(store.conn.query_row("SELECT count(*) FROM messages WHERE deleted_from IS NOT NULL AND done=1 AND done_at=123",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert!(store.pending_changes("work").unwrap().is_empty());
    }

    #[test]
    fn mail_without_a_message_id_is_deleted_and_restored_here_but_not_looked_for_on_the_server() {
        let mut store=store_with_work();
        let id=ingest(&mut store,"1",&mail().id("first@x").build()).thread_id;
        ingest(&mut store,"2",&mail().without("Message-ID").reply_to(&["first@x"]).build());
        let seen=std::cell::RefCell::new(vec![]);
        let build=|_: &Store,targets: ThreadTargets| { seen.borrow_mut().push(targets); Ok(vec!["intent".into()]) };
        store.change_thread_with_changes(&id,"work",ThreadChange::Delete,NOV_1,build).unwrap();
        store.change_thread_with_changes(&id,"work",ThreadChange::Restore,NOV_1,build).unwrap();
        assert_eq!(*seen.borrow(),[
            ThreadTargets {trash:vec!["first@x".into()],untrash_inbox:vec![],untrash_archive:vec![]},
            ThreadTargets {trash:vec![],untrash_inbox:vec!["first@x".into()],untrash_archive:vec![]},
        ]);
        assert_eq!(store.conn.query_row("SELECT count(*) FROM messages WHERE deleted_from IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    }

    #[test]
    fn empty_thread_actions_do_not_build_server_intent() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        let called = std::cell::Cell::new(false);
        let build = |_: &Store, _| {
            called.set(true);
            Ok(vec!["intent".into()])
        };
        assert_eq!(store.change_thread_with_changes(&id, "work", ThreadChange::Read(true), NOV_1, build).unwrap(), Some("work".into()));
        assert!(called.get());
        let pending = store.pending_changes("work").unwrap();
        assert_eq!(pending.len(), 1);
        assert!(store.drop_change(pending[0].0).unwrap());
        store.conn.execute("DELETE FROM messages", []).unwrap();
        for action in [ThreadChange::Done(true), ThreadChange::Done(false), ThreadChange::Read(true), ThreadChange::Delete, ThreadChange::Restore] {
            called.set(false);
            let result = store.change_thread_with_changes(&id, "work", action, NOV_1, build).unwrap();
            assert_eq!(result, None);
            assert!(!called.get());
            assert!(store.pending_changes("work").unwrap().is_empty());
        }
    }

    #[test]
    fn unreadable_deletion_targets_leave_state_and_queue_unchanged() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        let build = |_: &Store, _| Ok(vec![]);
        assert_eq!(store.change_thread_with_changes(&id, "work", ThreadChange::Read(true), NOV_1, build).unwrap(), Some("work".into()));
        store.conn.execute("UPDATE messages SET message_id=x'ff',deleted_from='inbox'", []).unwrap();
        for action in [ThreadChange::Restore, ThreadChange::Delete] {
            if action == ThreadChange::Delete { store.conn.execute("UPDATE messages SET deleted_from=NULL", []).unwrap(); }
            assert!(matches!(store.change_thread_with_changes(&id, "work", action, NOV_1, build), Err(Error::Sqlite(_))));
            let deleted: Option<String> = store.conn.query_row("SELECT deleted_from FROM messages", [], |r| r.get(0)).unwrap();
            assert_eq!(deleted.as_deref(), if action == ThreadChange::Restore { Some("inbox") } else { None });
            assert!(store.pending_changes("work").unwrap().is_empty());
        }
    }

    fn summary(store: &Store, id: &str) -> moss_core::ThreadSummary {
        store.thread(id, NOV_1).unwrap().unwrap().summary
    }

    #[test]
    fn a_failed_snooze_keeps_the_previous_choice_and_order() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        store.snooze(&id, Some(NOV_1 + DAY)).unwrap();
        let before: i64 = store.conn.query_row("SELECT id FROM snooze_edits WHERE thread_id = ?1", [&id], |r| r.get(0)).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_snooze BEFORE UPDATE OF snoozed_until ON messages BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.snooze(&id, None).is_err());
        assert_eq!(summary(&store, &id).snoozed_until, Some(NOV_1 + DAY));
        let after: i64 = store.conn.query_row("SELECT id FROM snooze_edits WHERE thread_id = ?1", [&id], |r| r.get(0)).unwrap();
        assert_eq!(after, before, "a failed action must not supersede a pending send's reminder");
        store.conn.execute_batch("DROP TRIGGER refuse_snooze").unwrap();
        store.snooze(&id, None).unwrap();
        assert_eq!(summary(&store, &id).snoozed_until, None);
        let after: i64 = store.conn.query_row("SELECT id FROM snooze_edits WHERE thread_id = ?1", [&id], |r| r.get(0)).unwrap();
        assert!(after > before);
        let count: i64 = store.conn.query_row("SELECT count(*) FROM snooze_edits", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "only the latest choice is retained");
    }

    #[test]
    fn done_clears_snooze_and_can_be_undone() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        store.snooze(&id, Some(NOV_1 + DAY)).unwrap();
        store.mark_done(&id, true, NOV_1 + 5).unwrap();
        let done = summary(&store, &id);
        assert!(done.done);
        assert_eq!(done.done_at, Some(NOV_1 + 5));
        store.mark_done(&id, false, NOV_1 + 9).unwrap();
        let back = summary(&store, &id);
        assert!(!back.done);
        assert_eq!(back.snoozed_until, None);
        assert_eq!(back.done_at, None);
        let stamped: u32 = store
            .conn
            .query_row("SELECT count(*) FROM messages WHERE done_at IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stamped, 0, "not done forgets when it was done");
    }

    #[test]
    fn read_split_and_section_change_the_whole_thread() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        ingest(&mut store, "2", &mail().id("r@x").reply_to(&["q4@northwind.co"]).build());
        store.mark_read(&id, true).unwrap();
        assert!(!summary(&store, &id).unread);
        store.mark_read(&id, false).unwrap();
        assert!(summary(&store, &id).unread);
        store.set_split(&id, SplitId::Other, NOV_1).unwrap();
        store.set_section(&id, Section::Info, NOV_1).unwrap();
        let moved = summary(&store, &id);
        assert_eq!((moved.split, moved.section), (SplitId::Other, Section::Info));
    }

    #[test]
    fn snoozing_leaves_done_messages_alone() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        store.mark_done(&id, true, NOV_1).unwrap();
        store.snooze(&id, Some(NOV_1 + DAY)).unwrap();
        let snoozed: u32 = store
            .conn
            .query_row("SELECT count(*) FROM messages WHERE snoozed_until IS NOT NULL", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(snoozed, 0);
    }

    #[test]
    fn changing_a_missing_thread_says_so() {
        let mut store = store_with_work();
        let err = store.mark_done("42", true, NOV_1).unwrap_err();
        assert_eq!(err.to_string(), "no thread with id 42");
        assert!(store.mark_read("x", true).is_err());
        assert!(store.snooze("x", None).is_err());
        assert!(store.set_split("x", SplitId::Other, NOV_1).is_err());
        assert!(store.set_section("x", Section::Info, NOV_1).is_err());
    }

    #[test]
    fn writes_that_fail_are_errors() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        store.delete_thread(&id).unwrap();
        read_only(&store);
        assert!(store.delete_thread(&id).is_err());
        assert!(store.restore_thread(&id).is_err());
        assert!(store.mark_done(&id, true, NOV_1).is_err());
        assert!(store.mark_read(&id, true).is_err());
        assert!(store.snooze(&id, None).is_err());
        assert!(store.set_split(&id, SplitId::Other, NOV_1).is_err());
        assert!(store.set_section(&id, Section::Info, NOV_1).is_err());
    }

    #[test]
    fn a_broken_thread_table_is_an_error() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        break_schema(&store, "ALTER TABLE threads RENAME TO old_threads");
        assert!(matches!(store.mark_done(&id, true, NOV_1), Err(crate::Error::Sqlite(_))));
    }
}
