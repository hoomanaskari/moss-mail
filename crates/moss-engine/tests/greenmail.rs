//! End to end against a real IMAP/SMTP server on this machine (GreenMail), so it is opt-in:
//!
//! ```bash
//! MOSS_E2E_CA=/path/to/ca.der cargo test -p moss-engine --test greenmail -- --ignored
//! ```
//!
//! The server needs a TLS certificate for 127.0.0.1 (the DER file above) and these users,
//! each with password "secret":
//!
//! ```bash
//! java -Dgreenmail.setup.test.all -Dgreenmail.hostname=127.0.0.1 \
//!   -Dgreenmail.users=maya:secret@moss.test,daniel:secret@moss.test,delete:secret@moss.test,important:secret@moss.test,sendlater:secret@moss.test,drafts:secret@moss.test \
//!   -Dgreenmail.users.login=email \
//!   -Dgreenmail.tls.keystore.file=gm.p12 -Dgreenmail.tls.keystore.password=changeit \
//!   -jar greenmail-standalone-2.1.15.jar
//! ```
//!
//! Then put their mail on it, once per server start: `python3 -I tests/greenmail_seed.py ca.der`.
//! Each test has its own mailbox (the seed script lists whose is whose), and Daniel only
//! writes and gets mail with subjects no other test uses, so the tests run together, in
//! parallel or one at a time, in any order. Run them again on a fresh, seeded server.
//!
//! Another server already on those ports (another checkout's)? Start yours on others, e.g.
//! `-Dgreenmail.imaps.hostname=127.0.0.1 -Dgreenmail.imaps.port=33993
//! -Dgreenmail.smtps.hostname=127.0.0.1 -Dgreenmail.smtps.port=33465` in place of
//! `-Dgreenmail.setup.test.all`, and set `MOSS_E2E_PORT_OFFSET=30000` for the seed and the tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use moss_accounts::{
    Accounts, MailService, MemorySecretStore, PasswordSignIn, Security, Server, ServerSettings,
};
use moss_core::{ComposeMode, Draft, Person};
use moss_engine::{Engine, Event, Events, NetConnector, OutgoingDraft, Parts};
use moss_providers::{
    Address, Credentials, ImapConfig, ImapProvider, OutgoingMessage, Provider, RawMessage, Secret,
    SmtpConfig, TlsRoots,
};

/// GreenMail's port, moved by `MOSS_E2E_PORT_OFFSET` so several servers can run at once.
fn port(base: u16) -> u16 {
    base + std::env::var("MOSS_E2E_PORT_OFFSET")
        .map_or(0, |v| v.parse().expect("MOSS_E2E_PORT_OFFSET"))
}

#[derive(Default)]
struct Recorded(Mutex<Vec<Event>>);

impl Events for Recorded {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

async fn until(what: &str, events: &Recorded, mut ready: impl FnMut() -> bool) {
    for _ in 0..600 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}; errors: {:?}", sync_errors(events));
}

/// The seed script's attachment bytes.
fn pattern(n: usize, step: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * step + 7) % 256) as u8).collect()
}

/// A new email from account `id` to no one yet.
fn new_draft_for(id: &str) -> OutgoingDraft {
    OutgoingDraft {
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: id.into(),
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: String::new(),
        body: String::new(),
        body_html: String::new(),
        signature: vec![],
        from_address: None,
        attachments: vec![],
        remind_if_no_reply: false,
    }
}

fn sync_errors(events: &Recorded) -> Vec<Event> {
    events.0.lock().unwrap().iter().filter(|e| matches!(e, Event::SyncError { .. })).cloned().collect()
}

/// An engine on a fresh database, signed in to GreenMail as `email`.
async fn signed_in(email: &str, dir: &std::path::Path) -> (Engine, Arc<Recorded>, String) {
    let ca = std::fs::read(std::env::var("MOSS_E2E_CA").expect("MOSS_E2E_CA")).unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let events = Arc::new(Recorded::default());
    let engine = Engine::new(Parts {
        store: moss_store::Store::open(dir.join(format!("{email}.db"))).unwrap(),
        secrets: secrets.clone(),
        accounts: Accounts::new(secrets),
        connector: Arc::new(NetConnector { roots: TlsRoots::Custom(vec![ca]) }),
        events: events.clone(),
        clock: now,
    });
    let servers = ServerSettings {
        imap: Server::new("127.0.0.1", port(3993), Security::Tls, email),
        smtp: Server::new("127.0.0.1", port(3465), Security::Tls, email),
    };
    let sign_in = PasswordSignIn {
        service: MailService::Imap,
        email: email.into(),
        password: "secret".into(),
        servers: Some(servers),
        login: None,
    };
    let id = engine.add_password_account(sign_in).await.unwrap().account.id;
    (engine, events, id)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local GreenMail server"]
async fn send_later_goes_at_its_time() {
    let dir = tempfile::tempdir().unwrap();
    let (sam, events, id) = signed_in("sendlater@moss.test", dir.path()).await;
    sam.start().unwrap();
    let subject = format!("Send later {}", now());
    let draft = moss_core::Draft {
        revision: None,
        id: "later-1".into(),
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: id.clone(),
        to: vec![Person { name: "Daniel Okafor".into(), email: "daniel@moss.test".into() }],
        cc: vec![],
        bcc: vec![],
        subject: subject.clone(),
        body: "Sent while you slept.".into(),
        body_html: "<p>Sent while you slept.</p>".into(),
        remind_if_no_reply: false,
        updated_at: now(),
        signature_id: None,
        signature_chosen: false,
        from_address: None,
        attachments: vec![],
    };
    let email = OutgoingDraft {
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: id,
        to: draft.to.clone(),
        cc: vec![],
        bcc: vec![],
        subject: subject.clone(),
        body: draft.body.clone(),
        body_html: draft.body_html.clone(),
        signature: vec![],
        from_address: None,
        remind_if_no_reply: false,
        attachments: vec![],
    };
    let at = now() + 3_000;
    sam.schedule(&draft, &email, at).unwrap();
    assert_eq!(sam.scheduled().unwrap().len(), 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(sam.scheduled().unwrap().len(), 1, "not before its time");

    let went = || events.0.lock().unwrap().iter().any(|e| matches!(e, Event::ScheduledSent { late: false, .. }));
    until("the email set to send later", &events, went).await;
    assert!(now() >= at, "never early");
    assert!(sam.scheduled().unwrap().is_empty());

    // It really went: Daniel has it.
    let (daniel, daniel_events, _) = signed_in("daniel@moss.test", dir.path()).await;
    let arrived = || daniel.threads(None).unwrap().iter().any(|t| t.subject == subject);
    until("Daniel to get it", &daniel_events, arrived).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local GreenMail server"]
async fn sign_in_sync_done_reply_and_send() {
    let ca = std::fs::read(std::env::var("MOSS_E2E_CA").expect("MOSS_E2E_CA")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let events = Arc::new(Recorded::default());
    let engine = Engine::new(Parts {
        store: moss_store::Store::open(dir.path().join("moss.db")).unwrap(),
        secrets: secrets.clone(),
        accounts: Accounts::new(secrets),
        connector: Arc::new(NetConnector { roots: TlsRoots::Custom(vec![ca]) }),
        events: events.clone(),
        clock: now,
    });

    let servers_for = |login: &str| ServerSettings {
        imap: Server::new("127.0.0.1", port(3993), Security::Tls, login),
        smtp: Server::new("127.0.0.1", port(3465), Security::Tls, login),
    };
    let servers = servers_for("maya@moss.test");
    let wrong = PasswordSignIn {
        service: MailService::Imap,
        email: "maya@moss.test".into(),
        password: "nope".into(),
        servers: Some(servers.clone()),
        login: None,
    };
    let err = engine.add_password_account(wrong).await.unwrap_err();
    assert_eq!(serde_json::to_value(&err).unwrap()["kind"], "wrongPassword", "{err}");
    assert!(engine.accounts().unwrap().is_empty());

    let right = PasswordSignIn {
        service: MailService::Imap,
        email: "maya@moss.test".into(),
        password: "secret".into(),
        servers: Some(servers),
        login: None,
    };
    let info = engine.add_password_account(right).await.unwrap();
    let id = info.account.id.clone();
    println!("signed in as {id}");

    let inbox = [
        "Q4 design budget",
        "Venue options for the offsite",
        "The weekly design digest",
        "Your receipt from Framely",
        "Q4 deck",
    ];
    let synced = || {
        let threads = engine.threads(None).unwrap();
        inbox.iter().all(|s| threads.iter().any(|t| t.subject == *s))
    };
    until("the first sync", &events, synced).await;
    let threads = engine.threads(None).unwrap();
    for t in threads.iter().filter(|t| inbox.contains(&t.subject.as_str())) {
        println!("{:?} / {:?} — {}", t.split, t.section, t.subject);
    }
    let budget = threads.iter().find(|t| t.subject == "Q4 design budget").unwrap().clone();
    assert_eq!(budget.split, moss_core::SplitId::Important);
    let digest = threads.iter().find(|t| t.subject == "The weekly design digest").unwrap();
    assert_eq!(digest.split, moss_core::SplitId::Newsletters);
    let receipt = threads.iter().find(|t| t.subject == "Your receipt from Framely").unwrap();
    assert_eq!(receipt.split, moss_core::SplitId::Receipts);
    let html = engine.thread(&digest.id).unwrap();
    assert!(html.messages[0].body_html.as_deref().unwrap().contains("Design digest"));
    let progress = engine.sync_progress(&id).unwrap();
    println!("progress: {progress:?}");

    // A big message: the PDF and logo stay on the server until it's opened; then the logo
    // is fetched, kept and drawn.
    let deck = threads.iter().find(|t| t.subject == "Q4 deck").unwrap();
    let message = engine.thread(&deck.id).unwrap().messages[0].clone();
    assert!(message.images_missing);
    assert!(engine.load_message(&message.id).await.unwrap());
    let message = engine.thread(&deck.id).unwrap().messages[0].clone();
    let html = message.body_html.as_deref().unwrap();
    assert!(html.contains("src=\"data:image/png;base64,") && !html.contains("cid:"), "{html}");
    let pdf = message.attachments.iter().find(|a| a.name == "Q4.pdf").unwrap();
    // Worked out from its base64 size on the server, so close rather than exact.
    assert!(pdf.bytes.abs_diff(300_000) < 3_000, "{}", pdf.bytes);
    assert_eq!(engine.attachment(&message.id, pdf.index).await.unwrap(), pattern(300_000, 31));

    // Sent past the newest 1,000 comes headers only, and its body comes when opened.
    let db = rusqlite::Connection::open_with_flags(
        dir.path().join("moss.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    let headers_only = "SELECT COUNT(*) FROM messages WHERE headers_only = 1";
    until("older Sent mail", &events, || count(headers_only) == 3).await;
    assert_eq!(count("SELECT COUNT(*) FROM messages WHERE subject LIKE 'Sent note %'"), 1003);
    assert_eq!(count("SELECT COUNT(*) FROM message_raw"), 0);
    let (oldest, oldest_thread): (i64, i64) = db
        .query_row("SELECT id, thread_id FROM messages WHERE subject = 'Sent note 0'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert!(engine.thread(&oldest_thread.to_string()).unwrap().messages[0].headers_only);
    assert!(engine.load_message(&oldest.to_string()).await.unwrap());
    let opened = engine.thread(&oldest_thread.to_string()).unwrap().messages[0].clone();
    assert!(!opened.headers_only);
    assert_eq!(opened.body_text.trim(), "Note number 0.");

    // Reply, which threads and files a copy in Sent.
    let draft = OutgoingDraft {
        mode: ComposeMode::Reply,
        thread_id: Some(budget.id.clone()),
        from_account_id: id.clone(),
        to: vec![Person { name: "Daniel Okafor".into(), email: "daniel@moss.test".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Re: Q4 design budget".into(),
        body: "Approved. Thanks!".into(),
        body_html: "<p>Approved. Thanks!</p>".into(),
        signature: vec!["Maya Ross".into()],
        from_address: None,
        remind_if_no_reply: false,
        attachments: vec![],
    };
    let thread_id = engine.send(draft).await.unwrap();
    assert_eq!(thread_id, budget.id);
    let messages = engine.thread(&budget.id).unwrap().messages;
    assert_eq!(messages.len(), 2);
    assert!(messages[1].body_text.contains("-- \nMaya Ross"), "{}", messages[1].body_text);

    // Done archives on the server: the next sync must not bring it back.
    engine.mark_read(&budget.id, true).unwrap();
    engine.mark_done(&budget.id).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    engine.sync_now(None);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!engine.threads(None).unwrap().iter().any(|t| t.id == budget.id));
    let done = engine.done_threads(None, 10).unwrap();
    assert!(done.iter().any(|t| t.id == budget.id && t.done_at.is_some()));

    // Not done puts it back in the server's Inbox, so the next sync keeps it here.
    engine.mark_not_done(&budget.id).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    engine.sync_now(None);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(engine.threads(None).unwrap().iter().any(|t| t.id == budget.id));
    assert!(!engine.done_threads(None, 10).unwrap().iter().any(|t| t.id == budget.id));

    // Search over the local index.
    assert_eq!(engine.search("venues", None, 10).unwrap().len(), 1);

    // Files: one attached from disk, and a forward that keeps the deck's PDF. Both go out
    // as multipart MIME, and the copies in Sent carry them.
    let daniel = Person { name: "Daniel Okafor".into(), email: "daniel@moss.test".into() };
    let plan = dir.path().join("Plan.pdf");
    std::fs::write(&plan, pattern(200_000, 7)).unwrap();
    let files = engine.attach_files("e2e-new", &[plan]).unwrap();
    let new = OutgoingDraft {
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: id.clone(),
        to: vec![daniel.clone()],
        cc: vec![],
        bcc: vec![],
        subject: "The plan".into(),
        body: "Attached.".into(),
        body_html: "<p>Attached.</p>".into(),
        signature: vec![],
        from_address: None,
        attachments: files,
        remind_if_no_reply: false,
    };
    let plan_thread = engine.send(new).await.unwrap();
    let copy = engine.thread(&plan_thread).unwrap().messages[0].clone();
    let file = copy.attachments.iter().find(|a| a.name == "Plan.pdf").expect("the Sent copy has the file");
    assert_eq!(file.content_type, "application/pdf");
    assert_eq!(engine.attachment(&copy.id, file.index).await.unwrap(), pattern(200_000, 7));

    let forwarded = moss_core::DraftFile {
        id: format!("{}:{}", message.id, pdf.index),
        name: pdf.name.clone(),
        bytes: pdf.bytes,
        content_type: pdf.content_type.clone(),
        forwarded: Some(moss_core::FileRef { message_id: message.id.clone(), index: pdf.index }),
    };
    let forward = OutgoingDraft {
        mode: ComposeMode::Forward,
        thread_id: Some(deck.id.clone()),
        subject: "Fwd: Q4 deck".into(),
        body: "Here it is.".into(),
        body_html: "<p>Here it is.</p>".into(),
        attachments: vec![forwarded],
        ..OutgoingDraft { to: vec![daniel], ..new_draft_for(&id) }
    };
    let forward_thread = engine.send(forward).await.unwrap();
    let copy = engine.thread(&forward_thread).unwrap().messages[0].clone();
    let file = copy.attachments.iter().find(|a| a.name == "Q4.pdf").expect("the forward has the PDF");
    assert_eq!(engine.attachment(&copy.id, file.index).await.unwrap(), pattern(300_000, 31));

    // Daniel gets both, with the files in them.
    let theirs = PasswordSignIn {
        service: MailService::Imap,
        email: "daniel@moss.test".into(),
        password: "secret".into(),
        servers: Some(servers_for("daniel@moss.test")),
        login: None,
    };
    let daniel_id = engine.add_password_account(theirs).await.unwrap().account.id;
    let received = |subject: &str| {
        engine.threads(Some(&daniel_id)).unwrap().into_iter().find(|t| t.subject == subject)
    };
    // A conversation's subject leaves out "Fwd:".
    until("Daniel's mail", &events, || received("The plan").is_some() && received("Q4 deck").is_some()).await;
    for (subject, name, data) in [("The plan", "Plan.pdf", pattern(200_000, 7)), ("Q4 deck", "Q4.pdf", pattern(300_000, 31))] {
        let m = engine.thread(&received(subject).unwrap().id).unwrap().messages[0].clone();
        let file = m.attachments.iter().find(|a| a.name == name).unwrap_or_else(|| panic!("{subject}: {:?}", m.attachments));
        assert_eq!(engine.attachment(&m.id, file.index).await.unwrap(), data, "{subject}");
    }

    let errors = sync_errors(&events);
    assert!(errors.is_empty(), "{errors:?}");
}

/// One user's mail as the server has it, folder by folder, read straight from IMAP.
struct OnServer(&'static str);

impl OnServer {
    /// A new connection each time, so nothing it remembers about a folder is stale.
    fn connect(&self) -> ImapProvider {
        client(self.0)
    }

    /// The subjects in `mailbox`.
    async fn subjects(&self, mailbox: &str) -> Vec<String> {
        let imap = self.connect();
        let mut out = Vec::new();
        let mut checkpoint = None;
        loop {
            let batch = imap
                .fetch_since(mailbox, checkpoint.as_ref(), 500)
                .await
                .unwrap();
            for m in &batch.messages {
                let raw = String::from_utf8_lossy(&m.raw);
                let subject = raw.lines().find_map(|l| l.strip_prefix("Subject: "));
                out.extend(subject.map(str::to_string));
            }
            checkpoint = Some(batch.checkpoint);
            if !batch.more {
                return out;
            }
        }
    }

    /// Whether the message with `subject` is starred (`\Flagged`), and its id.
    async fn starred(&self, mailbox: &str, subject: &str) -> (bool, String) {
        let batch = self.connect().fetch_since(mailbox, None, 500).await.unwrap();
        let m = batch
            .messages
            .iter()
            .find(|m| String::from_utf8_lossy(&m.raw).contains(&format!("Subject: {subject}")))
            .unwrap_or_else(|| panic!("{subject} in {mailbox}"));
        (m.flags.flagged, m.provider_id.clone())
    }

    async fn has(&self, mailbox: &str, subject: &str) -> bool {
        self.subjects(mailbox).await.iter().any(|s| s == subject)
    }

    /// Waits until the engine's queued changes have reached the server.
    async fn until(&self, what: &str, mut ready: impl AsyncFnMut(&Self) -> bool) {
        for _ in 0..100 {
            if ready(self).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("timed out waiting for {what} on the server");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local GreenMail server"]
async fn delete_and_undo_from_the_inbox_and_done() {
    const USER: &str = "delete@moss.test";
    let server = OnServer(USER);
    let dir = tempfile::tempdir().unwrap();
    let (engine, events, _) = signed_in(USER, dir.path()).await;

    let (open, done) = ("Delete me from the inbox", "Delete me once done");
    let find = |subject: &str| {
        engine
            .threads(None)
            .unwrap()
            .into_iter()
            .find(|t| t.subject == subject)
    };
    until("the first sync", &events, || {
        find(open).is_some() && find(done).is_some()
    })
    .await;
    let open_id = find(open).unwrap().id;
    let done_id = find(done).unwrap().id;
    let searched = |words: &str| engine.search(words, None, 10).unwrap().len();
    assert_eq!(searched("delete"), 2);

    // Done first: it goes to the Archive.
    engine.mark_done(&done_id).unwrap();
    server
        .until("Done in the Archive", async |s| {
            s.has("Archive", done).await
        })
        .await;

    // Delete both: each goes to Trash from where it was, and leaves every list and search.
    engine.delete_thread(&open_id).unwrap();
    engine.delete_thread(&done_id).unwrap();
    server
        .until("both in Trash", async |s| {
            s.has("Trash", open).await && s.has("Trash", done).await
        })
        .await;
    assert!(!server.has("INBOX", open).await);
    assert!(!server.has("Archive", done).await);
    let listed = |id: &str| {
        engine.threads(None).unwrap().iter().any(|t| t.id == id)
            || engine
                .done_threads(None, 50)
                .unwrap()
                .iter()
                .any(|t| t.id == id)
    };
    // A sync doesn't bring them back.
    engine.sync_now(None);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!listed(&open_id) && !listed(&done_id));
    assert_eq!(searched("delete"), 0);

    // Undo: the inbox one back to the Inbox, the done one back to the Archive only.
    engine.restore_thread(&open_id).unwrap();
    engine.restore_thread(&done_id).unwrap();
    server
        .until("both out of Trash", async |s| {
            let trash = s.subjects("Trash").await;
            !trash.iter().any(|t| t == open || t == done)
        })
        .await;
    assert!(server.has("INBOX", open).await);
    assert!(server.has("Archive", done).await);
    assert!(!server.has("INBOX", done).await);
    assert!(!server.has("Archive", open).await);

    // And here, after the next sync too: in the list and in Done, and found again.
    engine.sync_now(None);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(engine
        .threads(None)
        .unwrap()
        .iter()
        .any(|t| t.id == open_id));
    assert!(engine
        .done_threads(None, 50)
        .unwrap()
        .iter()
        .any(|t| t.id == done_id));
    assert!(!engine
        .threads(None)
        .unwrap()
        .iter()
        .any(|t| t.id == done_id));
    assert_eq!(searched("delete"), 2);

    let errors = sync_errors(&events);
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local GreenMail server"]
async fn moving_to_important_stars_it_on_the_server_and_a_star_from_elsewhere_moves_it() {
    use moss_core::SplitId;
    use moss_providers::Change;

    const USER: &str = "important@moss.test";
    let server = OnServer(USER);
    let dir = tempfile::tempdir().unwrap();
    let (engine, events, _) = signed_in(USER, dir.path()).await;

    let subject = "Your invoice from Linear";
    let find = || engine.threads(None).unwrap().into_iter().find(|t| t.subject == subject);
    until("the first sync", &events, || find().is_some()).await;
    let id = find().unwrap().id;
    assert_eq!(find().unwrap().split, SplitId::Receipts);
    let (starred, uid) = server.starred("INBOX", subject).await;
    assert!(!starred);

    // Moved to Important here: starred on the server.
    engine.move_thread(&id, Some(SplitId::Important)).unwrap();
    assert_eq!(find().unwrap().split, SplitId::Important);
    server.until("the star on the server", async |s| s.starred("INBOX", subject).await.0).await;

    // Out of Important: the star goes.
    engine.move_thread(&id, Some(SplitId::Other)).unwrap();
    server.until("no star on the server", async |s| !s.starred("INBOX", subject).await.0).await;
    // A sync now (Moss's first look at the flags) keeps it where you put it.
    engine.sync_now(None);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(find().unwrap().split, SplitId::Other);

    // Starred on the phone: Important here after the next sync.
    let phone = server.connect();
    phone.apply("INBOX", std::slice::from_ref(&uid), &Change::Flagged(true)).await.unwrap();
    let split = || find().unwrap().split;
    for _ in 0..50 {
        engine.sync_now(None);
        tokio::time::sleep(Duration::from_millis(400)).await;
        if split() == SplitId::Important {
            break;
        }
    }
    assert_eq!(split(), SplitId::Important, "a star from another app moves it to Important");
    assert!(find().unwrap().moved_by_you);

    // Unstarred on the phone: back where you had put it.
    phone.apply("INBOX", std::slice::from_ref(&uid), &Change::Flagged(false)).await.unwrap();
    for _ in 0..50 {
        engine.sync_now(None);
        tokio::time::sleep(Duration::from_millis(400)).await;
        if split() == SplitId::Other {
            break;
        }
    }
    assert_eq!(split(), SplitId::Other);

    let errors = sync_errors(&events);
    assert!(errors.is_empty(), "{errors:?}");
}

// Drafts in the server's Drafts folder, in Dara's mailbox (DRAFTS_USER). Needs no seed: a
// second IMAP client plays Dara's phone, which starts a draft, sees Moss's, and deletes one.

const DRAFTS_USER: &str = "drafts@moss.test";

fn ca() -> Vec<u8> {
    std::fs::read(std::env::var("MOSS_E2E_CA").expect("MOSS_E2E_CA")).unwrap()
}

/// A plain IMAP client signed in as `user`, fresh each time so it never has a stale view.
fn client(user: &str) -> ImapProvider {
    let credentials = || Credentials::Password { username: user.into(), password: Secret::from("secret") };
    ImapProvider::new(
        ImapConfig {
            host: "127.0.0.1".into(),
            port: port(3993),
            security: moss_providers::Security::Tls,
            credentials: credentials(),
            tls_roots: TlsRoots::Custom(vec![ca()]),
        },
        SmtpConfig {
            host: "127.0.0.1".into(),
            port: port(3465),
            security: moss_providers::Security::Tls,
            credentials: credentials(),
            tls_roots: TlsRoots::Custom(vec![ca()]),
        },
    )
}

/// Everything in Dara's Drafts folder, as their phone sees it.
async fn phone_drafts() -> Vec<String> {
    let phone = client(DRAFTS_USER);
    let ids = phone.all_ids("Drafts").await.unwrap().unwrap();
    let messages: Vec<RawMessage> = phone.fetch_messages("Drafts", &ids).await.unwrap();
    messages.iter().map(|m| String::from_utf8_lossy(&m.raw).into_owned()).collect()
}

fn message_id(raw: &str) -> String {
    let line = raw.lines().find(|l| l.to_ascii_lowercase().starts_with("message-id:")).unwrap();
    line[11..].trim().trim_matches(['<', '>']).to_string()
}

/// Waits (syncing now and then) until `ready` says yes.
async fn until_synced<F, Fut>(what: &str, engine: &Engine, events: &Recorded, mut ready: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for i in 0..120 {
        if ready().await {
            return;
        }
        if i % 10 == 9 {
            engine.sync_now(None);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {what}; errors: {:?}", sync_errors(events));
}

fn someone(name: &str, email: &str) -> Person {
    Person { name: name.into(), email: email.into() }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local GreenMail server"]
async fn drafts_go_up_come_in_and_leave_once_sent_or_discarded() {
    let run = now();
    let phone = client(DRAFTS_USER);
    if !phone.list_mailboxes().await.unwrap().iter().any(|m| m.id == "Drafts") {
        phone.create_mailbox("Drafts").await.unwrap();
    }
    // Start clean: whatever an earlier run left in Drafts goes.
    let left: Vec<String> = phone_drafts().await.iter().map(|r| message_id(r)).collect();
    phone.delete_drafts("Drafts", &left).await.unwrap();

    // Daniel writes; Dara starts a reply on their phone.
    let lunch = format!("lunch-{run}@moss.test");
    let subject = format!("Lunch {run}");
    let daniel = client("daniel@moss.test");
    daniel
        .send(&OutgoingMessage {
            from: Address::named("Daniel Okafor", "daniel@moss.test"),
            to: vec![Address::named("Dara Fitz", DRAFTS_USER)],
            subject: subject.clone(),
            text: Some("Friday at 1?".into()),
            message_id: Some(lunch.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let started = OutgoingMessage {
        from: Address::named("Dara Fitz", DRAFTS_USER),
        to: vec![Address::named("Daniel Okafor", "daniel@moss.test")],
        subject: format!("Re: {subject}"),
        text: Some("Yes, Friday works.\n\nOn Friday, Daniel wrote:\n> Friday at 1?".into()),
        in_reply_to: Some(lunch.clone()),
        references: vec![lunch.clone()],
        attachments: vec![moss_providers::Attachment {
            filename: "menu.txt".into(),
            content_type: "text/plain".into(),
            content_id: None,
            data: b"Soup, bread, cake.".to_vec(),
        }],
        ..Default::default()
    };
    let from_phone = phone.save_draft("Drafts", &started).await.unwrap().unwrap().message_id;

    let dir = tempfile::tempdir().unwrap();
    let (engine, events, account) = signed_in(DRAFTS_USER, dir.path()).await;

    // The phone's reply comes in, in its conversation, without its quote.
    let phone_id = format!("server:{from_phone}");
    until_synced("the phone's draft", &engine, &events, || async {
        engine.drafts().unwrap().iter().any(|d| d.id == phone_id && d.thread_id.is_some())
    })
    .await;
    let thread = engine.threads(None).unwrap().into_iter().find(|t| t.subject == subject).unwrap();
    let reply = engine.drafts().unwrap().into_iter().find(|d| d.id == phone_id).unwrap();
    assert_eq!((reply.mode, reply.thread_id.as_deref()), (ComposeMode::Reply, Some(thread.id.as_str())));
    assert_eq!(reply.body, "Yes, Friday works.");
    // Its file comes in with it.
    let names: Vec<_> = reply.attachments.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["menu.txt"]);

    // A draft started in Moss goes up, and each save replaces the copy before.
    let mine = Draft {
        revision: None,
        id: format!("d-{run}"),
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: account.clone(),
        to: vec![someone("Daniel Okafor", "daniel@moss.test")],
        cc: vec![],
        bcc: vec![someone("", DRAFTS_USER)],
        subject: format!("Picnic spots {run}"),
        body: "The meadow or the riverbank?".into(),
        body_html: "<p>The meadow or the riverbank?</p>".into(),
        remind_if_no_reply: false,
        updated_at: now(),
        signature_id: None,
        signature_chosen: false,
        from_address: None,
        attachments: vec![],
    };
    // With a file, which goes up with it.
    let files = tempfile::tempdir().unwrap();
    let map = files.path().join("picnic-map.txt");
    std::fs::write(&map, "Meadow: north gate. Riverbank: by the bridge.").unwrap();
    let mine = Draft {
        revision: None, attachments: engine.attach_files(&mine.id, &[map]).unwrap(), ..mine };
    engine.save_draft(&mine).unwrap();
    let draft_header = format!("X-Moss-Draft: {}", mine.id);
    let copies = |text: &'static str| {
        let header = draft_header.clone();
        async move {
            let all = phone_drafts().await;
            let ours: Vec<_> = all.into_iter().filter(|r| r.contains(&header)).collect();
            ours.len() == 1 && ours[0].contains(text)
        }
    };
    until_synced("Moss's draft on the phone", &engine, &events, || copies("meadow or the riverbank")).await;
    let copy = phone_drafts().await.into_iter().find(|r| r.contains(&draft_header)).unwrap();
    assert!(copy.contains(&format!("Bcc: {DRAFTS_USER}")), "{copy}");
    assert!(copy.contains("picnic-map.txt"), "{copy}");
    let edited = Draft { body: "The riverbank, then.".into(), updated_at: now(), ..mine.clone() };
    let edited = engine.save_draft(&edited).unwrap();
    until_synced("the edit replacing the copy", &engine, &events, || copies("The riverbank, then.")).await;

    // Sent: it goes, and leaves the server's Drafts.
    let outgoing = OutgoingDraft {
        mode: edited.mode,
        thread_id: None,
        from_account_id: account.clone(),
        to: edited.to.clone(),
        cc: vec![],
        bcc: vec![],
        subject: edited.subject.clone(),
        body: edited.body.clone(),
        body_html: edited.body_html.clone(),
        signature: vec![],
        from_address: None,
        attachments: edited.attachments.clone(),
        remind_if_no_reply: false,
    };
    // As the composer does: the draft goes, its files once the email has gone.
    assert!(engine.delete_editable_draft(&edited.id, edited.revision.as_deref(), true).unwrap());
    engine.send(outgoing).await.unwrap();
    until_synced("the sent draft to leave Drafts", &engine, &events, || async {
        !phone_drafts().await.iter().any(|r| r.contains(&draft_header))
    })
    .await;

    // Discarded in Moss: the phone's draft goes too.
    let phone = engine.drafts().unwrap().into_iter().find(|d| d.id == phone_id).unwrap();
    assert!(engine.delete_editable_draft(&phone_id, phone.revision.as_deref(), false).unwrap());
    until_synced("the discarded draft to leave Drafts", &engine, &events, || async {
        !phone_drafts().await.iter().any(|r| message_id(r) == from_phone)
    })
    .await;

    // Deleted on the phone: it goes here too.
    let later = Draft { id: format!("later-{run}"), updated_at: now(), attachments: vec![], ..mine.clone() };
    engine.save_draft(&later).unwrap();
    let later_header = format!("X-Moss-Draft: {}", later.id);
    until_synced("the second draft on the phone", &engine, &events, || async {
        phone_drafts().await.iter().any(|r| r.contains(&later_header))
    })
    .await;
    let on_phone = phone_drafts().await.into_iter().find(|r| r.contains(&later_header)).unwrap();
    client(DRAFTS_USER).delete_drafts("Drafts", &[message_id(&on_phone)]).await.unwrap();
    engine.sync_now(None);
    until_synced("the draft deleted on the phone to go here too", &engine, &events, || async {
        !engine.drafts().unwrap().iter().any(|d| d.id == later.id)
    })
    .await;

    assert!(phone_drafts().await.is_empty());
    let errors = sync_errors(&events);
    assert!(errors.is_empty(), "{errors:?}");
}
