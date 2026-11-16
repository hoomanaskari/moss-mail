//! Drafts in the account's Drafts folder end to end: up as you type, gone once sent or
//! discarded, in from your phone, and the newest wins when both changed.

use std::time::Duration;

use moss_core::Draft;
use moss_store::ServerCopy;

use super::*;
use crate::drafts::DRAFT_PUSH_DELAY;

fn draft(id: &str, body: &str, at: i64) -> Draft {
    Draft {
        revision: None,
        id: id.into(),
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: "gmail".into(),
        to: vec![Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Lunch?".into(),
        body: body.into(),
        body_html: format!("<p>{body}</p>"),
        remind_if_no_reply: false,
        updated_at: at,
        signature_id: Some("s1".into()),
        signature_chosen: true,
        from_address: None,
        attachments: vec![],
    }
}

#[tokio::test(start_paused = true)]
async fn retiring_an_account_after_a_terminal_send_keeps_replay_from_sending_again() {
    for sent in [true, false] {
        let s = setup(); s.gmail().await; s.synced().await;
        let connection = s.engine.shared.connection("gmail").unwrap();
        let draft = draft("retire-draft", "Retained operation", NOW);
        let email = OutgoingDraft::from(&draft);
        assert!(s.engine.prepare_send("retire-op", &draft, &email).await.unwrap());
        if sent { s.engine.submit_send("retire-op").await.unwrap(); }
        else { assert!(s.engine.cancel_send("retire-op").unwrap()); }
        let state = s.engine.delivery_state("retire-op").unwrap();
        assert!(!s.engine.remove_account("gmail").unwrap().pending);
        assert!(s.secrets.load(&connection.secret).unwrap().is_none());
        assert!(!s.engine.shared.workers.lock().unwrap().contains_key("gmail"));
        assert!(s.engine.accounts().unwrap().is_empty());
        assert_eq!(s.engine.delivery_state("retire-op").unwrap(), state);
        assert!(!s.engine.prepare_send("retire-op", &draft, &email).await.unwrap());
        let outcome = s.engine.submit_send("retire-op").await.unwrap_err();
        if sent { assert!(matches!(outcome, Error::DeliveryAccepted(_)), "{outcome:?}"); }
        else { assert!(matches!(outcome, Error::Invalid(_)), "{outcome:?}"); }
        assert_eq!(s.fake.sent().len(), usize::from(sent));
    }
}

/// These provider tests model a fresh local edit of the currently displayed row.
fn current_edit(engine: &Engine, mut edited: Draft) -> Draft {
    edited.revision = engine.drafts().unwrap().into_iter().find(|d| d.id == edited.id).unwrap().revision;
    edited
}

fn account(id: &str, address: &str) -> moss_core::Account {
    moss_core::Account {
        id: id.into(),
        name: id.into(),
        letter: "W".into(),
        address: address.into(),
        sender_name: "Maya Ross".into(),
        color: moss_core::AccountColor::Work,
        provider: moss_core::ProviderKind::Imap,
        org: None,
        org_domain: None,
    }
}

/// Signed in to a server with a Drafts folder, first sync done.
async fn with_drafts() -> Setup {
    let s = setup();
    s.fake.with_drafts();
    s.gmail().await;
    s.synced().await;
    s
}

async fn after_typing() {
    tokio::time::sleep(DRAFT_PUSH_DELAY + Duration::from_millis(50)).await;
}

/// Every save, replace and delete that reached the server's Drafts folder.
fn draft_changes(s: &Setup) -> Vec<String> {
    s.fake.changes().into_iter().filter(|c| c.contains(DRAFTS)).collect()
}

fn copy_of(s: &Setup, id: &str) -> Option<ServerCopy> {
    s.engine.shared.store().synced_draft(id).unwrap().and_then(|d| d.copy)
}

fn pushed(s: &Setup, id: &str) -> bool {
    s.engine.shared.store().synced_draft(id).unwrap().is_some_and(|d| d.pushed)
}

/// A message in the server's Drafts folder, as a phone or webmail leaves it.
fn on_server(s: &Setup, uid: u32, raw: Vec<u8>) {
    s.fake.add(message(DRAFTS, uid, raw, NOW));
}

fn phone_draft(id: &str, subject: &str, body: &str, at: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(at).unwrap().to_rfc2822();
    format!(
        "From: Maya Ross <maya.ross@gmail.com>\r\nTo: Daniel Okafor <daniel@northwind.co>\r\n\
         Subject: {subject}\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

async fn sync(s: &Setup) {
    s.engine.sync_now(Some("gmail"));
    settle().await;
}

#[tokio::test(start_paused = true)]
async fn a_draft_goes_up_once_you_stop_typing_and_each_save_replaces_the_last() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    tokio::time::sleep(DRAFT_PUSH_DELAY / 2).await;
    s.engine.save_draft(&current_edit(&s.engine, draft("d1", "Friday at 1?", NOW + 1))).unwrap();
    tokio::time::sleep(DRAFT_PUSH_DELAY / 2).await;
    assert!(draft_changes(&s).is_empty(), "still typing");
    after_typing().await;
    assert_eq!(draft_changes(&s), [format!("save_draft {DRAFTS}")]);
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("X-Moss-Draft: d1\r\n") && texts[0].contains("X-Moss-Mode: new\r\n"), "{}", texts[0]);
    assert!(texts[0].contains("Friday at 1?") && texts[0].contains("To: Daniel Okafor <daniel@northwind.co>"));
    // Only what you wrote: no signature, no quote.
    assert!(!texts[0].contains("-- "), "{}", texts[0]);
    assert_eq!(copy_of(&s, "d1"), Some(ServerCopy { message_id: "draft-1@moss.test".into(), provider_id: Some("1:101".into()) }));
    assert!(pushed(&s, "d1"));

    s.engine.save_draft(&current_edit(&s.engine, draft("d1", "Friday at 2?", NOW + 2))).unwrap();
    assert!(!pushed(&s, "d1"));
    after_typing().await;
    assert_eq!(draft_changes(&s)[1..], [
        format!("save_draft {DRAFTS}"),
        format!("delete_drafts {DRAFTS} draft-1@moss.test"),
    ]);
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1, "never two copies");
    assert!(texts[0].contains("Friday at 2?"));
    // Nothing changed since: the next sync leaves it be.
    sync(&s).await;
    assert_eq!(draft_changes(&s).len(), 3);
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_reply_goes_up_in_its_conversation() {
    let s = setup();
    s.fake.with_drafts();
    s.inbox(1, "a@x", "Budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let reply = Draft {
        mode: ComposeMode::Reply,
        thread_id: Some(s.thread_id("Budget")),
        subject: "Re: Budget".into(),
        ..draft("d1", "Signed.", NOW)
    };
    s.engine.save_draft(&reply).unwrap();
    after_typing().await;
    let text = &s.fake.draft_texts()[0];
    assert!(text.contains("In-Reply-To: <a@x>\r\nReferences: <a@x>\r\n"), "{text}");
    assert!(text.contains("X-Moss-Mode: reply\r\n"), "{text}");
    assert!(!text.contains("wrote:"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn sending_or_discarding_removes_the_draft_from_the_server() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    assert_eq!(s.fake.drafts().len(), 1);
    assert!(s.engine.delete_draft("d1", false).unwrap());
    settle().await;
    assert!(s.fake.drafts().is_empty());
    assert_eq!(draft_changes(&s).last().unwrap(), &format!("delete_drafts {DRAFTS} draft-1@moss.test"));
    assert!(s.pending().is_empty());
    // One that never went up has nothing to remove.
    s.engine.save_draft(&draft("d2", "Hi", NOW)).unwrap();
    assert!(s.engine.delete_draft("d2", false).unwrap());
    after_typing().await;
    assert_eq!(draft_changes(&s).len(), 2);
    assert!(s.fake.drafts().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_draft_discarded_while_it_goes_up_doesnt_stay_there() {
    let s = with_drafts().await;
    s.fake.slow(Duration::from_secs(1));
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    // On its way up now.
    assert!(s.engine.delete_draft("d1", false).unwrap());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(s.fake.drafts().is_empty(), "{:?}", draft_changes(&s));
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn failed_cleanup_of_a_discarded_upload_is_retained_and_cannot_resurrect_it() {
    let s = with_drafts().await;
    s.fake.slow(Duration::from_secs(1));
    s.engine.save_draft(&draft("discarded-upload", "Do not resurrect", NOW)).unwrap();
    after_typing().await;
    assert!(s.engine.delete_draft("discarded-upload", false).unwrap());
    s.fake.fail_next_draft_delete(moss_providers::Error::Network("cleanup offline".into()));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(s.fake.drafts().len(), 1);
    assert_eq!(s.pending().len(), 1, "failed cleanup must remain queued");
    assert_eq!(s.engine.shared.store().earliest_change("gmail").unwrap().unwrap().state, moss_store::ChangeState::Queued);
    assert!(s.engine.shared.pull_drafts("gmail").await.is_err());
    assert!(s.engine.drafts().unwrap().is_empty(), "pending removal is not a new draft");
    s.engine.shared.send_all("gmail").await.unwrap();
    assert!(s.fake.drafts().is_empty());
    assert!(s.pending().is_empty());
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_draft_edited_while_it_goes_up_goes_up_again() {
    for edited_at in [NOW, NOW + 1] {
    let s = with_drafts().await;
    s.fake.slow(Duration::from_secs(1));
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    s.engine.save_draft(&current_edit(&s.engine, draft("d1", "Friday at 1?", edited_at))).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!pushed(&s, "d1"), "the copy has the older text");
    after_typing().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Friday at 1?"));
    assert!(pushed(&s, "d1"));
    }
}

#[tokio::test(start_paused = true)]
async fn drafts_written_offline_go_up_on_the_next_sync() {
    let s = with_drafts().await;
    s.events.clear();
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::Offline, .. })).await;
    assert!(s.fake.drafts().is_empty());
    assert!(!pushed(&s, "d1"));
    sync(&s).await;
    assert_eq!(s.fake.drafts().len(), 1);
    assert!(pushed(&s, "d1"));

    // A removal with no answer waits for the next sync, and goes then.
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    assert!(s.engine.delete_draft("d1", false).unwrap());
    settle().await;
    assert_eq!(s.fake.drafts().len(), 1);
    assert_eq!(s.pending().len(), 1);
    // Still there on the server, but not brought back while its removal waits.
    assert!(s.engine.shared.pull_drafts("gmail").await.is_err());
    assert!(s.engine.drafts().unwrap().is_empty());
    sync(&s).await;
    assert!(s.fake.drafts().is_empty());
    assert!(s.pending().is_empty());
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_draft_the_server_wont_take_is_skipped_and_the_rest_go_up() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    s.engine.save_draft(&draft("d2", "Monday?", NOW + 1)).unwrap();
    s.fake.fail_next(moss_providers::Error::Protocol("NO [OVERQUOTA]".into()));
    after_typing().await;
    assert_eq!(s.fake.drafts().len(), 1);
    // It tries again on the next sync.
    sync(&s).await;
    assert_eq!(s.fake.drafts().len(), 2);
    assert!(pushed(&s, "d1") && pushed(&s, "d2"));
}

#[tokio::test(start_paused = true)]
async fn a_draft_moved_to_another_account_leaves_the_first_ones_drafts() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    s.engine.shared.store().upsert_account(&account("work", "maya@northwind.co")).unwrap();
    s.engine.save_draft(&Draft { from_account_id: "work".into(), ..current_edit(&s.engine, draft("d1", "Friday?", NOW + 1)) }).unwrap();
    settle().await;
    assert!(s.fake.drafts().is_empty(), "{:?}", draft_changes(&s));
    assert_eq!(copy_of(&s, "d1"), None);
    // Saved again in the same account: nothing to remove.
    s.engine.save_draft(&Draft { from_account_id: "work".into(), ..current_edit(&s.engine, draft("d1", "Friday!", NOW + 2)) }).unwrap();
    settle().await;
    assert_eq!(draft_changes(&s).len(), 2);
}

#[tokio::test(start_paused = true)]
async fn importing_files_while_moving_accounts_removes_the_old_server_draft() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    s.engine.shared.store().upsert_account(&account("work", "maya@northwind.co")).unwrap();
    let moved = Draft { from_account_id: "work".into(), ..current_edit(&s.engine, draft("d1", "Friday?", NOW + 1)) };
    let saved = s.engine.attach_draft_contents(&moved, &[("notes.txt".into(), b"notes".to_vec())]).unwrap();
    settle().await;
    assert!(s.fake.drafts().is_empty(), "the old account must not retain a sendable draft");
    assert_eq!(copy_of(&s, "d1"), None);
    assert_eq!(saved.from_account_id, "work");
    assert_eq!(saved.attachments[0].name, "notes.txt");
    assert_eq!(s.engine.drafts().unwrap(), [saved]);
}

#[tokio::test(start_paused = true)]
async fn drafts_started_on_the_phone_come_in_and_reply_in_their_conversation() {
    let s = setup();
    s.fake.with_drafts();
    s.inbox(1, "a@x", "Budget", NOW - DAY);
    on_server(&s, 1, phone_draft("p1@phone", "Lunch?", "Friday?", NOW));
    let reply = reply_to(&["a@x"], phone_draft("p2@phone", "Re: Budget", "Signed.\n\n> Hello about Budget", NOW + 1));
    on_server(&s, 2, reply);
    s.gmail().await;
    s.synced().await;
    s.events.wait_for(|e| *e == Event::DraftsChanged { account_id: "gmail".into() }).await;
    let drafts = s.engine.drafts().unwrap();
    assert_eq!(drafts.len(), 2);
    let reply = drafts.iter().find(|d| d.id == "server:p2@phone").unwrap();
    assert_eq!((reply.mode, reply.thread_id.clone()), (ComposeMode::Reply, Some(s.thread_id("Budget"))));
    assert_eq!(reply.body, "Signed.");
    assert_eq!(reply.body_html, "Signed.");
    assert_eq!(reply.updated_at, (NOW + 1) / 1000 * 1000);
    let new = drafts.iter().find(|d| d.id == "server:p1@phone").unwrap();
    assert_eq!((new.mode, new.body.as_str(), new.body_html.as_str()), (ComposeMode::New, "Friday?", "Friday?"));
    assert!(pushed(&s, "server:p1@phone"));
    // Nothing to fetch again.
    let downloads = s.fake.downloads();
    sync(&s).await;
    assert_eq!(s.fake.downloads(), downloads);

    // Sent from Moss: it threads, and leaves the server's Drafts.
    let outgoing = OutgoingDraft {
        mode: reply.mode,
        thread_id: reply.thread_id.clone(),
        from_account_id: "gmail".into(),
        to: reply.to.clone(),
        cc: vec![],
        bcc: vec![],
        subject: reply.subject.clone(),
        body: reply.body.clone(),
        body_html: reply.body_html.clone(),
        signature: vec![],
        from_address: reply.from_address.clone(),
        attachments: vec![],
        remind_if_no_reply: false,
    };
    s.engine.send(outgoing).await.unwrap();
    assert_eq!(s.fake.sent()[0].in_reply_to.as_deref(), Some("a@x"));
    assert!(s.engine.delete_draft("server:p2@phone", false).unwrap());
    settle().await;
    let left: Vec<_> = s.fake.drafts().iter().filter_map(|m| message_id_of(&m.raw)).collect();
    assert_eq!(left, ["p1@phone"]);
}

const FULL: &str = "Maya Ross\nDesign lead, Northwind\nnorthwind.co";

/// Two saved signatures: a full one for replies, a one-line one for new emails.
fn with_signatures(s: &Setup) {
    for (id, lines) in [("full", FULL), ("short", "Maya")] {
        let lines = lines.lines().map(String::from).collect();
        s.engine.save_signature(&moss_core::Signature { id: id.into(), name: id.into(), lines }).unwrap();
    }
    s.engine.set_signature_choice("gmail", moss_core::SignatureUse::NewEmails, Some("short")).unwrap();
    s.engine.set_signature_choice("gmail", moss_core::SignatureUse::Replies, Some("full")).unwrap();
}

fn signature_of(s: &Setup, id: &str) -> (Option<String>, bool) {
    let d = s.engine.drafts().unwrap().into_iter().find(|d| d.id == id).unwrap();
    (d.signature_id, d.signature_chosen)
}

/// `raw` as a copy Moss wrote of draft `id`, with `headers` of its own.
fn moss_copy(raw: Vec<u8>, id: &str, headers: &str) -> Vec<u8> {
    let raw = String::from_utf8(raw).unwrap();
    raw.replacen("Content-Type", &format!("X-Moss-Draft: {id}\r\n{headers}Content-Type"), 1).into_bytes()
}

#[tokio::test(start_paused = true)]
async fn a_draft_from_another_app_gets_the_signature_a_new_one_would() {
    let s = setup();
    s.fake.with_drafts();
    s.inbox(1, "a@x", "Budget", NOW - DAY);
    s.inbox(2, "b@x", "Plans", NOW - DAY);
    // Sent from another address you added to the account.
    let yours = raw_from("mine@x", "Maya <hi@rossstudio.art>", "Daniel Okafor <daniel@northwind.co>", "Re: Plans", NOW - DAY / 2);
    s.fake.add(message("INBOX", 3, reply_to(&["b@x"], yours), NOW - DAY / 2));
    s.gmail().await;
    s.synced().await;
    s.engine.shared.store().add_address("gmail", "hi@rossstudio.art", None).unwrap();
    with_signatures(&s);
    on_server(&s, 1, phone_draft("p1@phone", "Lunch?", "Friday?", NOW));
    on_server(&s, 2, reply_to(&["a@x"], phone_draft("p2@phone", "Re: Budget", "Signed.", NOW)));
    // You already wrote in this one: a back-and-forth gets the one-line signature.
    on_server(&s, 3, reply_to(&["b@x", "mine@x"], phone_draft("p3@phone", "Re: Plans", "Yes.", NOW)));
    // A forward goes to someone new: the one for new emails.
    let forward = moss_copy(phone_draft("f1@moss", "Fwd: Budget", "See below.", NOW), "f1", "X-Moss-Mode: forward\r\n");
    on_server(&s, 4, reply_to(&["a@x"], forward));
    sync(&s).await;
    assert_eq!(signature_of(&s, "f1"), (Some("short".into()), false));
    assert_eq!(signature_of(&s, "server:p1@phone"), (Some("short".into()), false));
    assert_eq!(signature_of(&s, "server:p2@phone"), (Some("full".into()), false));
    assert_eq!(signature_of(&s, "server:p3@phone"), (Some("short".into()), false));
    // Its text is as it was, and the server's copy is left alone.
    assert_eq!(s.engine.drafts().unwrap().iter().find(|d| d.id == "server:p1@phone").unwrap().body, "Friday?");
    assert!(pushed(&s, "server:p1@phone"));
}

#[tokio::test(start_paused = true)]
async fn a_draft_that_ends_with_a_saved_signature_takes_it_out_of_its_text() {
    let s = with_drafts().await;
    with_signatures(&s);
    on_server(&s, 1, phone_draft("p1@phone", "Lunch?", &format!("Friday?\r\n\r\n-- \r\n{FULL}"), NOW));
    sync(&s).await;
    let d = s.engine.drafts().unwrap().remove(0);
    // Not the one for new emails: the one already there, now as its signature.
    assert_eq!((d.body.as_str(), d.body_html.as_str()), ("Friday?", "Friday?"));
    assert_eq!((d.signature_id.as_deref(), d.signature_chosen), (Some("full"), false));

    // Edited here, it goes up without a signature in its text, and comes back with one.
    let mut edited = d.clone();
    edited.body = "Friday at 1?".into();
    s.engine.save_draft(&current_edit(&s.engine, edited)).unwrap();
    after_typing().await;
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Friday at 1?") && !texts[0].contains("Design lead") && !texts[0].contains("X-Moss-Signature"), "{}", texts[0]);
    sync(&s).await;
    let d = s.engine.drafts().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!((d[0].body.as_str(), d[0].signature_id.as_deref()), ("Friday at 1?", Some("full")));
}

/// A draft as Gmail web leaves it: only HTML, a `<div>` for each line.
fn web_draft(id: &str, divs: &str) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(NOW).unwrap().to_rfc2822();
    format!(
        "From: Maya Ross <maya.ross@gmail.com>\r\nTo: Daniel Okafor <daniel@northwind.co>\r\n\
         Subject: Lunch?\r\nMessage-ID: <{id}>\r\nDate: {date}\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n<div dir=\"ltr\">{divs}</div>\r\n"
    )
    .into_bytes()
}

#[tokio::test(start_paused = true)]
async fn a_gmail_web_draft_ending_with_a_saved_signature_takes_it_out_of_its_text() {
    let s = with_drafts().await;
    with_signatures(&s);
    let text = "<div>Hi Daniel,</div><div><br></div><div>Friday works.</div><div><br></div>";
    let signature = "<div class=\"gmail_signature\"><div><b>Maya</b> Ross</div><div>Design lead, Northwind</div>\
                     <div><a href=\"https://northwind.co\">northwind.co</a></div></div>";
    on_server(&s, 1, web_draft("w1@web", &format!("{text}{signature}")));
    on_server(&s, 2, web_draft("w2@web", &format!("{text}<div>-- </div>{signature}")));
    on_server(&s, 3, web_draft("w3@web", text));
    sync(&s).await;
    let drafts = s.engine.drafts().unwrap();
    let read = |id: &str| drafts.iter().find(|d| d.id == id).unwrap();
    for id in ["server:w1@web", "server:w2@web"] {
        let d = read(id);
        assert_eq!((d.body.as_str(), d.body_html.as_str()), ("Hi Daniel,\n\nFriday works.", "Hi Daniel,<br><br>Friday works."), "{id}");
        assert_eq!((d.signature_id.as_deref(), d.signature_chosen), (Some("full"), false), "{id}");
    }
    // No signature in it: the one for new emails, and its text as it was.
    let d = read("server:w3@web");
    assert_eq!((d.signature_id.as_deref(), d.body.as_str()), (Some("short"), "Hi Daniel,\n\nFriday works."));
    assert!(d.body_html.contains(text), "{}", d.body_html);
}

#[tokio::test(start_paused = true)]
async fn a_signature_you_took_off_doesnt_come_back() {
    let s = with_drafts().await;
    with_signatures(&s);
    on_server(&s, 1, phone_draft("p1@phone", "Lunch?", "Friday?", NOW));
    sync(&s).await;
    assert_eq!(signature_of(&s, "server:p1@phone"), (Some("short".into()), false));
    let mut removed = s.engine.drafts().unwrap().remove(0);
    (removed.signature_id, removed.signature_chosen) = (None, true);
    s.engine.save_draft(&current_edit(&s.engine, removed)).unwrap();
    after_typing().await;
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("X-Moss-Signature: none\r\n"), "{}", texts[0]);
    sync(&s).await;
    sync(&s).await;
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
    assert_eq!(signature_of(&s, "server:p1@phone"), (None, true));

    // Read by another Mac (or this one after starting over): still none, and chosen ones stay.
    let other = with_drafts().await;
    with_signatures(&other);
    on_server(&other, 1, texts[0].clone().into_bytes());
    on_server(&other, 2, moss_copy(phone_draft("m2@moss", "Lunch?", "Friday?", NOW), "m2", "X-Moss-Signature: full\r\n"));
    // One that's gone since picks again, as does one Moss picked.
    on_server(&other, 3, moss_copy(phone_draft("m3@moss", "Lunch?", "Friday?", NOW), "m3", "X-Moss-Signature: gone\r\n"));
    on_server(&other, 4, moss_copy(phone_draft("m4@moss", "Lunch?", "Friday?", NOW), "m4", ""));
    sync(&other).await;
    assert_eq!(signature_of(&other, "server:p1@phone"), (None, true));
    assert_eq!(signature_of(&other, "m2"), (Some("full".into()), true));
    assert_eq!(signature_of(&other, "m3"), (Some("short".into()), false));
    assert_eq!(signature_of(&other, "m4"), (Some("short".into()), false));
}

#[tokio::test(start_paused = true)]
async fn a_draft_from_an_address_you_havent_added_goes_from_the_account() {
    let s = setup();
    s.fake.with_drafts();
    let raw = String::from_utf8(phone_draft("p1@phone", "Lunch?", "Friday?", NOW)).unwrap();
    on_server(&s, 1, raw.replace("maya.ross@gmail.com", "someone@else.co").into_bytes());
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert_eq!(s.engine.drafts().unwrap()[0].from_address, None);
}

#[tokio::test(start_paused = true)]
async fn a_draft_sent_or_deleted_elsewhere_goes_here_too() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    s.fake.forget(DRAFTS, "1:101");
    s.events.clear();
    sync(&s).await;
    assert!(s.engine.drafts().unwrap().is_empty());
    assert!(s.events.all().contains(&Event::DraftsChanged { account_id: "gmail".into() }));
}

#[tokio::test(start_paused = true)]
async fn a_draft_deleted_elsewhere_while_you_edit_it_here_goes_up_again() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    s.fake.forget(DRAFTS, "1:101");
    s.engine.save_draft(&current_edit(&s.engine, draft("d1", "Friday at 1?", NOW + 1))).unwrap();
    // The server is read before the edit goes up.
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Friday at 1?"));
}

#[tokio::test(start_paused = true)]
async fn newer_remote_edits_update_clean_drafts_and_conflicts_keep_both_versions() {
    let s = with_drafts().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    // Edited on another Mac an hour later: a new copy, the old one gone.
    let later = String::from_utf8(phone_draft("mac2@moss", "Lunch?", "Friday at 1?", NOW + 3_600_000)).unwrap();
    let later = later.replace("Content-Type", "X-Moss-Draft: d1\r\nX-Moss-Mode: new\r\nContent-Type");
    s.fake.forget(DRAFTS, "1:101");
    on_server(&s, 50, later.into_bytes());
    s.events.clear();
    sync(&s).await;
    let kept = s.engine.drafts().unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].body, "Friday at 1?");
    assert_eq!(kept[0].updated_at, NOW + 3_600_000);
    // What only this Mac knows stays.
    assert_eq!((kept[0].signature_id.as_deref(), kept[0].signature_chosen), (Some("s1"), true));
    assert_eq!(copy_of(&s, "d1").unwrap().message_id, "mac2@moss");
    assert!(pushed(&s, "d1"));
    assert!(s.events.all().contains(&Event::DraftsChanged { account_id: "gmail".into() }));

    // An unknown older copy may be a clock-skewed edit. Keep it independently.
    let older = String::from_utf8(phone_draft("old@moss", "Lunch?", "Thursday?", NOW - 3_600_000)).unwrap();
    on_server(&s, 51, older.replace("Content-Type", "X-Moss-Draft: d1\r\nContent-Type").into_bytes());
    sync(&s).await;
    assert_eq!(s.engine.drafts().unwrap()[0].body, "Friday at 1?");
    let left: Vec<_> = s.fake.drafts().iter().filter_map(|m| message_id_of(&m.raw)).collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(s.fake.draft_texts().iter().any(|text|text.contains("Friday at 1?")));
    assert!(s.engine.drafts().unwrap().iter().any(|draft|draft.body=="Thursday?"));
    assert!(pushed(&s, "d1"));

    // Changed here after the other Mac's copy was replaced elsewhere: still the newest wins.
    s.engine.save_draft(&current_edit(&s.engine, draft("d1", "Saturday?", NOW + 7_200_000))).unwrap();
    let stale = String::from_utf8(phone_draft("mac3@moss", "Lunch?", "Sunday?", NOW + 5_400_000)).unwrap();
    for m in s.fake.drafts() {
        s.fake.forget(DRAFTS, &m.provider_id);
    }
    on_server(&s, 52, stale.replace("Content-Type", "X-Moss-Draft: d1\r\nContent-Type").into_bytes());
    sync(&s).await;
    after_typing().await;
    assert_eq!(s.engine.drafts().unwrap()[0].body, "Saturday?");
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts.iter().any(|text|text.contains("Saturday?")));
    assert!(s.engine.drafts().unwrap().iter().any(|draft|draft.body=="Sunday?"));
}

#[tokio::test(start_paused = true)]
async fn its_own_copy_is_found_when_the_server_didnt_say_where_it_went() {
    let s = with_drafts().await;
    s.fake.hide_draft_ids();
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    assert_eq!(copy_of(&s, "d1").unwrap().provider_id, None);
    sync(&s).await;
    assert_eq!(copy_of(&s, "d1").unwrap().provider_id.as_deref(), Some("1:101"));
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
    assert!(pushed(&s, "d1"));
}

#[tokio::test(start_paused = true)]
async fn a_copy_found_after_you_edited_again_still_goes_up() {
    let s = with_drafts().await;
    s.fake.hide_draft_ids();
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    s.engine.save_draft(&current_edit(&s.engine, draft("d1", "Friday at 1?", NOW + 1))).unwrap();
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Friday at 1?"));
    assert!(pushed(&s, "d1"));
}

#[tokio::test(start_paused = true)]
async fn a_server_message_that_isnt_an_email_is_left_alone() {
    let s = setup();
    s.fake.with_drafts();
    on_server(&s, 1, b"\r\n\r\n".to_vec());
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert!(s.engine.drafts().unwrap().is_empty());
    assert_eq!(s.fake.drafts().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn without_a_drafts_folder_drafts_stay_on_this_mac() {
    // Resend: no folders at all.
    let s = setup();
    s.fake.with_drafts();
    s.fake.local_only();
    s.gmail().await;
    s.synced().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    sync(&s).await;
    assert!(s.fake.drafts().is_empty());
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
    assert!(s.engine.delete_draft("d1", false).unwrap());
    s.engine.shared.pull_drafts("gmail").await.unwrap();

    // An IMAP server that lists no Drafts folder.
    let s = setup();
    s.gmail().await;
    s.synced().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    assert!(s.fake.changes().iter().all(|c| !c.contains("draft")), "{:?}", s.fake.changes());
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
}

#[test]
fn saving_a_draft_outside_the_async_runtime_still_saves_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = setup_with(dir, FakeProvider::new(), Arc::new(MemorySecretStore::new()));
    s.engine.shared.store().upsert_account(&account("gmail", "maya.ross@gmail.com")).unwrap();
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_provider_that_keeps_no_drafts_leaves_them_here() {
    let s = with_drafts().await;
    s.fake.drafts_unsupported();
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    after_typing().await;
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    assert_eq!(copy_of(&s, "d1"), None);
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn uncertain_changes_hold_back_draft_reconciliation() {
    let s = setup();
    s.fake.with_drafts();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    settle().await;
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    s.engine.mark_done(&s.thread_id("Budget")).unwrap();
    settle().await;
    assert_eq!(s.pending().len(), 1);
    on_server(&s, 1, phone_draft("p1@phone", "Lunch?", "Friday?", NOW));
    assert!(s.engine.shared.pull_drafts("gmail").await.is_err());
    assert!(s.engine.drafts().unwrap().is_empty());
    assert_eq!(s.pending().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_look_at_the_drafts_folder_says_so() {
    let s = with_drafts().await;
    s.events.clear();
    on_server(&s, 1, phone_draft("p1@phone", "Lunch?", "Friday?", NOW));
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    s.engine.shared.pull_in_background("gmail");
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::Offline, .. })).await;
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_missing_drafts_folder_does_not_discard_cleanup_evidence() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    s.engine.save_draft(&draft("d1", "Friday?", NOW)).unwrap();
    let copy = ServerCopy { message_id: "old@x".into(), provider_id: None };
    {
        let store=s.engine.shared.store();
        let revision=store.synced_draft("d1").unwrap().unwrap().revision;
        store.set_server_copy("d1", Some(&copy), Some(&revision)).unwrap();
    }
    assert!(s.engine.delete_draft("d1", false).unwrap());
    settle().await;
    assert_eq!(s.pending().len(), 1);
    assert_eq!(s.engine.shared.store().earliest_change("gmail").unwrap().unwrap().state, moss_store::ChangeState::Blocked);
    assert!(s.fake.changes().iter().all(|c| !c.contains("draft")));
}

#[tokio::test(start_paused = true)]
async fn an_email_set_to_send_later_leaves_the_servers_drafts_and_comes_back_if_cancelled() {
    let s = with_drafts().await;
    let d = draft("d1", "Friday?", NOW);
    let d = s.engine.save_draft(&d).unwrap();
    after_typing().await;
    assert_eq!(s.fake.drafts().len(), 1);
    // It waits on this Mac, not in Drafts, so your phone can't send it twice.
    s.engine.schedule(&d, &OutgoingDraft::from(&d), NOW + DAY).unwrap();
    settle().await;
    assert!(s.fake.drafts().is_empty(), "{:?}", draft_changes(&s));
    // Won't send it: a draft again, here and on the server.
    let back = s.engine.unschedule("d1").unwrap();
    after_typing().await;
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("X-Moss-Draft: d1"));
    assert!(pushed(&s, &back.id));
}

/// `name` attached to draft `id`, kept like the composer keeps it.
fn attach(s: &Setup, id: &str, name: &str, data: &[u8]) -> moss_core::DraftFile {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, data).unwrap();
    s.engine.attach_files(id, &[path]).unwrap().remove(0)
}

#[tokio::test(start_paused = true)]
async fn a_drafts_files_go_up_with_it() {
    let s = with_drafts().await;
    let plan = attach(&s, "d1", "plan.pdf", b"PDF-DATA");
    let lena = Person { name: "".into(), email: "lena@northwind.co".into() };
    s.engine.save_draft(&Draft { attachments: vec![plan], cc: vec![lena], ..draft("d1", "Plan attached.", NOW) }).unwrap();
    after_typing().await;
    let text = &s.fake.draft_texts()[0];
    assert!(text.contains("Cc: lena@northwind.co\r\n"), "{text}");
    assert!(text.contains("filename=\"plan.pdf\"") && text.contains("PDF-DATA"), "{text}");
    assert!(pushed(&s, "d1"));
}

#[tokio::test(start_paused = true)]
async fn a_draft_whose_file_is_gone_waits_and_the_rest_go_up() {
    let s = with_drafts().await;
    s.engine.attach_draft_contents(&draft("d1", "Lost.", NOW), &[("lost.pdf".into(), b"fixture".to_vec())]).unwrap();
    // Simulate bytes disappearing after a valid save, rather than accepting an invalid import.
    rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap()
        .execute("DELETE FROM draft_files WHERE draft_id='d1'", []).unwrap();
    s.engine.save_draft(&draft("d2", "Fine.", NOW + 1)).unwrap();
    after_typing().await;
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Fine."));
    assert!(!pushed(&s, "d1"));
}

#[tokio::test(start_paused = true)]
async fn a_drafts_files_come_in_with_it() {
    let s = setup();
    s.fake.with_drafts();
    let raw = String::from_utf8(phone_draft("p1@phone", "Plan", "Here it is.", NOW)).unwrap().replace(
        "Content-Type: text/plain; charset=utf-8\r\n\r\nHere it is.\r\n",
        "Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nHere it is.\r\n\
         --m\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nUERGLURBVEE=\r\n--m--\r\n",
    );
    on_server(&s, 1, raw.into_bytes());
    s.gmail().await;
    s.synced().await;
    s.events.wait_for(|e| *e == Event::DraftsChanged { account_id: "gmail".into() }).await;
    let d = s.engine.drafts().unwrap().remove(0);
    assert_eq!(d.body, "Here it is.");
    assert_eq!(d.attachments.len(), 1);
    assert_eq!((d.attachments[0].name.as_str(), d.attachments[0].bytes), ("plan.pdf", 8));
    let kept = s.engine.shared.store().draft_file(&d.attachments[0].id).unwrap().unwrap();
    assert_eq!(kept, ("plan.pdf".into(), "application/pdf".into(), b"PDF-DATA".to_vec()));
}

#[tokio::test(start_paused = true)]
async fn an_email_with_files_set_to_send_later_comes_back_up_with_them() {
    let s = with_drafts().await;
    let plan = attach(&s, "d1", "plan.pdf", b"PDF-DATA");
    let d = Draft { attachments: vec![plan], ..draft("d1", "Plan attached.", NOW) };
    let d = s.engine.save_draft(&d).unwrap();
    after_typing().await;
    s.engine.schedule(&d, &OutgoingDraft::from(&d), NOW + DAY).unwrap();
    settle().await;
    assert!(s.fake.drafts().is_empty());
    s.engine.unschedule("d1").unwrap();
    after_typing().await;
    let texts = s.fake.draft_texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("filename=\"plan.pdf\""), "{}", texts[0]);
}

#[tokio::test(start_paused = true)]
async fn a_draft_gone_between_listing_and_fetching_is_left_for_the_next_sync() {
    let s = with_drafts().await;
    s.fake.set_all_ids(DRAFTS, &["1:77"]);
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn preparing_a_send_persists_the_payload_before_any_submission() {
    let s = with_drafts().await;
    let draft = draft("durable", "Friday?", NOW);
    let draft = s.engine.save_draft(&draft).unwrap();
    let email = OutgoingDraft::from(&draft);
    assert!(s.engine.prepare_send("operation", &draft, &email).await.unwrap());
    assert!(s.fake.sent().is_empty());
    assert!(s.engine.drafts().unwrap().is_empty());
    let kept = s.engine.shared.store().delivery("operation").unwrap().unwrap();
    let prepared: crate::delivery::PreparedSend = serde_json::from_str(&kept.outgoing).unwrap();
    assert_eq!(prepared.email, email);
    assert_eq!(prepared.message.text.as_deref(), Some("Friday?"));
    assert_eq!(prepared.message.date.unwrap().timestamp_millis(), NOW);
    assert_eq!(prepared.message.message_id.as_deref(), Some(kept.message_id.as_str()));
    assert_eq!(kept.state, moss_store::DeliveryState::Held);
    assert!(!s.engine.prepare_send("operation", &draft, &email).await.unwrap());
    let changed = OutgoingDraft { body: "Different".into(), ..email };
    assert!(s.engine.prepare_send("operation", &draft, &changed).await.is_err());
    assert_eq!(s.engine.shared.store().delivery("operation").unwrap().unwrap(), kept);
}

#[tokio::test(start_paused = true)]
async fn preparation_removes_the_server_draft_before_it_can_be_sent_elsewhere() {
    let s = with_drafts().await;
    let draft = draft("durable-server", "Friday?", NOW);
    let draft = s.engine.save_draft(&draft).unwrap();
    after_typing().await;
    assert_eq!(s.fake.drafts().len(), 1);
    s.engine.prepare_send("server-operation", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
    settle().await;
    assert!(s.fake.drafts().is_empty());
    assert!(s.fake.sent().is_empty());
    assert_eq!(s.engine.shared.store().delivery("server-operation").unwrap().unwrap().state, moss_store::DeliveryState::Held);
}

#[tokio::test(start_paused = true)]
async fn a_durable_send_is_submitted_once_and_returns_its_receipt_again() {
    let s = with_drafts().await;
    let draft = draft("once", "Friday?", NOW);
    s.engine.prepare_send("once-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
    let thread = s.engine.submit_send("once-op").await.unwrap();
    assert_eq!(s.fake.sent().len(), 1);
    assert_eq!(s.engine.submit_send("once-op").await.unwrap(), thread);
    assert_eq!(s.fake.sent().len(), 1);
    assert_eq!(s.engine.shared.store().delivery("once-op").unwrap().unwrap().state, moss_store::DeliveryState::Complete);
    assert_eq!(s.engine.thread(&thread).unwrap().messages.last().unwrap().body_text.trim(), "Friday?");
}

#[tokio::test(start_paused = true)]
async fn a_saved_delivery_that_cannot_be_read_is_refused_and_never_sent() {
    let s = with_drafts().await;
    let draft = draft("unreadable", "Friday?", NOW);
    let email = OutgoingDraft::from(&draft);
    assert!(s.engine.prepare_send("unreadable-op", &draft, &email).await.unwrap());
    rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap()
        .execute("UPDATE deliveries SET outgoing='{' WHERE id='unreadable-op'", []).unwrap();
    let changed = OutgoingDraft { body: "Different".into(), ..email };
    assert!(matches!(s.engine.prepare_send("unreadable-op", &draft, &changed).await, Err(Error::Invalid(m)) if m.contains("cannot be read")));
    assert!(matches!(s.engine.submit_send("unreadable-op").await, Err(Error::Invalid(m)) if m.contains("cannot be read")));
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_acceptance_that_cannot_be_saved_is_uncertain_and_never_sent_again() {
    let s = with_drafts().await;
    let draft = draft("unsaved-receipt", "Friday?", NOW);
    s.engine.prepare_send("receipt-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_acceptance BEFORE UPDATE OF state ON deliveries WHEN NEW.state = 'accepted' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("receipt-op").await, Err(Error::DeliveryUnknown(m)) if m.contains("receipt could not be saved")));
    assert_eq!(s.fake.sent().len(), 1);
    db.execute_batch("DROP TRIGGER refuse_acceptance;").unwrap();
    assert!(matches!(s.engine.submit_send("receipt-op").await, Err(Error::DeliveryUnknown(_))));
    assert_eq!(s.fake.sent().len(), 1, "a claimed send is never submitted twice");
}

#[tokio::test(start_paused = true)]
async fn a_claim_without_an_outcome_cannot_be_submitted_again() {
    let s = with_drafts().await;
    let draft = draft("crashed", "Friday?", NOW);
    s.engine.prepare_send("crashed-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
    s.engine.shared.store().claim_delivery("crashed-op").unwrap();
    assert!(matches!(s.engine.submit_send("crashed-op").await, Err(Error::DeliveryUnknown(_))));
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn accepted_mail_repairs_a_failed_local_insert_without_resending() {
    let s = with_drafts().await;
    let draft = draft("repair", "Friday?", NOW);
    s.engine.prepare_send("repair-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("repair-op").await, Err(Error::DeliveryAccepted(_))));
    assert_eq!(s.fake.sent().len(), 1);
    let kept = s.engine.shared.store().delivery("repair-op").unwrap().unwrap();
    assert_eq!(kept.state, moss_store::DeliveryState::Accepted);
    assert!(kept.raw.is_some());
    db.execute_batch("DROP TRIGGER refuse_sent").unwrap();
    let thread = s.engine.submit_send("repair-op").await.unwrap();
    assert_eq!(s.fake.sent().len(), 1, "repair must never cross the network submission boundary");
    assert_eq!(s.engine.thread(&thread).unwrap().messages.len(), 1);
    assert_eq!(s.engine.submit_send("repair-op").await.unwrap(), thread);
}

#[tokio::test(start_paused = true)]
async fn uncertain_submission_stays_blocked_while_a_proven_rejection_can_retry() {
    for uncertain in [false, true] {
        let s = with_drafts().await;
        let draft = draft("outcome", "Friday?", NOW);
        s.engine.prepare_send("outcome-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
        let error = if uncertain { moss_providers::Error::DeliveryUnknown("lost reply".into()) }
            else { moss_providers::Error::Network("connection refused".into()) };
        s.fake.fail_next(error);
        let result = s.engine.submit_send("outcome-op").await;
        assert!(result.is_err());
        let state = s.engine.shared.store().delivery("outcome-op").unwrap().unwrap().state;
        assert_eq!(state, if uncertain { moss_store::DeliveryState::Uncertain } else { moss_store::DeliveryState::Held });
        if uncertain {
            assert!(matches!(s.engine.submit_send("outcome-op").await, Err(Error::DeliveryUnknown(_))));
            assert!(s.fake.sent().is_empty());
        } else {
            s.engine.submit_send("outcome-op").await.unwrap();
            assert_eq!(s.fake.sent().len(), 1);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn undo_uses_the_durable_claim_and_restores_only_unsent_mail() {
    let s = with_drafts().await;
    let draft = draft("undo", "Friday?", NOW);
    s.engine.prepare_send("undo-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
    assert!(s.engine.cancel_send("undo-op").unwrap());
    assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([draft.clone()]));
    assert!(s.engine.cancel_send("undo-op").unwrap(), "a repeated successful cancellation is safe");
    assert!(s.engine.submit_send("undo-op").await.is_err());
    let draft = current_edit(&s.engine, draft);
    s.engine.prepare_send("claimed-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap();
    s.engine.shared.store().claim_delivery("claimed-op").unwrap();
    assert!(!s.engine.cancel_send("claimed-op").unwrap());
    assert!(s.engine.drafts().unwrap().is_empty());
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn restart_restores_unsent_mail_and_never_retries_an_interrupted_submission() {
    let s = with_drafts().await;
    let held = draft("held-before-crash", "Unsent", NOW);
    let claimed = draft("claimed-before-crash", "Unknown", NOW);
    s.engine.prepare_send("held-op", &held, &OutgoingDraft::from(&held)).await.unwrap();
    s.engine.prepare_send("claimed-op", &claimed, &OutgoingDraft::from(&claimed)).await.unwrap();
    s.engine.shared.store().claim_delivery("claimed-op").unwrap();
    s.engine.shared.stop_worker("gmail");
    let Setup { _dir: dir, fake, secrets, engine, .. } = s;
    drop(engine);
    settle().await; // Let aborted workers and queued changes release the old process's store.
    let again = setup_with(dir, fake.clone(), secrets);
    let new = draft("new-after-restart", "New", NOW);
    again.engine.prepare_send("new-op", &new, &OutgoingDraft::from(&new)).await.unwrap();
    assert_eq!(draft_contents(again.engine.drafts().unwrap()), draft_contents([held.clone()]));
    // You pressed Send, so you are told it didn't go, once.
    assert!(again.events.all().iter().any(|e| serde_json::to_value(e).unwrap() == serde_json::json!({"type":"sendsInterrupted"})));
    let noted = again.engine.interrupted_sends().unwrap();
    assert_eq!(noted.iter().map(|n| (n.id.as_str(), n.subject.as_str())).collect::<Vec<_>>(), [("held-op", held.subject.as_str())]);
    again.engine.dismiss_interrupted_sends(&["held-op".into()]).unwrap();
    assert!(again.engine.interrupted_sends().unwrap().is_empty());
    assert_eq!(again.engine.delivery_state("held-op").unwrap(), Some(moss_store::DeliveryState::Cancelled));
    assert_eq!(again.engine.delivery_state("claimed-op").unwrap(), Some(moss_store::DeliveryState::Uncertain));
    assert!(matches!(again.engine.submit_send("claimed-op").await, Err(Error::DeliveryUnknown(_))));
    assert!(fake.sent().is_empty());
    again.engine.recover_deliveries().unwrap();
    assert_eq!(again.engine.delivery_state("new-op").unwrap(), Some(moss_store::DeliveryState::Held), "recovery must not cancel a hold created in this process");
}

#[tokio::test(start_paused = true)]
async fn restart_repairs_accepted_mail_without_another_submission() {
    let s = with_drafts().await;
    let d = draft("accepted-before-crash", "Accepted", NOW);
    s.engine.prepare_send("accepted-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("accepted-op").await, Err(Error::DeliveryAccepted(_))));
    db.execute_batch("DROP TRIGGER refuse_sent").unwrap();
    drop(db);
    s.engine.shared.stop_worker("gmail");
    let Setup { _dir: dir, fake, secrets, engine, .. } = s;
    drop(engine);
    settle().await; // Let aborted workers and queued changes release the old process's store.
    let again = setup_with(dir, fake.clone(), secrets);
    again.engine.start().unwrap();
    assert_eq!(again.engine.delivery_state("accepted-op").unwrap(), Some(moss_store::DeliveryState::Complete));
    let thread = again.engine.submit_send("accepted-op").await.unwrap();
    assert_eq!(again.engine.thread(&thread).unwrap().messages.len(), 1);
    assert_eq!(fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn direct_sending_retains_accepted_mail_when_local_storage_fails() {
    let s = with_drafts().await;
    let d = draft("direct", "Accepted direct", NOW);
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.send(OutgoingDraft::from(&d)).await, Err(Error::DeliveryAccepted(_))));
    let pending = s.engine.shared.store().pending_deliveries().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, moss_store::DeliveryState::Accepted);
    assert!(pending[0].raw.is_some());
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn sending_a_saved_draft_deduplicates_and_allows_retry_only_after_rejection() {
    let s = with_drafts().await;
    let d = draft("saved-send", "Saved", NOW);
    let email = OutgoingDraft::from(&d);
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    assert!(s.engine.send_saved(&d, &email).await.is_err());
    assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([d.clone()]));
    let d = current_edit(&s.engine, d);
    let thread = s.engine.send_saved(&d, &email).await.unwrap();
    let retried = Draft { updated_at: NOW + 1000, ..d.clone() };
    assert_eq!(s.engine.send_saved(&retried, &email).await.unwrap(), thread);
    assert_eq!(s.fake.sent().len(), 1);
    let changed = OutgoingDraft { body: "Changed".into(), ..email };
    assert!(s.engine.send_saved(&d, &changed).await.is_err());
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_direct_send_cannot_claim_or_cancel_an_existing_undo_hold() {
    for refusal in [false, true] {
        let s = with_drafts().await;
        let d = draft("held-by-window", "Keep Undo", NOW);
        let email = OutgoingDraft::from(&d);
        s.engine.prepare_send("window-hold", &d, &email).await.unwrap();
        if refusal { s.fake.fail_next(moss_providers::Error::Network("offline".into())); }
        assert!(s.engine.send_saved(&d, &email).await.is_err());
        assert!(s.fake.sent().is_empty());
        assert_eq!(s.engine.delivery_state("window-hold").unwrap(), Some(moss_store::DeliveryState::Held));
        assert!(s.engine.drafts().unwrap().is_empty());
        assert!(s.engine.cancel_send("window-hold").unwrap(), "the window still owns Undo");
        assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([d]));
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_cancellation_preserves_the_provider_rejection() {
    for intent_failure in [false, true] {
    let s = with_drafts().await;
    let d = draft("cancel-fails", "Recover later", NOW);
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch(if intent_failure { "CREATE TRIGGER refuse_intent BEFORE INSERT ON delivery_cancellations BEGIN SELECT RAISE(ABORT, 'fixture'); END;" } else { "CREATE TRIGGER refuse_cancel BEFORE UPDATE OF state ON deliveries WHEN NEW.state = 'cancelled' BEGIN SELECT RAISE(ABORT, 'fixture'); END;" }).unwrap();
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    assert!(matches!(s.engine.send_saved(&d, &OutgoingDraft::from(&d)).await, Err(Error::Offline(_))));
    let pending = s.engine.shared.store().pending_deliveries().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, moss_store::DeliveryState::Held);
    assert_eq!(pending[0].draft, d);
    assert!(s.fake.sent().is_empty());
    assert_eq!(s.engine.shared.store().delivery_cancellation_recorded(&pending[0].id).unwrap(), !intent_failure);
    }
}

#[tokio::test(start_paused = true)]
async fn failure_to_persist_a_submission_error_keeps_the_claim_uncertain() {
    for uncertain in [false, true] {
        let s = with_drafts().await;
        let d = draft("failed-outcome-write", "Do not duplicate", NOW);
        s.engine.prepare_send("outcome-write", &d, &OutgoingDraft::from(&d)).await.unwrap();
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_outcome BEFORE UPDATE OF state ON deliveries WHEN NEW.state IN ('held', 'uncertain') BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        s.fake.fail_next(if uncertain { moss_providers::Error::DeliveryUnknown("missing receipt".into()) }
            else { moss_providers::Error::Network("refused".into()) });
        let result = s.engine.submit_send("outcome-write").await;
        assert!(matches!(result, Err(Error::DeliveryUnknown(ref message)) if message == "The outcome could not be saved."));
        assert_eq!(s.engine.delivery_state("outcome-write").unwrap(), Some(moss_store::DeliveryState::Claimed));
        assert!(matches!(s.engine.submit_send("outcome-write").await, Err(Error::DeliveryUnknown(_))));
        assert!(s.fake.sent().is_empty());
        assert!(s.engine.drafts().unwrap().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn startup_keeps_failed_accepted_repair_and_existing_uncertainty_without_resending() {
    let s = with_drafts().await;
    let accepted = draft("accepted-repair-pending", "Accepted", NOW);
    s.engine.prepare_send("accepted-pending", &accepted, &OutgoingDraft::from(&accepted)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("accepted-pending").await, Err(Error::DeliveryAccepted(_))));
    let kept = s.engine.shared.store().delivery("accepted-pending").unwrap().unwrap();
    let uncertain = draft("uncertain-before-restart", "No receipt", NOW);
    s.engine.prepare_send("uncertain-pending", &uncertain, &OutgoingDraft::from(&uncertain)).await.unwrap();
    s.fake.fail_next(moss_providers::Error::DeliveryUnknown("missing receipt".into()));
    assert!(matches!(s.engine.submit_send("uncertain-pending").await, Err(Error::DeliveryUnknown(_))));
    s.engine.shared.stop_worker("gmail");
    let Setup { _dir: dir, fake, secrets, engine, .. } = s;
    drop(engine);
    settle().await;
    let again = setup_with(dir, fake.clone(), secrets);
    again.engine.start().unwrap();
    assert_eq!(again.engine.delivery_state("accepted-pending").unwrap(), Some(moss_store::DeliveryState::Accepted));
    assert_eq!(again.engine.delivery_state("uncertain-pending").unwrap(), Some(moss_store::DeliveryState::Uncertain));
    assert_eq!(again.engine.shared.store().delivery("accepted-pending").unwrap().unwrap().raw, kept.raw);
    assert!(again.engine.drafts().unwrap().is_empty());
    assert_eq!(fake.sent().len(), 1);
    db.execute_batch("DROP TRIGGER refuse_sent").unwrap();
    let thread = again.engine.repair_send("accepted-pending").unwrap();
    assert_eq!(again.engine.repair_send("accepted-pending").unwrap(), thread);
    assert_eq!(fake.sent().len(), 1);
    assert_eq!(again.engine.thread(&thread).unwrap().messages.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn retrying_local_delivery_repair_does_not_reenable_a_disabled_reminder() {
    let s = with_drafts().await;
    let d = Draft { remind_if_no_reply: true, ..draft("reminder-repair", "Reply when ready", NOW) };
    s.engine.prepare_send("reminder-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_completion BEFORE UPDATE OF state ON deliveries WHEN NEW.state = 'complete' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("reminder-op").await, Err(Error::DeliveryAccepted(_))));
    let thread = db.query_row("SELECT thread_id FROM messages WHERE from_me = 1 ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, i64>(0)).unwrap().to_string();
    assert!(s.engine.thread(&thread).unwrap().summary.waiting.is_some());
    assert!(s.engine.remind_if_no_reply(&thread, false).unwrap());
    assert!(s.engine.thread(&thread).unwrap().summary.waiting.is_none());
    db.execute_batch("DROP TRIGGER refuse_completion").unwrap();
    assert_eq!(s.engine.repair_send("reminder-op").unwrap(), thread);
    assert!(s.engine.thread(&thread).unwrap().summary.waiting.is_none(), "local repair must preserve the user's later choice");
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_reminder_step_rolls_back_and_preserves_a_choice_before_retry() {
    let s = with_drafts().await;
    let d = Draft { remind_if_no_reply: true, ..draft("reminder-choice", "Reply when ready", NOW) };
    s.engine.prepare_send("choice-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_reminder BEFORE INSERT ON ai_decisions WHEN NEW.source = 'you' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("choice-op").await, Err(Error::DeliveryAccepted(_))));
    let applied: i64 = db.query_row("SELECT reminder_applied FROM deliveries WHERE id = 'choice-op'", [], |r| r.get(0)).unwrap();
    assert_eq!(applied, 0, "a failed reminder must leave the step retryable");
    let thread = db.query_row("SELECT thread_id FROM messages WHERE from_me = 1 ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, i64>(0)).unwrap().to_string();
    db.execute_batch("DROP TRIGGER refuse_reminder").unwrap();
    assert!(s.engine.remind_if_no_reply(&thread, false).unwrap());
    assert_eq!(s.engine.repair_send("choice-op").unwrap(), thread);
    assert!(s.engine.thread(&thread).unwrap().summary.waiting.is_none());
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn delayed_reminder_repair_does_not_apply_an_old_choice_to_a_newer_sent_message() {
    let s = with_drafts().await;
    let d = Draft { remind_if_no_reply: true, ..draft("old-reminder", "First message", NOW) };
    s.engine.prepare_send("old-reminder-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_reminder BEFORE INSERT ON ai_decisions WHEN NEW.source = 'you' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("old-reminder-op").await, Err(Error::DeliveryAccepted(_))));
    let thread = db.query_row("SELECT thread_id FROM messages WHERE from_me = 1 ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, i64>(0)).unwrap().to_string();
    let later = Draft { mode: ComposeMode::Reply, thread_id: Some(thread.clone()), ..draft("newer-message", "A newer choice without a reminder", NOW) };
    assert_eq!(s.engine.send_saved(&later, &OutgoingDraft::from(&later)).await.unwrap(), thread);
    assert!(s.engine.thread(&thread).unwrap().summary.waiting.is_none());
    db.execute_batch("DROP TRIGGER refuse_reminder").unwrap();
    assert_eq!(s.engine.repair_send("old-reminder-op").unwrap(), thread);
    assert!(s.engine.thread(&thread).unwrap().summary.waiting.is_none(), "an older send must not change the newest message's reminder choice");
    assert_eq!(s.fake.sent().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn delayed_reminder_repair_preserves_a_later_done_action() {
    let s = with_drafts().await;
    let d = Draft { remind_if_no_reply: true, ..draft("reminder-done", "Reply when ready", NOW) };
    s.engine.prepare_send("done-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_reminder BEFORE INSERT ON ai_decisions WHEN NEW.source = 'you' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("done-op").await, Err(Error::DeliveryAccepted(_))));
    let thread = db.query_row("SELECT thread_id FROM messages WHERE from_me = 1 ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, i64>(0)).unwrap().to_string();
    s.engine.mark_done(&thread).unwrap();
    assert_eq!(s.engine.thread(&thread).unwrap().summary.done_at, Some(NOW));
    db.execute_batch("DROP TRIGGER refuse_reminder").unwrap();
    assert_eq!(s.engine.repair_send("done-op").unwrap(), thread);
    assert_eq!(s.engine.thread(&thread).unwrap().summary.done_at, Some(NOW), "repair must not undo a later Done action");
    assert!(s.engine.thread(&thread).unwrap().summary.waiting.is_none());
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn delayed_reminder_repair_preserves_a_later_snooze_or_wake() {
    for until in [Some(NOW + DAY), None] {
        let s = with_drafts().await;
        let d = Draft { remind_if_no_reply: true, ..draft("reminder-snooze", "Reply when ready", NOW) };
        s.engine.prepare_send("snooze-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_reminder BEFORE INSERT ON ai_decisions WHEN NEW.source = 'you' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(matches!(s.engine.submit_send("snooze-op").await, Err(Error::DeliveryAccepted(_))));
        let thread = db.query_row("SELECT thread_id FROM messages WHERE from_me = 1 ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, i64>(0)).unwrap().to_string();
        s.engine.mark_not_done(&thread).unwrap();
        s.engine.snooze(&thread, until).unwrap();
        assert_eq!(s.engine.thread(&thread).unwrap().summary.snoozed_until, until);
        db.execute_batch("DROP TRIGGER refuse_reminder").unwrap();
        assert_eq!(s.engine.repair_send("snooze-op").unwrap(), thread);
        let repaired = s.engine.thread(&thread).unwrap();
        assert_eq!(repaired.summary.snoozed_until, until, "repair must preserve a later snooze or explicit wake");
        assert!(repaired.summary.waiting.is_none());
        assert_eq!(s.fake.sent().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn local_repair_never_claims_acceptance_for_an_unsent_or_unknown_operation() {
    for state in ["missing", "held", "claimed", "uncertain", "cancelled"] {
        let s = with_drafts().await;
        if state != "missing" {
            let d = draft("not-accepted", "No acceptance", NOW);
            s.engine.prepare_send("not-accepted-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
            let mut store = s.engine.shared.store();
            if matches!(state, "claimed" | "uncertain") { assert!(store.claim_delivery("not-accepted-op").unwrap()); }
            if state == "uncertain" { assert!(store.uncertain_delivery("not-accepted-op", "No receipt").unwrap()); }
            if state == "cancelled" { assert!(store.cancel_delivery("not-accepted-op").unwrap()); }
        }
        let result = s.engine.repair_send("not-accepted-op");
        let correct = match state {
            "missing" => matches!(result, Err(Error::Unavailable(_))),
            "claimed" | "uncertain" => matches!(result, Err(Error::DeliveryUnknown(_))),
            _ => matches!(result, Err(Error::Invalid(_))),
        };
        assert!(correct, "local repair must preserve the actual delivery state for {state}: {result:?}");
        assert!(s.fake.sent().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn damaged_accepted_receipts_keep_acceptance_without_resubmitting() {
    for damage in ["outgoing", "source", "raw", "completed"] {
        let s = with_drafts().await;
        let d = draft("accepted-damaged", "Already accepted", NOW);
        s.engine.prepare_send("damaged-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
        {
            let mut store = s.engine.shared.store();
            assert!(store.claim_delivery("damaged-op").unwrap());
            let source = moss_core::MessageSource { account_id: "gmail".into(), mailbox: "Sent".into(), provider_id: "accepted".into(), ..Default::default() };
            assert!(store.accept_delivery("damaged-op", b"accepted", &serde_json::to_string(&source).unwrap()).unwrap());
            if damage == "completed" { assert!(store.complete_delivery("damaged-op").unwrap()); }
        }
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        match damage {
            "outgoing" => db.execute("UPDATE deliveries SET outgoing = 'unreadable' WHERE id = 'damaged-op'", []).unwrap(),
            "source" => db.execute("UPDATE deliveries SET source = 'unreadable' WHERE id = 'damaged-op'", []).unwrap(),
            "raw" => db.execute("UPDATE deliveries SET raw = NULL WHERE id = 'damaged-op'", []).unwrap(),
            _ => 0,
        };
        let expected = match damage {
            "outgoing" => "The saved delivery cannot be read.",
            "source" => "The delivery source cannot be read.",
            "raw" => "The accepted email copy is missing.",
            _ => "This email was sent; its conversation is no longer on this Mac.",
        };
        for result in [s.engine.repair_send("damaged-op"), s.engine.submit_send("damaged-op").await] {
            assert!(matches!(result, Err(Error::DeliveryAccepted(ref reason)) if reason.contains(expected)), "corruption must not revoke proven acceptance: {damage}: {result:?}");
        }
        assert!(s.fake.sent().is_empty(), "repair must never cross the submission boundary");
    }
}

#[tokio::test(start_paused = true)]
async fn a_receipt_read_failure_after_submission_preserves_known_acceptance() {
    let s = with_drafts().await;
    let d = draft("accepted-read-failure", "Already delivered", NOW);
    s.engine.prepare_send("accepted-read-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER damage_receipt AFTER UPDATE OF state ON deliveries WHEN NEW.state = 'accepted' BEGIN UPDATE deliveries SET outgoing = 'unreadable' WHERE id = NEW.id; END;").unwrap();
    let result = s.engine.submit_send("accepted-read-op").await;
    assert!(matches!(result, Err(Error::DeliveryAccepted(_))), "the committed acceptance must survive a later read failure: {result:?}");
    assert_eq!(s.engine.delivery_state("accepted-read-op").unwrap(), Some(moss_store::DeliveryState::Accepted));
    assert!(matches!(s.engine.repair_send("accepted-read-op"), Err(Error::DeliveryAccepted(_))));
    assert!(matches!(s.engine.submit_send("accepted-read-op").await, Err(Error::DeliveryAccepted(_))));
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn losing_the_submission_claim_reports_the_winners_outcome() {
    for state in ["claimed", "uncertain", "accepted", "complete", "cancelled", "held", "missing"] {
        let s = with_drafts().await;
        let d = draft("claim-race", "One submission", NOW);
        s.engine.prepare_send("claim-race-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        let source = moss_core::MessageSource { account_id: "gmail".into(), mailbox: "[Gmail]/Sent Mail".into(), provider_id: "winner".into(), ..Default::default() };
        let winning = s.engine.shared.store().ingest(&raw_from("winner@example.test", MAYA, DANIEL, "Delivered", NOW), &source).unwrap();
        db.execute("UPDATE deliveries SET raw = ?1, source = ?2, thread_id = 'stale-thread', message_id='<winner@example.test>' WHERE id = 'claim-race-op'",
            rusqlite::params![raw_from("winner@example.test", MAYA, DANIEL, "Delivered", NOW), serde_json::to_string(&source).unwrap()]).unwrap();
        // Deterministically change the durable state at the compare-and-swap boundary.
        // This caller must not submit if another owner has changed it first.
        let change = if state == "missing" { "DELETE FROM deliveries WHERE id = OLD.id;".into() }
            else { format!("UPDATE deliveries SET state = '{state}' WHERE id = OLD.id;") };
        db.execute_batch(&format!("CREATE TRIGGER lose_claim BEFORE UPDATE OF state ON deliveries WHEN OLD.state = 'held' AND NEW.state = 'claimed' BEGIN {change} SELECT RAISE(IGNORE); END;")).unwrap();
        let result = s.engine.submit_send("claim-race-op").await;
        let correct = match state {
            "claimed" | "uncertain" | "missing" => matches!(result, Err(Error::DeliveryUnknown(_))),
            "cancelled" => matches!(result, Err(Error::Invalid(_))),
            "held" => matches!(result, Err(Error::DeliveryBusy)),
            "complete" => matches!(result, Ok(ref thread) if thread == &winning.thread_id),
            _ => result.is_ok(),
        };
        assert!(correct, "losing a claim must preserve the winner's outcome ({state}): {result:?}");
        assert!(s.fake.sent().is_empty(), "the caller that lost the claim must never submit");
    }
}

#[tokio::test(start_paused = true)]
async fn concurrent_submissions_send_once_and_report_in_progress_truthfully() {
    let s = with_drafts().await;
    let d = draft("concurrent-submit", "One message", NOW);
    s.engine.prepare_send("concurrent-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    s.fake.delay_sends(Duration::from_secs(2));
    let (first, second) = tokio::join!(s.engine.submit_send("concurrent-op"), s.engine.submit_send("concurrent-op"));
    assert_eq!(s.fake.sent().len(), 1);
    let results = [first, second];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|r| matches!(r, Err(Error::DeliveryUnknown(_)))).count(), 1);
    assert_eq!(s.engine.delivery_state("concurrent-op").unwrap(), Some(moss_store::DeliveryState::Complete));
}

#[tokio::test(start_paused = true)]
async fn a_busy_direct_submission_does_not_cancel_the_owned_hold() {
    let s = with_drafts().await;
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER lose_claim BEFORE UPDATE OF state ON deliveries WHEN OLD.state = 'held' AND NEW.state = 'claimed' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let d = draft("busy-direct", "Still held", NOW);
    assert!(matches!(s.engine.send_saved(&d, &OutgoingDraft::from(&d)).await, Err(Error::DeliveryBusy)));
    let store = s.engine.shared.store();
    let operation = store.active_delivery_id(&d.id).unwrap();
    assert!(operation.is_some(), "a busy outcome must retain the operation's ownership");
    assert_eq!(store.delivery_state(&operation.unwrap()).unwrap(), Some(moss_store::DeliveryState::Held));
    assert!(store.drafts(None).unwrap().is_empty());
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn undo_before_preparation_prevents_submission_and_preserves_the_saved_draft() {
    let s = with_drafts().await;
    let draft = draft("early-undo", "Latest saved text", NOW);
    let draft = s.engine.save_draft(&draft).unwrap();
    assert!(s.engine.cancel_send("early-undo-op").unwrap());
    assert!(!s.engine.prepare_send("early-undo-op", &draft, &OutgoingDraft::from(&draft)).await.unwrap());
    assert!(s.engine.submit_send("early-undo-op").await.is_err());
    assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([draft]));
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_late_engine_discard_cannot_delete_delivery_or_schedule_files() {
    for scheduled in [false, true] {
        let s = with_drafts().await;
        let d = {
            let mut store = s.engine.shared.store();
            let d = store.attach_and_save_draft(&draft("owned-discard", "Keep the file", NOW), &[("keep.txt", "text/plain", b"keep")]).unwrap();
            if scheduled { store.schedule_email(&d, "{}", NOW, NOW).unwrap(); }
            else { store.prepare_delivery("owned-discard", &d, "{}", "owned@fixture", NOW).unwrap(); }
            d
        };
        for keep_files in [false, true] {
            assert!(s.engine.delete_draft(&d.id, keep_files).is_err());
            assert_eq!(s.engine.shared.store().draft_file(&d.attachments[0].id).unwrap().unwrap().2, b"keep");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn scheduling_server_cleanup_failure_preserves_the_draft_and_removed_files() {
    let s = with_drafts().await;
    let mut d = draft("atomic-schedule", "Keep my draft", NOW);
    d.attachments = vec![attach(&s, &d.id, "keep.txt", b"keep")];
    let d = s.engine.save_draft(&d).unwrap();
    {
        let store=s.engine.shared.store();
        let revision=store.synced_draft(&d.id).unwrap().unwrap().revision;
        store.set_server_copy(&d.id, Some(&ServerCopy { message_id: "server-copy@fixture".into(), provider_id: None }), Some(&revision)).unwrap();
    }
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_schedule_cleanup BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    let mut edited = d.clone();
    edited.attachments.clear();
    assert!(s.engine.schedule(&edited, &OutgoingDraft::from(&edited), NOW + DAY).is_err());
    assert_eq!(draft_contents(s.engine.drafts().unwrap()), draft_contents([d.clone()]));
    assert!(s.engine.scheduled().unwrap().is_empty());
    assert!(s.engine.shared.store().draft_file(&d.attachments[0].id).unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn schedule_operation_retries_and_early_cancellation_keep_their_outcomes() {
    use moss_store::ScheduleOutcome;
    let s = with_drafts().await;
    let d = draft("identified-schedule", "Keep this content", NOW);
    let d = s.engine.save_draft(&d).unwrap();
    {
        let store=s.engine.shared.store();
        let revision=store.synced_draft(&d.id).unwrap().unwrap().revision;
        store.set_server_copy(&d.id, Some(&ServerCopy { message_id: "server@fixture".into(), provider_id: None }), Some(&revision)).unwrap();
    }
    assert_eq!(s.engine.schedule_with_operation("op", &d, &OutgoingDraft::from(&d), NOW + DAY).unwrap(), ScheduleOutcome::Scheduled);
    assert_eq!(s.engine.schedule_with_operation("op", &d, &OutgoingDraft::from(&d), NOW + 2 * DAY).unwrap(), ScheduleOutcome::AlreadyScheduled);
    assert_eq!(s.engine.scheduled().unwrap()[0].send_at, NOW + DAY);
    assert_eq!(s.pending().len(), 1, "server-copy cleanup is queued once");
    let cancelled = draft("cancelled-schedule", "Do not send", NOW);
    let cancelled = s.engine.save_draft(&cancelled).unwrap();
    s.engine.shared.store().cancel_schedule_operation("cancelled-op", &cancelled.id, NOW).unwrap();
    assert_eq!(s.engine.schedule_with_operation("cancelled-op", &cancelled, &OutgoingDraft::from(&cancelled), NOW + DAY).unwrap(), ScheduleOutcome::Cancelled { early: true, restored: None });
    assert!(s.engine.drafts().unwrap().contains(&cancelled));
}


#[tokio::test]
async fn inline_image_references_read_back_as_they_were_written() {
    for html in ["<img src=cid:x/>", "<div style=\"background:url(&quot;cid:x&quot;)\"></div>", "<img src=\"CID:x\">", "<p>Literal cid: text</p>", "<img src=\"cid:x\">", "<p>Plain HTML</p>"] {
        let s=setup();
        let mut store=s.engine.shared.store();
        store.upsert_account(&account("work","sender@example.test")).unwrap();
        store.set_offline(moss_store::Offline::Everything,NOW).unwrap();
        let raw=format!("From: Alex <alex@example.test>\r\nMessage-ID: <image@example.test>\r\nSubject: Image\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=part\r\n\r\n--part\r\nContent-Type: text/html\r\n\r\n{html}\r\n--part\r\nContent-Type: image/png\r\nContent-ID: <x>\r\nContent-Disposition: inline; filename=x.png\r\n\r\nimagebytes\r\n--part--\r\n");
        let a=store.ingest(raw.as_bytes(),&moss_core::MessageSource {account_id:"work".into(),mailbox:"INBOX".into(),provider_id:"a".into(),..Default::default()}).unwrap();
        let message=store.thread(&a.thread_id,NOW).unwrap().unwrap().messages.remove(0);
        assert!(!message.images_missing);
        if html == "<img src=\"cid:x\">" { assert!(message.body_html.unwrap().contains("data:image/png;base64,"), "the picture is shown"); }
        else { assert_eq!(message.body_html.as_deref(),Some(html)); }
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_account_move_cleanup_rolls_back_text_files_and_server_identity() {
    for importing in [false, true] {
        let s = with_drafts().await;
        s.engine.shared.store().upsert_account(&account("work", "work@example.test")).unwrap();
        let mut d = draft("atomic-move", "Keep the old draft", NOW);
        d.attachments = vec![attach(&s, &d.id, "keep.txt", b"keep")];
        let d = s.engine.save_draft(&d).unwrap();
        let before = {
            let store = s.engine.shared.store();
            let revision = store.synced_draft(&d.id).unwrap().unwrap().revision;
            store.set_server_copy(&d.id, Some(&ServerCopy {message_id:"old@fixture".into(),provider_id:Some("1:9".into())}), Some(&revision)).unwrap();
            store.synced_draft(&d.id).unwrap().unwrap()
        };
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_move_cleanup BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        let mut moved = d.clone();
        moved.from_account_id = "work".into();
        moved.body = "New text".into();
        moved.attachments.clear();
        let result = if importing {
            s.engine.attach_draft_contents(&moved, &[("new.txt".into(),b"new".to_vec())]).map(|_| ())
        } else { s.engine.save_draft(&moved).map(|_| ()) };
        assert!(result.is_err());
        let store = s.engine.shared.store();
        assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), before, "importing={importing}");
        assert_eq!(store.draft_file_usage(&d.id).unwrap(), (1,4));
        assert_eq!(store.draft_file(&d.attachments[0].id).unwrap().unwrap().2,b"keep");
        assert!(store.pending_changes("gmail").unwrap().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn a_draft_delete_with_no_answer_holds_back_later_changes_and_uploads_until_it_goes() {
    let s = with_drafts().await;
    let change = serde_json::to_string(&ServerChange::DeleteDrafts { message_ids: vec!["old@fixture".into()] }).unwrap();
    let seen = serde_json::to_string(&ServerChange::Seen { mailbox: "INBOX".into(), ids: vec!["1:42".into()], seen: true }).unwrap();
    {
        let store = s.engine.shared.store();
        store.queue_change("gmail", &change, NOW).unwrap();
        store.queue_change("gmail", &seen, NOW).unwrap();
        store.save_draft(&draft("unrelated", "Still upload this", NOW)).unwrap();
    }
    s.fake.fail_next_draft_delete(moss_providers::Error::Network("offline".into()));
    assert!(s.engine.shared.send_all("gmail").await.is_err());
    assert_eq!(s.pending().len(), 2);
    assert_eq!(s.pending()[0].1, change);
    assert!(s.fake.applied().is_empty());
    assert!(s.fake.drafts().is_empty());
    s.engine.shared.send_all("gmail").await.unwrap();
    assert!(s.pending().is_empty());
    assert_eq!(s.fake.applied(), [("INBOX".into(), vec!["1:42".into()], moss_providers::Change::Seen(true))]);
    assert_eq!(s.fake.drafts().len(), 1);
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_direct_cleanup_preserves_the_remaining_local_uploads() {
    let s = with_drafts().await;
    s.fake.slow(Duration::from_secs(1));
    {
        let store = s.engine.shared.store();
        store.save_draft(&draft("first", "First upload", NOW)).unwrap();
        store.save_draft(&draft("second", "Second upload", NOW + 1)).unwrap();
    }
    let shared = s.engine.shared.clone();
    let pushing = tokio::spawn(async move { shared.push_drafts("gmail").await });
    settle().await;
    // Discover which row the ordered snapshot uploads first, then remove that row.
    let first = s.engine.shared.store().synced_drafts("gmail").unwrap()[0].draft.id.clone();
    s.engine.shared.store().delete_draft(&first).unwrap();
    s.fake.fail_next_draft_delete(moss_providers::Error::Protocol("cleanup refused".into()));
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(pushing.await.unwrap().is_err());
    assert_eq!(s.pending().len(), 1);
    assert_eq!(s.fake.drafts().len(), 1, "the orphan waits for recovery; later uploads remain local");
    let remaining = s.engine.shared.store().synced_drafts("gmail").unwrap();
    assert_eq!(remaining.len(), 1);
    assert!(!remaining[0].pushed);
}


#[tokio::test(start_paused = true)]
async fn failed_discard_cleanup_keeps_the_draft_and_its_files() {
    for keep_files in [false, true] {
        let s = with_drafts().await;
        let mut d = draft("atomic-discard", "Keep until deletion commits", NOW);
        d.attachments = vec![attach(&s, &d.id, "keep.txt", b"keep")];
        let d = s.engine.save_draft(&d).unwrap();
        let before = {
            let store = s.engine.shared.store();
            let revision = store.synced_draft(&d.id).unwrap().unwrap().revision;
            store.set_server_copy(&d.id, Some(&ServerCopy { message_id: "discard@fixture".into(), provider_id: Some("1:9".into()) }), Some(&revision)).unwrap();
            store.synced_draft(&d.id).unwrap().unwrap()
        };
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_discard_cleanup BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(s.engine.delete_draft(&d.id, keep_files).is_err());
        let store = s.engine.shared.store();
        assert_eq!(store.synced_draft(&d.id).unwrap(), Some(before));
        assert_eq!(store.draft_file_usage(&d.id).unwrap(), (1,4));
        assert!(store.pending_changes("gmail").unwrap().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn a_remote_draft_hint_cannot_overwrite_a_draft_in_another_account() {
    let s = with_drafts().await;
    s.engine.shared.store().upsert_account(&account("work", "work@example.test")).unwrap();
    let local = Draft { from_account_id: "work".into(), ..draft("shared-hint", "Private work edits", NOW) };
    s.engine.shared.store().save_draft(&local).unwrap();
    let before = s.engine.shared.store().synced_draft(&local.id).unwrap().unwrap();
    let raw = String::from_utf8(phone_draft("stale@phone", "Old personal draft", "Remote personal text", NOW + DAY)).unwrap();
    on_server(&s, 900, format!("X-Moss-Draft: shared-hint\r\n{raw}").into_bytes());
    on_server(&s,901,phone_draft("independent@example.test","Independent draft","Keep syncing",NOW));
    let result = s.engine.shared.pull_drafts("gmail").await;
    assert_eq!(s.engine.shared.store().synced_draft(&local.id).unwrap(), Some(before));
    assert!(s.fake.draft_texts().iter().any(|text| text.contains("Remote personal text")), "unresolved content must remain on the server");
    result.unwrap();
    assert!(s.engine.drafts().unwrap().iter().any(|draft|draft.body=="Keep syncing"));
    assert!(s.events.all().iter().any(|event|matches!(event,Event::SyncError{message,..} if message.contains("another account"))),"the conflict must be reported while unrelated drafts continue");
}

#[tokio::test(start_paused = true)]
async fn refused_remote_draft_import_rolls_back_its_files() {
    let s = with_drafts().await;
    let raw = String::from_utf8(phone_draft("atomic@phone", "Plan", "Here it is.", NOW)).unwrap().replace(
        "Content-Type: text/plain; charset=utf-8\r\n\r\nHere it is.\r\n",
        "Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nHere it is.\r\n--m\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=plan.txt\r\n\r\nkept remotely\r\n--m--\r\n",
    );
    on_server(&s, 901, raw.into_bytes());
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_remote_draft BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(s.engine.shared.pull_drafts("gmail").await.is_err());
    assert_eq!(s.engine.shared.store().draft_file_usage("server:atomic@phone").unwrap(), (0, 0));
    assert!(s.engine.shared.store().synced_draft("server:atomic@phone").unwrap().is_none());
    assert!(s.fake.draft_texts().iter().any(|text| text.contains("kept remotely")));
}

#[tokio::test(start_paused = true)]
async fn acknowledged_upload_survives_failed_old_copy_cleanup_without_reuploading() {
    for offline in [false, true] {
        let s = with_drafts().await;
        let mut d = draft("accepted-upload", "First copy", NOW);
        s.engine.shared.store().save_draft(&d).unwrap();
        s.engine.shared.push_drafts("gmail").await.unwrap();
        let old = copy_of(&s, &d.id).unwrap();
        d.subject = "Updated copy".into();
        s.engine.shared.store().save_draft(&d).unwrap();
        let failure = if offline { moss_providers::Error::Network("offline".into()) }
            else { moss_providers::Error::Protocol("cleanup refused".into()) };
        s.fake.fail_next_draft_delete(failure);
        assert!(s.engine.shared.push_drafts("gmail").await.is_err());
        assert!(pushed(&s, &d.id), "the acknowledged upload must be recorded before cleanup");
        let accepted = copy_of(&s, &d.id).unwrap();
        assert_ne!(accepted.message_id, old.message_id);
        assert_eq!(s.fake.drafts().len(), 2);
        assert_eq!(s.pending().iter().map(|(_, json)| json.clone()).collect::<Vec<_>>(),
            vec![serde_json::to_string(&ServerChange::DeleteDrafts { message_ids:vec![old.message_id] }).unwrap()]);
        // No answer: tried again, and the old copy goes. A refusal waits for you.
        assert_eq!(s.engine.shared.send_all("gmail").await.is_err(), !offline);
        assert_eq!(s.pending().len(), usize::from(!offline));
        assert_eq!(copy_of(&s, &d.id), Some(accepted));
        assert_eq!(s.fake.drafts().len(), if offline { 1 } else { 2 });
        assert!(s.fake.draft_texts().iter().any(|text| text.contains("Updated copy")));
    }
}

#[tokio::test(start_paused = true)]
async fn a_conflicting_upload_receipt_does_not_block_other_drafts() {
    let s = with_drafts().await;
    let mut first = draft("receipt-conflict", "First version", NOW + 1);
    {
        let mut store = s.engine.shared.store();
        store.keep_server_draft(&first, &ServerCopy { message_id:"draft-1@moss.test".into(),provider_id:Some("1:50".into()) }).unwrap();
        first.body.push_str(" new content");
        store.save_draft(&first).unwrap();
        store.save_draft(&draft("unrelated-upload", "Still upload this", NOW)).unwrap();
    }
    let before = s.engine.shared.store().synced_draft(&first.id).unwrap();
    s.engine.shared.push_drafts("gmail").await.unwrap();
    assert_eq!(s.engine.shared.store().synced_draft(&first.id).unwrap(), before);
    assert!(!pushed(&s, &first.id));
    assert!(pushed(&s, "unrelated-upload"));
    assert!(s.pending().is_empty());
    assert_eq!(s.fake.drafts().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_failed_receipt_transaction_stops_other_uploads() {
    for action in ["ABORT,'fixture'", "IGNORE"] {
        let s = with_drafts().await;
        let mut first = draft("receipt-storage-failure", "First version", NOW + 1);
        {
            let mut store = s.engine.shared.store();
            store.keep_server_draft(&first, &ServerCopy { message_id:"old@fixture".into(),provider_id:Some("1:50".into()) }).unwrap();
            first.body.push_str(" new content");
            store.save_draft(&first).unwrap();
            store.save_draft(&draft("not-uploaded", "Wait for healthy storage", NOW)).unwrap();
        }
        let before = s.engine.shared.store().synced_draft(&first.id).unwrap();
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER refuse_upload_cleanup BEFORE INSERT ON outbox BEGIN SELECT RAISE({action}); END;")).unwrap();
        assert!(s.engine.shared.push_drafts("gmail").await.is_err());
        assert_eq!(s.engine.shared.store().synced_draft(&first.id).unwrap(), before);
        assert!(!pushed(&s, "not-uploaded"));
        assert!(s.pending().is_empty());
        assert_eq!(s.fake.drafts().len(), 1, "stop after the accepted but unrecorded upload");
    }
}

#[tokio::test(start_paused = true)]
async fn successful_direct_upload_cleanup_records_provider_success() {
    let s=with_drafts().await;
    let mut draft=draft("cleanup-outcome","First",NOW);
    s.engine.shared.store().save_draft(&draft).unwrap();
    s.engine.shared.push_drafts("gmail").await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TABLE observed_part_outcomes(outcome TEXT); CREATE TRIGGER observe_part_outcome AFTER UPDATE OF outcome ON outbox_batch_parts BEGIN INSERT INTO observed_part_outcomes VALUES(NEW.outcome); END;").unwrap();
    draft.subject="Second".into();
    s.engine.shared.store().save_draft(&draft).unwrap();
    s.engine.shared.push_drafts("gmail").await.unwrap();
    let outcomes=db.prepare("SELECT outcome FROM observed_part_outcomes").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(outcomes,["completed"]);
    assert!(s.pending().is_empty());
    assert_eq!(s.fake.drafts().len(),1);
}

#[tokio::test(start_paused = true)]
async fn a_lost_acceptance_update_keeps_submission_unknown_without_resending() {
    let s = with_drafts().await;
    let d = draft("acceptance-lost", "One message", NOW);
    s.engine.prepare_send("acceptance-lost-op", &d, &OutgoingDraft::from(&d)).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER lose_acceptance BEFORE UPDATE OF state ON deliveries WHEN NEW.state='accepted' BEGIN UPDATE deliveries SET state='uncertain' WHERE id=OLD.id; SELECT RAISE(IGNORE); END;").unwrap();
    let result = s.engine.submit_send("acceptance-lost-op").await;
    assert!(matches!(result, Err(Error::DeliveryUnknown(ref text)) if text == "The acceptance state could not be recorded."));
    assert_eq!(s.fake.sent().len(), 1);
    assert_eq!(s.engine.delivery_state("acceptance-lost-op").unwrap(), Some(moss_store::DeliveryState::Uncertain));
    assert!(matches!(s.engine.submit_send("acceptance-lost-op").await, Err(Error::DeliveryUnknown(_))));
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn preparation_losing_ownership_during_attachment_download_reports_busy() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("original@example.test", "Attachment", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let original = s.message_with_subject("Attachment");
    let mut d = draft("preparation-race", "One message", NOW);
    d.attachments = vec![moss_core::DraftFile {
        id: format!("{}:1", original.id), name:"Q4.pdf".into(), bytes:7, content_type:"application/pdf".into(),
        forwarded:Some(moss_core::FileRef { message_id:original.id.clone(), index:1 }),
    }];
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("DELETE FROM attachment_data WHERE message_id=?1", [&original.id]).unwrap();
    db.execute("DELETE FROM message_raw WHERE message_id=?1", [&original.id]).unwrap();
    s.fake.delay_fetches(Duration::from_millis(200));
    let email = OutgoingDraft::from(&d);
    let competitor = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        s.engine.shared.store().prepare_delivery("winning-preparation", &d, "{}", "winning@example.test", NOW)
    };
    let (lost, won) = tokio::join!(s.engine.send_saved(&d, &email), competitor);
    assert!(matches!(lost, Err(Error::DeliveryBusy)), "{lost:?}");
    assert!(won.unwrap());
    assert_eq!(s.engine.shared.store().active_delivery_id(&d.id).unwrap().as_deref(), Some("winning-preparation"));
    assert_eq!(s.engine.delivery_state("winning-preparation").unwrap(), Some(moss_store::DeliveryState::Held));
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn invalid_server_draft_cleanup_keeps_pre_network_actions_atomic() {
    for action in ["discard", "move", "prepare", "schedule"] {
        for target in ["cannot search this".to_string(), "x".repeat(1_048_576)] {
            let s = with_drafts().await;
            s.engine.shared.store().upsert_account(&account("other", "other@example.test")).unwrap();
            let mut d = draft("bounded-cleanup", "Keep this", NOW);
            let path = s._dir.path().join("kept.txt");
            std::fs::write(&path, b"kept").unwrap();
            d.attachments = s.engine.attach_files(&d.id, &[path]).unwrap();
            s.engine.shared.store().keep_server_draft(&d, &ServerCopy { message_id:target, provider_id:Some("1:1".into()) }).unwrap();
            let before = s.engine.shared.store().synced_draft(&d.id).unwrap();
            d.revision = before.as_ref().map(|saved| saved.revision.clone());
            let result = match action {
                "discard" => s.engine.delete_draft(&d.id, false).map(|_| ()),
                "move" => s.engine.save_draft(&Draft { from_account_id:"other".into(), ..d.clone() }).map(|_| ()),
                "prepare" => s.engine.prepare_send("bounded-operation", &d, &OutgoingDraft::from(&d)).await.map(|_| ()),
                _ => s.engine.schedule_with_operation("bounded-operation", &d, &OutgoingDraft::from(&d), NOW + DAY).map(|_| ()),
            };
            assert!(matches!(result, Err(Error::Invalid(_))), "{action}: {result:?}");
            let store = s.engine.shared.store();
            assert_eq!(store.synced_draft(&d.id).unwrap(), before, "{action}");
            assert!(store.draft_file(&d.attachments[0].id).unwrap().is_some());
            assert!(store.delivery("bounded-operation").unwrap().is_none());
            assert!(store.scheduled_operation(&d.id).unwrap().is_none());
            assert!(store.pending_changes("gmail").unwrap().is_empty());
            assert!(store.pending_changes("other").unwrap().is_empty());
            assert!(s.fake.sent().is_empty());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn retained_queue_intent_stops_direct_draft_upload_and_pull_mutations() {
    for state in ["queued", "claimed", "blocked"] {
        let s = with_drafts().await;
        let d = draft("held-by-recovery", "Retain locally", NOW);
        s.engine.shared.store().save_draft(&d).unwrap();
        let id = s.engine.shared.store().queue_change("gmail", "unreadable", NOW).unwrap();
        if state == "claimed" { s.engine.shared.store().claim_change("gmail", id, 0).unwrap(); }
        if state == "blocked" { s.engine.shared.store().block_change("gmail", id, 0, moss_store::ChangeFailure::Unreadable).unwrap(); }
        let before = s.fake.changes();
        assert!(s.engine.shared.push_drafts("gmail").await.is_err());
        assert!(s.engine.shared.pull_drafts("gmail").await.is_err());
        assert_eq!(s.fake.changes(), before);
        assert_eq!(s.engine.shared.store().draft(&d.id).unwrap(), Some(d));
        assert_eq!(s.pending().len(), 1);
        s.fake.deliver(message("INBOX", 40, raw("new-read@example.test", "Read still works", NOW), NOW));
        s.engine.shared.pass("gmail", false).await.unwrap();
        assert!(s.engine.thread(&s.thread_id("Read still works")).is_ok());
        assert_eq!(s.pending().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn direct_upload_waits_for_the_account_mutation_attempt_to_finish() {
    let s = with_drafts().await;
    let d = draft("serialized-upload", "Upload once", NOW);
    s.engine.shared.store().save_draft(&d).unwrap();
    let json = serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["1:1".into()], seen:true }).unwrap();
    s.engine.shared.store().queue_change("gmail", &json, NOW).unwrap();
    s.fake.slow(Duration::from_secs(1));
    let sending = s.engine.shared.send_all("gmail");
    tokio::pin!(sending);
    assert!(std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(sending.as_mut(), cx).is_pending())).await);
    let uploading = s.engine.shared.push_drafts("gmail");
    tokio::pin!(uploading);
    assert!(std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(uploading.as_mut(), cx).is_pending())).await);
    let (sent, uploaded) = tokio::join!(sending, uploading);
    sent.unwrap();
    uploaded.unwrap();
    assert!(s.pending().is_empty());
    assert_eq!(s.fake.drafts().len(), 1);
    assert!(pushed(&s, &d.id));
}

#[tokio::test(start_paused = true)]
async fn a_queue_barrier_arriving_during_draft_download_prevents_local_reconciliation() {
    let s = with_drafts().await;
    on_server(&s, 71, phone_draft("late-barrier@example.test", "Remote draft", "Keep remotely", NOW));
    s.fake.delay_fetches(Duration::from_millis(200));
    let pulling = s.engine.shared.pull_drafts("gmail");
    tokio::pin!(pulling);
    assert!(std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(pulling.as_mut(), cx).is_pending())).await);
    let payload = serde_json::to_string(&ServerChange::DeleteDrafts { message_ids:vec!["late-barrier@example.test".into()] }).unwrap();
    s.engine.shared.store().queue_change("gmail", &payload, NOW).unwrap();
    assert!(pulling.await.is_err());
    assert!(s.engine.drafts().unwrap().is_empty(), "downloaded data cannot bypass newly queued intent");
    assert_eq!(s.fake.drafts().len(), 1);
    assert_eq!(s.pending().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn sending_cannot_borrow_another_drafts_attachment_bytes() {
    let s = with_drafts().await;
    let mut owner = draft("file-owner", "Private draft", NOW);
    owner.attachments = s.engine.attach_draft_contents(&owner, &[("private.txt".into(), b"private fixture bytes".to_vec())]).unwrap().attachments;
    let target = draft("file-target", "Other draft", NOW);
    let target = s.engine.save_draft(&target).unwrap();
    let mut outgoing = OutgoingDraft::from(&target);
    outgoing.attachments = owner.attachments.clone();
    assert!(s.engine.prepare_send("foreign-files", &target, &outgoing).await.is_err());
    assert!(s.fake.sent().is_empty());
    assert!(s.engine.shared.store().delivery("foreign-files").unwrap().is_none());
    assert!(s.engine.drafts().unwrap().iter().any(|draft| draft.id == target.id));
}

#[tokio::test(start_paused = true)]
async fn a_local_edit_during_remote_draft_fetch_survives_and_keeps_the_fetched_copy() {
    for unknown_receipt in [false,true] {
    let s=with_drafts().await;
    if unknown_receipt {s.fake.hide_draft_ids();}
    s.engine.save_draft(&draft("fetch-race","Original",NOW)).unwrap();
    after_typing().await;
    if unknown_receipt {for copy in s.fake.drafts(){s.fake.forget(DRAFTS,&copy.provider_id);}}
    let remote=String::from_utf8(phone_draft("remote-race@example.test","Lunch?","Remote text",NOW+1000)).unwrap();
    on_server(&s,71,remote.replace("Content-Type","X-Moss-Draft: fetch-race\r\nX-Moss-Mode: new\r\nContent-Type").into_bytes());
    s.fake.delay_fetches(Duration::from_millis(200));
    let pulling=s.engine.shared.pull_drafts("gmail");tokio::pin!(pulling);
    assert!(std::future::poll_fn(|cx|std::task::Poll::Ready(std::future::Future::poll(pulling.as_mut(),cx).is_pending())).await);
    let newer=draft("fetch-race","Typed while downloading",NOW+1);
    s.engine.shared.store().save_draft(&newer).unwrap();
    pulling.await.unwrap();
    let kept=s.engine.drafts().unwrap();
    assert_eq!(kept.len(),2);
    assert!(draft_contents(kept.clone()).contains(&newer));
    assert!(kept.iter().any(|draft|draft.body=="Remote text"));
    assert!(pushed(&s,"fetch-race"),"the normal push at the end of pull uploads the preserved local edit");
    let copies=s.fake.drafts();
    assert_eq!(copies.len(),2);
    let recovered=kept.iter().find(|draft|draft.body=="Remote text").unwrap();
    assert!(s.fake.draft_texts().iter().any(|text|text.contains("Remote text") && text.contains(&format!("X-Moss-Draft: {}\r\n",recovered.id))));
    assert!(s.fake.draft_texts().iter().any(|text|text.contains("Typed while downloading")));
    }
}

#[tokio::test(start_paused = true)]
async fn older_remote_replacement_keeps_both_edits_across_repeated_pulls() {
    let s=with_drafts().await;
    s.engine.save_draft(&draft("clock-skew","Local text",NOW+1)).unwrap();
    after_typing().await;
    for copy in s.fake.drafts(){s.fake.forget(DRAFTS,&copy.provider_id);}
    let remote=String::from_utf8(phone_draft("clock-skew@example.test","Lunch?","Remote text",NOW)).unwrap();
    on_server(&s,71,remote.replace("Content-Type","X-Moss-Draft: clock-skew\r\nX-Moss-Mode: new\r\nContent-Type").into_bytes());
    for _ in 0..2 {
        s.engine.shared.pull_drafts("gmail").await.unwrap();
        let mut bodies:Vec<_>=s.engine.drafts().unwrap().into_iter().map(|draft|draft.body).collect();bodies.sort();
        assert_eq!(bodies,["Local text","Remote text"]);
        assert_eq!(s.fake.drafts().len(),2);
    }
}

#[tokio::test(start_paused = true)]
async fn recovered_conflicts_survive_reconciliation_by_two_devices() {
    let a = with_drafts().await;
    a.engine.save_draft(&draft("shared", "Original", NOW - 10_000)).unwrap();
    a.engine.shared.push_drafts("gmail").await.unwrap();
    let b = setup_with(tempfile::tempdir().unwrap(), a.fake.clone(), Arc::new(MemorySecretStore::new()));
    b.gmail().await;
    b.synced().await;
    b.engine.shared.pull_drafts("gmail").await.unwrap();
    assert_eq!(b.engine.drafts().unwrap()[0].body, "Original");
    b.engine.save_draft(&current_edit(&b.engine, draft("shared", "Device B edit", NOW - 5_000))).unwrap();
    b.engine.shared.push_drafts("gmail").await.unwrap();
    a.engine.save_draft(&current_edit(&a.engine, draft("shared", "Device A edit", NOW))).unwrap();
    for _ in 0..3 {
        a.engine.shared.pull_drafts("gmail").await.unwrap();
        b.engine.shared.pull_drafts("gmail").await.unwrap();
    }
    for device in [&a, &b] {
        let mut bodies: Vec<_> = device.engine.drafts().unwrap().into_iter().map(|d| d.body).collect();
        bodies.sort();
        assert_eq!(bodies, ["Device A edit", "Device B edit"]);
    }
    let texts=a.fake.draft_texts();
    assert_eq!(texts.len(),2);
    for body in ["Device A edit", "Device B edit"] {
        assert!(texts.iter().any(|text|text.contains(body)));
    }
}

#[tokio::test(start_paused = true)]
async fn a_fresh_device_recovers_multiple_remote_copies_of_one_draft() {
    let s=with_drafts().await;
    for (uid,body) in [(70,"First remote version"),(71,"Second remote version")] {
        let raw=String::from_utf8(phone_draft(&format!("copy-{uid}@example.test"),"Conflict",body,NOW)).unwrap();
        on_server(&s,uid,raw.replace("Content-Type","X-Moss-Draft: shared\r\nContent-Type").into_bytes());
    }
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    let mut bodies:Vec<_>=s.engine.drafts().unwrap().into_iter().map(|d|d.body).collect();
    bodies.sort();
    assert_eq!(bodies,["First remote version","Second remote version"]);
    assert_eq!(s.fake.draft_texts().len(),2);
}
#[tokio::test(start_paused = true)]
async fn independently_pushed_edits_survive_when_the_other_device_pulls_first() {
    let a = with_drafts().await;
    a.engine.save_draft(&draft("shared", "Original", NOW - 10_000)).unwrap();
    a.engine.shared.push_drafts("gmail").await.unwrap();
    let b = setup_with(tempfile::tempdir().unwrap(), a.fake.clone(), Arc::new(MemorySecretStore::new()));
    b.gmail().await;
    b.synced().await;
    b.engine.shared.pull_drafts("gmail").await.unwrap();
    assert_eq!(b.engine.drafts().unwrap()[0].body, "Original");
    b.engine.save_draft(&current_edit(&b.engine, draft("shared", "Device B edit", NOW - 5_000))).unwrap();
    b.engine.shared.push_drafts("gmail").await.unwrap();
    a.engine.save_draft(&current_edit(&a.engine, draft("shared", "Device A edit", NOW))).unwrap();
    a.engine.shared.push_drafts("gmail").await.unwrap();
    for _ in 0..3 {
        b.engine.shared.pull_drafts("gmail").await.unwrap();
        a.engine.shared.pull_drafts("gmail").await.unwrap();
    }
    for device in [&a, &b] {
        let mut bodies: Vec<_> = device.engine.drafts().unwrap().into_iter().map(|d| d.body).collect();
        bodies.sort();
        bodies.dedup();
        assert_eq!(bodies, ["Device A edit", "Device B edit"]);
    }
    let texts=a.fake.draft_texts();
    assert!(texts.len() >= 2);
    for body in ["Device A edit", "Device B edit"] {
        assert!(texts.iter().any(|text|text.contains(body)));
    }
}
#[tokio::test(start_paused = true)]
async fn conflict_recovery_converges_when_upload_receipts_omit_provider_ids() {
    let a = with_drafts().await;
    a.fake.hide_draft_ids();
    a.engine.save_draft(&draft("shared", "Original", NOW - 10_000)).unwrap();
    a.engine.shared.push_drafts("gmail").await.unwrap();
    let b = setup_with(tempfile::tempdir().unwrap(), a.fake.clone(), Arc::new(MemorySecretStore::new()));
    b.gmail().await;
    b.synced().await;
    b.engine.shared.pull_drafts("gmail").await.unwrap();
    assert_eq!(b.engine.drafts().unwrap()[0].body, "Original");
    b.engine.save_draft(&current_edit(&b.engine, draft("shared", "Device B edit", NOW - 5_000))).unwrap();
    b.engine.shared.push_drafts("gmail").await.unwrap();
    a.engine.save_draft(&current_edit(&a.engine, draft("shared", "Device A edit", NOW))).unwrap();
    a.engine.shared.push_drafts("gmail").await.unwrap();
    let mut counts=Vec::new();
    for _ in 0..6 {
        b.engine.shared.pull_drafts("gmail").await.unwrap();
        a.engine.shared.pull_drafts("gmail").await.unwrap();
        counts.push(a.fake.draft_texts().len());
    }
    assert_eq!(counts[4],counts[5],"recovery must converge, not multiply on every pull: {counts:?}");
    assert_eq!(counts[5],3,"this conservative recovery retains one duplicate, not an arbitrary plateau");
    for device in [&a, &b] {
        let mut bodies: Vec<_> = device.engine.drafts().unwrap().into_iter().map(|d| d.body).collect();
        bodies.sort();
        bodies.dedup();
        assert_eq!(bodies, ["Device A edit", "Device B edit"]);
    }
    let texts=a.fake.draft_texts();
    assert!(texts.len() >= 2);
    for body in ["Device A edit", "Device B edit"] {
        assert!(texts.iter().any(|text|text.contains(body)));
    }
}

#[tokio::test(start_paused = true)]
async fn an_absent_pushed_copy_without_a_uid_receipt_is_removed_after_a_complete_listing() {
    let s=with_drafts().await;
    s.fake.hide_draft_ids();
    s.engine.save_draft(&draft("unknown-uid","Removed elsewhere",NOW)).unwrap();
    s.engine.shared.push_drafts("gmail").await.unwrap();
    assert!(copy_of(&s,"unknown-uid").unwrap().provider_id.is_none());
    for copy in s.fake.drafts() {s.fake.forget(DRAFTS,&copy.provider_id);}
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn unreadable_headers_cannot_prove_an_unknown_upload_absent() {
    let s=with_drafts().await;
    s.fake.hide_draft_ids();
    s.engine.save_draft(&draft("unverified-copy","Keep this text",NOW)).unwrap();
    s.engine.shared.push_drafts("gmail").await.unwrap();
    for copy in s.fake.drafts() {s.fake.forget(DRAFTS,&copy.provider_id);}
    on_server(&s,75,b"\r\n\r\n".to_vec());
    on_server(&s,76,phone_draft("valid@example.test","Valid","Unrelated draft",NOW));
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    let kept=s.engine.drafts().unwrap();
    assert!(kept.iter().any(|draft|draft.body=="Keep this text"));
    assert!(kept.iter().any(|draft|draft.body=="Unrelated draft"));
    assert_eq!(s.fake.drafts().len(),2);
}

#[tokio::test(start_paused = true)]
async fn an_undated_remote_draft_uses_the_engine_clock_without_provider_receipt_time() {
    let s=with_drafts().await;
    let raw=b"From: Sender <sender@example.test>\r\nTo: Reader <reader@example.test>\r\nMessage-ID: <undated@example.test>\r\nSubject: Undated\r\nContent-Type: text/plain\r\n\r\nKeep this undated draft\r\n".to_vec();
    let mut remote=message(DRAFTS,81,raw,NOW);
    remote.received_at=None;
    s.fake.add(remote);
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    let kept=s.engine.drafts().unwrap();
    assert_eq!(kept.len(),1);
    assert_eq!(kept[0].updated_at,NOW);
    assert_eq!(kept[0].body,"Keep this undated draft");
}

#[tokio::test(start_paused = true)]
async fn partial_header_fetch_cannot_authorize_replacing_a_still_present_copy() {
    let s=with_drafts().await;
    s.fake.hide_draft_ids();
    s.engine.save_draft(&draft("partial","Local edit",NOW)).unwrap();
    s.engine.shared.push_drafts("gmail").await.unwrap();
    let raw=String::from_utf8(phone_draft("independent@example.test","Conflict","Remote edit",NOW+1000)).unwrap();
    on_server(&s,70,raw.replace("Content-Type","X-Moss-Draft: partial\r\nContent-Type").into_bytes());
    s.fake.set_all_ids(DRAFTS,&["1:70","1:101"]);
    s.fake.omit_headers(vec!["1:101".into()]);
    s.engine.shared.pull_drafts("gmail").await.unwrap();
    let kept=s.engine.drafts().unwrap();
    assert!(kept.iter().any(|draft|draft.body=="Local edit"));
    assert!(kept.iter().any(|draft|draft.body=="Remote edit"));
}

#[tokio::test(start_paused = true)]
async fn unreadable_held_payloads_preserve_ownership_without_claiming_or_resending() {
    let s=with_drafts().await;
    let d=draft("damaged-held","Keep my recoverable text",NOW);
    assert_eq!(s.engine.delivery_receipt(&d.id).unwrap(),None);
    s.engine.prepare_send("damaged-held-op",&d,&OutgoingDraft::from(&d)).await.unwrap();
    assert_eq!(s.engine.delivery_receipt(&d.id).unwrap(),Some(moss_store::DeliveryReceipt { id: "damaged-held-op".into(), state: moss_store::DeliveryState::Held, source_revision: d.revision.clone() }));
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE deliveries SET outgoing='unreadable' WHERE id='damaged-held-op'",[]).unwrap();
    assert!(!s.engine.prepare_send("damaged-held-op",&d,&OutgoingDraft::from(&d)).await.unwrap(),"the matching durable fingerprint identifies a retry without reading the damaged payload");
    assert!(matches!(s.engine.submit_send("damaged-held-op").await,Err(Error::Invalid(message)) if message=="The saved delivery cannot be read."));
    assert_eq!(s.engine.delivery_state("damaged-held-op").unwrap(),Some(moss_store::DeliveryState::Held));
    assert_eq!(s.engine.delivery_receipt(&d.id).unwrap().unwrap().state,moss_store::DeliveryState::Held);
    assert!(s.fake.sent().is_empty());
    db.execute("UPDATE deliveries SET draft='unreadable' WHERE id='damaged-held-op'",[]).unwrap();
    assert_eq!(s.engine.delivery_receipt("damaged-held").unwrap().unwrap().id,"damaged-held-op");
}

#[tokio::test(start_paused = true)]
async fn compacted_delivery_replays_before_loading_any_message_content() {
    let s=with_drafts().await;
    let d=draft("compact-replay","Sent only once",NOW);
    let email=OutgoingDraft::from(&d);
    s.engine.prepare_send("compact-replay-op",&d,&email).await.unwrap();
    let legacy_db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    legacy_db.execute("UPDATE deliveries SET logical_hash=NULL WHERE id='compact-replay-op'",[]).unwrap();
    assert!(!s.engine.prepare_send("compact-replay-op",&d,&email).await.unwrap(),"uncompacted legacy receipts still compare their saved payload");
    let sent=s.engine.submit_send("compact-replay-op").await.unwrap();
    let legacy_outgoing=s.engine.shared.store().delivery("compact-replay-op").unwrap().unwrap().outgoing;
    let fingerprint=moss_store::delivery_fingerprint("logical",vec![
        serde_json::to_value(moss_core::Draft {revision:None,updated_at:0,..d.clone()}).unwrap(),
        serde_json::to_value(&email).unwrap()]);
    assert!(s.engine.shared.store().compact_delivery_step(NOW,|_,_|Some(fingerprint)).unwrap());
    let later=moss_core::Draft {updated_at:NOW+100,revision:Some("irrelevant".into()),..d.clone()};
    assert!(!s.engine.prepare_send("compact-replay-op",&later,&email).await.unwrap());
    assert_eq!(s.engine.submit_send("compact-replay-op").await.unwrap(),sent);
    let changed=OutgoingDraft {body:"changed".into(),..email};
    assert!(matches!(s.engine.prepare_send("compact-replay-op",&d,&changed).await,Err(Error::Invalid(_))));
    assert_eq!(s.fake.sent().len(),1);
    // Maintenance also drains malformed legacy receipts instead of starving behind one.
    let legacy=draft("legacy-terminal","Legacy text",NOW);
    s.engine.shared.store().prepare_delivery("legacy-terminal",&legacy,"{","fixture@example.test",NOW).unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE deliveries SET state='complete',storage_hash=NULL WHERE id='legacy-terminal'",[]).unwrap();
    let valid_legacy=moss_core::Draft {id:"valid-legacy".into(),..d.clone()};
    s.engine.shared.store().prepare_delivery("valid-legacy",&valid_legacy,&legacy_outgoing,"fixture@example.test",NOW).unwrap();
    db.execute("UPDATE deliveries SET state='complete',storage_hash=NULL WHERE id='valid-legacy'",[]).unwrap();
    s.engine.tidy().await.unwrap();
    assert_eq!(s.engine.shared.store().delivery_replay_hash("legacy-terminal").unwrap(),Some((None,true)));
    assert!(s.engine.prepare_send("legacy-terminal",&legacy,&OutgoingDraft::from(&legacy)).await.is_err());
    assert!(!s.engine.prepare_send("valid-legacy",&valid_legacy,&OutgoingDraft::from(&d)).await.unwrap());
}

#[test]
fn logical_fingerprint_v1_has_a_fixed_protocol_vector() {
    let d=draft("protocol-vector","Fixed protocol text",NOW);
    assert_eq!(crate::delivery::logical_fingerprint(&d,&OutgoingDraft::from(&d)),"v1:5a05b33b35a138783c0bc90771b6c7d5f7a0643472f2ff9b2036bec8c4ed8e54");
}

#[tokio::test(start_paused = true)]
async fn missing_delivery_identity_cannot_be_submitted_or_reported_as_sent() {
    let s=with_drafts().await;
    let d=draft("kept-draft","Still editable",NOW);
    let d = s.engine.save_draft(&d).unwrap();
    assert_eq!(s.engine.delivery_state("missing-operation").unwrap(),None);
    assert!(matches!(s.engine.submit_send("missing-operation").await,Err(Error::Unavailable(message)) if message=="The delivery operation is missing."));
    assert_eq!(s.engine.drafts().unwrap(),vec![d]);
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn draft_upload_uses_the_account_sender_when_a_saved_alias_is_no_longer_allowed() {
    let s=with_drafts().await;
    let mut d=draft("removed-alias","Retain my draft",NOW);
    d.from_address=Some("removed@example.test".into());
    s.engine.save_draft(&d).unwrap();
    s.engine.shared.push_drafts("gmail").await.unwrap();
    let texts=s.fake.draft_texts();
    assert_eq!(texts.len(),1);
    assert!(texts[0].contains("From: Maya Ross <maya.ross@gmail.com>"));
    assert!(!texts[0].contains("removed@example.test"));
    assert!(texts[0].contains("Retain my draft"));
}

#[tokio::test(start_paused = true)]
async fn editable_drafts_expose_revisions_and_stale_engine_saves_cannot_overwrite_them() {
    let s=with_drafts().await;
    let original=draft("external-revisions","First text",NOW);
    s.engine.save_draft(&original).unwrap();
    let observed=s.engine.drafts().unwrap().remove(0);
    assert_eq!(serde_json::to_value(&observed).unwrap()["revision"].as_str().map(str::len),Some(32));
    let mut newer=observed.clone();newer.body="Newer edit".into();
    s.engine.save_draft(&newer).unwrap();
    assert!(s.engine.save_draft(&observed).is_err());
    let current=s.engine.drafts().unwrap().remove(0);
    assert_eq!(current.body,"Newer edit");
    assert_ne!(serde_json::to_value(current).unwrap()["revision"],serde_json::to_value(observed).unwrap()["revision"]);
}

async fn stale_handoff_is_refused(scheduled: bool) {
        let s=with_drafts().await;
        let original=draft("stale-handoff","Original",NOW);
        let stale=s.engine.save_draft(&original).unwrap();
        let mut newer=stale.clone();newer.body="Winning edit".into();
        let winner=s.engine.save_draft(&newer).unwrap();
        let email=OutgoingDraft::from(&stale);
        let refused=if scheduled {
            s.engine.schedule_with_operation("stale-operation",&stale,&email,NOW+DAY).is_err()
        } else {
            s.engine.prepare_send("stale-operation",&stale,&email).await.is_err()
        };
        assert!(refused,"stale editor must not transfer ownership");
        assert_eq!(s.engine.drafts().unwrap(),vec![winner]);
        assert_eq!(s.engine.delivery_state("stale-operation").unwrap(),None);
        assert!(s.engine.scheduled().unwrap().is_empty());
        assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn stale_editor_cannot_send_the_winners_draft() { stale_handoff_is_refused(false).await; }

#[tokio::test(start_paused = true)]
async fn stale_editor_cannot_schedule_the_winners_draft() { stale_handoff_is_refused(true).await; }

#[test]
fn draft_revision_conflicts_have_a_distinct_recovery_kind() {
    let error=Error::from(moss_store::Error::DraftConflict);
    assert_eq!(serde_json::to_value(error).unwrap()["kind"],"draftConflict");
}

#[tokio::test(start_paused = true)]
async fn undo_returns_the_restored_editable_revision_and_replays_the_current_row() {
    let s=with_drafts().await;
    let original=draft("restored-editor","Before Undo",NOW);
    s.engine.prepare_send("restored-hold",&original,&OutgoingDraft::from(&original)).await.unwrap();
    let outcome=s.engine.cancel_send_restoring("restored-hold").unwrap();
    assert!(outcome.cancelled);
    let mut restored=outcome.restored.unwrap();
    assert_eq!(restored.revision.as_ref().map(String::len),Some(32));
    restored.body="Edited after Undo".into();
    let saved=s.engine.save_draft(&restored).unwrap();
    let replay=s.engine.cancel_send_restoring("restored-hold").unwrap();
    assert!(replay.cancelled);
    assert_eq!(replay.restored,Some(saved));
}

#[tokio::test(start_paused = true)]
async fn undo_reports_early_cancellation_separately_from_a_claimed_delivery() {
    let s=with_drafts().await;
    let early=s.engine.cancel_send_restoring("early-editor").unwrap();
    assert!(early.cancelled);
    assert!(early.restored.is_none());
    assert_eq!(serde_json::to_value(early).unwrap()["early"],true);
    let d=draft("claimed-editor","Claimed",NOW);
    s.engine.prepare_send("claimed-editor-op",&d,&OutgoingDraft::from(&d)).await.unwrap();
    s.engine.shared.store().claim_delivery("claimed-editor-op").unwrap();
    let claimed=s.engine.cancel_send_restoring("claimed-editor-op").unwrap();
    assert!(!claimed.cancelled);
    assert!(claimed.restored.is_none());
    assert_eq!(serde_json::to_value(claimed).unwrap()["early"],false);
}

#[tokio::test(start_paused = true)]
async fn completed_delivery_replay_resolves_legacy_merged_receipts_without_resending() {
    let s=with_drafts().await;
    let source=moss_core::MessageSource {account_id:"gmail".into(),mailbox:"INBOX".into(),provider_id:"receipt-earlier".into(),..Default::default()};
    let earlier=s.engine.shared.store().ingest(&raw("receipt-earlier@example.test","Earlier conversation",NOW),&source).unwrap();
    let d=draft("legacy-receipt-draft","Exactly one send",NOW);
    s.engine.prepare_send("legacy-receipt",&d,&OutgoingDraft::from(&d)).await.unwrap();
    let old=s.engine.submit_send("legacy-receipt").await.unwrap();
    assert_ne!(old,earlier.thread_id);
    let message_id=s.engine.shared.store().delivery("legacy-receipt").unwrap().unwrap().message_id;
    let linked=[format!("References: <receipt-earlier@example.test> {message_id}\r\n").into_bytes(),raw("receipt-link@example.test","Re: Earlier conversation",NOW)].concat();
    let merged=s.engine.shared.store().ingest(&linked,&moss_core::MessageSource {provider_id:"receipt-link".into(),..source}).unwrap();
    assert_eq!(merged.thread_id,earlier.thread_id);
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE deliveries SET thread_id=?1 WHERE id='legacy-receipt'",[old]).unwrap();
    assert_eq!(s.engine.repair_send("legacy-receipt").unwrap(),earlier.thread_id);
    assert_eq!(s.engine.submit_send("legacy-receipt").await.unwrap(),earlier.thread_id);
    db.execute("DELETE FROM messages WHERE account_id='gmail' AND message_id=trim(?1,'<>')",[&message_id]).unwrap();
    assert!(matches!(s.engine.repair_send("legacy-receipt"),Err(Error::DeliveryAccepted(_))));
    s.engine.shared.store().ingest(&raw(message_id.trim_matches(['<','>']),"An unrelated inbound message",NOW),&moss_core::MessageSource {account_id:"gmail".into(),mailbox:"INBOX".into(),provider_id:"receipt-replacement".into(),..Default::default()}).unwrap();
    assert!(matches!(s.engine.repair_send("legacy-receipt"),Err(Error::DeliveryAccepted(_))));
    assert_eq!(s.fake.sent().len(),1);
}

#[tokio::test(start_paused = true)]
async fn background_local_repair_finishes_accepted_mail_without_resending() {
    let s=with_drafts().await;
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    for id in ["a-damaged","b-repair"] {
        let d=draft(id,"Already sent",NOW);
        s.engine.prepare_send(id,&d,&OutgoingDraft::from(&d)).await.unwrap();
        assert!(matches!(s.engine.submit_send(id).await,Err(Error::DeliveryAccepted(_))));
    }
    db.execute_batch("DROP TRIGGER refuse_sent; UPDATE deliveries SET outgoing='unreadable' WHERE id='a-damaged'").unwrap();
    s.engine.send_due().await;
    assert_eq!(s.engine.delivery_state("b-repair").unwrap(),Some(moss_store::DeliveryState::Complete));
    assert_eq!(s.engine.delivery_state("a-damaged").unwrap(),Some(moss_store::DeliveryState::Accepted));
    assert_eq!(db.query_row("SELECT repair_next FROM deliveries WHERE id='a-damaged'",[],|r|r.get::<_,i64>(0)).unwrap(),NOW+30_000);
    s.engine.send_due().await;
    assert_eq!(db.query_row("SELECT repair_attempt FROM deliveries WHERE id='a-damaged'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(s.fake.sent().len(),2,"local retry must not submit mail");
    db.execute_batch("DROP INDEX delivery_repair_due; ALTER TABLE deliveries DROP COLUMN repair_next").unwrap();
    s.engine.send_due().await;
    assert_eq!(s.fake.sent().len(),2,"failed retry bookkeeping cannot submit mail");
}

#[tokio::test(start_paused = true)]
async fn startup_recovery_does_not_depend_on_accepted_repair() {
    for damage in ["reservation","payload"] {
        let s=with_drafts().await;
        let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        let d=draft("accepted-startup","Already accepted",NOW);
        s.engine.prepare_send("accepted-startup",&d,&OutgoingDraft::from(&d)).await.unwrap();
        db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
        assert!(matches!(s.engine.submit_send("accepted-startup").await,Err(Error::DeliveryAccepted(_))));
        db.execute_batch("DROP TRIGGER refuse_sent").unwrap();
        if damage=="reservation" {
            db.execute_batch("CREATE TRIGGER refuse_repair BEFORE UPDATE OF repair_next ON deliveries BEGIN SELECT RAISE(IGNORE); END").unwrap();
        } else {
            db.execute_batch("UPDATE deliveries SET outgoing='unreadable' WHERE id='accepted-startup'").unwrap();
        }
        *s.engine.shared.delivery_recovered.lock().unwrap()=false;
        s.engine.recover_deliveries().unwrap();
        assert!(*s.engine.shared.delivery_recovered.lock().unwrap());
        assert_eq!(s.engine.delivery_state("accepted-startup").unwrap(),Some(moss_store::DeliveryState::Accepted));
        let fresh=draft("fresh-after-recovery","Fresh text",NOW);
        s.engine.prepare_send("fresh-after-recovery",&fresh,&OutgoingDraft::from(&fresh)).await.unwrap();
        s.engine.recover_deliveries().unwrap();
        assert_eq!(s.engine.delivery_state("fresh-after-recovery").unwrap(),Some(moss_store::DeliveryState::Held));
        s.engine.submit_send("fresh-after-recovery").await.unwrap();
        assert_eq!(s.fake.sent().len(),2);
    }
}

#[tokio::test(start_paused = true)]
async fn recovery_isolates_damaged_held_and_claimed_payloads_without_losing_them() {
    let s=with_drafts().await;
    for id in ["damaged-held","damaged-claimed","healthy-held"] {
        let d=draft(id,"Preserve this text",NOW);
        s.engine.prepare_send(id,&d,&OutgoingDraft::from(&d)).await.unwrap();
    }
    s.engine.shared.store().claim_delivery("damaged-claimed").unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("UPDATE deliveries SET draft='unreadable' WHERE id LIKE 'damaged-%'").unwrap();
    *s.engine.shared.delivery_recovered.lock().unwrap()=false;
    s.engine.recover_deliveries().unwrap();
    for id in ["damaged-held","damaged-claimed"] {
        assert_eq!(s.engine.delivery_state(id).unwrap(),Some(moss_store::DeliveryState::Uncertain));
        assert_eq!(db.query_row("SELECT draft FROM deliveries WHERE id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),"unreadable");
        assert_eq!(db.query_row("SELECT interrupted_from FROM deliveries WHERE id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),if id=="damaged-held" {"held"} else {"claimed"});
        let error=s.engine.submit_send(id).await.unwrap_err();
        if id=="damaged-held" { assert!(matches!(error,Error::Invalid(message) if message.contains("cancelled"))); }
        else { assert!(matches!(error,Error::DeliveryUnknown(_))); }
    }
    assert!(s.engine.drafts().unwrap().iter().any(|d|d.id=="healthy-held" && d.body=="Preserve this text"));
    let fresh=draft("fresh-after-damage","New mail",NOW);
    s.engine.prepare_send("fresh-after-damage",&fresh,&OutgoingDraft::from(&fresh)).await.unwrap();
    s.engine.submit_send("fresh-after-damage").await.unwrap();
    assert_eq!(s.fake.sent().len(),1);
}

#[tokio::test(start_paused = true)]
async fn recovery_bookkeeping_failure_does_not_silently_delay_scheduled_mail() {
    let s=with_drafts().await;
    let d=draft("accepted-needs-repair","Already sent",NOW);
    s.engine.prepare_send("accepted-needs-repair",&d,&OutgoingDraft::from(&d)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert!(matches!(s.engine.submit_send("accepted-needs-repair").await,Err(Error::DeliveryAccepted(_))));
    db.execute_batch("DROP TRIGGER refuse_sent; CREATE TRIGGER refuse_repair BEFORE UPDATE OF repair_next ON deliveries BEGIN SELECT RAISE(IGNORE); END").unwrap();
    let due=draft("scheduled-independent","On time",NOW);
    s.engine.schedule(&due,&OutgoingDraft::from(&due),NOW).unwrap();
    s.engine.send_due().await;
    assert!(s.engine.scheduled().unwrap().is_empty());
    assert_eq!(s.engine.delivery_state("accepted-needs-repair").unwrap(),Some(moss_store::DeliveryState::Accepted));
    assert_eq!(s.fake.sent().len(),2);
    assert!(s.events.all().iter().any(|e|matches!(e,Event::ScheduledSent {email,..} if email.draft.id==due.id)));
}

#[tokio::test(start_paused = true)]
async fn ownership_recovery_retries_transient_restore_errors_without_freezing_healthy_text() {
    let s=with_drafts().await;
    let d=draft("restore-temporary","Definitely unsent text",NOW);
    s.engine.prepare_send("restore-temporary",&d,&OutgoingDraft::from(&d)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT,'temporary fixture'); END").unwrap();
    *s.engine.shared.delivery_recovered.lock().unwrap()=false;
    assert!(s.engine.recover_deliveries().is_err());
    assert_eq!(s.engine.delivery_state("restore-temporary").unwrap(),Some(moss_store::DeliveryState::Held));
    assert_eq!(s.engine.delivery_receipt(&d.id).unwrap().unwrap().id,"restore-temporary");
    assert!(!*s.engine.shared.delivery_recovered.lock().unwrap());
    db.execute_batch("DROP TRIGGER refuse_restore").unwrap();
    s.engine.recover_deliveries().unwrap();
    assert!(s.engine.drafts().unwrap().iter().any(|saved|saved.id==d.id && saved.body==d.body));
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn ownership_recovery_wait_is_visible_without_counting_a_send_attempt() {
    for persist_reason in [true,false] {
        let s=with_drafts().await;
        let held=draft("blocked-recovery","Preserve",NOW);
        s.engine.prepare_send("blocked-recovery",&held,&OutgoingDraft::from(&held)).await.unwrap();
        s.engine.shared.store().claim_delivery("blocked-recovery").unwrap();
        let due=draft("waiting-schedule","Waiting",NOW);
        s.engine.schedule(&due,&OutgoingDraft::from(&due),NOW).unwrap();
        let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_attention BEFORE UPDATE OF state ON deliveries WHEN NEW.state='uncertain' BEGIN SELECT RAISE(IGNORE); END").unwrap();
        if !persist_reason {
            db.execute_batch("CREATE TRIGGER refuse_wait BEFORE UPDATE OF error ON scheduled BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
        }
        *s.engine.shared.delivery_recovered.lock().unwrap()=false;
        assert!(s.engine.prepare_send("new-refused",&draft("new-refused","New",NOW),&OutgoingDraft::from(&due)).await.is_err());
        s.engine.send_due().await;
        let waiting=s.engine.scheduled().unwrap().remove(0);
        assert_eq!(waiting.tries,0);
        assert_eq!(waiting.next_try,NOW);
        assert_eq!(waiting.error.is_some(),persist_reason);
        assert!(s.events.all().iter().any(|e|matches!(e,Event::ScheduledWaiting {email} if email.draft.id==due.id && email.tries==0 && email.error.as_deref().is_some_and(|why|why.contains("waiting for delivery recovery")))));
        assert!(s.fake.sent().is_empty());
        s.events.clear();
        s.engine.send_due().await;
        assert!(!s.events.all().iter().any(|e|matches!(e,Event::ScheduledWaiting {..})),"even failed reason writes must not flood the UI");
        db.execute_batch("DROP TRIGGER IF EXISTS refuse_wait; DROP TRIGGER refuse_attention").unwrap();
        s.engine.send_due().await;
        assert!(s.engine.scheduled().unwrap().is_empty());
        assert_eq!(s.fake.sent().len(),1);
    }
}

#[tokio::test(start_paused = true)]
async fn a_stale_startup_hold_snapshot_does_not_restore_or_notify_twice() {
    let s=with_drafts().await;
    let d=draft("already-restored","Once",NOW);
    s.engine.prepare_send("already-restored",&d,&OutgoingDraft::from(&d)).await.unwrap();
    s.engine.cancel_send("already-restored").unwrap();
    s.events.clear();
    s.engine.restore_interrupted_hold("already-restored","gmail".into()).unwrap();
    assert!(s.events.all().is_empty());
    assert_eq!(s.engine.drafts().unwrap().iter().filter(|row|row.id==d.id).count(),1);
}

#[tokio::test]
async fn fake_provider_generates_missing_ids_and_preserves_explicit_ids() {
    let fake=FakeProvider::new();
    let d=draft("fixture-id","Fixture",NOW);
    let email=OutgoingDraft::from(&d);
    let from=Person {name:"Sender".into(),email:"sender@example.test".into()};
    let mut message=crate::compose::outgoing(&email,&from,None,None,chrono::FixedOffset::east_opt(0).unwrap());
    let generated=moss_providers::Provider::send(fake.as_ref(),&message).await.unwrap();
    assert_eq!(generated.message_id,"<sent-1@moss.test>");
    assert!(String::from_utf8(generated.raw).unwrap().contains("Message-ID: <sent-1@moss.test>\r\n"));
    message.message_id=Some("<explicit@example.test>".into());
    let explicit=moss_providers::Provider::send(fake.as_ref(),&message).await.unwrap();
    assert_eq!(explicit.message_id,"<explicit@example.test>");
    assert!(String::from_utf8(explicit.raw).unwrap().contains("Message-ID: <explicit@example.test>\r\n"));
}

#[tokio::test(start_paused = true)]
async fn failed_undo_keeps_no_send_intent_and_can_restore_on_retry() {
    let s=with_drafts().await;
    let original=draft("failed-undo-editor","Retain this body",NOW);
    s.engine.prepare_send("failed-undo-op",&original,&OutgoingDraft::from(&original)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_undo_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    let outcome=s.engine.cancel_send_restoring("failed-undo-op").unwrap();
    assert!(outcome.cancelled);
    assert_eq!(serde_json::to_value(outcome).unwrap()["restoreFailed"],true);
    assert!(s.engine.submit_send("failed-undo-op").await.is_err());
    assert!(s.fake.sent().is_empty());
    assert_eq!(s.engine.delivery_state("failed-undo-op").unwrap(),Some(moss_store::DeliveryState::Held));
    db.execute_batch("DROP TRIGGER refuse_undo_restore").unwrap();
    let retry=s.engine.cancel_send_restoring("failed-undo-op").unwrap();
    assert!(retry.cancelled);
    assert_eq!(retry.restored.unwrap().body,"Retain this body");
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn undo_distinguishes_unreadable_held_mail_from_already_restored_read_errors() {
    for restored_first in [false, true] {
        let s=with_drafts().await;
        let original=draft("damaged-undo","Keep original bytes",NOW);
        s.engine.prepare_send("damaged-undo-op",&original,&OutgoingDraft::from(&original)).await.unwrap();
        let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        if restored_first {
            assert!(s.engine.cancel_send_restoring("damaged-undo-op").unwrap().restored.is_some());
            db.execute("UPDATE drafts SET to_json='{' WHERE id='damaged-undo'",[]).unwrap();
            assert!(s.engine.cancel_send_restoring("damaged-undo-op").is_err(), "a broken read of an already-restored draft is not failed restoration");
        } else {
            db.execute("UPDATE deliveries SET draft='{' WHERE id='damaged-undo-op'",[]).unwrap();
            for _ in 0..2 {
                let outcome=s.engine.cancel_send_restoring("damaged-undo-op").unwrap();
                assert!(outcome.cancelled);
                assert_eq!(serde_json::to_value(outcome).unwrap()["restoreFailed"],true);
                assert_eq!(s.engine.delivery_state("damaged-undo-op").unwrap(),Some(moss_store::DeliveryState::Uncertain));
                assert!(!s.engine.shared.store().claim_delivery("damaged-undo-op").unwrap());
            }
            let evidence:(String,String)=db.query_row("SELECT draft,interrupted_from FROM deliveries WHERE id='damaged-undo-op'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(evidence,("{".into(),"held".into()));
        }
        assert!(s.fake.sent().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn reviewed_accepted_delivery_never_recreates_a_local_sent_copy() {
    let s=with_drafts().await;let original=draft("reviewed-accepted","Already sent",NOW);
    s.engine.prepare_send("reviewed-accepted-op",&original,&OutgoingDraft::from(&original)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("reviewed-accepted-op").await,Err(Error::DeliveryAccepted(_))));
    s.engine.shared.store().resolve_delivery("reviewed-accepted-op",moss_store::DeliveryResolution::Reviewed,NOW).unwrap();
    db.execute_batch("DROP TRIGGER refuse_sent").unwrap();
    let before:i64=db.query_row("SELECT count(*) FROM messages",[],|row|row.get(0)).unwrap();
    assert!(matches!(s.engine.repair_send("reviewed-accepted-op"),Err(Error::DeliveryAccepted(_))));
    assert!(matches!(s.engine.submit_send("reviewed-accepted-op").await,Err(Error::DeliveryAccepted(_))));
    assert_eq!(db.query_row("SELECT count(*) FROM messages",[],|row|row.get::<_,i64>(0)).unwrap(),before);
    assert_eq!(s.fake.sent().len(),1);
    assert_eq!(s.engine.delivery_state("reviewed-accepted-op").unwrap(),Some(moss_store::DeliveryState::Accepted));
}

#[tokio::test(start_paused = true)]
async fn undo_replay_of_reviewed_unsent_mail_never_reopens_or_calls_it_sent() {
    let s=with_drafts().await;let original=draft("reviewed-unsent","Never submitted",NOW);
    s.engine.prepare_send("reviewed-unsent-op",&original,&OutgoingDraft::from(&original)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert!(s.engine.cancel_send_restoring("reviewed-unsent-op").unwrap().restore_failed);
    s.engine.shared.store().resolve_delivery("reviewed-unsent-op",moss_store::DeliveryResolution::Reviewed,NOW).unwrap();
    db.execute_batch("DROP TRIGGER refuse_restore").unwrap();
    let outcome=s.engine.cancel_send_restoring("reviewed-unsent-op").unwrap();
    assert!(outcome.cancelled);
    assert!(!outcome.restore_failed);
    assert!(outcome.restored.is_none());
    assert!(s.engine.send_cancelled("reviewed-unsent-op").unwrap());
    assert!(matches!(s.engine.submit_send("reviewed-unsent-op").await,Err(Error::Invalid(message)) if message.contains("cancelled")));
    assert!(matches!(s.engine.repair_send("reviewed-unsent-op"),Err(Error::Invalid(message)) if message.contains("cancelled")));
    assert!(s.fake.sent().is_empty());
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn ignored_local_receipt_completion_remains_accepted_and_repairable() {
    let s=with_drafts().await;let original=draft("ignored-receipt","Accepted once",NOW);
    s.engine.prepare_send("ignored-receipt-op",&original,&OutgoingDraft::from(&original)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER ignore_receipt BEFORE UPDATE OF state ON deliveries WHEN NEW.state='complete' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    s.events.clear();
    assert!(matches!(s.engine.submit_send("ignored-receipt-op").await,Err(Error::DeliveryAccepted(_))));
    assert!(s.events.all().contains(&Event::MailChanged { account_id:"gmail".into() }));
    assert_eq!(s.engine.delivery_state("ignored-receipt-op").unwrap(),Some(moss_store::DeliveryState::Accepted));
    db.execute_batch("DROP TRIGGER ignore_receipt").unwrap();
    let receipt=s.engine.repair_send("ignored-receipt-op").unwrap();
    assert_eq!(s.engine.repair_send("ignored-receipt-op").unwrap(),receipt);
    assert_eq!(s.fake.sent().len(),1);
}

#[tokio::test(start_paused = true)]
async fn accepted_repair_needs_the_provider_receipt_but_not_the_editor_snapshot() {
    let s=with_drafts().await;let d=draft("damaged-editor","Accepted once",NOW);
    s.engine.prepare_send("damaged-editor-op",&d,&OutgoingDraft::from(&d)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_sent BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(matches!(s.engine.submit_send("damaged-editor-op").await,Err(Error::DeliveryAccepted(_))));
    db.execute_batch("DROP TRIGGER refuse_sent; UPDATE deliveries SET draft='{' WHERE id='damaged-editor-op';").unwrap();
    let receipt=s.engine.repair_send("damaged-editor-op").unwrap();
    assert_eq!(s.engine.repair_send("damaged-editor-op").unwrap(),receipt);
    assert_eq!(s.fake.sent().len(),1);
}

#[tokio::test(start_paused = true)]
async fn undo_receipt_uses_metadata_when_the_old_delivery_payload_is_unreadable() {
    let s=with_drafts().await;let d=draft("cancelled-editor","Restored text",NOW);
    s.engine.prepare_send("cancelled-editor-op",&d,&OutgoingDraft::from(&d)).await.unwrap();
    assert!(s.engine.cancel_send_restoring("cancelled-editor-op").unwrap().restored.is_some());
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("UPDATE deliveries SET draft='{',outgoing='{' WHERE id='cancelled-editor-op';").unwrap();
    let outcome=s.engine.cancel_send_restoring("cancelled-editor-op").unwrap();
    assert!(outcome.cancelled);assert!(!outcome.restore_failed);
    assert_eq!(outcome.restored.unwrap().body,d.body);
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn delivery_review_remains_available_when_startup_restoration_fails() {
    let s=with_drafts().await;let d=draft("blocked-review","Keep this text",NOW);
    s.engine.prepare_send("blocked-review-op",&d,&OutgoingDraft::from(&d)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(s.engine.cancel_send_restoring("blocked-review-op").unwrap().restore_failed);
    *s.engine.shared.delivery_recovered.lock().unwrap()=false;
    assert!(s.engine.recover_deliveries().is_err());
    let page=s.engine.delivery_attention(None).unwrap();
    assert_eq!(page.entries.len(),1);assert_eq!(page.entries[0].id,"blocked-review-op");
    assert_eq!(s.engine.delivery_attention_draft("blocked-review-op").unwrap().unwrap().body,d.body);
    s.events.clear();
    assert!(matches!(s.engine.resolve_delivery("blocked-review-op",moss_store::DeliveryResolution::Copied).unwrap(),moss_store::DeliveryResolutionOutcome::Resolved {recovery_id:Some(_)}));
    assert!(s.events.all().contains(&Event::ScheduledChanged));
    assert!(s.engine.delivery_attention(None).unwrap().entries.is_empty());
    assert!(s.engine.delivery_attention_draft("blocked-review-op").unwrap().is_none());
    s.engine.recover_deliveries().unwrap();
    assert!(s.fake.sent().is_empty());
}

#[tokio::test(start_paused = true)]
async fn delivery_review_waits_for_the_matching_scheduled_worker() {
    let s=with_drafts().await;let d=draft("scheduled-review","Keep this text",NOW);
    s.engine.prepare_send("scheduled-review-op",&d,&OutgoingDraft::from(&d)).await.unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("UPDATE deliveries SET state='uncertain' WHERE id='scheduled-review-op'").unwrap();
    s.engine.shared.scheduled_going.lock().unwrap().insert(d.id.clone());
    assert_eq!(s.engine.resolve_delivery("scheduled-review-op",moss_store::DeliveryResolution::Reviewed).unwrap(),moss_store::DeliveryResolutionOutcome::Sending);
    s.engine.shared.scheduled_going.lock().unwrap().remove(&d.id);
    assert_eq!(s.engine.resolve_delivery("scheduled-review-op",moss_store::DeliveryResolution::Reviewed).unwrap(),moss_store::DeliveryResolutionOutcome::Resolved {recovery_id:None});
    assert!(s.engine.resolve_delivery("missing-review",moss_store::DeliveryResolution::Reviewed).is_err());
}

#[tokio::test(start_paused = true)]
async fn one_failed_restore_does_not_hide_later_interrupted_deliveries() {
    let s=with_drafts().await;
    for id in ["a-broken","z-claimed","z-healthy"] {
        let d=draft(id,"Retained text",NOW);
        s.engine.prepare_send(id,&d,&OutgoingDraft::from(&d)).await.unwrap();
    }
    s.engine.shared.store().claim_delivery("z-claimed").unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_one_restore BEFORE INSERT ON drafts WHEN NEW.id='a-broken' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    *s.engine.shared.delivery_recovered.lock().unwrap()=false;
    assert!(s.engine.recover_deliveries().is_err());
    assert_eq!(s.engine.delivery_state("z-claimed").unwrap(),Some(moss_store::DeliveryState::Uncertain));
    assert_eq!(s.engine.delivery_state("z-healthy").unwrap(),Some(moss_store::DeliveryState::Cancelled));
    let entries=s.engine.delivery_attention(None).unwrap().entries;
    assert_eq!(entries.iter().map(|e|e.id.as_str()).collect::<Vec<_>>(),["a-broken","z-claimed"]);
    assert!(s.fake.sent().is_empty());
    s.engine.resolve_delivery("a-broken",moss_store::DeliveryResolution::Copied).unwrap();
    s.engine.recover_deliveries().unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_local_recovery_the_store_turns_down_says_so_by_its_own_kind() {
    let s = setup();
    let err = s.engine.keep_local_draft_recovery("", None, &draft("d1", "Hi", NOW)).unwrap_err();
    assert!(matches!(err, Error::LocalRecoveryRejected(_)), "{err:?}");
    assert_eq!(serde_json::to_value(&err).unwrap()["kind"], "localRecoveryRejected");
}
