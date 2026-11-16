//! Moving a thread to another split tab yourself, for good: Moss's sorting never moves it
//! back, and new emails in it land there too.
//!
//! Moved to Important, its latest email is starred (`flagged`, IMAP `\Flagged`, a star in
//! Gmail), which is how the server and your phone know. A star from another app moves a thread
//! to Important the same way. Anywhere else is `threads.moved_to`, kept only on this device,
//! and moving there takes every star away. A thread in Important by a star needs you: it
//! counts toward what's waiting and sits at the top. See `query.rs`.

use moss_core::SplitId;
use rusqlite::{params, OptionalExtension};

use crate::codec::name;
use crate::{Error, Result, Store, SPLIT};

impl Store {
    /// Moves a thread to `to`; `None` gives it back to Moss's sorting. Returns the emails
    /// whose star changed (starred when `to` is Important, unstarred otherwise), to change
    /// on the server too.
    ///
    /// The move is also your answer to "which tab?" for each email from someone else
    /// (`decisions.rs`), so the decision model never asks about them again or moves them back.
    /// What Moss had decided stays in `messages.split`, so `None` puts that back, and the
    /// model may decide again.
    pub fn move_thread(&mut self, thread_id: &str, to: Option<SplitId>, now: i64) -> Result<Vec<String>> {
        self.delivery_transaction(|store| store.move_thread_in_transaction(thread_id, to, now))
    }

    /// Save the preference, local stars and all corresponding server changes in one commit.
    /// The builder reads locations in that transaction; the account comes from the same row.
    pub fn move_thread_with_changes(&mut self, thread_id: &str, expected_account: &str, to: Option<SplitId>, now: i64,
        build: impl FnOnce(&Self, &[String]) -> Result<Vec<String>>) -> Result<String> {
        self.delivery_transaction(|store| {
            let account: String = store.conn.query_row("SELECT account_id FROM threads WHERE id=?1",
                [store.thread_pk(thread_id)?], |row| row.get(0))?;
            if account != expected_account { return Err(Error::Invalid("This conversation belongs to a different account. Refresh it before changing it.".into())); }
            let changed = store.move_thread_in_transaction(thread_id, to, now)?;
            store.queue_batch_in_transaction(&account, crate::BatchKind::Move, &build(store, &changed)?, now)?;
            Ok(account)
        })
    }

    fn move_thread_in_transaction(&self, thread_id: &str, to: Option<SplitId>, now: i64) -> Result<Vec<String>> {
        let id = self.thread_pk(thread_id)?;
        match to {
            Some(split) => self.correct(thread_id, "from_me = 0", SPLIT, Some(&name(&split)), false, now)?,
            None => {
                self.conn.execute(
                    "DELETE FROM ai_decisions WHERE question = ?2 AND source = 'you'
                         AND message_id IN (SELECT id FROM messages WHERE thread_id = ?1)",
                    params![id, SPLIT],
                )?;
                let remains: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM ai_decisions WHERE question=?2 AND source='you'
                     AND message_id IN (SELECT id FROM messages WHERE thread_id=?1))",
                    params![id,SPLIT], |row| row.get(0))?;
                if remains { return Err(Error::Invalid("The conversation preference could not be reset.".into())); }
            }
        }
        let important = to == Some(SplitId::Important);
        let changed: Vec<i64> = {
            let sql = if important {
                // Already starred anywhere in it: it is in Important already.
                "SELECT id FROM messages WHERE thread_id = ?1
                     AND NOT EXISTS (SELECT 1 FROM messages WHERE thread_id = ?1 AND flagged = 1)
                 ORDER BY sent_at DESC, id DESC LIMIT 1"
            } else {
                "SELECT id FROM messages WHERE thread_id = ?1 AND flagged = 1 ORDER BY id"
            };
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map([id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for message in &changed {
            if self.conn.execute("UPDATE messages SET flagged = ?2 WHERE id = ?1", params![message, important])? != 1 { return Err(Error::Invalid("The conversation star was not saved.".into())); }
        }
        let moved_to = to.filter(|_| !important).map(|s| name(&s));
        if self.conn.execute("UPDATE threads SET moved_to = ?2 WHERE id = ?1", params![id, moved_to])? != 1 { return Err(Error::Invalid("The conversation move was not saved.".into())); }
        Ok(changed.into_iter().map(|m| m.to_string()).collect())
    }

    /// A star set or taken away in another mail app. Returns whether the message is stored.
    pub fn set_flagged(
        &mut self,
        account_id: &str,
        mailbox: &str,
        provider_id: &str,
        flagged: bool,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE messages SET flagged = ?4 WHERE id IN (SELECT l.message_id FROM message_locations l
             JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE b.account_id = ?1 AND b.name = ?2 AND l.provider_id = ?3)",
            params![account_id, mailbox, provider_id, flagged],
        )? > 0)
    }

    /// Waiting for an answer to your latest email in a thread: out of sight until `until`
    /// (ms), like Remind me, and back early when someone replies. It's your "Remind me if no
    /// reply" (`decisions.rs`), at the time you picked rather than the usual wait. Returns
    /// false, changing nothing, when the latest email isn't yours.
    pub fn wait_for_reply(&mut self, thread_id: &str, until: i64, now: i64) -> Result<bool> {
        let id = self.thread_pk(thread_id)?;
        let sent_at: Option<i64> = self
            .conn
            .query_row(
                "SELECT sent_at FROM messages WHERE thread_id = ?1 AND from_me = 1
                   AND id = (SELECT id FROM messages WHERE thread_id = ?1 ORDER BY sent_at DESC, id DESC LIMIT 1)",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        match sent_at {
            Some(sent_at) => self.remind_if_no_reply(thread_id, true, until - sent_at, now),
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use moss_core::{Section, SplitId, ThreadSummary};

    use crate::test_support::*;
    use crate::Store;

    #[test]
    fn move_parts_share_one_bounded_atomic_batch() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &receipt().build()).thread_id;
        let before = summary(&store, &id, NOV_1);
        for changes in [vec!["x".repeat(crate::MAX_CHANGE_PAYLOAD_BYTES + 1)], vec!["x".into(); 65]] {
            assert!(store.move_thread_with_changes(&id, "work", Some(SplitId::Important), NOV_1, |_, _| Ok(changes)).is_err());
            assert_eq!(summary(&store, &id, NOV_1), before);
            assert!(your_answers(&store, &id).is_empty());
            assert!(store.pending_changes("work").unwrap().is_empty());
        }
        store.conn.execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON outbox WHEN NEW.change='second' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(store.move_thread_with_changes(&id, "work", Some(SplitId::Important), NOV_1, |_, _| Ok(vec!["first".into(), "second".into()])).is_err());
        assert_eq!(summary(&store, &id, NOV_1), before);
        assert!(your_answers(&store, &id).is_empty());
        assert!(store.pending_changes("work").unwrap().is_empty());
        store.conn.execute_batch("DROP TRIGGER reject_second").unwrap();
        store.move_thread_with_changes(&id, "work", Some(SplitId::Important), NOV_1, |_, _| Ok(vec!["first".into(), "second".into()])).unwrap();
        let pending = store.pending_changes("work").unwrap();
        let batch = store.batch_for_change("work", pending[0].0).unwrap().unwrap();
        assert_eq!(batch.kind, crate::BatchKind::Move);
        assert_eq!(batch.parts.len(), 2);
        assert_eq!(store.batch_for_change("work", pending[1].0).unwrap().unwrap(), batch);
    }

    #[test]
    fn move_intent_checks_the_observed_account_and_rolls_back_builder_failure() {
        let mut store=store_with_work();
        let id=ingest(&mut store,"1",&receipt().build()).thread_id;
        let before=summary(&store,&id,NOV_1);
        let build=|_: &Store, _: &[String]|Ok(vec!["flagged".into()]);
        assert!(store.move_thread_with_changes(&id,"other",Some(SplitId::Important),NOV_1,build).is_err());
        assert_eq!(summary(&store,&id,NOV_1),before);
        assert!(store.move_thread_with_changes(&id,"work",Some(SplitId::Important),NOV_1,|_,_|Err(crate::Error::Invalid("fixture".into()))).is_err());
        assert_eq!(summary(&store,&id,NOV_1),before);
        assert!(your_answers(&store,&id).is_empty());
        assert!(store.pending_changes("work").unwrap().is_empty());
        assert_eq!(store.move_thread_with_changes(&id,"work",Some(SplitId::Important),NOV_1,build).unwrap(),"work");
        assert_eq!(store.pending_changes("work").unwrap()[0].1,"flagged");
    }

    fn summary(store: &Store, id: &str, now: i64) -> ThreadSummary {
        store.thread(id, now).unwrap().unwrap().summary
    }

    fn flagged(store: &Store, message_id: &str) -> bool {
        store
            .conn
            .query_row("SELECT flagged FROM messages WHERE id = ?1", [message_id], |r| r.get(0))
            .unwrap()
    }

    fn listed(store: &Store, split: SplitId, now: i64) -> Vec<String> {
        store.list_threads(None, split, now).unwrap().into_iter().map(|t| t.id).collect()
    }

    fn receipt() -> Mail {
        mail().id("r1@shop.co").from("Shop <orders@shop.co>").subject("Your receipt")
    }

    #[test]
    fn moved_to_important_it_stars_the_latest_email_and_needs_you() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1", &receipt().build());
        let id = first.thread_id.clone();
        let later = ingest(&mut store, "2", &receipt().id("r2@shop.co").reply_to(&["r1@shop.co"]).at(NOV_1 + 60_000).build());
        assert_eq!(summary(&store, &id, NOV_1).split, SplitId::Receipts);

        assert_eq!(store.move_thread(&id, Some(SplitId::Important), NOV_1).unwrap(), [later.message_id.clone()]);
        assert!(flagged(&store, &later.message_id) && !flagged(&store, &first.message_id));
        let moved = summary(&store, &id, NOV_1);
        assert_eq!((moved.split, moved.section, moved.moved_by_you), (SplitId::Important, Section::Reply, true));
        assert_eq!(listed(&store, SplitId::Important, NOV_1), [id.clone()]);
        assert!(listed(&store, SplitId::Receipts, NOV_1).is_empty());
        assert_eq!(store.split_counts(None, NOV_1).unwrap().important, 1, "it counts toward what's waiting");

        // Moving it there again stars nothing more.
        assert!(store.move_thread(&id, Some(SplitId::Important), NOV_1).unwrap().is_empty());
    }

    /// Your answer to "which tab?" for each email from someone else: the decision model never
    /// asks about or overwrites those (see `decisions.rs`).
    fn your_answers(store: &Store, thread: &str) -> Vec<String> {
        let mut stmt = store
            .conn
            .prepare(
                "SELECT d.value FROM ai_decisions d JOIN messages m ON m.id = d.message_id
                 WHERE m.thread_id = ?1 AND d.question = 'split' AND d.source = 'you' ORDER BY m.id",
            )
            .unwrap();
        stmt.query_map([thread], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn your_move_is_your_answer_so_the_decision_model_never_moves_it_back() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &receipt().build()).thread_id;
        store.move_thread(&id, Some(SplitId::Important), NOV_1).unwrap();
        assert_eq!(your_answers(&store, &id), ["important"]);
        let first = store.thread(&id, NOV_1).unwrap().unwrap().messages[0].id.parse::<i64>().unwrap();
        let model = crate::Decision {
            source: crate::Source::Model,
            yes: Some(0.99),
            acted: true,
            value: Some("receipts".into()),
            evidence: None,
        };
        store.decide(first, crate::SPLIT, &model, NOV_1).unwrap();
        assert_eq!(summary(&store, &id, NOV_1).split, SplitId::Important);
        store.move_thread(&id, Some(SplitId::Calendar), NOV_1).unwrap();
        assert_eq!(your_answers(&store, &id), ["calendar"]);

        // Let Moss sort it: your answer goes, so the model may decide again.
        store.move_thread(&id, None, NOV_1).unwrap();
        assert!(your_answers(&store, &id).is_empty());
    }

    #[test]
    fn a_new_email_in_a_moved_thread_stays_where_you_put_it() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &receipt().build()).thread_id;
        store.move_thread(&id, Some(SplitId::Important), NOV_1).unwrap();
        ingest(&mut store, "2", &receipt().id("r2@shop.co").reply_to(&["r1@shop.co"]).at(NOV_1 + 60_000).build());
        assert_eq!(summary(&store, &id, NOV_1).split, SplitId::Important);

        let other = ingest(&mut store, "3", &mail().id("o1@x.co").subject("Lunch").build()).thread_id;
        store.move_thread(&other, Some(SplitId::Newsletters), NOV_1).unwrap();
        ingest(&mut store, "4", &mail().id("o2@x.co").subject("Re: Lunch").reply_to(&["o1@x.co"]).at(NOV_1 + 60_000).build());
        let moved = summary(&store, &other, NOV_1);
        assert_eq!((moved.split, moved.moved_by_you), (SplitId::Newsletters, true));
    }

    #[test]
    fn moved_out_of_important_every_star_goes_and_it_stays_out() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1", &mail().build());
        let id = first.thread_id.clone();
        let reply = ingest(&mut store, "2", &mail().id("r@x").reply_to(&["q4@northwind.co"]).at(NOV_1 + 60_000).build());
        // Starred on the phone, both of them.
        store.set_flagged("work", "INBOX", "1", true).unwrap();
        store.set_flagged("work", "INBOX", "2", true).unwrap();

        let unstarred = store.move_thread(&id, Some(SplitId::Other), NOV_1).unwrap();
        assert_eq!(unstarred, [first.message_id.clone(), reply.message_id.clone()]);
        assert!(!flagged(&store, &first.message_id) && !flagged(&store, &reply.message_id));
        let moved = summary(&store, &id, NOV_1);
        assert_eq!((moved.split, moved.section, moved.moved_by_you), (SplitId::Other, Section::Info, true));
        assert_eq!(listed(&store, SplitId::Other, NOV_1), [id.clone()]);

        // Given back to Moss, it goes where the rules put it.
        assert!(store.move_thread(&id, None, NOV_1).unwrap().is_empty());
        let back = summary(&store, &id, NOV_1);
        assert_eq!((back.split, back.section, back.moved_by_you), (SplitId::Important, Section::Reply, false));
    }

    #[test]
    fn a_star_from_another_app_moves_it_to_important_and_taking_it_away_moves_it_back() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &receipt().build()).thread_id;
        store.move_thread(&id, Some(SplitId::Calendar), NOV_1).unwrap();
        assert!(store.set_flagged("work", "INBOX", "1", true).unwrap());
        assert_eq!(summary(&store, &id, NOV_1).split, SplitId::Important, "the star wins");
        store.set_flagged("work", "INBOX", "1", false).unwrap();
        assert_eq!(summary(&store, &id, NOV_1).split, SplitId::Calendar);
        assert!(!store.set_flagged("work", "INBOX", "nope", true).unwrap());
    }

    #[test]
    fn moved_to_important_sits_on_top_of_what_needs_you_and_the_ai_says_nothing() {
        let mut store = store_with_work();
        let older = ingest(&mut store, "1", &receipt().build()).thread_id;
        let newer = ingest(&mut store, "2", &mail().at(NOV_1 + 60_000).build()).thread_id;
        store
            .conn
            .execute("INSERT INTO ai_decisions (message_id, question, yes, source, acted, decided_at) SELECT id, 'needsReply', 0.1, 'model', 1, 0 FROM messages", [])
            .unwrap();
        store.conn.execute("UPDATE messages SET section = 'info'", []).unwrap();
        store.move_thread(&older, Some(SplitId::Important), NOV_1).unwrap();
        let list = store.list_threads(None, SplitId::Important, NOV_1).unwrap();
        assert_eq!(list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [older.as_str(), newer.as_str()]);
        assert_eq!((list[0].section, list[0].ai_sure), (Section::Reply, None), "you said it needs you");
        assert_eq!((list[1].section, list[1].ai_sure), (Section::Info, Some(90)));
    }

    #[test]
    fn snoozed_or_done_it_is_out_of_the_way_but_keeps_its_star() {
        let mut store = store_with_work();
        let first = ingest(&mut store, "1", &mail().build());
        let id = first.thread_id.clone();
        store.move_thread(&id, Some(SplitId::Important), NOV_1).unwrap();
        store.snooze(&id, Some(NOV_1 + DAY)).unwrap();
        assert_eq!(summary(&store, &id, NOV_1).section, Section::Later);
        assert_eq!(store.split_counts(None, NOV_1).unwrap().important, 0);
        store.mark_done(&id, true, NOV_1).unwrap();
        assert!(listed(&store, SplitId::Important, NOV_1).is_empty());
        assert!(flagged(&store, &first.message_id));
        assert!(store.list_done(None, 10, NOV_1).unwrap()[0].moved_by_you);
    }

    #[test]
    fn waiting_for_a_reply_brings_your_email_back_unless_someone_answers() {
        let mut store = store_with_work();
        let sent = mail().from("Maya Ross <maya@northwind.co>").to("Daniel <daniel@northwind.co>").subject("Deck?");
        store.upsert_mailbox(&moss_core::Mailbox { account_id: "work".into(), name: "Sent".into(), role: Some(moss_core::MailboxRole::Sent) }).unwrap();
        let id = store.ingest(&sent.build(), &moss_core::MessageSource { mailbox: "Sent".into(), ..source("9") }).unwrap().thread_id;
        assert!(listed(&store, SplitId::Important, NOV_1).is_empty(), "only you wrote: done");
        assert!(summary(&store, &id, NOV_1).you_wrote_last);

        store.move_thread(&id, Some(SplitId::Important), NOV_1).unwrap();
        assert!(store.wait_for_reply(&id, NOV_1 + DAY, NOV_1).unwrap());
        let waiting = summary(&store, &id, NOV_1);
        assert_eq!((waiting.section, waiting.snoozed_until), (Section::Later, Some(NOV_1 + DAY)));
        // Your "Remind me if no reply", at the time you picked.
        assert_eq!(waiting.waiting.map(|w| (w.until, w.why.by)), Some((NOV_1 + DAY, moss_core::DecidedBy::You)));
        let back = summary(&store, &id, NOV_1 + DAY + 1);
        assert_eq!((back.split, back.section), (SplitId::Important, Section::Reply), "no answer: it needs you");
        assert_eq!(store.split_counts(None, NOV_1 + DAY + 1).unwrap().important, 1);

        // An answer brings it back at once.
        ingest(&mut store, "10", &mail().id("ans@x").reply_to(&["q4@northwind.co"]).at(NOV_1 + 60_000).build());
        let answered = summary(&store, &id, NOV_1);
        assert_eq!((answered.section, answered.you_wrote_last), (Section::Reply, false));
    }

    #[test]
    fn waiting_needs_your_email_to_be_the_latest() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        assert!(!store.wait_for_reply(&id, NOV_1 + DAY, NOV_1).unwrap());
        assert_eq!(summary(&store, &id, NOV_1).snoozed_until, None);
    }

    #[test]
    fn a_missing_thread_or_a_failed_write_is_an_error() {
        let mut store = store_with_work();
        assert!(store.move_thread("x", Some(SplitId::Other), NOV_1).is_err());
        assert!(store.wait_for_reply("x", NOV_1, NOV_1).is_err());
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        let sent = mail().id("mine@x").from("Maya Ross <maya@northwind.co>").subject("Deck?");
        let mine = store.ingest(&sent.build(), &moss_core::MessageSource { mailbox: "Sent".into(), ..source("9") }).unwrap().thread_id;
        read_only(&store);
        assert!(store.move_thread(&id, Some(SplitId::Important), NOV_1).is_err());
        assert!(store.wait_for_reply(&mine, NOV_1 + DAY, NOV_1).is_err());
        assert!(store.set_flagged("work", "INBOX", "1", true).is_err());
    }

    #[test]
    fn a_broken_messages_table_is_an_error() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN flagged TO starred");
        assert!(store.move_thread(&id, Some(SplitId::Important), NOV_1).is_err());
        assert!(store.move_thread(&id, None, NOV_1).is_err());
    }

    #[test]
    fn giving_it_back_to_moss_fails_if_your_answer_cant_be_forgotten() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1", &mail().build()).thread_id;
        break_schema(&store, "ALTER TABLE ai_decisions RENAME TO old_decisions");
        assert!(store.move_thread(&id, None, NOV_1).is_err());
    }
}
