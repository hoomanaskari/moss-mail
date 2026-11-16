//! Moving mail between split tabs yourself, end to end: a star on the server for Important,
//! both ways, and waiting for an answer.

use super::*;

fn split_of(s: &Setup, subject: &str) -> SplitId {
    s.engine.thread(&s.thread_id(subject)).unwrap().summary.split
}

fn starred(update: &str, on: bool) -> FlagUpdate {
    FlagUpdate { provider_id: update.into(), flags: Flags { flagged: on, ..Default::default() }, labels: vec![] }
}

#[tokio::test(start_paused = true)]
async fn moving_to_important_stars_it_on_the_server_and_moving_out_takes_the_star_away() {
    let s = setup();
    s.inbox(1, "r@x", "Your receipt", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Your receipt");
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Receipts);

    s.engine.move_thread(&id, Some(SplitId::Important)).unwrap();
    let moved = s.engine.thread(&id).unwrap().summary;
    assert_eq!((moved.split, moved.section, moved.moved_by_you), (SplitId::Important, Section::Reply, true));
    assert_eq!(s.engine.split_counts(None).unwrap().important, 1);
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "Flagged(true) INBOX 1:1");

    s.engine.move_thread(&id, Some(SplitId::Other)).unwrap();
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Other);
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "Flagged(false) INBOX 1:1");

    // Back to Moss's sorting: no star to take away, so nothing for the server.
    let before = s.fake.changes().len();
    s.engine.move_thread(&id, None).unwrap();
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Receipts);
    settle().await;
    assert_eq!(s.fake.changes().len(), before);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_star_set_or_taken_away_on_the_phone_moves_it_here() {
    let s = setup();
    s.inbox(1, "r@x", "Your receipt", NOW);
    s.gmail().await;
    s.synced().await;

    s.fake.update_flags(starred("1:1", true));
    s.events.clear();
    s.engine.sync_now(Some("gmail"));
    s.events.wait_for(|e| matches!(e, Event::MailChanged { .. })).await;
    settle().await;
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Important);

    s.fake.update_flags(starred("1:1", false));
    s.events.clear();
    s.engine.sync_now(Some("gmail"));
    s.events.wait_for(|e| matches!(e, Event::MailChanged { .. })).await;
    settle().await;
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Receipts);
}

#[tokio::test(start_paused = true)]
async fn a_move_made_signed_out_is_kept_and_reaches_the_server_later() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    s.engine.shared.forget_provider("gmail");
    let key = s.engine.shared.connection("gmail").unwrap().secret;
    let secret = s.secrets.load(&key).unwrap().unwrap();
    s.secrets.delete(&key).unwrap();
    s.events.clear();

    s.engine.move_thread(&id, Some(SplitId::Important)).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::SignedOut, .. })).await;
    assert!(s.engine.thread(&id).unwrap().summary.moved_by_you, "moved here all the same");
    assert_eq!(s.pending().len(), 1);

    s.secrets.save(&key, &secret).unwrap();
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "Flagged(true) INBOX 1:1");
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn your_email_waits_for_an_answer_and_comes_back_without_one() {
    let s = setup();
    let sent = raw_from("s@x", "Maya Ross <maya.ross@gmail.com>", "daniel@northwind.co", "Deck?", NOW - DAY);
    s.fake.add(message("[Gmail]/Sent Mail", 1, sent, NOW - DAY));
    s.inbox(2, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let found = s.engine.search("Deck", None, 5).unwrap();
    let id = found[0].thread.id.clone();
    assert!(found[0].thread.you_wrote_last);

    s.engine.move_thread(&id, Some(SplitId::Important)).unwrap();
    assert!(s.engine.wait_for_reply(&id, NOW + DAY).unwrap());
    let waiting = s.engine.thread(&id).unwrap().summary;
    assert_eq!((waiting.split, waiting.section), (SplitId::Important, Section::Later));
    assert!(s.subjects().contains(&"Deck?".to_string()));
    // Not yours last: nothing to wait for.
    assert!(!s.engine.wait_for_reply(&s.thread_id("Budget"), NOW + DAY).unwrap());
    assert!(matches!(s.engine.wait_for_reply("99", NOW), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.move_thread("99", None), Err(Error::UnknownThread(_))));
}

#[tokio::test(start_paused = true)]
async fn every_star_in_a_mailbox_goes_in_one_change_and_an_email_only_here_stays_here() {
    let s = setup();
    s.inbox(1, "a@x", "Q4 budget", NOW - DAY);
    s.fake.add(message("INBOX", 2, reply_to(&["a@x"], raw("b@x", "Re: Q4 budget", NOW)), NOW));
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Q4 budget");
    for uid in ["1:1", "1:2"] {
        s.fake.update_flags(starred(uid, true));
    }
    s.events.clear();
    s.engine.sync_now(Some("gmail"));
    s.events.wait_for(|e| matches!(e, Event::MailChanged { .. })).await;
    settle().await;

    s.engine.move_thread(&id, Some(SplitId::Other)).unwrap();
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "Flagged(false) INBOX 1:1,1:2");

    // Your reply, sent from here: no copy on the server yet, so its star stays on this Mac.
    let reply = OutgoingDraft {
        mode: ComposeMode::Reply,
        thread_id: Some(id.clone()),
        from_account_id: "gmail".into(),
        to: vec![Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Re: Q4 budget".into(),
        body: "Approved.".into(),
        body_html: "<p>Approved.</p>".into(),
        signature: vec![],
        from_address: None,
        attachments: vec![],
        remind_if_no_reply: false,
    };
    s.engine.send(reply).await.unwrap();
    let before = s.fake.changes().len();
    s.engine.move_thread(&id, Some(SplitId::Important)).unwrap();
    settle().await;
    assert_eq!(s.fake.changes().len(), before);
    assert_eq!(s.engine.thread(&id).unwrap().summary.split, SplitId::Important);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_split_move_and_its_server_intent_commit_or_roll_back_together() {
    for failure in ["queue_abort", "queue_ignore", "local_update"] {
        let s=setup();
        s.inbox(1,"r@x","Your receipt",NOW);
        s.gmail().await;
        s.synced().await;
        let id=s.thread_id("Your receipt");
        let before=s.engine.thread(&id).unwrap().summary;
        let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        let corrections=||db.query_row("SELECT count(*) FROM ai_decisions WHERE source='you'",[],|r|r.get::<_,i64>(0)).unwrap();
        let before_corrections=corrections();
        let before_changes=s.fake.changes().len();
        db.execute_batch(match failure {
            "queue_abort"=>"CREATE TRIGGER fail_move BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            "queue_ignore"=>"CREATE TRIGGER fail_move BEFORE INSERT ON outbox BEGIN SELECT RAISE(IGNORE); END;",
            _=>"CREATE TRIGGER fail_move BEFORE UPDATE OF moved_to ON threads BEGIN SELECT RAISE(ABORT,'fixture'); END;",
        }).unwrap();
        assert!(s.engine.move_thread(&id,Some(SplitId::Important)).is_err());
        assert_eq!(s.engine.thread(&id).unwrap().summary,before,"{failure}");
        assert_eq!(corrections(),before_corrections,"{failure}: a failed move must not teach a preference");
        assert!(s.pending().is_empty());
        settle().await;
        assert_eq!(s.fake.changes().len(),before_changes);
        db.execute_batch("DROP TRIGGER fail_move").unwrap();
        s.engine.move_thread(&id,Some(SplitId::Important)).unwrap();
        assert_eq!(split_of(&s,"Your receipt"),SplitId::Important);
        settle().await;
        assert_eq!(s.fake.changes()[before_changes..],["Flagged(true) INBOX 1:1"]);
    }
}

#[tokio::test(start_paused = true)]
async fn thread_state_and_server_intent_are_atomic_in_both_failure_directions() {
    for action in ["done","undone","read","delete","restore"] {
        for failure in ["queue_abort","queue_ignore","local"] {
            let s=setup();
            s.inbox(1,"r@x","Your receipt",NOW);
            s.gmail().await;
            s.synced().await;
            let id=s.thread_id("Your receipt");
            if action=="undone" {s.engine.mark_done(&id).unwrap();settle().await;}
            if action=="restore" {s.engine.delete_thread(&id).unwrap();settle().await;}
            let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
            let state=||db.query_row("SELECT done,read,deleted_from FROM messages WHERE thread_id=?1",[&id],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<String>>(2)?))).unwrap();
            let before=state();let changes=s.fake.changes().len();
            db.execute_batch(match failure {
                "queue_abort"=>"CREATE TRIGGER fail_state BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;",
                "queue_ignore"=>"CREATE TRIGGER fail_state BEFORE INSERT ON outbox BEGIN SELECT RAISE(IGNORE); END;",
                _=>"CREATE TRIGGER fail_state BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            }).unwrap();
            let apply=||match action {
                "done"=>s.engine.mark_done(&id),
                "undone"=>s.engine.mark_not_done(&id),
                "read"=>s.engine.mark_read(&id,true),
                "delete"=>s.engine.delete_thread(&id),
                _=>s.engine.restore_thread(&id),
            };
            assert!(apply().is_err(),"{action}: {failure}");
            assert_eq!(state(),before,"{action}: {failure}");
            assert!(s.pending().is_empty(),"{action}: {failure}: no orphaned intent");
            settle().await;
            assert_eq!(s.fake.changes().len(),changes);
            db.execute_batch("DROP TRIGGER fail_state").unwrap();
            apply().unwrap();
            assert_ne!(state(),before,"{action}: successful retry changes local state");
            settle().await;
            assert!(s.pending().is_empty());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn ignored_local_writes_do_not_commit_server_intent_or_preferences() {
    for action in ["done","read","delete","restore","move","move_threads","move_preferences"] {
        let s=setup();
        s.inbox(1,"r@x","Your receipt",NOW);
        s.gmail().await;s.synced().await;
        let id=s.thread_id("Your receipt");
        if action=="restore" {s.engine.delete_thread(&id).unwrap();settle().await;}
        let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        let state=||db.query_row("SELECT done,read,deleted_from,flagged FROM messages WHERE thread_id=?1",[&id],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,bool>(3)?))).unwrap();
        let before=state();let changes=s.fake.changes().len();
        let corrections=db.query_row("SELECT count(*) FROM ai_decisions WHERE source='you'",[],|r|r.get::<_,i64>(0)).unwrap();
        db.execute_batch(match action {
            "move_threads"=>"CREATE TRIGGER ignore_state BEFORE UPDATE ON threads BEGIN SELECT RAISE(IGNORE); END;",
            "move_preferences"=>"CREATE TRIGGER ignore_state BEFORE INSERT ON ai_decisions BEGIN SELECT RAISE(IGNORE); END;",
            _=>"CREATE TRIGGER ignore_state BEFORE UPDATE ON messages BEGIN SELECT RAISE(IGNORE); END;",
        }).unwrap();
        let result=match action {
            "done"=>s.engine.mark_done(&id),
            "read"=>s.engine.mark_read(&id,true),
            "delete"=>s.engine.delete_thread(&id),
            "restore"=>s.engine.restore_thread(&id),
            _=>s.engine.move_thread(&id,Some(SplitId::Important)),
        };
        assert!(result.is_err(),"{action}");
        assert_eq!(state(),before);
        assert!(s.pending().is_empty());
        assert_eq!(db.query_row("SELECT count(*) FROM ai_decisions WHERE source='you'",[],|r|r.get::<_,i64>(0)).unwrap(),corrections);
        settle().await;
        assert_eq!(s.fake.changes().len(),changes);
    }
}

#[tokio::test(start_paused = true)]
async fn an_ignored_preference_reset_cannot_partly_move_a_thread() {
    let s=setup();s.inbox(1,"r@x","Your receipt",NOW);
    s.gmail().await;s.synced().await;
    let id=s.thread_id("Your receipt");
    s.engine.move_thread(&id,Some(SplitId::Important)).unwrap();settle().await;
    let before=s.engine.thread(&id).unwrap().summary;
    let changes=s.fake.changes().len();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER ignore_preference_reset BEFORE DELETE ON ai_decisions WHEN OLD.source='you' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(s.engine.move_thread(&id,None).is_err());
    assert_eq!(s.engine.thread(&id).unwrap().summary,before);
    assert!(s.pending().is_empty());
    settle().await;assert_eq!(s.fake.changes().len(),changes);
    db.execute_batch("DROP TRIGGER ignore_preference_reset").unwrap();
    s.engine.move_thread(&id,None).unwrap();settle().await;
    assert_eq!(split_of(&s,"Your receipt"),SplitId::Receipts);
}

#[tokio::test(start_paused = true)]
async fn restore_preserves_each_deleted_destination_and_ignores_newer_mail() {
    let s=setup();s.inbox(1,"first@x","Topic",NOW);
    for (uid,id) in [(2,"second@x"),(3,"new@x")] {
        s.fake.add(message("INBOX",uid,reply_to(&["first@x"],raw(id,"Re: Topic",NOW+uid as i64)),NOW));
    }
    s.gmail().await;s.synced().await;
    let id=s.thread_id("Topic");
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("UPDATE messages SET done=1,done_at=123,deleted_from=CASE message_id WHEN 'first@x' THEN 'inbox' WHEN 'second@x' THEN 'done' ELSE NULL END;").unwrap();
    s.engine.restore_thread(&id).unwrap();
    let rows=db.prepare("SELECT message_id,done,done_at,deleted_from FROM messages ORDER BY id").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<String>>(3)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(rows,vec![("first@x".into(),false,None,None),("second@x".into(),true,Some(123),None),("new@x".into(),true,Some(123),None)]);
    let changes:Vec<ServerChange>=s.pending().iter().map(|(_,json)|serde_json::from_str(json).unwrap()).collect();
    assert_eq!(changes,vec![ServerChange::Untrash {message_ids:vec!["first@x".into()],to_inbox:true},ServerChange::Untrash {message_ids:vec!["second@x".into()],to_inbox:false}]);
}

#[tokio::test(start_paused = true)]
async fn delete_targets_only_newly_deleted_mail_and_does_not_repeat_intent() {
    let s=setup();s.inbox(1,"first@x","Topic",NOW);
    s.fake.add(message("INBOX",2,reply_to(&["first@x"],raw("second@x","Re: Topic",NOW+1)),NOW));
    s.gmail().await;s.synced().await;
    let id=s.thread_id("Topic");
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE messages SET deleted_from='done' WHERE message_id='first@x'",[]).unwrap();
    s.engine.delete_thread(&id).unwrap();
    s.engine.delete_thread(&id).unwrap();
    let changes:Vec<ServerChange>=s.pending().iter().map(|(_,json)|serde_json::from_str(json).unwrap()).collect();
    assert_eq!(changes,vec![ServerChange::Trash {message_ids:vec!["second@x".into()]}]);
}

#[tokio::test(start_paused = true)]
async fn local_only_delete_and_restore_never_queue_empty_server_changes() {
    let s=setup();
    s.fake.add(message("INBOX",1,b"From: Sender <sender@example.test>\r\nSubject: No ID\r\n\r\nLocal only\r\n".to_vec(),NOW));
    s.gmail().await;s.synced().await;
    let id=s.thread_id("No ID");
    s.engine.delete_thread(&id).unwrap();assert!(s.pending().is_empty());
    s.engine.restore_thread(&id).unwrap();assert!(s.pending().is_empty());
    assert!(s.subjects().contains(&"No ID".to_string()));
}

#[test]
fn missing_thread_during_a_transaction_retains_the_thread_error_kind() {
    assert!(matches!(Error::from(moss_store::Error::NotFound {kind:"thread",id:"42".into()}),Error::UnknownThread(id) if id=="42"));
    assert!(matches!(Error::from(moss_store::Error::NotFound {kind:"message",id:"42".into()}),Error::Store(_)));
}

#[tokio::test(start_paused = true)]
async fn unencodable_thread_intent_rolls_back_the_local_action() {
    let s = setup();
    s.inbox(1, "valid@x", "Encoding", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Encoding");
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    let before = s.fake.changes();
    for target in ["cannot search this".to_string(), "x".repeat(1_048_576)] {
        db.execute("UPDATE messages SET message_id=?1", [&target]).unwrap();
        assert!(s.engine.delete_thread(&id).is_err());
        assert!(s.pending().is_empty());
        let deleted: Option<String> = db.query_row("SELECT deleted_from FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(deleted, None);
        settle().await;
        assert_eq!(s.fake.changes(), before);
    }
    db.execute("UPDATE messages SET message_id='valid@x'", []).unwrap();
    s.engine.delete_thread(&id).unwrap();
    assert_eq!(s.pending().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn resend_thread_actions_are_local_even_with_opaque_ids() {
    let s = setup();
    s.fake.local_only();
    s.inbox(1, "valid@x", "API mail", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("API mail");
    let mut connection = s.engine.shared.connection("gmail").unwrap();
    connection.service = MailService::Resend;
    s.engine.shared.store().set_setting(&connection_key("gmail"), Some(&serde_json::to_string(&connection).unwrap())).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE message_locations SET provider_id='opaque-api-id'", []).unwrap();
    db.execute("UPDATE messages SET message_id='非ASCII@example.test'", []).unwrap();
    let before = s.fake.changes();
    s.engine.mark_read(&id, true).unwrap();
    assert!(!s.engine.thread(&id).unwrap().summary.unread);
    s.engine.mark_done(&id).unwrap();
    assert!(s.engine.thread(&id).unwrap().summary.done);
    s.engine.mark_not_done(&id).unwrap();
    assert!(!s.engine.thread(&id).unwrap().summary.done);
    s.engine.delete_thread(&id).unwrap();
    assert_eq!(db.query_row("SELECT deleted_from FROM messages", [], |r| r.get::<_,String>(0)).unwrap(), "inbox");
    s.engine.restore_thread(&id).unwrap();
    assert_eq!(db.query_row("SELECT deleted_from FROM messages", [], |r| r.get::<_,Option<String>>(0)).unwrap(), None);
    assert!(s.pending().is_empty());
    settle().await;
    assert_eq!(s.fake.changes(), before);
}

#[tokio::test(start_paused = true)]
async fn missing_connection_preserves_local_actions_and_waiting_intent() {
    let s = setup();
    s.inbox(1, "valid@x", "Disconnected", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Disconnected");
    s.engine.shared.store().set_setting(&connection_key("gmail"), None).unwrap();
    s.engine.mark_read(&id, true).unwrap();
    s.engine.mark_done(&id).unwrap();
    assert!(s.engine.thread(&id).unwrap().summary.done);
    s.engine.mark_not_done(&id).unwrap();
    s.engine.delete_thread(&id).unwrap();
    s.engine.restore_thread(&id).unwrap();
    assert!(!s.engine.thread(&id).unwrap().summary.done);
    let changes: Vec<ServerChange> = s.pending().iter().map(|(_,json)| serde_json::from_str(json).unwrap()).collect();
    assert_eq!(changes, vec![
        ServerChange::Seen { mailbox: "INBOX".into(), ids: vec!["1:1".into()], seen: true },
        ServerChange::Archive { mailbox: "INBOX".into(), ids: vec!["1:1".into()] },
        ServerChange::Unarchive { message_ids: vec!["valid@x".into()] },
        ServerChange::Trash { message_ids: vec!["valid@x".into()] },
        ServerChange::Untrash { message_ids: vec!["valid@x".into()], to_inbox: true },
    ]);
}

#[tokio::test(start_paused = true)]
async fn corrupt_connection_fails_thread_actions_without_exposing_its_contents() {
    let s = setup();
    s.inbox(1, "valid@x", "Corrupt config", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Corrupt config");
    s.engine.shared.store().set_setting(&connection_key("gmail"), Some("private malformed settings")).unwrap();
    let error = s.engine.mark_done(&id).unwrap_err();
    assert!(matches!(error, Error::Invalid(ref text) if text == "This account's connection settings could not be read. No local changes were saved."));
    assert!(!s.engine.thread(&id).unwrap().summary.done);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn grouped_sender_records_provider_success_separately_from_refused_or_unreadable_work() {
    use moss_store::{BatchOutcome, ThreadChange};
    for (first, refused, expected) in [
        (serde_json::to_string(&ServerChange::Seen {mailbox:"INBOX".into(),ids:vec!["1:1".into()],seen:true}).unwrap(),false,BatchOutcome::Completed),
        (serde_json::to_string(&ServerChange::Seen {mailbox:"INBOX".into(),ids:vec!["1:1".into()],seen:true}).unwrap(),true,BatchOutcome::Pending),
        (serde_json::to_string(&ServerChange::Forwarded {mailbox:"INBOX".into(),ids:vec!["1:1".into()]}).unwrap(),true,BatchOutcome::Pending),
        ("{\"kind\":\"future\"}".into(),false,BatchOutcome::Pending),
    ] {
        let s=setup();s.fake.with_drafts();s.inbox(1,"valid@x","Grouped sender",NOW);s.gmail().await;s.synced().await;
        let thread=s.thread_id("Grouped sender");
        let last=serde_json::to_string(&ServerChange::DeleteDrafts {message_ids:vec!["old-copy@x".into()]}).unwrap();
        s.engine.shared.store().change_thread_with_changes(&thread,"gmail",ThreadChange::Read(true),NOW,|_,_|Ok(vec![first,last])).unwrap();
        let ids=s.pending();
        if refused { s.fake.fail_next(moss_providers::Error::Protocol("refused".into())); }
        s.fake.fail_next_draft_delete(moss_providers::Error::Network("offline".into()));
        assert!(s.engine.shared.send_all("gmail").await.is_err());
        let batch=s.engine.shared.store().batch_for_change("gmail",ids[1].0).unwrap().unwrap();
        assert_eq!(batch.parts[0].outcome,expected);
        assert_eq!(batch.parts[1].outcome,BatchOutcome::Pending);
    }
}

#[tokio::test(start_paused = true)]
async fn split_move_validates_targets_and_connection_before_committing() {
    for scenario in ["resend", "missing", "corrupt", "invalid", "oversized"] {
        let s = setup();
        s.fake.local_only();
        s.inbox(1, "r@x", "Your receipt", NOW);
        s.gmail().await;
        s.synced().await;
        let id = s.thread_id("Your receipt");
        let before = s.engine.thread(&id).unwrap().summary;
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        match scenario {
            "resend" => {
                let mut connection = s.engine.shared.connection("gmail").unwrap();
                connection.service = MailService::Resend;
                s.engine.shared.store().set_setting(&connection_key("gmail"), Some(&serde_json::to_string(&connection).unwrap())).unwrap();
                db.execute("UPDATE message_locations SET provider_id='opaque-api-id'", []).unwrap();
            }
            "missing" => s.engine.shared.store().set_setting(&connection_key("gmail"), None).unwrap(),
            "corrupt" => s.engine.shared.store().set_setting(&connection_key("gmail"), Some("private malformed settings")).unwrap(),
            "invalid" => { db.execute("UPDATE message_locations SET provider_id='invalid'", []).unwrap(); }
            _ => { db.execute("UPDATE message_locations SET provider_id=?1", ["1".repeat(1_048_577)]).unwrap(); }
        }
        let result = s.engine.move_thread(&id, Some(SplitId::Important));
        if matches!(scenario, "resend" | "missing") {
            result.unwrap();
            assert_eq!(split_of(&s, "Your receipt"), SplitId::Important);
            let pending = s.pending();
            assert_eq!(pending.len(), usize::from(scenario == "missing"), "{scenario}");
            if let Some((token, _)) = pending.first() {
                assert_eq!(s.engine.shared.store().batch_for_change("gmail", *token).unwrap().unwrap().kind, moss_store::BatchKind::Move);
            }
            if scenario == "resend" {
                s.engine.move_thread(&id, None).unwrap();
                assert!(!s.engine.thread(&id).unwrap().summary.moved_by_you);
                assert_eq!(db.query_row("SELECT count(*) FROM messages WHERE flagged=1", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
                assert!(s.pending().is_empty());
            }
        } else {
            assert!(result.is_err(), "{scenario}");
            assert_eq!(s.engine.thread(&id).unwrap().summary, before);
            assert!(s.pending().is_empty());
            assert_eq!(db.query_row("SELECT count(*) FROM ai_decisions WHERE source='you'", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn local_moves_do_not_require_readable_connection_settings() {
    let s = setup();
    s.inbox(1, "r@x", "Your receipt", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Your receipt");
    s.engine.shared.store().set_setting(&connection_key("gmail"), Some("private malformed settings")).unwrap();
    s.engine.move_thread(&id, Some(SplitId::Calendar)).unwrap();
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Calendar);
    s.engine.move_thread(&id, None).unwrap();
    assert!(!s.engine.thread(&id).unwrap().summary.moved_by_you);
    s.engine.shared.store().set_flagged("gmail", "INBOX", "1:1", true).unwrap();
    s.engine.move_thread(&id, Some(SplitId::Important)).unwrap();
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Important);
    assert!(s.engine.move_thread(&id, None).is_err());
    assert_eq!(split_of(&s, "Your receipt"), SplitId::Important);
    assert!(s.pending().is_empty());
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE message_locations SET provider_id='local:fixture'", []).unwrap();
    s.engine.move_thread(&id, None).unwrap();
    s.engine.mark_read(&id, true).unwrap();
    s.engine.mark_done(&id).unwrap();
    assert!(s.engine.thread(&id).unwrap().summary.done);
    assert!(!s.engine.thread(&id).unwrap().summary.unread);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn unstar_moves_group_all_mailboxes_and_reset_preferences_atomically() {
    for to in [None, Some(SplitId::Calendar)] {
        let s = setup();
        s.inbox(1, "r@x", "Your receipt", NOW);
        s.gmail().await;
        s.synced().await;
        let id = s.thread_id("Your receipt");
        s.engine.move_thread(&id, Some(SplitId::Other)).unwrap();
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute("INSERT INTO mailboxes(account_id,name) VALUES('gmail','Other folder')", []).unwrap();
        db.execute("INSERT INTO message_locations(message_id,mailbox_id,provider_id) SELECT message_id,(SELECT id FROM mailboxes WHERE name='Other folder'),'2:7' FROM message_locations LIMIT 1", []).unwrap();
        s.engine.shared.store().set_flagged("gmail", "INBOX", "1:1", true).unwrap();
        s.engine.shared.store().set_setting(&connection_key("gmail"), None).unwrap();
        s.engine.move_thread(&id, to).unwrap();
        let pending = s.pending();
        assert_eq!(pending.len(), 2);
        let batch = s.engine.shared.store().batch_for_change("gmail", pending[0].0).unwrap().unwrap();
        assert_eq!(batch.kind, moss_store::BatchKind::Move);
        assert_eq!(batch.parts.iter().map(|part|part.id).collect::<Vec<_>>(), pending.iter().map(|(id,_)|*id).collect::<Vec<_>>());
        let changes: Vec<ServerChange> = pending.iter().map(|(_,json)|serde_json::from_str(json).unwrap()).collect();
        assert_eq!(changes, vec![
            ServerChange::Flagged { mailbox:"INBOX".into(), ids:vec!["1:1".into()], flagged:false },
            ServerChange::Flagged { mailbox:"Other folder".into(), ids:vec!["2:7".into()], flagged:false },
        ]);
        assert_eq!(db.query_row("SELECT count(*) FROM messages WHERE flagged=1", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(s.engine.thread(&id).unwrap().summary.moved_by_you, to.is_some());
        assert_eq!(db.query_row("SELECT count(*) FROM ai_decisions WHERE source='you'", [], |r|r.get::<_,i64>(0)).unwrap(), i64::from(to.is_some()));
    }
}
