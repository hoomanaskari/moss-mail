//! The engine end to end over the in-memory fake server: sign in, sync, act, send.

use std::collections::HashMap;
use std::sync::Arc;

use moss_accounts::{
    AccountConnection, Accounts, AuthMethod, DnsMx, Endpoints, MailService, MemorySecretStore,
    PasswordSignIn, SecretRef, SecretStore, SecretStoreError,
};
use moss_core::{ComposeMode, Person, Section, SenderStatus, SplitId};
use moss_providers::{Change, FlagUpdate, Flags};
use moss_store::{Offline, DEFAULT_CACHE_LIMIT};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::testing::*;

const NOW: i64 = 1_793_524_320_000;
const DAY: i64 = 86_400_000;

fn clock() -> i64 {
    NOW
}

struct Setup {
    engine: Engine,
    fake: Arc<FakeProvider>,
    connector: Arc<FakeConnector>,
    events: Arc<RecordedEvents>,
    secrets: Arc<MemorySecretStore>,
    _dir: tempfile::TempDir,
}

fn setup_with(
    dir: tempfile::TempDir,
    fake: Arc<FakeProvider>,
    secrets: Arc<MemorySecretStore>,
) -> Setup {
    setup_endpoints(dir, fake, secrets, Endpoints::default())
}

fn setup_endpoints(
    dir: tempfile::TempDir,
    fake: Arc<FakeProvider>,
    secrets: Arc<MemorySecretStore>,
    endpoints: Endpoints,
) -> Setup {
    setup_all(dir, fake, secrets, endpoints, Arc::new(DnsMx::system()), clock)
}

fn setup_all(
    dir: tempfile::TempDir,
    fake: Arc<FakeProvider>,
    secrets: Arc<MemorySecretStore>,
    endpoints: Endpoints,
    mx: Arc<dyn moss_accounts::MxLookup>,
    clock: fn() -> i64,
) -> Setup {
    let connector = Arc::new(FakeConnector::new(fake.clone()));
    let events = Arc::new(RecordedEvents::default());
    let engine = Engine::new(Parts {
        store: Store::open(dir.path().join("moss.db")).unwrap(),
        secrets: secrets.clone(),
        accounts: Accounts::with(secrets.clone(), endpoints, mx),
        connector: connector.clone(),
        events: events.clone(),
        clock,
    });
    Setup { engine, fake, connector, events, secrets, _dir: dir }
}

#[tokio::test(start_paused = true)]
async fn a_sync_worker_that_panics_starts_again_by_itself_after_a_pause() {
    let s = setup();
    s.inbox(1, "a@x", "Q4 budget", NOW);
    s.fake.panic_next(1);
    s.gmail().await;
    s.synced().await;
    assert_eq!(s.fake.panics_left(), 0);
    assert_eq!(s.subjects(), ["Q4 budget"]);
}

#[tokio::test(start_paused = true)]
async fn a_panic_while_sync_held_its_folders_or_progress_does_not_stop_sync_for_good() {
    let s = setup();
    s.inbox(1, "a@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    poison(&s.engine.shared.folders);
    poison(&s.engine.shared.progress);
    assert_eq!(s.engine.sync_progress("gmail").map(|p| p.more), Some(false));
    s.inbox(2, "b@x", "Offsite", NOW);
    s.events.clear();
    s.engine.sync_now(Some("gmail"));
    s.synced().await;
    assert_eq!(s.subjects(), ["Offsite", "Q4 budget"]);
}

#[tokio::test(start_paused = true)]
async fn a_sync_worker_that_keeps_panicking_waits_longer_each_time_until_its_account_goes() {
    let s = setup();
    s.fake.panic_next(100);
    s.gmail().await;
    // Tries at 0 s, then after 5, 10 and 20 s more: 35 s in, the next waits 40 s.
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    assert_eq!(100 - s.fake.panics_left(), 4);
    s.engine.remove_account("gmail").unwrap();
    let left = s.fake.panics_left();
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    assert_eq!(s.fake.panics_left(), left, "a removed account's worker stays stopped");
}

#[tokio::test(start_paused = true)]
async fn a_claimed_queue_action_cannot_be_dropped_while_the_provider_is_running() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    assert!(s.pending().is_empty());
    let old_json = serde_json::to_string(&ServerChange::Seen { mailbox: "INBOX".into(), ids: vec!["1:1".into()], seen: true }).unwrap();
    let new_json = serde_json::to_string(&ServerChange::Seen { mailbox: "INBOX".into(), ids: vec!["1:1".into()], seen: false }).unwrap();
    let old = s.engine.shared.store().queue_change("gmail", &old_json, NOW).unwrap();
    let started = s.fake.applies_started();
    s.fake.slow(std::time::Duration::from_secs(1));
    let sending = s.engine.shared.send_all("gmail");
    tokio::pin!(sending);
    let pending = std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(sending.as_mut(), cx).is_pending())).await;
    assert!(pending);
    assert_eq!(s.fake.applies_started(), started + 1, "the old network operation has started");
    assert!(s.engine.shared.store().drop_change(old).is_err());
    let new = s.engine.shared.store().queue_change("gmail", &new_json, NOW).unwrap();
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    sending.await.unwrap();
    assert_ne!(old, new);
    assert!(s.pending().is_empty());
    s.engine.shared.send_all("gmail").await.unwrap();
    assert!(s.pending().is_empty());
    assert_eq!(s.fake.changes().last().unwrap(), "Seen(false) INBOX 1:1");
}

/// Compare content snapshots while external editor reads also carry issued identities.
fn draft_contents(drafts: impl IntoIterator<Item = moss_core::Draft>) -> Vec<moss_core::Draft> {
    drafts.into_iter().map(|mut draft| { draft.revision = None; draft }).collect()
}

fn setup() -> Setup {
    setup_with(
        tempfile::tempdir().unwrap(),
        FakeProvider::new(),
        Arc::new(MemorySecretStore::new()),
    )
}

fn gmail_sign_in(password: &str) -> PasswordSignIn {
    PasswordSignIn {
        service: MailService::Gmail,
        email: "maya.ross@gmail.com".into(),
        password: password.into(),
        servers: None,
        login: None,
    }
}

impl Setup {
    async fn gmail(&self) -> AccountInfo {
        self.engine.add_password_account(gmail_sign_in("abcd efgh ijkl mnop")).await.unwrap()
    }

    /// Waits until the account's first sync pass has finished.
    async fn synced(&self) {
        self.events.wait_for(|e| matches!(e, Event::SyncProgress(p) if !p.more)).await;
        // Sent and the inbox check come after the last inbox batch.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    fn inbox(&self, uid: u32, id: &str, subject: &str, at: i64) {
        self.fake.add(message("INBOX", uid, raw(id, subject, at), at));
    }

    fn subjects(&self) -> Vec<String> {
        self.engine.threads(None).unwrap().into_iter().map(|t| t.subject).collect()
    }

    /// Changes still waiting to reach the server.
    fn pending(&self) -> Vec<(moss_store::ChangeId, String)> {
        self.engine.shared.store().pending_changes("gmail").unwrap()
    }

    fn thread_id(&self, subject: &str) -> String {
        self.engine.threads(None).unwrap().into_iter().find(|t| t.subject == subject).unwrap().id
    }
}

mod scheduled;

async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[tokio::test(start_paused = true)]
async fn a_gmail_app_password_signs_in_saves_the_secret_and_syncs() {
    let s = setup();
    s.inbox(2, "b@x", "Newest", NOW - DAY);
    s.inbox(1, "a@x", "Older", NOW - 2 * DAY);
    let info = s.gmail().await;
    assert_eq!(info.account.id, "gmail");
    assert_eq!(info.account.name, "Gmail");
    assert_eq!(info.account.sender_name, "Maya Ross");
    assert_eq!(
        info.connection,
        Some(ConnectionInfo {
            provider: MailService::Gmail,
            summary: "Gmail · App password".into()
        })
    );
    // The app password is stored without spaces, and only in the keychain.
    let secret = s.secrets.load(&s.engine.shared.connection("gmail").unwrap().secret).unwrap();
    assert_eq!(
        secret,
        Some(moss_accounts::Secret::Password { password: "abcdefghijklmnop".into() })
    );
    assert_eq!(
        s.connector.last_credential(),
        Some(Credential::Password("abcdefghijklmnop".into()))
    );

    s.synced().await;
    assert_eq!(s.subjects(), ["Newest", "Older"]);
    let progress = s.engine.sync_progress("gmail").unwrap();
    assert_eq!(
        (progress.done, progress.folders, progress.more, progress.recent),
        (2, 4, false, 1.0)
    );
    assert!(s.events.all().contains(&Event::MailChanged { account_id: "gmail".into() }));
    assert_eq!(s.engine.split_counts(None).unwrap().important, 2);
    assert_eq!(s.engine.sync_progress("nobody"), None);
}

#[tokio::test(start_paused = true)]
async fn a_refused_password_saves_nothing() {
    let s = setup();
    s.fake.fail_next(moss_providers::Error::Auth(
        "[ALERT] Application-specific password required".into(),
    ));
    let err = s.engine.add_password_account(gmail_sign_in("abcdefghijklmnop")).await.unwrap_err();
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json["kind"], "gmailAppPassword");
    assert_eq!(json["problem"], "appPasswordRequired");
    assert!(s.engine.accounts().unwrap().is_empty());
    let err = s.engine.add_password_account(gmail_sign_in("hunter2 password")).await.unwrap_err();
    assert_eq!(serde_json::to_value(&err).unwrap()["problem"], "notAnAppPassword");
}

#[tokio::test(start_paused = true)]
async fn signing_in_again_keeps_the_account_and_its_settings() {
    let s = setup();
    s.gmail().await;
    s.engine
        .update_account("gmail", AccountPatch { name: Some("Home".into()), ..Default::default() })
        .unwrap();
    let again = s.gmail().await;
    assert_eq!((again.account.id.as_str(), again.account.name.as_str()), ("gmail", "Home"));
    assert_eq!(s.engine.accounts().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_first_sync_brings_recent_mail_first_then_keeps_going() {
    let s = setup();
    let total = 1300;
    for i in 0..total {
        // Newest first, an hour apart.
        let at = NOW - (i as i64) * 3_600_000;
        s.inbox(total - i, &format!("m{i}@x"), &format!("Mail {i}"), at);
    }
    s.gmail().await;
    let first = s.events.wait_for(|e| matches!(e, Event::SyncProgress(p) if p.done == 200)).await;
    let Event::SyncProgress(first) = first else { unreachable!() };
    assert!(first.more);
    assert!(first.recent > 0.2 && first.recent < 0.3, "{}", first.recent);
    s.synced().await;
    assert_eq!(s.engine.threads(None).unwrap().len(), total as usize);
    let last = s.engine.sync_progress("gmail").unwrap();
    assert_eq!((last.done, last.recent, last.oldest_year), (total, 1.0, Some(2026)));
    // Gmail history waits until the whole Inbox is in (here over two passes).
    let calls = s.fake.calls();
    let inbox_done = calls.iter().position(|c| c == "fetch INBOX done").unwrap();
    let first_lookup = calls.iter().position(|c| c.starts_with("conversation")).unwrap();
    assert!(calls[..inbox_done].iter().filter(|c| *c == "fetch INBOX more").count() > 5);
    assert!(first_lookup > inbox_done, "the Inbox comes first: {calls:?}");
}

#[tokio::test(start_paused = true)]
async fn sync_resumes_from_its_checkpoint_after_a_restart() {
    let fake = FakeProvider::new();
    let secrets = Arc::new(MemorySecretStore::new());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let s = setup_with(dir, fake.clone(), secrets.clone());
    s.inbox(1, "a@x", "First", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    s.engine.shared.stop_worker("gmail");
    let Setup { _dir: dir, .. } = s;
    assert_eq!(dir.path(), path);

    fake.add(message("INBOX", 2, raw("b@x", "While closed", NOW), NOW));
    let again = setup_with(dir, fake.clone(), secrets);
    let before = fake.fetches();
    again.engine.start().unwrap();
    again.synced().await;
    assert_eq!(again.subjects(), ["While closed", "First"]);
    // The inbox picked up where it stopped: one fetch for it and one for Sent.
    assert_eq!(fake.fetches() - before, 2);
}

#[tokio::test(start_paused = true)]
async fn new_mail_arrives_through_idle() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    s.events.clear();
    s.fake.deliver(message("INBOX", 5, raw("new@x", "Just in", NOW), NOW));
    s.events.wait_for(|e| matches!(e, Event::MailChanged { .. })).await;
    assert_eq!(s.subjects(), ["Just in"]);
}

#[tokio::test(start_paused = true)]
async fn mail_the_server_never_announces_still_arrives_within_five_minutes() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    // IDLE stays quiet: Gmail didn't say, or the connection died without anyone noticing.
    s.fake.add(message("INBOX", 5, raw("quiet@x", "Unannounced", NOW), NOW));
    tokio::time::sleep(std::time::Duration::from_secs(4 * 60)).await;
    assert!(s.subjects().is_empty());
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(s.subjects(), ["Unannounced"]);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_connection_is_retried_with_backoff() {
    let s = setup();
    s.inbox(1, "a@x", "Hello", NOW);
    s.gmail().await;
    // The worker hasn't run yet: its first call to the server fails.
    s.fake.fail_next(moss_providers::Error::Network("reset".into()));
    let err = s.events.wait_for(|e| matches!(e, Event::SyncError { .. })).await;
    assert!(matches!(err, Event::SyncError { problem: Problem::Offline, .. }));
    s.events.wait_for(|e| matches!(e, Event::SyncOk { .. })).await;
    assert_eq!(s.subjects(), ["Hello"]);

    // IDLE failing is retried too.
    s.events.clear();
    s.fake.fail_next_wait(moss_providers::Error::Network("idle dropped".into()));
    s.engine.sync_now(Some("gmail"));
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::Offline, .. })).await;
    s.fake.deliver(message("INBOX", 2, raw("b@x", "After", NOW), NOW));
    s.events.wait_for(|e| matches!(e, Event::MailChanged { .. })).await;
    assert_eq!(s.subjects(), ["After", "Hello"]);
}

#[tokio::test(start_paused = true)]
async fn a_refused_password_during_sync_waits_for_the_user() {
    let s = setup();
    s.inbox(1, "a@x", "Hello", NOW);
    s.gmail().await;
    s.synced().await;
    s.events.clear();
    s.fake.fail_next(moss_providers::Error::Auth("invalid credentials".into()));
    s.engine.sync_now(None);
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::SignedOut, .. })).await;
    // A window that opens later still learns why the account isn't syncing.
    let problems = s.engine.sync_problems();
    assert_eq!(problems.len(), 1);
    assert_eq!((problems[0].account_id.as_str(), problems[0].problem), ("gmail", Problem::SignedOut));
    assert!(!problems[0].message.is_empty());
    // It doesn't retry by itself, so a wrong password can't lock the account.
    let fetches = s.fake.fetches();
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    assert_eq!(s.fake.fetches(), fetches);
    let built = s.connector.built();
    s.engine.sync_now(Some("gmail"));
    s.events.wait_for(|e| matches!(e, Event::SyncOk { .. })).await;
    assert_eq!(s.connector.built(), built + 1, "the provider is built again with the saved secret");
    assert_eq!(s.engine.sync_problems(), [], "syncing again clears it");
}

#[tokio::test(start_paused = true)]
async fn problems_with_several_accounts_are_listed_in_account_order() {
    let s = setup();
    s.engine.shared.report("work", &Error::Offline("no route".into()));
    s.engine.shared.report("gmail", &Error::SignedOut("gmail".into()));
    let problems = s.engine.sync_problems();
    let ids: Vec<&str> = problems.iter().map(|p| p.account_id.as_str()).collect();
    assert_eq!(ids, ["gmail", "work"]);
}

#[tokio::test(start_paused = true)]
async fn changing_an_address_of_an_account_that_has_gone_says_the_account_is_unknown() {
    let s = setup();
    let err = s.engine.block_address("nobody", "a@example.com", false).unwrap_err();
    assert!(matches!(err, Error::UnknownAccount(ref id) if id == "nobody"), "{err:?}");
}

#[tokio::test(start_paused = true)]
async fn done_archives_on_the_server_and_read_marks_it_seen() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");

    s.engine.mark_read(&id, true).unwrap();
    settle().await;
    assert!(!s.engine.thread(&id).unwrap().summary.unread);
    assert!(s.fake.applied().contains(&("INBOX".into(), vec!["1:1".into()], Change::Seen(true))));

    s.engine.mark_done(&id).unwrap();
    assert!(s.subjects().is_empty());
    let done = s.engine.done_threads(None, 10).unwrap();
    assert_eq!((done[0].id.as_str(), done[0].done_at), (id.as_str(), Some(NOW)));
    settle().await;
    assert!(s.fake.applied().contains(&("INBOX".into(), vec!["1:1".into()], Change::Archive)));
    assert!(s.engine.shared.store().provider_ids("gmail", "INBOX").unwrap().is_empty());

    // Not done finds it in the archive by Message-ID and puts it back in the Inbox.
    s.engine.mark_not_done(&id).unwrap();
    assert_eq!(s.subjects(), ["Budget"]);
    assert!(s.engine.done_threads(None, 10).unwrap().is_empty());
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "unarchive a@x");
    assert_eq!(s.subjects(), ["Budget"]);
}

#[tokio::test(start_paused = true)]
async fn trash_lists_deleted_mail_and_back_to_inbox_moves_it_there_even_from_done() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    s.engine.mark_done(&id).unwrap();
    s.engine.delete_thread(&id).unwrap();
    let trash = s.engine.trash_threads(None, 10).unwrap();
    assert_eq!(trash.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
    assert!(s.engine.trash_threads(Some("other"), 10).unwrap().is_empty());
    settle().await;
    let before = s.fake.changes().len();

    s.engine.restore_to_inbox(&id).unwrap();
    assert!(s.engine.trash_threads(None, 10).unwrap().is_empty());
    assert!(s.engine.done_threads(None, 10).unwrap().is_empty());
    assert_eq!(s.subjects(), ["Budget"]);
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["untrash a@x to inbox"]);
    assert!(s.engine.restore_to_inbox("99").is_err());
}

#[tokio::test(start_paused = true)]
async fn delete_moves_to_trash_on_the_server_and_undo_brings_it_back() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    let before = s.fake.changes().len();

    s.engine.delete_thread(&id).unwrap();
    assert!(s.subjects().is_empty());
    assert!(s.engine.done_threads(None, 10).unwrap().is_empty());
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["trash a@x"]);

    s.engine.restore_thread(&id).unwrap();
    assert_eq!(s.subjects(), ["Budget"]);
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["trash a@x", "untrash a@x to inbox"]);

    // Deleted from the Done list: undo puts it back there.
    s.engine.mark_done(&id).unwrap();
    s.engine.delete_thread(&id).unwrap();
    assert!(s.engine.done_threads(None, 10).unwrap().is_empty());
    s.engine.restore_thread(&id).unwrap();
    let done = s.engine.done_threads(None, 10).unwrap();
    assert_eq!(done.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "untrash a@x to archive");

    // Nothing deleted, so nothing to undo.
    let sent = s.fake.changes().len();
    s.engine.restore_thread(&id).unwrap();
    settle().await;
    assert_eq!(s.fake.changes().len(), sent);
}

#[tokio::test(start_paused = true)]
async fn undoing_a_delete_keeps_the_email_open_when_the_server_reports_its_old_place_gone() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");

    s.engine.delete_thread(&id).unwrap();
    settle().await;
    s.engine.restore_thread(&id).unwrap();
    settle().await;
    // The server moved it to Trash and back: the Inbox has it under a new UID, and the next
    // sync says the old one went.
    s.fake.vanish("1:1");
    s.inbox(2, "a@x", "Budget", NOW);
    s.engine.sync_now(Some("gmail"));
    settle().await;

    assert_eq!(s.subjects(), ["Budget"]);
    assert!(s.engine.done_threads(None, 10).unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn undo_right_after_done_reaches_the_server_after_the_archive() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    let before = s.fake.changes().len();
    s.fake.slow(std::time::Duration::from_secs(1));
    s.engine.mark_done(&id).unwrap();
    s.engine.mark_not_done(&id).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert_eq!(s.fake.changes()[before..], ["Archive INBOX 1:1", "unarchive a@x"]);
    // The archive finishing late doesn't make the thread done again.
    assert_eq!(s.subjects(), ["Budget"]);
    assert!(s.engine.done_threads(Some("gmail"), 10).unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_server_failure_keeps_the_local_change_and_says_so() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    s.events.clear();
    s.fake.fail_next(moss_providers::Error::Protocol("NO [CANNOT] nope".into()));
    s.engine.mark_done(&id).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::Server, .. })).await;
    assert!(s.subjects().is_empty());
    // A refusal waits for you to try it again or discard it.
    assert_eq!(s.pending().len(), 1);
    let blocked = s.engine.server_change_status("gmail").unwrap().unwrap();
    assert_eq!(blocked.failure, Some(moss_store::ChangeFailure::Refused));
    s.engine.discard_server_change("gmail", &blocked.change, blocked.revision).unwrap();

    // With no provider at all (the secret is gone), the action still works here.
    s.engine.shared.forget_provider("gmail");
    s.secrets.delete(&s.engine.shared.connection("gmail").unwrap().secret).unwrap();
    s.engine.mark_not_done(&id).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::SignedOut, .. })).await;
    assert_eq!(s.subjects(), ["Budget"]);
    s.events.clear();
    s.engine.mark_read(&id, false).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::SignedOut, .. })).await;

    // Both wait until the account is signed in again, then go in the order they were made.
    assert_eq!(s.pending().len(), 2);
    let before = s.fake.changes().len();
    let secret = moss_accounts::Secret::Password { password: "abcdefghijklmnop".into() };
    s.secrets.save(&s.engine.shared.connection("gmail").unwrap().secret, &secret).unwrap();
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["unarchive a@x", "Seen(false) INBOX 1:1"]);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_change_without_a_definite_answer_goes_back_and_reaches_the_server_once_on_the_next_sync() {
    for error in [
        moss_providers::Error::Network("offline".into()),
        moss_providers::Error::RateLimited { retry_after: std::time::Duration::from_secs(1) },
        moss_providers::Error::Auth("token expired".into()),
    ] {
        let s = setup();
        s.inbox(1, "a@x", "Budget", NOW);
        s.gmail().await;
        s.synced().await;
        let id = s.thread_id("Budget");
        s.events.clear();
        s.fake.fail_next(error);
        s.engine.mark_done(&id).unwrap();
        s.events.wait_for(|e| matches!(e, Event::SyncError { .. })).await;
        let archived = || s.fake.applied().iter().filter(|(_, _, c)| *c == Change::Archive).count();
        assert_eq!(archived(), 0);
        assert_eq!(s.pending().len(), 1);
        let head = s.engine.shared.store().earliest_change("gmail").unwrap().unwrap();
        assert_eq!((head.state, head.failure), (moss_store::ChangeState::Queued, None));
        assert!(s.subjects().is_empty());

        s.engine.sync_now(Some("gmail"));
        settle().await;
        assert_eq!(archived(), 1);
        assert!(s.pending().is_empty());
        s.engine.sync_now(Some("gmail"));
        settle().await;
        assert_eq!(archived(), 1, "never twice");
    }
}

#[tokio::test(start_paused = true)]
async fn changes_waiting_for_the_server_keep_their_order() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    let before = s.fake.changes().len();
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    s.engine.mark_done(&id).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::Offline, .. })).await;
    // Undo waits behind the archive rather than overtaking it.
    s.fake.fail_next(moss_providers::Error::Network("still offline".into()));
    s.engine.mark_not_done(&id).unwrap();
    settle().await;
    assert!(s.fake.changes()[before..].is_empty());
    assert_eq!(s.pending().len(), 2);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["Archive INBOX 1:1", "unarchive a@x"]);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_refusal_for_mail_the_server_no_longer_has_drops_the_change_and_later_ones_go() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.inbox(2, "b@x", "Lunch", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let (budget, lunch) = (s.thread_id("Budget"), s.thread_id("Lunch"));
    let before = s.fake.changes().len();
    // Gone from the Inbox in another app, so the server says no.
    s.fake.set_all_ids("INBOX", &["1:2"]);
    s.fake.fail_next(moss_providers::Error::Protocol("NO [NONEXISTENT] no such message".into()));
    s.engine.mark_done(&budget).unwrap();
    s.engine.mark_read(&lunch, true).unwrap();
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["Seen(true) INBOX 1:2"]);
    assert!(s.pending().is_empty());
    assert_eq!(s.engine.server_change_status("gmail").unwrap(), None);
    assert!(s.engine.done_threads(None, 10).unwrap().iter().any(|t| t.id == budget), "still done here");
}

#[tokio::test(start_paused = true)]
async fn a_mailbox_reset_on_the_server_drops_the_change_and_later_ones_go() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.inbox(2, "b@x", "Lunch", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let (budget, lunch) = (s.thread_id("Budget"), s.thread_id("Lunch"));
    let before = s.fake.changes().len();
    s.fake.fail_next(moss_providers::Error::NotFound("INBOX was reset on the server; sync it again".into()));
    s.engine.mark_done(&budget).unwrap();
    s.engine.mark_read(&lunch, true).unwrap();
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["Seen(true) INBOX 1:2"]);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_draft_cleanup_that_cannot_find_the_drafts_folder_yet_waits_for_the_next_sync() {
    let s = setup();
    s.fake.with_drafts();
    s.gmail().await;
    s.synced().await;
    let cleanup = serde_json::to_string(&ServerChange::DeleteDrafts { message_ids: vec!["old@fixture".into()] }).unwrap();
    s.engine.shared.store().queue_change("gmail", &cleanup, NOW).unwrap();
    crate::lock(&s.engine.shared.folders).clear();
    s.fake.fail_next(moss_providers::Error::Network("offline".into()));
    assert!(matches!(s.engine.shared.send_all("gmail").await, Err(Error::Offline(_))));
    let head = s.engine.shared.store().earliest_change("gmail").unwrap().unwrap();
    assert_eq!((head.state, head.failure), (moss_store::ChangeState::Queued, None));
    s.engine.shared.send_all("gmail").await.unwrap();
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_broken_database_is_said_as_such_when_you_try_a_change_again() {
    let s = setup();
    s.gmail().await;
    let id = s.engine.shared.store().queue_change("gmail", "refused", NOW).unwrap();
    let revision = s.engine.shared.store().block_change("gmail", id, 0, moss_store::ChangeFailure::Refused).unwrap();
    rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap().execute_batch("ALTER TABLE outbox RENAME TO broken_outbox").unwrap();
    assert!(matches!(s.engine.retry_server_change("gmail", &id.to_hex(), revision), Err(Error::Store(_))));
    assert!(matches!(s.engine.discard_server_change("gmail", &id.to_hex(), revision), Err(Error::Store(_))));
}

#[tokio::test(start_paused = true)]
async fn a_refusal_for_mail_still_on_the_server_waits_and_try_again_runs_it_once() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.inbox(2, "b@x", "Lunch", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let (budget, lunch) = (s.thread_id("Budget"), s.thread_id("Lunch"));
    let before = s.fake.changes().len();
    s.fake.set_all_ids("INBOX", &["1:1", "1:2"]);
    s.fake.fail_next(moss_providers::Error::Protocol("NO [CANNOT] not now".into()));
    s.engine.mark_done(&budget).unwrap();
    s.engine.mark_read(&lunch, true).unwrap();
    settle().await;
    assert!(s.fake.changes()[before..].is_empty(), "later changes wait behind it");
    let status = s.engine.server_change_status("gmail").unwrap().unwrap();
    assert_eq!((status.state, status.failure, status.kind), (moss_store::ChangeState::Blocked, Some(moss_store::ChangeFailure::Refused), moss_store::BatchKind::Done));
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert!(s.fake.changes()[before..].is_empty(), "never tried again by itself");

    assert!(matches!(s.engine.retry_server_change("gmail", &status.change, status.revision + 1), Err(Error::Invalid(_))), "only as you saw it");
    assert!(matches!(s.engine.retry_server_change("gmail", "not a change", status.revision), Err(Error::Invalid(_))));
    assert!(matches!(s.engine.retry_server_change("removed", &status.change, status.revision), Err(Error::UnknownAccount(_))));
    s.events.clear();
    s.engine.retry_server_change("gmail", &status.change, status.revision).unwrap();
    assert!(s.events.all().iter().any(|e| matches!(e, Event::ServerChangesChanged { .. })));
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["Archive INBOX 1:1", "Seen(true) INBOX 1:2"]);
    assert!(s.pending().is_empty());
    assert!(s.engine.retry_server_change("gmail", &status.change, status.revision).is_err(), "once");
}

#[tokio::test(start_paused = true)]
async fn discarding_a_refused_change_stops_it_here_only_and_later_changes_go() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.inbox(2, "b@x", "Lunch", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let (budget, lunch) = (s.thread_id("Budget"), s.thread_id("Lunch"));
    let before = s.fake.changes().len();
    s.fake.fail_next(moss_providers::Error::Unsupported("this account has no Archive folder".into()));
    s.engine.mark_done(&budget).unwrap();
    s.engine.mark_read(&lunch, true).unwrap();
    settle().await;
    let status = s.engine.server_change_status("gmail").unwrap().unwrap();
    assert_eq!(status.failure, Some(moss_store::ChangeFailure::Refused));
    assert!(matches!(s.engine.discard_server_change("gmail", &status.change, status.revision + 1), Err(Error::Invalid(_))));
    assert!(matches!(s.engine.discard_server_change("gmail", "", status.revision), Err(Error::Invalid(_))));
    assert!(matches!(s.engine.discard_server_change("removed", &status.change, status.revision), Err(Error::UnknownAccount(_))));
    s.events.clear();
    s.engine.discard_server_change("gmail", &status.change, status.revision).unwrap();
    assert!(s.events.all().iter().any(|e| matches!(e, Event::ServerChangesChanged { .. })));
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["Seen(true) INBOX 1:2"]);
    assert!(s.pending().is_empty());
    assert!(s.engine.done_threads(None, 10).unwrap().iter().any(|t| t.id == budget), "the change stays on this Mac");
}

#[tokio::test(start_paused = true)]
async fn an_answer_that_may_mean_it_went_is_kept_for_you_to_check() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let before = s.fake.changes().len();
    s.fake.fail_next(moss_providers::Error::DeliveryUnknown("lost reply".into()));
    s.engine.mark_done(&s.thread_id("Budget")).unwrap();
    settle().await;
    assert_eq!(s.engine.server_change_status("gmail").unwrap().unwrap().failure, Some(moss_store::ChangeFailure::Uncertain));
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert!(s.fake.changes()[before..].is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_local_failure_after_the_server_archived_never_archives_again() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let before = s.fake.changes().len();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_forget BEFORE DELETE ON message_locations BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    s.events.clear();
    s.engine.mark_done(&s.thread_id("Budget")).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { .. })).await;
    db.execute_batch("DROP TRIGGER refuse_forget").unwrap();
    assert_eq!(s.fake.changes()[before..], ["Archive INBOX 1:1"]);
    assert!(s.pending().is_empty());
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["Archive INBOX 1:1"]);
}

#[tokio::test(start_paused = true)]
async fn a_change_on_its_way_when_moss_stopped_reaches_the_server_once_after_restart() {
    let fake = FakeProvider::new();
    let secrets = Arc::new(MemorySecretStore::new());
    let s = setup_with(tempfile::tempdir().unwrap(), fake.clone(), secrets.clone());
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    let before = fake.changes().len();
    fake.slow(std::time::Duration::from_secs(10));
    s.engine.mark_done(&id).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // The app quits with the archive still on its way.
    s.engine.shared.stop_worker("gmail");
    for (_, task) in s.engine.shared.on_server.lock().unwrap().drain() {
        task.abort();
    }
    let Setup { _dir: dir, .. } = s;
    fake.slow(std::time::Duration::ZERO);

    let again = setup_with(dir, fake.clone(), secrets);
    assert_eq!(again.engine.shared.store().earliest_change("gmail").unwrap().unwrap().state, moss_store::ChangeState::Claimed);
    again.engine.start().unwrap();
    again.synced().await;
    settle().await;
    assert_eq!(fake.changes()[before..], ["Archive INBOX 1:1"]);
    assert!(again.pending().is_empty());
    assert!(again.subjects().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_conversation_is_archived_in_one_go_and_mail_without_an_id_comes_back_here_only() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW - DAY);
    s.fake.add(message("INBOX", 2, reply_to(&["a@x"], raw("b@x", "Re: Budget", NOW)), NOW));
    let no_id = b"From: Daniel <daniel@northwind.co>\r\nSubject: Lunch\r\n\r\nNoon?\r\n".to_vec();
    s.fake.add(message("INBOX", 3, no_id, NOW - 2 * DAY));
    s.gmail().await;
    s.synced().await;
    let before = s.fake.changes().len();

    s.engine.mark_done(&s.thread_id("Budget")).unwrap();
    let lunch = s.thread_id("Lunch");
    s.engine.mark_done(&lunch).unwrap();
    s.engine.mark_not_done(&lunch).unwrap();
    settle().await;
    // Gmail can only be asked to put mail back by its Message-ID.
    assert_eq!(s.fake.changes()[before..], ["Archive INBOX 1:1,1:2", "Archive INBOX 1:3"]);
    assert_eq!(s.subjects(), ["Lunch"]);
}

#[tokio::test(start_paused = true)]
async fn a_waiting_change_this_moss_cant_read_is_retained() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    s.engine.shared.store().queue_change("gmail", "from a newer Moss", NOW).unwrap();
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert_eq!(s.pending().len(), 1);
    assert_eq!(s.engine.shared.store().earliest_change("gmail").unwrap().unwrap().failure, Some(moss_store::ChangeFailure::Unreadable));
}

#[tokio::test(start_paused = true)]
async fn remind_hides_a_thread_until_later() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    s.engine.snooze(&id, Some(NOW + DAY)).unwrap();
    assert_eq!(s.engine.thread(&id).unwrap().summary.section, Section::Later);
    s.engine.snooze(&id, None).unwrap();
    assert_eq!(s.engine.thread(&id).unwrap().summary.section, Section::Reply);
}

#[tokio::test(start_paused = true)]
async fn showing_images_from_a_sender_is_remembered() {
    let s = setup();
    s.inbox(1, "q@x", "Quote", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Quote");
    assert_eq!(s.engine.thread(&id).unwrap().messages[0].image_trust, moss_core::ImageTrust::Ask);
    s.engine.allow_images("gmail", "Daniel@Northwind.co").unwrap();
    assert_eq!(
        s.engine.thread(&id).unwrap().messages[0].image_trust,
        moss_core::ImageTrust::YouAllowed
    );
}

#[tokio::test(start_paused = true)]
async fn a_sender_you_allowed_shows_images_only_in_mail_gmail_confirmed_is_from_them() {
    let s = setup();
    let gmail_says = |result: &str, raw: Vec<u8>| [format!("Authentication-Results: mx.google.com; {result}\r\n").into_bytes(), raw].concat();
    s.fake.add(message("INBOX", 1, gmail_says("dkim=pass header.d=northwind.co", raw("real@x", "Real", NOW)), NOW));
    let forged = gmail_says("dkim=pass header.d=northwind.co", raw("forged@x", "Forged", NOW));
    s.fake.add(message("INBOX", 2, gmail_says("dmarc=fail header.from=northwind.co", forged), NOW));
    s.gmail().await;
    s.synced().await;
    s.engine.allow_images("gmail", "daniel@northwind.co").unwrap();
    let first = |subject: &str| s.engine.thread(&s.thread_id(subject)).unwrap().messages.remove(0);
    let (real, forged) = (first("Real"), first("Forged"));
    assert_eq!((real.image_trust, real.sender_unverified), (moss_core::ImageTrust::YouAllowed, false));
    assert_eq!((forged.image_trust, forged.sender_unverified), (moss_core::ImageTrust::YouAllowed, true));
}

#[test]
fn remembers_who_you_picked_from_the_suggestions_newest_first() {
    let s = setup();
    s.engine.pick_recipient("ross", "rossmaya@home.example").unwrap();
    s.engine.pick_recipient("ro", "kindle@x.io").unwrap();
    let picks: Vec<_> = s.engine.recipient_picks().unwrap().into_iter().map(|p| (p.typed, p.email)).collect();
    assert_eq!(picks, [("ro".into(), "kindle@x.io".into()), ("ross".to_string(), "rossmaya@home.example".to_string())]);
}

#[tokio::test(start_paused = true)]
async fn signatures_and_each_accounts_choice_are_kept() {
    let s = setup();
    s.gmail().await;
    let sig =
        moss_core::Signature { id: "s1".into(), name: "Work".into(), lines: vec!["Maya".into()] };
    s.engine.save_signature(&sig).unwrap();
    s.engine.save_signature(&moss_core::Signature { id: "s2".into(), ..sig.clone() }).unwrap();
    s.engine.set_signature_choice("gmail", moss_core::SignatureUse::Replies, Some("s1")).unwrap();
    s.engine.set_let_moss_choose(false).unwrap();
    s.engine.mark_signature_used("s2").unwrap();
    s.engine.delete_signature("s1").unwrap();
    let saved = s.engine.signatures().unwrap();
    assert_eq!(saved.signatures.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["s2"]);
    assert_eq!(saved.by_account["gmail"], moss_core::AccountSignatures::default());
    assert!(!saved.let_moss_choose);
    assert_eq!(saved.recent, ["s2"]);
    assert!(matches!(
        s.engine.set_signature_choice("nobody", moss_core::SignatureUse::NewEmails, None),
        Err(Error::Store(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn drafts_are_kept_until_deleted() {
    let s = setup();
    s.gmail().await;
    let draft = moss_core::Draft {
        revision: None,
        id: "d1".into(),
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: "gmail".into(),
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "Lunch?".into(),
        body: "Friday?".into(),
        body_html: "<p>Friday?</p>".into(),
        remind_if_no_reply: false,
        updated_at: NOW,
        signature_id: Some("s1".into()),
        signature_chosen: true,
        from_address: Some("hi@rossstudio.art".into()),
        attachments: vec![],
    };
    let draft = s.engine.save_draft(&draft).unwrap();
    assert_eq!(s.engine.drafts().unwrap()[0].from_address.as_deref(), Some("hi@rossstudio.art"));
    s.engine.save_draft(&moss_core::Draft { body: "Friday at 1?".into(), ..draft.clone() }).unwrap();
    assert_eq!(s.engine.drafts().unwrap()[0].body, "Friday at 1?");
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
    assert!(s.engine.delete_draft("d1", false).unwrap());
    assert!(!s.engine.delete_draft("d1", false).unwrap());
    assert!(s.engine.drafts().unwrap().is_empty());
    let stranger = moss_core::Draft { id: "stranger".into(), revision: None, from_account_id: "nobody".into(), ..draft };
    assert!(matches!(s.engine.save_draft(&stranger), Err(Error::Store(_))));
}

#[tokio::test(start_paused = true)]
async fn mail_archived_or_read_in_another_app_follows_here() {
    let s = setup();
    s.inbox(2, "b@x", "Second", NOW);
    s.inbox(1, "a@x", "First", NOW - DAY);
    s.gmail().await;
    s.synced().await;

    // Gmail can't report removals itself, so the inbox is compared with the server's list.
    s.fake.set_all_ids("INBOX", &["1:2"]);
    s.fake.update_flags(FlagUpdate {
        provider_id: "1:2".into(),
        flags: Flags { seen: true, ..Default::default() },
        labels: vec![],
    });
    s.events.clear();
    s.engine.sync_now(Some("gmail"));
    s.events.wait_for(|e| matches!(e, Event::MailChanged { .. })).await;
    settle().await;
    assert_eq!(s.subjects(), ["Second"]);
    assert!(!s.engine.thread(&s.thread_id("Second")).unwrap().summary.unread);
    let done = s.engine.done_threads(None, 10).unwrap();
    assert_eq!((done[0].subject.as_str(), done[0].done_at), ("First", Some(NOW)));

    // Servers with QRESYNC say what vanished.
    s.fake.vanish("1:2");
    s.engine.sync_now(None);
    settle().await;
    settle().await;
    assert!(s.subjects().is_empty());
    assert_eq!(s.engine.done_threads(None, 10).unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_new_uidvalidity_relinks_messages_without_losing_them() {
    let s = setup();
    s.inbox(1, "a@x", "Kept", NOW);
    s.gmail().await;
    s.synced().await;
    s.fake.reset_next();
    s.engine.sync_now(None);
    settle().await;
    settle().await;
    assert_eq!(s.subjects(), ["Kept"]);
    assert_eq!(s.engine.shared.store().provider_ids("gmail", "INBOX").unwrap(), ["1:1"]);
}

#[tokio::test(start_paused = true)]
async fn something_that_is_not_an_email_is_skipped() {
    let s = setup();
    s.fake.add(message("INBOX", 2, b"\r\n".to_vec(), NOW));
    s.inbox(1, "a@x", "Real", NOW);
    s.gmail().await;
    s.synced().await;
    assert_eq!(s.subjects(), ["Real"]);
}

#[tokio::test(start_paused = true)]
async fn the_sender_name_is_learned_from_sent_mail() {
    let s = setup();
    let sent =
        raw_from("s@x", "Maya R. Ross <maya.ross@gmail.com>", "daniel@northwind.co", "Hi", NOW);
    s.fake.add(message("[Gmail]/Sent Mail", 1, sent, NOW));
    s.gmail().await;
    let event = s.events.wait_for(|e| matches!(e, Event::AccountUpdated { .. })).await;
    let Event::AccountUpdated { account } = event else { unreachable!() };
    assert_eq!(account.account.sender_name, "Maya R. Ross");
    // Sent mail alone doesn't show in the list.
    assert!(s.subjects().is_empty());
    // A guess that was right already stays as it is.
    let same = setup();
    let sent = raw_from("s@x", "Maya Ross <maya.ross@gmail.com>", "daniel@northwind.co", "Hi", NOW);
    same.fake.add(message("[Gmail]/Sent Mail", 1, sent, NOW));
    same.gmail().await;
    same.synced().await;
    assert!(!same.events.all().iter().any(|e| matches!(e, Event::AccountUpdated { .. })));
    // Once learned (or typed in Settings), it's kept.
    s.engine
        .update_account(
            "gmail",
            AccountPatch { sender_name: Some("M".into()), ..Default::default() },
        )
        .unwrap();
    s.engine.sync_now(None);
    settle().await;
    assert_eq!(s.engine.accounts().unwrap()[0].account.sender_name, "M");
}

#[tokio::test(start_paused = true)]
async fn a_server_without_folder_roles_still_syncs_its_inbox() {
    let s = setup();
    s.fake.without_mailboxes();
    s.inbox(1, "a@x", "Hello", NOW);
    s.gmail().await;
    s.synced().await;
    assert_eq!(s.subjects(), ["Hello"]);
    assert_eq!(s.engine.sync_progress("gmail").unwrap().folders, 0);
    // No All Mail folder: no Gmail conversations to ask about.
    assert!(!s.fake.calls().iter().any(|c| c.starts_with("conversation")));
}

/// Forwards the newest message of thread `id` to Rebecca.
fn forward(id: &str) -> OutgoingDraft {
    OutgoingDraft {
        mode: ComposeMode::Forward,
        thread_id: Some(id.into()),
        from_account_id: "gmail".into(),
        to: vec![Person { name: "Rebecca Vale".into(), email: "rebecca@vale.example".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Fwd: Q4 budget".into(),
        body: "For you.".into(),
        body_html: "<p>For you.</p>".into(),
        signature: vec![],
        from_address: None,
        remind_if_no_reply: false,
        attachments: vec![],
    }
}

#[tokio::test(start_paused = true)]
async fn a_forward_is_remembered_on_the_email_it_forwarded_and_marked_on_the_server() {
    let s = setup();
    s.inbox(1, "q4@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Q4 budget");
    let forward_id = s.engine.send(forward(&id)).await.unwrap();
    settle().await;
    // The forward is a conversation of its own; the email it carried says where it went.
    assert_ne!(forward_id, id);
    let m = s.engine.thread(&id).unwrap().messages[0].clone();
    assert_eq!(m.forwards.len(), 1);
    assert_eq!(m.forwards[0].thread_id, forward_id);
    assert_eq!(m.forwards[0].to.iter().map(|p| p.email.as_str()).collect::<Vec<_>>(), ["rebecca@vale.example"]);
    assert!(m.forwarded);
    // Other mail apps see it as forwarded too.
    assert!(s.fake.applied().contains(&("INBOX".into(), vec!["1:1".into()], Change::Forwarded(true))));
    assert!(s.pending().is_empty());
}

/// A new email to Rebecca with these files.
fn with(files: Vec<moss_core::DraftFile>) -> OutgoingDraft {
    OutgoingDraft { mode: ComposeMode::New, thread_id: None, attachments: files, ..forward("") }
}

fn saved_with(id: &str, files: Vec<moss_core::DraftFile>) -> Draft {
    let email = with(files);
    Draft {
        revision: None,
        id:id.into(), mode:email.mode, thread_id:email.thread_id, from_account_id:email.from_account_id,
        to:email.to, cc:email.cc, bcc:email.bcc, subject:email.subject, body:email.body, body_html:email.body_html,
        remind_if_no_reply:email.remind_if_no_reply, updated_at:NOW, signature_id:None,
        signature_chosen:true, from_address:email.from_address, attachments:email.attachments,
    }
}

#[tokio::test(start_paused = true)]
async fn files_you_attach_are_kept_then_sent_with_the_email_and_its_sent_copy() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    let (plan, notes) = (s._dir.path().join("Plan.pdf"), s._dir.path().join("notes"));
    std::fs::write(&plan, b"%PDF-1.7").unwrap();
    std::fs::write(&notes, b"hi").unwrap();
    let files = s.engine.attach_files("d1", &[plan.clone(), notes]).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!((files[0].name.as_str(), files[0].bytes), ("Plan.pdf", 8));
    assert_eq!(files[0].content_type, "application/pdf");
    assert_eq!((files[1].name.as_str(), files[1].content_type.as_str()), ("notes", "application/octet-stream"));
    // A file that isn't there says so, and adds nothing.
    let missing = s.engine.attach_files("d1", &[plan.clone(), s._dir.path().join("gone.pdf")]);
    assert!(matches!(missing, Err(Error::Unavailable(ref why)) if why.contains("gone.pdf")), "{missing:?}");

    s.engine.send_saved(&saved_with("d1", files.clone()), &with(files.clone())).await.unwrap();
    let sent = s.fake.sent();
    let names: Vec<_> = sent[0].attachments.iter().map(|a| (a.filename.as_str(), a.content_type.as_str())).collect();
    assert_eq!(names, [("Plan.pdf", "application/pdf"), ("notes", "application/octet-stream")]);
    assert_eq!(sent[0].attachments[0].data, b"%PDF-1.7");
    assert!(sent[0].attachments.iter().all(|a| a.content_id.is_none()));
    // Gone with the email: the files are no longer kept for it.
    assert_eq!(s.engine.shared.store().draft_file(&files[0].id).unwrap(), None);
    // Sent again, it can't find them, and says so before anything goes.
    let again = s.engine.send(with(files.clone())).await;
    assert!(matches!(again, Err(Error::Unavailable(ref why)) if why.contains("Plan.pdf")), "{again:?}");
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_forward_carries_the_files_of_the_email_it_forwards() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Q4 signed");
    let pdf = moss_core::DraftFile {
        id: format!("{}:1", m.id),
        name: "Q4.pdf".into(),
        bytes: 7,
        content_type: "application/pdf".into(),
        forwarded: Some(moss_core::FileRef { message_id: m.id.clone(), index: 1 }),
    };
    let draft = OutgoingDraft { attachments: vec![pdf.clone()], ..forward(&m.thread_id) };
    s.engine.send(draft).await.unwrap();
    let sent = &s.fake.sent()[0].attachments;
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].filename.as_str(), sent[0].data.as_slice()), ("Q4.pdf", b"PDFDATA".as_slice()));
    assert_eq!(sent[0].content_type, "application/pdf");
}

#[tokio::test(start_paused = true)]
async fn too_many_files_for_the_service_are_refused_before_anything_goes() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    let big = s._dir.path().join("film.mov");
    std::fs::write(&big, vec![0u8; 26 * 1024 * 1024]).unwrap();
    let files = s.engine.attach_files("d1", &[big]).unwrap();
    let refused = s.engine.send_saved(&saved_with("d1", files.clone()), &with(files.clone())).await;
    let Err(Error::TooBig(why)) = refused else { panic!("{refused:?}") };
    assert_eq!(why, "these files are 26 MB, and Gmail sends up to 25 MB");
    assert!(s.fake.sent().is_empty());
    // Still kept, so you can take it off and send the rest.
    assert!(s.engine.shared.store().draft_file(&files[0].id).unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn files_go_with_their_draft_unless_it_is_being_sent() {
    let s = setup();
    s.gmail().await;
    let path = s._dir.path().join("a.pdf");
    std::fs::write(&path, b"a").unwrap();
    let draft = |id: &str, files| moss_core::Draft {
        revision: None,
        id: id.into(),
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: "gmail".into(),
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "Files".into(),
        body: String::new(),
        body_html: String::new(),
        remind_if_no_reply: false,
        updated_at: NOW,
        signature_id: None,
        signature_chosen: false,
        from_address: None,
        attachments: files,
    };
    let kept = |f: &moss_core::DraftFile| s.engine.shared.store().draft_file(&f.id).unwrap().is_some();
    let sending = s.engine.attach_files("d1", &[path.clone()]).unwrap();
    s.engine.save_draft(&draft("d1", sending.clone())).unwrap();
    assert_eq!(s.engine.drafts().unwrap()[0].attachments, sending);
    // Send deletes the draft but keeps its files until the email has gone (or Undo brings it back).
    assert!(s.engine.delete_draft("d1", true).unwrap());
    assert!(kept(&sending[0]));
    let discarded = s.engine.attach_files("d2", &[path.clone()]).unwrap();
    s.engine.save_draft(&draft("d2", discarded.clone())).unwrap();
    assert!(s.engine.delete_draft("d2", false).unwrap());
    assert!(!kept(&discarded[0]));
    // Starting up lets go of files whose draft is gone.
    s.engine.tidy().await.unwrap();
    assert!(!kept(&sending[0]));
}

#[tokio::test(start_paused = true)]
async fn a_file_you_attach_gets_a_thumbnail_like_one_you_received() {
    let s = setup();
    s.gmail().await;
    let (pdf, zip) = (s._dir.path().join("a.pdf"), s._dir.path().join("a.zip"));
    std::fs::write(&pdf, b"%PDF").unwrap();
    std::fs::write(&zip, b"PK").unwrap();
    let files = s.engine.attach_files("d1", &[pdf, zip]).unwrap();
    let dir = s._dir.path().join("files");
    let drawn = |bytes: &[u8], name: &str| Some(format!("{name}:{}", bytes.len()).into_bytes());
    let none = |_: &[u8], _: &str| None;
    assert_eq!(s.engine.draft_preview_with("d1", &files[0].id, &dir, drawn).await.unwrap(), Some(b"a.pdf:4".to_vec()));
    // Kept, so it isn't drawn again.
    assert_eq!(s.engine.draft_preview_with("d1", &files[0].id, &dir, none).await.unwrap(), Some(b"a.pdf:4".to_vec()));
    assert_eq!(s.engine.draft_preview_with("d1", &files[1].id, &dir, drawn).await.unwrap(), None);
    assert!(matches!(s.engine.draft_preview_with("d1", "999", &dir, drawn).await, Err(Error::Unavailable(_))));
    // Where no folder can be made, the thumbnail can't be kept, and it says so.
    let blocked = s._dir.path().join("not-a-folder");
    std::fs::write(&blocked, b"").unwrap();
    assert!(matches!(s.engine.draft_preview_with("d1", &files[0].id, &blocked, drawn).await, Err(Error::Unavailable(m)) if m.contains("can't save")));
    // Quick Look can't draw this one.
    let other = s.engine.attach_files("d1", &[s._dir.path().join("a.pdf")]).unwrap();
    assert_eq!(s.engine.draft_preview_with("d1", &other[0].id, &dir, none).await.unwrap(), None);
    assert!(s.engine.draft_preview("d1", &other[0].id, &dir).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_thumbnail_left_by_an_earlier_file_is_not_shown_for_a_new_one() {
    // Before V24 a new file could get the id of one that went, and with it that file's thumbnail.
    let s = setup();
    let pdf = s._dir.path().join("new.pdf");
    std::fs::write(&pdf, b"%PDF").unwrap();
    let dir = s._dir.path().join("files");
    let files = s.engine.attach_files("d1", &[pdf]).unwrap();
    let old = dir.join(crate::preview::PREVIEWS).join("drafts").join(format!("{}.jpg", files[0].id));
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, b"the first pdf").unwrap();
    let drawn = |bytes: &[u8], name: &str| Some(format!("{name}:{}", bytes.len()).into_bytes());
    assert_eq!(s.engine.draft_preview_with("d1", &files[0].id, &dir, drawn).await.unwrap(), Some(b"new.pdf:4".to_vec()));
}

#[tokio::test(start_paused = true)]
async fn replaying_forward_repair_queues_the_server_change_only_once() {
    let s = setup();
    s.inbox(1, "q4@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let thread = s.engine.thread(&s.thread_id("Q4 budget")).unwrap();
    let original = &thread.messages[0].id;
    s.engine.forwarded(original, original).unwrap();
    s.engine.forwarded(original, original).unwrap();
    assert_eq!(s.pending().len(), 1, "local repair must not duplicate a queued server update");
}

#[tokio::test(start_paused = true)]
async fn a_refused_forwarded_mark_preserves_local_receipt_and_remote_recovery() {
    let s = setup();
    s.inbox(1, "q4@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Q4 budget");
    s.events.clear();
    s.engine.send(forward(&id)).await.unwrap();
    s.fake.fail_next(moss_providers::Error::Protocol("NO [CANNOT] no keywords here".into()));
    settle().await;
    assert!(s.events.all().iter().any(|e| matches!(e, Event::SyncError { .. })));
    assert_eq!(s.pending().len(), 1);
    assert!(s.engine.thread(&id).unwrap().messages[0].forwarded);
    assert_eq!(s.engine.shared.store().earliest_change("gmail").unwrap().unwrap().failure, Some(moss_store::ChangeFailure::Refused));
    let before = s.fake.changes();
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert_eq!(s.pending().len(), 1);
    assert_eq!(s.fake.changes(), before);
}

#[tokio::test(start_paused = true)]
async fn forwarded_in_another_mail_app_shows_here_too() {
    let s = setup();
    let mut m = message("INBOX", 1, raw("q4@x", "Q4 budget", NOW), NOW);
    m.flags.keywords = vec!["$Forwarded".into()];
    s.fake.add(m);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Q4 budget");
    let forwarded = || s.engine.thread(&id).unwrap().messages[0].forwarded;
    assert!(forwarded());

    s.fake.update_flags(FlagUpdate { provider_id: "1:1".into(), flags: Flags::default(), labels: vec![] });
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert!(!forwarded());
}

#[tokio::test(start_paused = true)]
async fn a_reply_threads_with_the_conversation_and_shows_in_it() {
    let s = setup();
    s.inbox(1, "q4@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Q4 budget");
    let draft = OutgoingDraft {
        mode: ComposeMode::Reply,
        thread_id: Some(id.clone()),
        from_account_id: "gmail".into(),
        to: vec![Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Re: Q4 budget".into(),
        body: "Approved.".into(),
        body_html: "<p>Approved.</p>".into(),
        signature: vec!["Maya Ross".into()],
        from_address: None,
        remind_if_no_reply: false,
        attachments: vec![],
    };
    s.inbox(2, "q4-2@x", "Re: Q4 budget", NOW);
    s.engine.sync_now(None);
    settle().await;
    s.events.clear();
    let thread_id = s.engine.send(draft.clone()).await.unwrap();
    assert_eq!(thread_id, id);
    let sent = s.fake.sent();
    assert_eq!(sent[0].in_reply_to.as_deref(), Some("q4-2@x"));
    assert_eq!(sent[0].from.email, "maya.ross@gmail.com");
    assert!(sent[0].text.as_deref().unwrap().contains("> Hello about Re: Q4 budget"));
    // Signed under the body, above the quote, in both parts.
    assert!(sent[0].text.as_deref().unwrap().starts_with("Approved.\n\n-- \nMaya Ross\n\nOn "));
    let signed = "<p>Approved.</p><br><div class=\"gmail_signature\">Maya Ross</div><br>";
    assert!(sent[0].html.as_deref().unwrap().starts_with(signed));
    let thread = s.engine.thread(&id).unwrap();
    assert_eq!(thread.messages.len(), 3);
    // The copy filed in Sent, which the conversation shows at once, is signed too.
    let copy = thread.messages.iter().find(|m| m.from.email == "maya.ross@gmail.com").unwrap();
    assert!(copy.body_text.starts_with("Approved.\n\n-- \nMaya Ross\n"), "{}", copy.body_text);
    // You answered, so it no longer needs your reply.
    assert_eq!(thread.summary.section, Section::Info);
    // A reply isn't a forward.
    assert!(thread.messages.iter().all(|m| m.forwards.is_empty() && !m.forwarded));
    assert!(s.events.all().contains(&Event::MailChanged { account_id: "gmail".into() }));
    assert_eq!(
        s.engine.shared.store().provider_ids("gmail", "[Gmail]/Sent Mail").unwrap(),
        [format!("local:{}", sent[0].message_id.as_deref().unwrap())]
    );

    // A new email, before the folders are known, is filed under "Sent".
    let fresh = setup();
    fresh.gmail().await;
    fresh.engine.shared.stop_worker("gmail");
    fresh.engine.shared.folders.lock().unwrap().clear();
    let new = OutgoingDraft { mode: ComposeMode::New, thread_id: None, ..draft.clone() };
    let sent_in = fresh.engine.send(new).await.unwrap();
    assert_eq!(fresh.engine.shared.store().provider_ids("gmail", "Sent").unwrap().len(), 1);
    // Whoever you wrote to is suggested the next time you write. (This fake files bare addresses.)
    let suggested: Vec<_> = fresh.engine.contacts().unwrap().into_iter().map(|p| p.email).collect();
    assert_eq!(suggested, ["daniel@northwind.co"]);
    // What you wrote them is there for an AI reply to sound like you, without the signature;
    // not from the conversation it's replying in.
    assert_eq!(fresh.engine.my_words_to("gmail", "daniel@northwind.co", "elsewhere").unwrap(), ["Approved."]);
    assert!(fresh.engine.my_words_to("gmail", "daniel@northwind.co", &sent_in).unwrap().is_empty());

    let nobody = OutgoingDraft { to: vec![], ..draft.clone() };
    assert!(matches!(s.engine.send(nobody).await, Err(Error::NoRecipients)));
    let stranger = OutgoingDraft { from_account_id: "nope".into(), ..draft.clone() };
    assert!(matches!(s.engine.send(stranger).await, Err(Error::UnknownAccount(_))));
    s.fake.fail_next(moss_providers::Error::Protocol("554 rejected".into()));
    assert!(matches!(s.engine.send(draft).await, Err(Error::Server(_))));
}

#[tokio::test(start_paused = true)]
async fn search_finds_mail_by_its_words() {
    let s = setup();
    s.inbox(2, "b@x", "Venue options", NOW);
    s.inbox(1, "a@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let hits = s.engine.search("venue", None, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].thread.subject, "Venue options");
    assert!(s.engine.search("venue", Some("other"), 10).unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn accounts_can_be_renamed_reordered_and_removed() {
    let s = setup();
    s.gmail().await;
    let icloud = PasswordSignIn {
        service: MailService::ICloud,
        email: "maya@icloud.com".into(),
        password: "pw".into(),
        servers: None,
        login: None,
    };
    let second = s.engine.add_password_account(icloud).await.unwrap();
    assert_eq!(second.account.color, moss_core::AccountColor::Personal);
    assert_eq!(second.connection.unwrap().summary, "iCloud · App password");
    s.engine.reorder_accounts(&["icloud".into(), "gmail".into()]).unwrap();
    let ids: Vec<_> = s.engine.accounts().unwrap().into_iter().map(|a| a.account.id).collect();
    assert_eq!(ids, ["icloud", "gmail"]);
    let renamed = s
        .engine
        .update_account(
            "icloud",
            AccountPatch {
                name: Some("Home".into()),
                letter: Some("H".into()),
                color: Some(moss_core::AccountColor::Sky),
                sender_name: None,
            },
        )
        .unwrap();
    assert_eq!((renamed.account.name.as_str(), renamed.account.letter.as_str()), ("Home", "H"));

    let removed_secret = s.engine.shared.connection("icloud").unwrap().secret;
    s.engine.remove_account("icloud").unwrap();
    assert_eq!(s.engine.accounts().unwrap().len(), 1);
    assert_eq!(s.secrets.load(&removed_secret).unwrap(), None);
    assert!(matches!(
        s.engine.update_account("icloud", AccountPatch::default()),
        Err(Error::UnknownAccount(_))
    ));
    // Removing what isn't there is fine.
    s.engine.remove_account("icloud").unwrap();
}

#[tokio::test(start_paused = true)]
async fn start_skips_accounts_without_a_connection() {
    let s = setup();
    s.engine
        .shared
        .store()
        .upsert_account(&naming::new_account(MailService::Imap, "a@b.co", &[]))
        .unwrap();
    s.engine.start().unwrap();
    assert!(s.engine.shared.workers.lock().unwrap().is_empty());
    assert_eq!(s.engine.accounts().unwrap()[0].connection, None);
    // An account whose connection disappears stops its worker quietly.
    s.engine.shared.start_worker("b");
    settle().await;
    assert!(s.engine.shared.workers.lock().unwrap()["b"].task.is_finished());
    // Starting a running worker just wakes it.
    s.gmail().await;
    s.synced().await;
    s.engine.shared.start_worker("gmail");
    s.engine.shared.start_worker("b");
}

#[tokio::test(start_paused = true)]
async fn missing_things_are_errors() {
    let s = setup();
    assert!(matches!(s.engine.thread("99"), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.mark_done("99"), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.mark_read("99", true), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.snooze("99", None), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.mark_not_done("99"), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.delete_thread("99"), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.restore_thread("99"), Err(Error::UnknownThread(_))));
    assert!(s.engine.done_threads(Some("nobody"), 10).unwrap().is_empty());
    let err = s.engine.discover("not an address").await.unwrap_err();
    assert_eq!(serde_json::to_value(&err).unwrap()["kind"], "invalidEmail");
    let err = s.engine.check_resend_key(" ").await.unwrap_err();
    assert_eq!(serde_json::to_value(&err).unwrap()["kind"], "invalidResendKey");
    assert!(s.engine.add_resend_account("", "a@b.co").await.is_err());
}

// Real time: the HTTP client's timeout would fire at once in paused time.
#[tokio::test]
async fn a_resend_account_connects_with_its_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/domains"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"name": "rossstudio.art", "status": "verified"}]
        })))
        .mount(&server)
        .await;
    let endpoints = Endpoints { resend_api: server.uri(), ..Default::default() };
    let s = setup_endpoints(
        tempfile::tempdir().unwrap(),
        FakeProvider::new(),
        Arc::new(MemorySecretStore::new()),
        endpoints,
    );
    let domains = s.engine.check_resend_key("re_123").await.unwrap();
    assert_eq!(domains[0].name, "rossstudio.art");
    let info = s.engine.add_resend_account("re_123", "hello@rossstudio.art").await.unwrap();
    assert_eq!(info.connection.unwrap().summary, "Resend · API key");
    settle().await;
    assert_eq!(s.connector.last_credential(), Some(Credential::ApiKey("re_123".into())));
}

/// The provider's sign-in page coming back to Moss: the loopback redirect with a code.
async fn come_back(authorize_url: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let param = |name: &str| {
        authorize_url
            .split(['?', '&'])
            .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
            .unwrap()
            .replace("%3A", ":")
            .replace("%2F", "/")
    };
    let port = param("redirect_uri").trim_start_matches("http://127.0.0.1:").trim_end_matches('/').to_string();
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
    let request = format!("GET /?code=c&state={} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", param("state"));
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
}

fn oauth_endpoints(server: &MockServer) -> Endpoints {
    Endpoints {
        oauth: moss_accounts::OAuthEndpoints {
            microsoft_authorize: format!("{}/ms/authorize", server.uri()),
            microsoft_token: format!("{}/ms/token", server.uri()),
            google_authorize: format!("{}/google/auth", server.uri()),
            google_token: format!("{}/google/token", server.uri()),
        },
        ..Endpoints::default()
    }
}

// Real time: the sign-in listens on a real port.
#[tokio::test]
async fn outlook_signs_in_through_the_browser_and_syncs_with_its_token() {
    let server = MockServer::start().await;
    // An id_token whose payload, {"preferred_username":"maya.ross@outlook.com"}, says who signed in.
    let id_token = "e30.eyJwcmVmZXJyZWRfdXNlcm5hbWUiOiJtYXlhLnJvc3NAb3V0bG9vay5jb20ifQ.sig";
    Mock::given(method("POST"))
        .and(path("/ms/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at", "refresh_token": "rt", "expires_in": 3599, "id_token": id_token,
        })))
        .mount(&server)
        .await;
    let s = setup_endpoints(
        tempfile::tempdir().unwrap(),
        FakeProvider::new(),
        Arc::new(MemorySecretStore::new()),
        oauth_endpoints(&server),
    );

    let started = s.engine.start_browser_sign_in(BrowserSignIn::outlook()).await.unwrap();
    assert!(started.url.starts_with(&format!("{}/ms/authorize?", server.uri())));
    assert_eq!(started.client_id, moss_accounts::MOSS_MICROSOFT_CLIENT_ID);
    let url = started.url.clone();
    tokio::spawn(async move { come_back(&url).await });
    let info = s.engine.finish_browser_sign_in().await.unwrap();

    assert_eq!(info.account.address, "maya.ross@outlook.com");
    assert_eq!(info.connection.unwrap().summary, "Microsoft 365 · Signed in with browser");
    assert_eq!(s.engine.accounts().unwrap().len(), 1);
    settle().await;
    assert_eq!(s.connector.last_credential(), Some(Credential::Token("at".into())));
}

#[tokio::test]
async fn gmail_signs_in_through_the_browser_with_your_own_client() {
    let server = MockServer::start().await;
    let s = setup_endpoints(
        tempfile::tempdir().unwrap(),
        FakeProvider::new(),
        Arc::new(MemorySecretStore::new()),
        oauth_endpoints(&server),
    );
    let request = BrowserSignIn {
        provider: BrowserProvider::Gmail,
        email: Some("maya.ross@gmail.com".into()),
        client_id: Some("g-id".into()),
        client_secret: Some("g-secret".into()),
    };
    let started = s.engine.start_browser_sign_in(request).await.unwrap();
    assert!(started.url.starts_with(&format!("{}/google/auth?", server.uri())));
    assert!(started.url.contains("login_hint=maya.ross%40gmail.com"));
    assert_eq!(started.client_id, "g-id");

    let no_client = BrowserSignIn { provider: BrowserProvider::Gmail, ..BrowserSignIn::outlook() };
    let err = s.engine.start_browser_sign_in(no_client).await.unwrap_err();
    assert_eq!(serde_json::to_value(&err).unwrap()["kind"], "invalidOAuthClient");
}

#[tokio::test]
async fn finishing_a_browser_sign_in_that_never_started_is_an_error() {
    let s = setup();
    let err = s.engine.finish_browser_sign_in().await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

struct BrokenKeychain;

impl SecretStore for BrokenKeychain {
    fn save(&self, _: &SecretRef, _: &moss_accounts::Secret) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Unavailable("locked".into()))
    }
    fn load(&self, _: &SecretRef) -> Result<Option<moss_accounts::Secret>, SecretStoreError> {
        Err(SecretStoreError::Unavailable("locked".into()))
    }
    fn delete(&self, _: &SecretRef) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Unavailable("locked".into()))
    }
}

fn connection(auth: AuthMethod) -> AccountConnection {
    AccountConnection {
        email: "a@b.co".into(),
        kind: moss_core::ProviderKind::Imap,
        service: MailService::Imap,
        servers: MailService::Fastmail.known_servers("a@b.co"),
        auth,
        secret: SecretRef::for_account("a@b.co", "imap"),
    }
}

fn with_connection(s: &Setup, auth: AuthMethod) {
    let account = naming::new_account(MailService::Imap, "a@b.co", &[]);
    let mut store = s.engine.shared.store();
    store.upsert_account(&account).unwrap();
    let json = serde_json::to_string(&connection(auth)).unwrap();
    store.set_setting(&connection_key(&account.id), Some(&json)).unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_locked_keychain_or_missing_token_means_signing_in_again() {
    let dir = tempfile::tempdir().unwrap();
    let events = Arc::new(RecordedEvents::default());
    let engine = Engine::new(Parts {
        store: Store::open(dir.path().join("moss.db")).unwrap(),
        secrets: Arc::new(BrokenKeychain),
        accounts: Accounts::new(Arc::new(BrokenKeychain)),
        connector: Arc::new(FakeConnector::new(FakeProvider::new())),
        events: events.clone(),
        clock,
    });
    let s = Setup {
        engine,
        fake: FakeProvider::new(),
        connector: Arc::new(FakeConnector::new(FakeProvider::new())),
        events,
        secrets: Arc::new(MemorySecretStore::new()),
        _dir: dir,
    };
    with_connection(&s, AuthMethod::Password);
    assert!(matches!(s.engine.shared.provider("b").await, Err(Error::Keychain(_))));
    assert!(s.engine.remove_account("b").unwrap().pending);
    assert!(s.engine.accounts().unwrap().is_empty());
    assert!(s.engine.credential_cleanup_status().unwrap().pending);
    let err = s.engine.add_password_account(gmail_sign_in("abcdefghijklmnop")).await.unwrap_err();
    assert_eq!(serde_json::to_value(&err).unwrap()["kind"], "keychain");

    let oauth = setup();
    with_connection(
        &oauth,
        AuthMethod::OAuth2 { issuer: moss_accounts::OAuthIssuer::Google, client_id: "c".into() },
    );
    assert!(matches!(oauth.engine.shared.provider("b").await, Err(Error::SignedOut(_))));
    // A token saved for a password account is still used.
    let tokens = moss_accounts::TokenSet {
        access_token: "t".into(),
        refresh_token: None,
        expires_at: 0,
        refresh_issued_at: 0,
    };
    let token = setup();
    with_connection(&token, AuthMethod::Password);
    token
        .secrets
        .save(
            &SecretRef::for_account("a@b.co", "imap"),
            &moss_accounts::Secret::OAuth { tokens, client_secret: None },
        )
        .unwrap();
    token.engine.shared.provider("b").await.unwrap();
    assert_eq!(token.connector.last_credential(), Some(Credential::Token("t".into())));
    // Broken connection settings are an error too.
    token.engine.shared.store().set_setting("connection:b", Some("{")).unwrap();
    token.engine.shared.forget_provider("b");
    assert!(matches!(token.engine.shared.provider("b").await, Err(Error::Server(_))));
}

#[tokio::test(start_paused = true)]
async fn an_account_removed_during_sync_stops_its_worker() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    // The row goes, but the worker still knows the connection and folders.
    s.engine.shared.store().remove_account("gmail").unwrap();
    s.fake.deliver(message("INBOX", 1, raw("a@x", "Late", NOW), NOW));
    settle().await;
    assert!(s.engine.shared.workers.lock().unwrap()["gmail"].task.is_finished());
}

#[tokio::test]
async fn the_fake_server_behaves_like_a_provider() {
    let fake = FakeProvider::new();
    assert!(fake.capabilities().server_flags);
    assert!(fake.fetch_message("INBOX", "1:1").await.is_err());
}

#[test]
fn errors_tell_the_ui_what_kind_they_are() {
    assert_eq!(Error::SignIn(moss_accounts::SignInError::InvalidEmail).kind(), "signIn");
    let cases = [
        (Error::Store(moss_store::Error::NotEmail), "store"),
        (Error::UnknownAccount("x".into()), "unknownAccount"),
        (Error::UnknownThread("x".into()), "unknownThread"),
        (Error::SignedOut("x".into()), "signedOut"),
        (Error::Offline("x".into()), "offline"),
        (Error::Server("x".into()), "server"),
        (Error::DeliveryBusy, "deliveryBusy"),
        (Error::DeliveryUnknown("x".into()), "deliveryUnknown"),
        (Error::DeliveryAccepted("x".into()), "deliveryAccepted"),
        (Error::Keychain("x".into()), "keychain"),
        (Error::NoRecipients, "noRecipients"),
        (Error::PublicDomain("gmail.com".into()), "publicDomain"),
        (Error::Unavailable("x".into()), "unavailable"),
        (Error::InvalidAddress("x".into()), "invalidAddress"),
        (Error::PrimaryAddress("x".into()), "primaryAddress"),
        (Error::NotYourAddress { address: "x".into(), account: "y".into() }, "notYourAddress"),
        (Error::TooBig("these files are 26 MB, and Gmail sends up to 25 MB".into()), "tooBig"),
    ];
    for (error, kind) in cases {
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["kind"], kind);
        assert_eq!(json["message"], error.to_string());
    }
    use moss_providers::Error as P;
    assert!(matches!(Error::from(P::Auth("x".into())), Error::SignedOut(_)));
    assert!(matches!(Error::from(P::Network("x".into())), Error::Offline(_)));
    let limited = P::RateLimited { retry_after: std::time::Duration::from_secs(9) };
    assert!(matches!(Error::from(limited), Error::Offline(_)));
    assert!(matches!(Error::from(P::NotFound("x".into())), Error::Server(_)));
    assert_eq!(Error::Keychain("x".into()).problem(), Problem::SignedOut);
}

#[test]
fn connections_are_summarised_for_settings() {
    let mut c = connection(AuthMethod::Password);
    assert_eq!(summary(&c), "IMAP · imap.fastmail.com");
    c.servers = None;
    assert_eq!(summary(&c), "IMAP · ");
    for (service, text) in [
        (MailService::Outlook, "Microsoft 365 · Signed in with browser"),
        (MailService::Gmail, "Gmail · Signed in with browser"),
        (MailService::Yahoo, "Yahoo · Signed in with browser"),
        (MailService::Fastmail, "Fastmail · Signed in with browser"),
    ] {
        c.service = service;
        c.auth = AuthMethod::OAuth2 {
            issuer: moss_accounts::OAuthIssuer::Microsoft,
            client_id: "x".into(),
        };
        assert_eq!(summary(&c), text);
    }
    assert_eq!(SplitId::Important, SplitId::Important);
}

#[tokio::test(start_paused = true)]
async fn gmail_system_labels_stay_hidden() {
    let s = setup();
    let mut m = message("INBOX", 1, raw("a@x", "Labelled", NOW), NOW);
    m.labels = vec!["\\Important".into(), "\\Inbox".into(), "Clients/Acme".into()];
    s.fake.add(m);
    s.gmail().await;
    s.synced().await;
    let thread = s.engine.thread(&s.thread_id("Labelled")).unwrap();
    assert_eq!(thread.summary.labels, ["Clients/Acme"]);
}

impl Setup {
    /// `count` messages you sent, newest first; subject "Hi {n}", n = 0 for the newest.
    fn sent_mail(&self, count: usize) {
        for i in 0..count {
            let at = NOW - (i as i64) * 60_000;
            let raw = raw_from(&format!("s{i}@x"), MAYA, "d@x.io", &format!("Hi {i}"), at);
            self.fake.add(message(SENT_MAIL, (count - i) as u32, raw, at));
        }
    }

    fn message_with_subject(&self, subject: &str) -> moss_core::Message {
        let hit = self.engine.search(&format!("\"{subject}\""), None, 1).unwrap().remove(0);
        let thread = self.engine.thread(&hit.thread.id).unwrap();
        thread.messages.into_iter().find(|m| m.subject == subject).unwrap()
    }
}

#[tokio::test(start_paused = true)]
async fn older_sent_mail_comes_as_headers_only_once_plenty_is_in() {
    let s = setup();
    let total = sync::SENT_KEEP + 300;
    s.sent_mail(total);
    s.gmail().await;
    s.synced().await;
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    // All of it, for threading and for knowing you replied; the older part without bodies.
    let stored = s.engine.shared.store().provider_ids("gmail", SENT_MAIL).unwrap().len();
    assert_eq!(stored, total);
    let newer = s.message_with_subject(&format!("Hi {}", sync::SENT_KEEP - 1));
    assert!(!newer.headers_only && newer.body_text.contains("Hello about"));
    let older = s.message_with_subject(&format!("Hi {}", sync::SENT_KEEP));
    assert!(older.headers_only && older.body_text.is_empty());
    assert_eq!(older.from.email, "maya.ross@gmail.com");
    // Then it's all in: later passes only look for new sent mail.
    let shallow = |s: &Setup| s.fake.calls().iter().filter(|c| c.starts_with("headers")).count();
    let before = shallow(&s);
    assert!(before >= 1);
    s.engine.shared.pass("gmail", true).await.unwrap();
    assert_eq!(shallow(&s), before, "history-only passes must not refetch mature Sent mail");
    s.engine.sync_now(None);
    settle().await;
    assert_eq!(shallow(&s), before + 1);
    let downloads = s.fake.downloads();
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    assert_eq!(s.fake.downloads(), downloads);
}

#[tokio::test(start_paused = true)]
async fn a_headers_only_message_gets_its_body_when_opened() {
    let s = setup();
    s.sent_mail(sync::SENT_KEEP + 2);
    s.gmail().await;
    s.synced().await;
    settle().await;
    let oldest = s.message_with_subject(&format!("Hi {}", sync::SENT_KEEP + 1));
    assert!(oldest.headers_only);
    assert!(s.engine.load_message(&oldest.id).await.unwrap());
    let filled = s.message_with_subject(&format!("Hi {}", sync::SENT_KEEP + 1));
    assert!(!filled.headers_only);
    assert_eq!(filled.body_text, format!("Hello about Hi {}\n", sync::SENT_KEEP + 1));
    // Once whole, opening it again asks nothing of the server.
    let calls = s.fake.calls().len();
    assert!(!s.engine.load_message(&oldest.id).await.unwrap());
    assert_eq!(s.fake.calls().len(), calls);

    // Gone from the server, or never stored: nothing to load.
    let other = s.message_with_subject(&format!("Hi {}", sync::SENT_KEEP));
    s.fake.forget(SENT_MAIL, "1:2");
    assert!(matches!(s.engine.load_message(&other.id).await, Err(Error::Unavailable(_))));
    assert!(matches!(s.engine.load_message("none").await, Err(Error::Unavailable(_))));
    // Offline: says so.
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    assert!(matches!(s.engine.load_message(&other.id).await, Err(Error::Offline(_))));
}

#[tokio::test(start_paused = true)]
async fn attachments_come_from_the_server_when_opened() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Q4 signed");
    let names: Vec<&str> = m.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["attachment", "Q4.pdf"]);
    // Files aren't kept when mail arrives: opening one fetches it.
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    assert!(matches!(s.engine.attachment(&m.id, 1).await, Err(Error::Offline(_))));
    assert_eq!(s.engine.attachment(&m.id, 1).await.unwrap(), b"PDFDATA");
    assert!(s.fake.calls().contains(&"fetch_message INBOX 1:5".to_string()));
    // Then it's in the cache: no server needed, even offline. The failure waits for the logo.
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    assert_eq!(s.engine.attachment(&m.id, 1).await.unwrap(), b"PDFDATA");
    assert!(matches!(s.engine.attachment(&m.id, 0).await, Err(Error::Offline(_))));
    assert_eq!(s.engine.attachment(&m.id, 0).await.unwrap(), b"PNG");

    assert!(matches!(s.engine.attachment(&m.id, 7).await, Err(Error::Unavailable(_))));
    assert!(matches!(s.engine.attachment("999", 0).await, Err(Error::Unavailable(_))));
    s.fake.forget("INBOX", "1:5");
    assert_eq!(s.engine.attachment(&m.id, 1).await.unwrap(), b"PDFDATA", "kept from before");
    let files = tempfile::tempdir().unwrap();
    assert_eq!(s.engine.clear_cache(files.path()).unwrap(), 10);
    assert!(matches!(s.engine.attachment(&m.id, 1).await, Err(Error::Unavailable(_))));
}

#[tokio::test(start_paused = true)]
async fn an_attachment_is_saved_as_a_file_to_open() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Q4 signed");
    let dir = tempfile::tempdir().unwrap();
    let path = s.engine.attachment_file(&m.id, 1, dir.path()).await.unwrap();
    assert_eq!(path, dir.path().join(&m.id).join("1").join("Q4.pdf"));
    assert_eq!(std::fs::read(&path).unwrap(), b"PDFDATA");
    #[cfg(target_os = "macos")]
    {
        let quarantine = std::process::Command::new("/usr/bin/xattr").args(["-p", "com.apple.quarantine"]).arg(&path).output().unwrap();
        assert!(quarantine.status.success(), "downloaded mail attachments must be quarantined");
        assert!(!quarantine.stdout.is_empty());
    }
    // Again: the same file, written over.
    assert_eq!(s.engine.attachment_file(&m.id, 1, dir.path()).await.unwrap(), path);
    assert!(matches!(s.engine.attachment_file(&m.id, 9, dir.path()).await, Err(Error::Unavailable(_))));
    // A folder it can't write to.
    let file = dir.path().join("file");
    std::fs::write(&file, b"").unwrap();
    assert!(matches!(s.engine.attachment_file(&m.id, 1, &file).await, Err(Error::Unavailable(_))));
}

/// A pretend Quick Look: "thumb" and the file's size.
fn fake_thumbnail(bytes: &[u8], _name: &str) -> Option<Vec<u8>> {
    Some(format!("thumb {}", bytes.len()).into_bytes())
}

fn no_thumbnail(_bytes: &[u8], _name: &str) -> Option<Vec<u8>> {
    None
}

/// An email with a zip file, which shows its icon.
fn with_zip(id: &str, date_ms: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(date_ms).unwrap().to_rfc2822();
    format!(
        "From: Daniel Okafor <daniel@northwind.co>\r\nTo: Maya Ross <maya.ross@gmail.com>\r\n\
         Subject: Brand assets\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n\
         Content-Type: text/plain\r\n\r\nAll the logos\r\n--m\r\n\
         Content-Type: application/zip\r\nContent-Disposition: attachment; filename=Assets.zip\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nWklQ\r\n--m--\r\n"
    )
    .into_bytes()
}

#[tokio::test(start_paused = true)]
async fn an_attachment_gets_a_thumbnail_kept_on_disk() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Q4 signed");
    let dir = tempfile::tempdir().unwrap();
    let max = preview::PREVIEW_MAX_BYTES;
    let thumb = s.engine.preview_with(&m.id, 1, dir.path(), max, fake_thumbnail).await.unwrap();
    assert_eq!(thumb.as_deref(), Some(&b"thumb 7"[..]));
    let kept = dir.path().join(".previews").join(&m.id).join("1.jpg");
    assert_eq!(std::fs::read(&kept).unwrap(), b"thumb 7");
    // Next time it comes from disk: no server, no Quick Look.
    let calls = s.fake.calls().len();
    let again = s.engine.preview_with(&m.id, 1, dir.path(), max, no_thumbnail).await.unwrap();
    assert_eq!(again.as_deref(), Some(&b"thumb 7"[..]));
    assert_eq!(s.fake.calls().len(), calls);
    // It counts as cache, and goes with it.
    assert!(s.engine.storage(dir.path()).unwrap().usage.cache >= 7);
    s.engine.clear_cache(dir.path()).unwrap();
    assert!(!kept.exists());
}

#[tokio::test(start_paused = true)]
async fn a_file_with_no_thumbnail_is_not_fetched_again() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Q4 signed");
    let dir = tempfile::tempdir().unwrap();
    let max = preview::PREVIEW_MAX_BYTES;
    // Offline: nothing is remembered, so it tries again later.
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    assert!(matches!(s.engine.preview_with(&m.id, 1, dir.path(), max, fake_thumbnail).await, Err(Error::Offline(_))));
    assert_eq!(s.engine.preview_with(&m.id, 1, dir.path(), max, no_thumbnail).await.unwrap(), None);
    let calls = s.fake.calls().len();
    assert_eq!(s.engine.preview_with(&m.id, 1, dir.path(), max, fake_thumbnail).await.unwrap(), None);
    assert_eq!(s.fake.calls().len(), calls);
    assert!(matches!(s.engine.preview_with(&m.id, 9, dir.path(), max, fake_thumbnail).await, Err(Error::Unavailable(_))));
    // A folder it can't write to.
    let file = dir.path().join("file");
    std::fs::write(&file, b"").unwrap();
    let blocked = s.engine.preview_with(&m.id, 1, &file, max, fake_thumbnail).await;
    assert!(matches!(blocked, Err(Error::Unavailable(_))));
}

#[tokio::test(start_paused = true)]
async fn big_files_and_zips_show_their_icon_without_a_download() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.fake.add(message("INBOX", 6, with_zip("zip@x", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let pdf = s.message_with_subject("Q4 signed");
    let zip = s.message_with_subject("Brand assets");
    let dir = tempfile::tempdir().unwrap();
    let calls = s.fake.calls().len();
    assert_eq!(s.engine.preview_with(&pdf.id, 1, dir.path(), 6, fake_thumbnail).await.unwrap(), None);
    assert_eq!(s.engine.attachment_preview(&zip.id, 0, dir.path()).await.unwrap(), None);
    assert_eq!(s.fake.calls().len(), calls);
}

#[test]
fn opened_files_make_room_without_touching_thumbnails() {
    let dir = tempfile::tempdir().unwrap();
    let thumb = dir.path().join(".previews").join("1").join("0.jpg");
    std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
    std::fs::write(&thumb, b"thumb").unwrap();
    let opened = dir.path().join("1").join("0").join("Q4.pdf");
    std::fs::create_dir_all(opened.parent().unwrap()).unwrap();
    std::fs::write(&opened, b"PDF").unwrap();
    storage::keep_newest_files(dir.path(), 0);
    assert!(!opened.exists());
    assert!(thumb.exists());
}

#[test]
fn attachment_names_cannot_leave_their_folder() {
    assert_eq!(safe_file_name("Q4.pdf"), "Q4.pdf");
    assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_file_name("C:\\Users\\a.txt"), "C__Users_a.txt");
    assert_eq!(safe_file_name(".."), "attachment");
    assert_eq!(safe_file_name(""), "attachment");
    assert_eq!(safe_file_name(".hidden"), "hidden");
}

#[tokio::test(start_paused = true)]
async fn tidying_finishes_a_rebuild_trims_the_cache_and_gives_the_space_back() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Q4 signed");
    s.engine.attachment(&m.id, 1).await.unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    // As if the index were half rebuilt, the cache over its limit and the file never vacuumed.
    db.execute_batch(
        "DELETE FROM message_search; INSERT INTO settings (key, value) VALUES ('index.rebuiltThrough', '0');
         INSERT INTO settings (key, value) VALUES ('storage.cacheLimit', '0');
         PRAGMA auto_vacuum = NONE; VACUUM;",
    )
    .unwrap();
    assert!(s.engine.search("signed", None, 10).unwrap().is_empty());
    db.execute_batch("INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at)
        VALUES('tidy-fault','tidy-fault','gmail','{','{','fixture@example.test','complete',0);
        CREATE TRIGGER refuse_compaction BEFORE UPDATE OF compacted_at ON deliveries BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    s.engine.tidy().await.unwrap();
    assert_eq!(s.engine.search("signed", None, 10).unwrap().len(), 1);
    let cached: i64 = db.query_row("SELECT count(*) FROM attachment_data", [], |r| r.get(0)).unwrap();
    assert_eq!(cached, 0);
    let mode: i64 = db.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, 2);
    db.execute_batch("DROP TRIGGER refuse_compaction").unwrap();
    // More free space than one step gives back: it goes back a step at a time.
    db.execute_batch("CREATE TABLE junk (x); INSERT INTO junk VALUES (zeroblob(8000000)); DROP TABLE junk;").unwrap();
    let free = |db: &rusqlite::Connection| db.query_row("PRAGMA freelist_count", [], |r| r.get::<_, u32>(0)).unwrap();
    assert!(free(&db) > moss_store::FREE_PAGES);
    s.engine.tidy().await.unwrap();
    assert_eq!(free(&db), 0);
    assert_eq!(s.engine.shared.store().delivery_replay_hash("tidy-fault").unwrap(),Some((None,true)));
    // Starting the engine tidies up too.
    s.engine.start().unwrap();
}

#[tokio::test]
async fn account_removal_drains_legacy_delivery_payloads_without_waiting_for_restart() {
    let s=setup();s.gmail().await;
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at)
        VALUES('removed-legacy','removed-legacy','gmail','{','private legacy payload','fixture@example.test','complete',0);").unwrap();
    s.engine.remove_account("gmail").unwrap();
    for _ in 0..100 {
        if s.engine.shared.store().delivery_replay_hash("removed-legacy").unwrap()==Some((None,true)) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(s.engine.shared.store().delivery_replay_hash("removed-legacy").unwrap(),Some((None,true)));
    assert!(s.engine.accounts().unwrap().is_empty());
}

#[tokio::test]
async fn failed_orphan_cleanup_does_not_starve_terminal_delivery_retirement() {
    let s=setup();
    s.engine.shared.store().add_draft_file("orphan","fixture.txt","text/plain",b"fixture",0).unwrap();
    let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,created_at)
        VALUES('independent-cleanup','independent-cleanup','removed-account','{','private legacy payload','fixture@example.test','complete',0);
        CREATE TRIGGER refuse_orphan_cleanup BEFORE DELETE ON draft_files BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(s.engine.tidy().await.is_err());
    assert_eq!(s.engine.shared.store().delivery_replay_hash("independent-cleanup").unwrap(),Some((None,true)));
}

const ALL_MAIL: &str = "[Gmail]/All Mail";
const SENT_MAIL: &str = "[Gmail]/Sent Mail";
const MAYA: &str = "Maya Ross <maya.ross@gmail.com>";
const DANIEL: &str = "Daniel Okafor <daniel@northwind.co>";

impl Setup {
    /// A message in `mailbox` with Gmail labels, in Gmail conversation `conversation`.
    fn gmail_message(
        &self,
        (mailbox, uid): (&str, u32),
        raw: Vec<u8>,
        at: i64,
        labels: &[&str],
        conversation: u64,
    ) {
        let mut m = message(mailbox, uid, raw, at);
        m.labels = labels.iter().map(|l| l.to_string()).collect();
        self.fake.add(m);
        self.fake.in_conversation(mailbox, uid, conversation);
    }

    fn conversation_calls(&self) -> Vec<String> {
        self.fake.calls().into_iter().filter(|c| c.starts_with("conversation")).collect()
    }
}

#[tokio::test(start_paused = true)]
async fn archived_replies_fill_in_their_conversation() {
    let s = setup();
    let subject = "Estimate for a studio cabinet";
    let re = format!("Re: {subject}");
    let q1 = raw_from("q1@x", MAYA, DANIEL, subject, NOW - 5 * DAY);
    let r1 = reply_to(&["q1@x"], raw("r1@x", &re, NOW - 4 * DAY));
    let q2 = reply_to(&["q1@x", "r1@x"], raw_from("q2@x", MAYA, DANIEL, &re, NOW - 3 * DAY));
    let r2 = reply_to(&["q1@x", "r1@x", "q2@x"], raw("r2@x", &re, NOW - 2 * DAY));
    let r3 = reply_to(&["q1@x", "r1@x", "q2@x", "r2@x"], raw("r3@x", &re, NOW - DAY));
    let draft = reply_to(&["q1@x", "r3@x"], raw_from("d@x", MAYA, DANIEL, &re, NOW));
    let older_sent = raw_from("q0@x", MAYA, DANIEL, subject, NOW - 6 * DAY);
    // What Moss syncs: your two messages in Sent, and the latest reply in the Inbox.
    s.gmail_message((SENT_MAIL, 1), q1.clone(), NOW - 5 * DAY, &["\\Sent"], 77);
    s.gmail_message((SENT_MAIL, 2), q2.clone(), NOW - 3 * DAY, &["\\Sent"], 77);
    s.gmail_message(("INBOX", 1), r3.clone(), NOW - DAY, &["\\Inbox"], 77);
    // All Mail has every one of them, plus the two replies you archived.
    s.gmail_message((ALL_MAIL, 10), q1, NOW - 5 * DAY, &["\\Sent"], 77);
    s.gmail_message((ALL_MAIL, 11), r1, NOW - 4 * DAY, &[], 77);
    s.gmail_message((ALL_MAIL, 12), q2, NOW - 3 * DAY, &["\\Sent"], 77);
    s.gmail_message((ALL_MAIL, 13), r2, NOW - 2 * DAY, &["\\Important"], 77);
    s.gmail_message((ALL_MAIL, 14), r3, NOW - DAY, &["\\Inbox", "\\Important"], 77);
    // Not mail to show, or mail that comes through its own folder.
    s.gmail_message((ALL_MAIL, 15), draft, NOW, &["\\Draft"], 77);
    s.gmail_message((ALL_MAIL, 16), older_sent, NOW - 6 * DAY, &["\\Sent"], 77);
    // A conversation nothing here belongs to.
    s.gmail_message((ALL_MAIL, 20), raw("x@x", "Unrelated", NOW), NOW, &[], 99);
    // One you started, whose only reply you archived: it's only in Sent, so it isn't filled in.
    let lunch = raw_from("l1@x", MAYA, DANIEL, "Lunch?", NOW - 9 * DAY);
    let answer = reply_to(&["l1@x"], raw("l2@x", "Re: Lunch?", NOW - 8 * DAY));
    s.gmail_message((SENT_MAIL, 3), lunch.clone(), NOW - 9 * DAY, &["\\Sent"], 88);
    s.gmail_message((ALL_MAIL, 30), lunch, NOW - 9 * DAY, &["\\Sent"], 88);
    s.gmail_message((ALL_MAIL, 31), answer, NOW - 8 * DAY, &[], 88);
    s.gmail().await;
    s.synced().await;
    settle().await;

    assert_eq!(s.subjects(), [subject]);
    let lunch = s.engine.search("lunch", None, 1).unwrap().remove(0).thread;
    assert_eq!((lunch.message_count, lunch.done), (1, true));
    let thread = s.engine.thread(&s.thread_id(subject)).unwrap();
    let from: Vec<_> = thread.messages.iter().map(|m| m.from.email.as_str()).collect();
    assert_eq!(
        from,
        [
            "maya.ross@gmail.com",
            "daniel@northwind.co",
            "maya.ross@gmail.com",
            "daniel@northwind.co",
            "daniel@northwind.co"
        ]
    );
    // Still open and waiting on you: the archived replies came in as done.
    assert!(!thread.summary.done);
    assert_eq!(thread.summary.section, Section::Reply);
    // The archived ones came as headers only; their bodies come when they're opened.
    let archived: Vec<bool> = thread.messages.iter().map(|m| m.headers_only).collect();
    assert_eq!(archived, [false, true, false, true, false]);
    // Each message was downloaded once: four from Inbox and Sent, then the two archived.
    assert_eq!(s.fake.downloads(), 6);
    assert!(!s.fake.calls().iter().any(|c| c.starts_with(&format!("fetch {ALL_MAIL}"))));
}

#[tokio::test(start_paused = true)]
async fn a_conversation_brought_back_from_done_stays_in_the_inbox() {
    let s = setup();
    let subject = "Estimate for a studio cabinet";
    let re = format!("Re: {subject}");
    let q1 = raw_from("q1@x", MAYA, DANIEL, subject, NOW - 5 * DAY);
    let r1 = reply_to(&["q1@x"], raw("r1@x", &re, NOW - 4 * DAY));
    let r2 = reply_to(&["q1@x", "r1@x"], raw("r2@x", &re, NOW - DAY));
    s.gmail_message((SENT_MAIL, 1), q1.clone(), NOW - 5 * DAY, &["\\Sent"], 77);
    s.gmail_message(("INBOX", 1), r2.clone(), NOW - DAY, &["\\Inbox"], 77);
    s.gmail_message((ALL_MAIL, 10), q1, NOW - 5 * DAY, &["\\Sent"], 77);
    s.gmail_message((ALL_MAIL, 11), r1, NOW - 4 * DAY, &[], 77);
    s.gmail_message((ALL_MAIL, 12), r2.clone(), NOW - DAY, &["\\Inbox"], 77);
    s.gmail().await;
    s.synced().await;
    settle().await;
    let id = s.thread_id(subject);
    assert_eq!(s.engine.thread(&id).unwrap().messages.len(), 3);

    s.engine.mark_done(&id).unwrap();
    settle().await;
    s.engine.mark_not_done(&id).unwrap();
    settle().await;
    // Gmail puts it back in the Inbox under a new id, which the next sync finds.
    s.gmail_message(("INBOX", 2), r2, NOW - DAY, &["\\Inbox"], 77);
    for _ in 0..3 {
        s.engine.sync_now(Some("gmail"));
        settle().await;
        settle().await;
    }
    assert_eq!(s.subjects(), [subject], "back in the inbox");
    assert!(s.engine.done_threads(None, 10).unwrap().is_empty(), "and not in Done");
    assert!(!s.engine.thread(&id).unwrap().summary.done);
}

#[tokio::test(start_paused = true)]
async fn a_done_email_put_back_in_the_inbox_on_gmail_comes_back_here() {
    let s = setup();
    s.gmail_message(("INBOX", 1), raw("a@x", "Budget", NOW), NOW, &["\\Inbox"], 5);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Budget");
    s.engine.mark_done(&id).unwrap();
    settle().await;
    assert!(s.subjects().is_empty());
    // Moved back to the Inbox in Gmail: it gets a new id there.
    s.gmail_message(("INBOX", 2), raw("a@x", "Budget", NOW), NOW, &["\\Inbox"], 5);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    assert_eq!(s.subjects(), ["Budget"]);
    assert!(s.engine.done_threads(None, 10).unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_done_conversation_stays_done_when_its_history_fills_in() {
    let s = setup();
    let old = raw("old@x", "Budget", NOW - 2 * DAY);
    let latest = reply_to(&["old@x"], raw("a@x", "Re: Budget", NOW - DAY));
    s.gmail_message(("INBOX", 1), latest.clone(), NOW - DAY, &["\\Inbox"], 5);
    s.gmail_message((ALL_MAIL, 10), old, NOW - 2 * DAY, &[], 5);
    s.gmail_message((ALL_MAIL, 11), latest, NOW - DAY, &["\\Inbox"], 5);
    s.gmail().await;
    s.synced().await;
    settle().await;
    let id = s.thread_id("Budget");
    assert_eq!(s.engine.thread(&id).unwrap().messages.len(), 2);

    s.engine.mark_done(&id).unwrap();
    settle().await;
    assert!(s.subjects().is_empty());
    // You answer from your phone: the conversation is looked at again, and stays done.
    let mine = reply_to(&["old@x", "a@x"], raw_from("me@x", MAYA, DANIEL, "Re: Budget", NOW));
    s.gmail_message((ALL_MAIL, 12), mine.clone(), NOW, &["\\Sent"], 5);
    s.gmail_message((SENT_MAIL, 1), mine, NOW, &["\\Sent"], 5);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    assert!(s.subjects().is_empty());
    let thread = s.engine.thread(&id).unwrap();
    assert_eq!(thread.messages.len(), 3);
    assert!(thread.summary.done);
    // Your answer came through Sent, so the conversation needn't be looked at again.
    assert_eq!(s.conversation_calls().iter().filter(|c| c.contains("messages")).count(), 1);
    assert_eq!(s.fake.downloads(), 3, "nothing downloaded twice");
}

#[tokio::test(start_paused = true)]
async fn mail_synced_before_this_version_fills_in_too() {
    let s = setup();
    let old = raw("old@x", "Budget", NOW - 2 * DAY);
    let latest = reply_to(&["old@x"], raw("a@x", "Re: Budget", NOW - DAY));
    s.gmail_message(("INBOX", 1), latest, NOW - DAY, &["\\Inbox"], 5);
    s.gmail_message((ALL_MAIL, 10), old, NOW - 2 * DAY, &[], 5);
    // An IMAP server with an All Mail folder that isn't Gmail: nothing changes.
    s.fake.set_gmail(false);
    s.gmail().await;
    s.synced().await;
    settle().await;
    let id = s.thread_id("Budget");
    assert_eq!(s.engine.thread(&id).unwrap().messages.len(), 1);
    assert!(!s.conversation_calls().iter().any(|c| c.contains("messages")));

    // Gmail: mail already here is looked up, and its conversation filled in.
    s.fake.set_gmail(true);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    assert_eq!(s.engine.thread(&id).unwrap().messages.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn history_fills_in_after_the_inbox_a_few_conversations_at_a_time() {
    let s = setup();
    let total = sync::LINK_PER_PASS + 5;
    for uid in 1..=total as u32 {
        let at = NOW - uid as i64 * 60_000;
        let subject = format!("Re: Hi {uid}");
        let reply = reply_to(&[&format!("p{uid}@x")], raw(&format!("m{uid}@x"), &subject, at));
        s.gmail_message(("INBOX", uid), reply, at, &["\\Inbox"], uid as u64);
    }
    // The oldest conversation has an archived message to fetch.
    let at = NOW - (total as i64 + 1) * 60_000;
    let parent = raw(&format!("p{total}@x"), &format!("Hi {total}"), at);
    s.gmail_message((ALL_MAIL, 1), parent, at, &[], total as u64);
    s.gmail().await;
    s.synced().await;
    for _ in 0..20 {
        settle().await;
    }
    let calls = s.fake.calls();
    let inbox_done = calls.iter().position(|c| c == "fetch INBOX done").unwrap();
    let first_lookup = calls.iter().position(|c| c.starts_with("conversation")).unwrap();
    assert!(first_lookup > inbox_done, "the Inbox comes first: {calls:?}");
    let lookups: Vec<_> =
        s.conversation_calls().into_iter().filter(|c| c.contains("_of INBOX")).collect();
    assert_eq!(
        lookups,
        [
            format!("conversations_of INBOX {}", sync::LINK_PER_PASS),
            "conversations_of INBOX 5".into()
        ]
    );
    let fills: Vec<usize> = s
        .conversation_calls()
        .iter()
        .filter_map(|c| c.strip_prefix(&format!("conversation_messages {ALL_MAIL} ")))
        .map(|n| n.parse().unwrap())
        .collect();
    assert!(fills.iter().all(|n| *n <= sync::FILL_PER_PASS), "{fills:?}");
    assert_eq!(fills.iter().sum::<usize>(), total);
    let threads = s.engine.threads(None).unwrap();
    assert_eq!(threads.len(), total);
    assert_eq!(threads.iter().filter(|t| t.message_count == 2).count(), 1);
}

/// The decision model, faked: answers by subject, and can be "not downloaded".
#[derive(Default)]
struct FakeSorter {
    answers: std::sync::Mutex<HashMap<String, f64>>,
    splits: std::sync::Mutex<HashMap<String, (SplitId, f64)>>,
    pitches: std::sync::Mutex<HashMap<String, crate::Pitch>>,
    pitch_asked: std::sync::Mutex<Vec<String>>,
    deadlines: std::sync::Mutex<HashMap<String, f64>>,
    expects: std::sync::Mutex<HashMap<String, f64>>,
    ready: std::sync::atomic::AtomicBool,
    asked: std::sync::Mutex<Vec<String>>,
    split_asked: std::sync::Mutex<Vec<String>>,
    /// Calls still to panic, like a bug in the model's code would.
    panics: std::sync::atomic::AtomicUsize,
}

/// Panics while `left` says a bug is still to happen, one fewer each time.
fn bug(left: &std::sync::atomic::AtomicUsize) {
    use std::sync::atomic::Ordering::SeqCst;
    let n = left.load(SeqCst);
    left.store(n.saturating_sub(1), SeqCst);
    assert!(n == 0, "a bug in a background model");
}

/// Makes `mutex` poisoned, as a panic while it was held does.
fn poison<T: Send>(mutex: &std::sync::Mutex<T>) {
    std::thread::scope(|s| {
        let held = s.spawn(|| {
            let _held = mutex.lock();
            panic!("a bug while it was held");
        });
        assert!(held.join().is_err());
    });
    assert!(mutex.is_poisoned());
}

impl FakeSorter {
    fn new(answers: &[(&str, f64)]) -> Arc<Self> {
        let s = FakeSorter::default();
        *s.answers.lock().unwrap() = answers.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        s.ready.store(true, std::sync::atomic::Ordering::Relaxed);
        Arc::new(s)
    }

    fn ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl Sorter for FakeSorter {
    async fn needs_reply(&self, email: &moss_store::Undecided) -> Option<f64> {
        if !self.ready() {
            return None;
        }
        bug(&self.panics);
        self.asked.lock().unwrap().push(email.subject.clone());
        self.answers.lock().unwrap().get(&email.subject).copied()
    }

    async fn split(&self, email: &moss_store::Undecided) -> Option<(SplitId, f64)> {
        let known = self.splits.lock().unwrap().get(&email.subject).copied();
        if self.ready() {
            self.split_asked.lock().unwrap().push(email.subject.clone());
        }
        self.ready().then_some(known.unwrap_or((SplitId::Important, 0.5)))
    }

    async fn pitch(&self, email: &moss_store::Undecided) -> Option<crate::Pitch> {
        let known = self.pitches.lock().unwrap().get(&email.subject).copied();
        if self.ready() {
            self.pitch_asked.lock().unwrap().push(email.subject.clone());
        }
        self.ready().then_some(known.unwrap_or(crate::Pitch { pitch: 0.1, wants: 0.1 }))
    }

    async fn deadline(&self, email: &moss_store::Undecided) -> Option<f64> {
        let known = self.deadlines.lock().unwrap().get(&email.subject).copied();
        self.ready().then_some(known.unwrap_or(0.5))
    }

    async fn expects_reply(&self, email: &moss_store::Undecided) -> Option<f64> {
        let known = self.expects.lock().unwrap().get(&email.subject).copied();
        self.ready().then_some(known.unwrap_or(0.5))
    }
}

/// A decision model that only knows whether mail needs a reply.
struct ReplyOnly;

#[async_trait::async_trait]
impl Sorter for ReplyOnly {
    async fn needs_reply(&self, _email: &moss_store::Undecided) -> Option<f64> {
        Some(0.5)
    }
}

impl Setup {
    fn section(&self, subject: &str) -> (Section, Option<u8>) {
        let t = self.engine.threads(None).unwrap().into_iter().find(|t| t.subject == subject).unwrap();
        (t.section, t.ai_sure)
    }
}

const JORDAN: &str = "Jordan Blake <jordan@growthleads.example>";

impl Setup {
    fn from(&self, mailbox: &str, uid: u32, id: &str, sender: &str, subject: &str) {
        let at = NOW - uid as i64 * 60_000;
        let raw = raw_from(id, sender, "Maya Ross <maya.ross@gmail.com>", subject, at);
        self.fake.add(message(mailbox, uid, raw, at));
    }

    fn blocked(&self) -> Vec<String> {
        self.engine.blocked_threads(None, 10).unwrap().into_iter().map(|t| t.subject).collect()
    }

    async fn sync_again(&self) {
        self.events.clear();
        self.engine.sync_now(Some("gmail"));
        self.synced().await;
        settle().await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_decision_model_moves_new_mail_only_when_sure() {
    let s = setup();
    s.inbox(1, "a@x", "Send the deck", NOW - DAY);
    s.inbox(2, "b@x", "Weekly report", NOW - DAY);
    s.inbox(3, "c@x", "Maybe", NOW - DAY);
    s.inbox(4, "d@x", "Old news", NOW - 40 * DAY);
    let sorter = FakeSorter::new(&[("Send the deck", 0.95), ("Weekly report", 0.05), ("Maybe", 0.5)]);
    s.engine.set_sorter(sorter.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;

    // Written to Maya directly, so the rule said Reply for all of them.
    assert_eq!(s.section("Send the deck"), (Section::Reply, Some(95)));
    assert_eq!(s.section("Weekly report"), (Section::Info, Some(95)));
    assert_eq!(s.section("Maybe"), (Section::Reply, None));
    assert_eq!(s.section("Old news"), (Section::Reply, None));
    let mut asked = sorter.asked.lock().unwrap().clone();
    asked.sort();
    assert_eq!(asked, ["Maybe", "Send the deck", "Weekly report"], "each asked once; old mail never");
}

#[tokio::test(start_paused = true)]
async fn a_decision_model_that_panics_starts_again_by_itself_after_a_pause() {
    let s = setup();
    s.inbox(1, "a@x", "Send the deck", NOW - DAY);
    let sorter = FakeSorter::new(&[("Send the deck", 0.95)]);
    sorter.panics.store(1, std::sync::atomic::Ordering::SeqCst);
    s.engine.set_sorter(sorter.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert_eq!(s.section("Send the deck"), (Section::Reply, None));
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(s.section("Send the deck"), (Section::Reply, Some(95)));
}

#[tokio::test(start_paused = true)]
async fn sorting_waits_until_the_model_can_run() {
    let s = setup();
    s.inbox(1, "b@x", "Weekly report", NOW - DAY);
    let sorter = FakeSorter::new(&[("Weekly report", 0.1)]);
    sorter.ready.store(false, std::sync::atomic::Ordering::Relaxed);
    s.engine.set_sorter(sorter.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert_eq!(s.section("Weekly report"), (Section::Reply, None));

    sorter.ready.store(true, std::sync::atomic::Ordering::Relaxed);
    s.events.clear();
    s.engine.wake_sorter();
    settle().await;
    assert_eq!(s.section("Weekly report"), (Section::Info, Some(90)));
    assert!(s.events.all().contains(&Event::MailChanged { account_id: "gmail".into() }));
}

#[tokio::test(start_paused = true)]
async fn sorting_stops_when_the_database_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("moss.db");
    let s = setup_with(dir, FakeProvider::new(), Arc::new(MemorySecretStore::new()));
    s.inbox(1, "b@x", "Weekly report", NOW - DAY);
    s.inbox(2, "c@x", "Send the deck", NOW - 2 * DAY);
    s.gmail().await;
    s.synced().await;
    // Another connection breaks the database under the engine.
    let other = rusqlite::Connection::open(&db).unwrap();
    other
        .execute_batch(
            "CREATE TRIGGER full BEFORE INSERT ON ai_decisions BEGIN SELECT RAISE(FAIL, 'disk full'); END;",
        )
        .unwrap();
    let sorter = FakeSorter::new(&[("Weekly report", 0.1), ("Send the deck", 0.9)]);
    s.engine.shared.sort_waiting(sorter.as_ref()).await;
    assert_eq!(*sorter.split_asked.lock().unwrap(), ["Weekly report"], "stops at the first failed save");
    assert!(sorter.asked.lock().unwrap().is_empty(), "and asks nothing else");

    other.execute_batch("DROP TABLE ai_decisions").unwrap();
    sorter.split_asked.lock().unwrap().clear();
    s.engine.shared.sort_waiting(sorter.as_ref()).await;
    assert!(sorter.split_asked.lock().unwrap().is_empty(), "nothing to ask when mail can’t be listed");
    assert!(sorter.asked.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn blocking_moves_their_mail_to_the_blocked_folder_and_unblocking_brings_it_back() {
    let s = setup();
    s.from("INBOX", 1, "a@x", JORDAN, "Quick question");
    s.from("INBOX", 2, "b@x", "Alex <alex@seo-boost.example>", "Ranking");
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Quick question");
    assert_eq!(s.engine.thread(&id).unwrap().summary.sender, SenderStatus::New);

    s.engine.block_sender(&id, false).unwrap();
    assert_eq!(s.subjects(), ["Ranking"]);
    assert_eq!(s.blocked(), ["Quick question"]);
    assert_eq!(s.engine.thread(&id).unwrap().summary.sender, SenderStatus::Blocked);
    settle().await;
    // No Blocked folder yet: Moss makes one, like Spark does.
    let changes = s.fake.changes();
    assert_eq!(
        changes[changes.len() - 2..],
        ["create Blocked", "Move { to: \"Blocked\" } INBOX 1:1"]
    );
    assert_eq!(s.engine.shared.store().provider_ids("gmail", "INBOX").unwrap(), ["1:2"]);

    // The server now has it in Blocked; the next sync finds it there.
    s.from("Blocked", 7, "a@x", JORDAN, "Quick question");
    s.sync_again().await;
    assert_eq!(s.blocked(), ["Quick question"]);

    s.engine.unblock_sender(&id).unwrap();
    assert_eq!(s.subjects(), ["Quick question", "Ranking"]);
    assert!(s.blocked().is_empty());
    assert_eq!(s.engine.thread(&id).unwrap().summary.sender, SenderStatus::Known);
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "Move { to: \"INBOX\" } Blocked 1:7");
}

#[tokio::test(start_paused = true)]
async fn new_mail_from_a_blocked_sender_goes_straight_to_blocked() {
    let s = setup();
    s.from("INBOX", 1, "a@x", JORDAN, "Quick question");
    s.gmail().await;
    s.synced().await;
    s.engine.block_sender(&s.thread_id("Quick question"), true).unwrap();
    settle().await;
    s.from("INBOX", 3, "c@x", "Sam <sam@growthleads.example>", "Following up");
    s.sync_again().await;
    assert!(s.subjects().is_empty());
    assert_eq!(s.blocked(), ["Quick question", "Following up"]);
    assert_eq!(s.fake.changes().last().unwrap(), "Move { to: \"Blocked\" } INBOX 1:3");
    assert_eq!(s.fake.changes().iter().filter(|c| c.starts_with("create")).count(), 1);
}

#[tokio::test(start_paused = true)]
async fn mail_spark_blocked_blocks_its_sender_here_too() {
    let s = setup();
    s.fake.add_mailbox("Blocked");
    s.from("Blocked", 1, "a@x", JORDAN, "Quick question");
    s.from("INBOX", 2, "b@x", JORDAN, "Following up");
    s.from("INBOX", 3, "c@x", "Alex <alex@seo-boost.example>", "Ranking");
    s.gmail().await;
    s.synced().await;
    s.sync_again().await;
    assert_eq!(s.subjects(), ["Ranking"]);
    assert_eq!(s.blocked(), ["Quick question", "Following up"]);
    assert!(s.fake.changes().contains(&"Move { to: \"Blocked\" } INBOX 1:2".to_string()));
    assert!(!s.fake.changes().iter().any(|c| c.starts_with("create")));
}

#[tokio::test(start_paused = true)]
async fn accepting_a_new_sender_stops_the_question_and_leaves_the_server_alone() {
    let s = setup();
    s.fake.add_mailbox("Blocked");
    s.from("INBOX", 1, "a@x", JORDAN, "Quick question");
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Quick question");
    let before = s.fake.changes().len();
    s.engine.accept_sender(&id, false).unwrap();
    settle().await;
    assert_eq!(s.engine.thread(&id).unwrap().summary.sender, SenderStatus::Known);
    assert_eq!(s.fake.changes().len(), before);
    // Unblocking someone who wasn't blocked just accepts them.
    s.engine.unblock_sender(&id).unwrap();
    settle().await;
    assert_eq!(s.fake.changes().len(), before);
    assert!(matches!(s.engine.accept_sender("999", true), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.block_sender("999", false), Err(Error::UnknownThread(_))));
    assert!(matches!(s.engine.unblock_sender("999"), Err(Error::UnknownThread(_))));
}

#[tokio::test(start_paused = true)]
async fn accepting_a_domain_accepts_everyone_there_and_brings_back_its_blocked_mail() {
    let s = setup();
    s.from("INBOX", 1, "a@x", JORDAN, "Quick question");
    s.from("INBOX", 2, "b@x", "Sam <sam@growthleads.example>", "Following up");
    s.gmail().await;
    s.synced().await;
    let sam = s.thread_id("Following up");
    s.engine.block_sender(&sam, true).unwrap();
    settle().await;
    // The server now has their mail in Blocked; the next sync finds it there.
    s.from("Blocked", 8, "b@x", "Sam <sam@growthleads.example>", "Following up");
    s.sync_again().await;
    assert_eq!(s.blocked(), ["Quick question", "Following up"]);
    s.engine.accept_sender(&sam, true).unwrap();
    settle().await;
    assert!(s.blocked().is_empty());
    let jordan = s.thread_id("Quick question");
    assert_eq!(s.engine.thread(&jordan).unwrap().summary.sender, SenderStatus::Known);
    assert_eq!(s.fake.changes().last().unwrap(), "Move { to: \"INBOX\" } Blocked 1:8");
}

#[tokio::test(start_paused = true)]
async fn lists_sender_rules_and_forgetting_a_block_moves_their_mail_back_on_the_server() {
    let s = setup();
    s.from("INBOX", 1, "a@x", JORDAN, "Quick question");
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Quick question");
    s.engine.block_sender(&id, false).unwrap();
    settle().await;
    s.from("Blocked", 7, "a@x", JORDAN, "Quick question");
    s.sync_again().await;
    let rules = s.engine.sender_rules().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!((rules[0].sender.as_str(), rules[0].blocked), ("jordan@growthleads.example", true));

    s.engine.forget_sender_rule("gmail", "jordan@growthleads.example").unwrap();
    settle().await;
    assert!(s.engine.sender_rules().unwrap().is_empty());
    assert_eq!(s.subjects(), ["Quick question"]);
    assert_eq!(s.engine.thread(&id).unwrap().summary.sender, SenderStatus::New);
    assert_eq!(s.fake.changes().last().unwrap(), "Move { to: \"INBOX\" } Blocked 1:7");
}

#[tokio::test(start_paused = true)]
async fn a_public_domain_cannot_be_blocked_and_server_failures_are_reported() {
    let s = setup();
    s.from("INBOX", 1, "a@x", "Pat <pat@gmail.com>", "Hello");
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Hello");
    let err = s.engine.block_sender(&id, true).unwrap_err();
    assert_eq!(
        err.to_string(),
        "gmail.com is a public mail service, so it can't be blocked or accepted as a whole"
    );
    assert!(matches!(s.engine.accept_sender(&id, true), Err(Error::PublicDomain(_))));
    assert_eq!(s.subjects(), ["Hello"]);

    s.events.clear();
    s.fake.fail_next(moss_providers::Error::Protocol("NO [NOPERM] nope".into()));
    s.engine.block_sender(&id, false).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::Server, .. })).await;
    assert!(s.subjects().is_empty(), "blocked here even though the server said no");

    s.fake.add_mailbox("Blocked");
    s.engine.shared.folders.lock().unwrap().clear();
    s.from("Blocked", 9, "a@x", "Pat <pat@gmail.com>", "Hello");
    s.sync_again().await;
    s.events.clear();
    s.fake.fail_next(moss_providers::Error::Protocol("NO [NOPERM] nope".into()));
    s.engine.unblock_sender(&id).unwrap();
    s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::Server, .. })).await;
    assert_eq!(s.subjects(), ["Hello"]);

    // Signed out: blocking still works here, and the UI hears why the server didn't follow.
    s.engine.shared.forget_provider("gmail");
    s.secrets.delete(&s.engine.shared.connection("gmail").unwrap().secret).unwrap();
    for block in [true, false] {
        s.events.clear();
        if block {
            s.engine.block_sender(&id, false).unwrap();
        } else {
            s.engine.unblock_sender(&id).unwrap();
        }
        s.events.wait_for(|e| matches!(e, Event::SyncError { problem: Problem::SignedOut, .. })).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_server_without_folders_blocks_only_here() {
    let s = setup();
    s.fake.local_only();
    s.from("INBOX", 1, "a@x", JORDAN, "Quick question");
    s.gmail().await;
    s.synced().await;
    let before = s.fake.changes().len();
    let id = s.thread_id("Quick question");
    s.engine.block_sender(&id, false).unwrap();
    settle().await;
    assert!(s.subjects().is_empty());
    // No Blocked folder to bring anything back from, either.
    s.engine.unblock_sender(&id).unwrap();
    settle().await;
    assert_eq!(s.subjects(), ["Quick question"]);
    assert_eq!(s.fake.changes().len(), before);
}

#[tokio::test(start_paused = true)]
async fn storage_says_where_the_space_goes_and_follows_its_settings() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let files = tempfile::tempdir().unwrap();
    let info = s.engine.storage(files.path()).unwrap();
    assert_eq!((info.offline, info.cache_limit, info.rebuilding), (Offline::Text, DEFAULT_CACHE_LIMIT, false));
    assert!(info.attachment_text, "reads attachments unless turned off");
    assert!(info.usage.offline > 0 && info.usage.index > 0);
    assert_eq!(info.usage.cache, 0);
    // An opened file counts as cache, as the bytes and as the saved file.
    let m = s.message_with_subject("Q4 signed");
    s.engine.attachment_file(&m.id, 1, files.path()).await.unwrap();
    assert_eq!(s.engine.storage(files.path()).unwrap().usage.cache, 14);
    // A smaller limit trims at once.
    s.engine.set_cache_limit(7).unwrap();
    assert_eq!(s.engine.storage(files.path()).unwrap().cache_limit, 7);
    assert_eq!(s.engine.storage(files.path()).unwrap().usage.cache, 7 + 7);
    assert_eq!(s.engine.clear_cache(files.path()).unwrap(), 14);
    assert!(!files.path().exists());
    // Keeping recent mail only makes older bodies cache, which goes when over the limit.
    s.engine.set_cache_limit(0).unwrap();
    s.engine.set_offline(Offline::Recent).unwrap();
    assert_eq!(s.engine.storage(files.path()).unwrap().offline, Offline::Recent);
    s.engine.set_reading_attachments(false).unwrap();
    assert!(!s.engine.storage(files.path()).unwrap().attachment_text);
}

#[tokio::test(start_paused = true)]
async fn rebuilding_the_index_runs_in_the_background() {
    let s = setup();
    s.fake.add(message("INBOX", 5, raw("a@x", "Pineapple pizza", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    s.engine.rebuild_index().unwrap();
    settle().await;
    let files = tempfile::tempdir().unwrap();
    assert!(!s.engine.storage(files.path()).unwrap().rebuilding);
    assert_eq!(s.engine.search("pineapple", None, 10).unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn preloading_fetches_the_bodies_that_arent_here() {
    let s = setup();
    let old = NOW - 400 * DAY;
    s.fake.add(message("INBOX", 5, raw("a@x", "Old news", old), old));
    s.fake.add(message("INBOX", 6, raw("b@x", "New news", NOW), NOW));
    s.engine.set_offline(Offline::Recent).unwrap();
    s.gmail().await;
    s.synced().await;
    let threads: Vec<String> = s.engine.threads(None).unwrap().into_iter().map(|t| t.id).collect();
    assert!(s.message_with_subject("Old news").headers_only);
    s.engine.preload(&threads).await.unwrap();
    assert!(!s.message_with_subject("Old news").headers_only);
    let opened: Vec<String> = s.fake.calls().into_iter().filter(|c| c.starts_with("fetch_message")).collect();
    assert_eq!(opened, ["fetch_message INBOX 1:5"], "only the missing body");
    // Unknown threads and failures don't stop it.
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    s.engine.preload(&["none".into()]).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn sync_stops_for_the_day_at_gmails_download_limit() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    let today = NOW / DAY;
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    let used = |day: i64| format!("INSERT OR REPLACE INTO settings (key, value) VALUES ('sync.downloaded:gmail', '{day}:{}')", sync::DAILY_DOWNLOAD);
    db.execute_batch(&used(today)).unwrap();
    s.gmail().await;
    settle().await;
    assert!(s.subjects().is_empty(), "nothing more today");
    // The next day it carries on, and counts what it fetched.
    db.execute_batch(&used(today - 1)).unwrap();
    s.engine.sync_now(Some("gmail"));
    settle().await;
    assert_eq!(s.subjects(), ["Budget"]);
    let counted: String = db.query_row("SELECT value FROM settings WHERE key = 'sync.downloaded:gmail'", [], |r| r.get(0)).unwrap();
    let bytes: u64 = counted.split_once(':').unwrap().1.parse().unwrap();
    assert_eq!(counted.split_once(':').unwrap().0, today.to_string());
    assert!(bytes > 0 && bytes < 10_000, "{bytes}");
}

#[tokio::test(start_paused = true)]
async fn reaching_gmails_download_limit_leaves_conversations_to_fill_in_tomorrow() {
    let s = setup();
    let re = "Re: Quote";
    let q1 = raw_from("q1@x", MAYA, DANIEL, "Quote", NOW - 3 * DAY);
    let r1 = reply_to(&["q1@x"], raw("r1@x", re, NOW - 2 * DAY));
    let r2 = reply_to(&["q1@x", "r1@x"], raw("r2@x", re, NOW - DAY));
    s.gmail_message(("INBOX", 1), r2.clone(), NOW - DAY, &["\\Inbox"], 77);
    s.gmail_message((ALL_MAIL, 10), q1, NOW - 3 * DAY, &["\\Sent"], 77);
    s.gmail_message((ALL_MAIL, 11), r1, NOW - 2 * DAY, &[], 77);
    s.gmail_message((ALL_MAIL, 12), r2, NOW - DAY, &["\\Inbox"], 77);
    // One byte short of today's limit: the inbox comes, and that uses up the day.
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    let almost = format!(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('sync.downloaded:gmail', '{}:{}')",
        NOW / DAY,
        sync::DAILY_DOWNLOAD - 1
    );
    db.execute_batch(&almost).unwrap();
    s.gmail().await;
    s.synced().await;
    settle().await;
    let thread = s.engine.thread(&s.thread_id("Quote")).unwrap();
    assert_eq!(thread.messages.len(), 1, "the rest of the conversation waits for tomorrow");
}

impl Setup {
    fn row(&self, subject: &str) -> moss_core::ThreadSummary {
        self.engine.threads(None).unwrap().into_iter().find(|t| t.subject == subject).unwrap()
    }

    /// An email Maya sent from Gmail, filed in Sent Mail.
    fn sent(&self, uid: u32, id: &str, subject: &str, at: i64) {
        let raw = raw_from(id, "Maya Ross <maya.ross@gmail.com>", "Dana Wu <dana@acme.co>", subject, at);
        self.fake.add(message(SENT_MAIL, uid, raw, at));
    }
}

#[tokio::test(start_paused = true)]
async fn without_the_model_rules_find_deadlines_and_wait_on_your_questions() {
    let s = setup();
    s.inbox(1, "a@x", "Contract due 2026-11-13", NOW - DAY);
    s.inbox(2, "b@x", "Hello", NOW - DAY);
    s.sent(1, "s@x", "Can we meet?", NOW - DAY);
    s.sent(2, "t@x", "Thanks", NOW - DAY);
    s.sent(3, "o@x", "Old question?", NOW - 10 * DAY);
    let sorter = FakeSorter::new(&[]);
    sorter.ready.store(false, std::sync::atomic::Ordering::Relaxed);
    s.engine.set_sorter(sorter);
    s.gmail().await;
    s.synced().await;
    settle().await;

    let due = s.row("Contract due 2026-11-13").due.unwrap();
    assert_eq!(due.date, "2026-11-13");
    assert_eq!(due.why, moss_core::Why { by: moss_core::DecidedBy::Rule, sure: None, evidence: Some("due 2026-11-13".into()) });
    assert_eq!(s.row("Hello").due, None);
    let waiting = s.row("Can we meet?");
    assert_eq!(waiting.section, Section::Later);
    assert_eq!(waiting.waiting.unwrap().until, NOW - DAY + sort::WAIT);
    assert!(!s.subjects().contains(&"Thanks".to_string()), "nothing to wait for");
    assert!(!s.subjects().contains(&"Old question?".to_string()), "sent too long ago");
}

#[tokio::test(start_paused = true)]
async fn the_model_moves_splits_reads_deadlines_and_waits_only_when_sure() {
    let s = setup();
    s.inbox(1, "a@x", "Lunch order", NOW - DAY);
    s.inbox(2, "b@x", "Sign the contract", NOW - DAY);
    s.inbox(3, "c@x", "Maybe due 2026-11-20", NOW - DAY);
    s.sent(1, "s@x", "Lunch", NOW - DAY);
    let sorter = FakeSorter::new(&[("Sign the contract", 0.5), ("Maybe due 2026-11-20", 0.5)]);
    sorter.splits.lock().unwrap().insert("Lunch order".into(), (SplitId::Receipts, 0.9));
    sorter.deadlines.lock().unwrap().insert("Sign the contract".into(), 0.9);
    sorter.expects.lock().unwrap().insert("Lunch".into(), 0.95);
    s.engine.set_sorter(sorter.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;

    let order = s.engine.threads(None).unwrap().into_iter().find(|t| t.subject == "Lunch order").unwrap();
    assert_eq!(order.split, SplitId::Receipts);
    assert_eq!(order.split_why.unwrap().sure, Some(90));
    // Sure of a deadline, but the email names no day: no chip.
    assert_eq!(s.row("Sign the contract").due, None);
    // Unsure: the rule's day, said as the rule's.
    assert_eq!(s.row("Maybe due 2026-11-20").due.unwrap().why.by, moss_core::DecidedBy::Rule);
    let lunch = s.row("Lunch").waiting.unwrap();
    assert_eq!(lunch.why, moss_core::Why { by: moss_core::DecidedBy::Ai, sure: Some(95), evidence: None });
    // Only Important mail is asked whether it needs a reply.
    let mut asked = sorter.asked.lock().unwrap().clone();
    asked.sort();
    assert_eq!(asked, ["Maybe due 2026-11-20", "Sign the contract"]);
}

/// `raw` with mailing-list headers: the rules hold it in Newsletters until the model says.
fn listed(id: &str, subject: &str, at: i64) -> Vec<u8> {
    let mut out = b"List-Unsubscribe: <mailto:leave@list.example>\r\n".to_vec();
    out.extend(raw(id, subject, at));
    out
}

#[tokio::test(start_paused = true)]
async fn the_model_decides_whether_list_mail_is_a_newsletter_but_your_move_wins() {
    let s = setup();
    for (uid, subject) in [(1, "Build failed"), (2, "Weekly digest"), (3, "Maybe news"), (4, "Moved by you")] {
        s.fake.add(message("INBOX", uid, listed(&format!("{uid}@x"), subject, NOW - DAY), NOW - DAY));
    }
    let sorter = FakeSorter::new(&[]);
    sorter.ready.store(false, std::sync::atomic::Ordering::Relaxed);
    for (subject, answer) in [
        ("Build failed", (SplitId::Other, 0.55)),
        ("Weekly digest", (SplitId::Newsletters, 0.95)),
        ("Maybe news", (SplitId::Newsletters, 0.45)),
        ("Moved by you", (SplitId::Other, 0.99)),
    ] {
        sorter.splits.lock().unwrap().insert(subject.into(), answer);
    }
    s.engine.set_sorter(sorter.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    let split = |subject: &str| {
        let t = s.row(subject);
        (t.split, t.split_why.and_then(|w| w.sure))
    };
    // No model yet: the list headers hold all of it in Newsletters.
    for subject in ["Build failed", "Weekly digest", "Maybe news", "Moved by you"] {
        assert_eq!(split(subject), (SplitId::Newsletters, None), "{subject}");
    }
    s.engine.move_thread(&s.thread_id("Moved by you"), Some(SplitId::Newsletters)).unwrap();

    sorter.ready.store(true, std::sync::atomic::Ordering::Relaxed);
    s.engine.wake_sorter();
    settle().await;
    // Not sure, but the model's pick: a notice, not a newsletter.
    assert_eq!(split("Build failed"), (SplitId::Other, Some(55)));
    assert_eq!(split("Weekly digest"), (SplitId::Newsletters, None));
    assert_eq!(split("Maybe news"), (SplitId::Newsletters, None));
    assert_eq!(split("Moved by you"), (SplitId::Newsletters, None));
    let mut asked = sorter.split_asked.lock().unwrap().clone();
    asked.sort();
    assert_eq!(asked, ["Build failed", "Maybe news", "Weekly digest"], "each once; never what you moved");
    assert!(sorter.asked.lock().unwrap().is_empty(), "none of it is asked whether it needs a reply");
}

#[tokio::test(start_paused = true)]
async fn a_strangers_sales_pitch_goes_to_newsletters_but_never_mail_from_someone_you_write_to() {
    let s = setup();
    // Maya has written to Dana before; everyone else is new to her.
    s.sent(1, "s@x", "Lunch?", NOW - 3 * DAY);
    s.from("INBOX", 1, "p@x", "Leo Grant <leo@clipcast.example>", "Faster product videos");
    s.from("INBOX", 2, "c@x", "Joanna Field <joanna@school.example>", "Licences for our class");
    s.from("INBOX", 3, "d@x", "Dana Wu <dana@acme.co>", "Our offer");
    s.from("INBOX", 4, "o@x", "Brightsoft <noreply@brightsoft.example>", "An invitation to partner");
    s.from("INBOX", 5, "n@x", "Lumen <noreply@lumen.example>", "New sign-in");
    s.fake.add(message("INBOX", 6, listed("6@x", "Weekly digest", NOW - DAY), NOW - DAY));
    let sorter = FakeSorter::new(&[]);
    let pitch = |pitch, wants| crate::Pitch { pitch, wants };
    for (subject, answer) in [
        ("Faster product videos", pitch(0.9, 0.2)),
        // Just as sure the sender wants to buy from Maya: a would-be customer.
        ("Licences for our class", pitch(0.9, 0.9)),
        ("Our offer", pitch(0.95, 0.1)),
        ("An invitation to partner", pitch(0.9, 0.1)),
        ("New sign-in", pitch(0.2, 0.1)),
        ("Weekly digest", pitch(0.95, 0.1)),
    ] {
        sorter.pitches.lock().unwrap().insert(subject.into(), answer);
    }
    sorter.splits.lock().unwrap().insert("Weekly digest".into(), (SplitId::Newsletters, 0.9));
    s.engine.set_sorter(sorter.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    let split = |subject: &str| {
        let t = s.row(subject);
        (t.split, t.split_why.and_then(|w| w.sure))
    };
    assert_eq!(split("Faster product videos"), (SplitId::Newsletters, Some(90)));
    assert_eq!(split("An invitation to partner"), (SplitId::Newsletters, Some(90)));
    assert_eq!(split("Licences for our class"), (SplitId::Important, None));
    assert_eq!(split("Our offer"), (SplitId::Important, None), "Maya writes to Dana");
    assert_eq!(split("New sign-in"), (SplitId::Other, None));
    assert_eq!(split("Weekly digest"), (SplitId::Newsletters, None));
    let sorted = |list: &std::sync::Mutex<Vec<String>>| {
        let mut list = list.lock().unwrap().clone();
        list.sort();
        list
    };
    assert_eq!(sorted(&sorter.pitch_asked), ["An invitation to partner", "Faster product videos", "Licences for our class", "New sign-in"]);
    // Mail in Other is only asked whether it is a pitch.
    assert_eq!(sorted(&sorter.split_asked), ["Faster product videos", "Licences for our class", "Our offer", "Weekly digest"]);
}

#[tokio::test(start_paused = true)]
async fn mail_in_other_waits_for_a_model_that_can_say_whether_it_is_a_pitch() {
    let s = setup();
    s.from("INBOX", 1, "o@x", "Brightsoft <noreply@brightsoft.example>", "An invitation to partner");
    s.engine.set_sorter(Arc::new(ReplyOnly));
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert_eq!(s.row("An invitation to partner").split, SplitId::Other);
    assert!(s.engine.shared.store().undecided(moss_store::SPLIT, 0, 10, true).unwrap().len() == 1, "asked again later");
}

#[tokio::test(start_paused = true)]
async fn the_model_looks_again_at_what_rules_decided_once_it_can_run() {
    let s = setup();
    s.inbox(1, "a@x", "Contract due 2026-11-13", NOW - DAY);
    let sorter = FakeSorter::new(&[]);
    sorter.ready.store(false, std::sync::atomic::Ordering::Relaxed);
    sorter.deadlines.lock().unwrap().insert("Contract due 2026-11-13".into(), 0.05);
    s.engine.set_sorter(sorter.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert!(s.row("Contract due 2026-11-13").due.is_some());

    sorter.ready.store(true, std::sync::atomic::Ordering::Relaxed);
    s.engine.wake_sorter();
    settle().await;
    assert_eq!(s.row("Contract due 2026-11-13").due, None, "the model is sure there's no deadline");
}

#[tokio::test(start_paused = true)]
async fn a_model_that_only_knows_replies_leaves_the_rest_to_rules() {
    let s = setup();
    s.inbox(1, "a@x", "Contract due 2026-11-13", NOW - DAY);
    s.sent(1, "s@x", "Can we meet?", NOW - DAY);
    s.engine.set_sorter(Arc::new(ReplyOnly));
    s.gmail().await;
    s.synced().await;
    settle().await;
    let row = s.row("Contract due 2026-11-13");
    assert_eq!(row.due.unwrap().why.by, moss_core::DecidedBy::Rule);
    assert_eq!((row.section, row.ai_sure), (Section::Reply, None));
    assert_eq!(s.row("Can we meet?").waiting.unwrap().why.by, moss_core::DecidedBy::Rule);
}

/// An embedding model that puts mail about trips on one axis and the rest on another.
#[derive(Default)]
struct FakeEmbedder {
    down: std::sync::atomic::AtomicBool,
    calls: std::sync::Mutex<usize>,
    panics: std::sync::atomic::AtomicUsize,
}

fn meaning(text: &str) -> Vec<f32> {
    let trip = ["flight", "trip", "travel", "lisbon"].iter().any(|w| text.to_lowercase().contains(w));
    if trip { vec![1.0, 0.0] } else { vec![0.0, 1.0] }
}

#[async_trait::async_trait]
impl Embedder for FakeEmbedder {
    fn model(&self) -> String {
        "fake-embed".into()
    }

    async fn documents(&self, emails: &[moss_store::ToEmbed]) -> Option<Vec<Vec<f32>>> {
        bug(&self.panics);
        *self.calls.lock().unwrap() += 1;
        if self.down.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        Some(emails.iter().map(|e| meaning(&format!("{} {}", e.subject, e.text))).collect())
    }

    async fn query(&self, text: &str) -> Option<Vec<f32>> {
        (!self.down.load(std::sync::atomic::Ordering::Relaxed)).then(|| meaning(text))
    }
}

#[tokio::test(start_paused = true)]
async fn an_embedding_model_that_panics_starts_again_by_itself_after_a_pause() {
    let s = setup();
    s.inbox(1, "a@x", "Your flight to Lisbon", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    let embedder = Arc::new(FakeEmbedder::default());
    embedder.panics.store(1, std::sync::atomic::Ordering::SeqCst);
    s.engine.set_embedder(embedder.clone());
    settle().await;
    assert!(s.engine.find("travel plans", None, 10).await.unwrap().is_empty());
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(s.engine.find("travel plans", None, 10).await.unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn search_finds_mail_by_meaning_once_the_model_has_indexed_it() {
    let s = setup();
    s.inbox(1, "a@x", "Your flight to Lisbon", NOW - DAY);
    s.inbox(2, "b@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    // Without the model, words only.
    assert!(s.engine.find("travel plans", None, 10).await.unwrap().is_empty());
    let embedder = Arc::new(FakeEmbedder::default());
    s.engine.set_embedder(embedder.clone());
    settle().await;
    let hits = s.engine.find("travel plans", None, 10).await.unwrap();
    let subjects: Vec<&str> = hits.iter().map(|h| h.thread.subject.as_str()).collect();
    assert_eq!(subjects, ["Your flight to Lisbon"]);
    // Word matches come first, and a thread only once.
    let hits = s.engine.find("lisbon", None, 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    // New mail gets its vector when it arrives.
    s.inbox(3, "c@x", "Trip receipts", NOW);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    assert_eq!(s.engine.find("travel plans", None, 10).await.unwrap().len(), 2);
    // A model that's down leaves search to the words, and indexing waits.
    embedder.down.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(s.engine.find("travel plans", None, 10).await.unwrap().is_empty());
    s.inbox(4, "d@x", "Hotel", NOW);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    let calls = *embedder.calls.lock().unwrap();
    s.engine.wake_embedder();
    settle().await;
    assert_eq!(*embedder.calls.lock().unwrap(), calls + 1, "it tries again when woken");
    assert!(s.engine.find("", None, 10).await.unwrap().is_empty());
}

struct WrongEmbedder;

#[tokio::test(start_paused = true)]
async fn semantic_search_supports_memory_previews_and_reports_snapshot_failure() {
    let s = setup();
    *s.engine.shared.store() = Store::open_in_memory().unwrap();
    s.inbox(1, "memory@example.test", "Your flight to Lisbon", NOW);
    s.gmail().await; s.synced().await;
    *s.engine.shared.embedder.lock().unwrap() = Some(Arc::new(FakeEmbedder::default()));
    let input = s.engine.shared.store().unembedded("fake-embed",1).unwrap().remove(0);
    s.engine.shared.store().set_embedding(input.message_id,"fake-embed",&meaning("flight")).unwrap();
    let hits = s.engine.find("travel plans",Some("gmail"),10).await.unwrap();
    assert_eq!(hits.len(),1);
    assert_eq!(hits[0].thread.subject,"Your flight to Lisbon");

    let disk = setup();
    *disk.engine.shared.embedder.lock().unwrap() = Some(Arc::new(FakeEmbedder::default()));
    rusqlite::Connection::open(disk._dir.path().join("moss.db")).unwrap().pragma_update(None,"user_version",0).unwrap();
    let error = disk.engine.find("travel plans",None,10).await.unwrap_err();
    assert!(matches!(error,Error::Invalid(_)));
    assert_eq!(error.to_string(),"The search database schema changed. Reopen Moss before searching.");
}

#[async_trait::async_trait]
impl Embedder for WrongEmbedder {
    fn model(&self) -> String {
        "wrong".into()
    }

    async fn documents(&self, _emails: &[moss_store::ToEmbed]) -> Option<Vec<Vec<f32>>> {
        Some(vec![])
    }

    async fn query(&self, _text: &str) -> Option<Vec<f32>> {
        None
    }
}

#[tokio::test(start_paused = true)]
async fn an_embedding_model_that_answers_wrongly_stops_indexing() {
    let s = setup();
    s.inbox(1, "a@x", "Your flight", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    s.engine.set_embedder(Arc::new(WrongEmbedder));
    settle().await;
    assert!(s.engine.find("flight plans", None, 10).await.unwrap().len() <= 1);
}

#[derive(Default)]
struct FakeSummarizer {
    down: std::sync::atomic::AtomicBool,
    read: std::sync::Mutex<Vec<String>>,
    panics: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Summarizer for FakeSummarizer {
    fn model(&self) -> String {
        "fake-writer".into()
    }

    async fn summarize(&self, c: &moss_store::ToSummarize) -> Option<String> {
        if self.down.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        bug(&self.panics);
        self.read.lock().unwrap().push(c.subject.clone());
        Some(format!("{} messages about {}.", c.messages.len(), c.subject))
    }

    async fn summarize_as(&self, c: &moss_store::ToSummarize, request: &moss_core::SummaryRequest) -> Option<moss_core::SummaryContent> {
        if self.down.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        self.read.lock().unwrap().push(format!("{} {}", request.kind.name(), c.subject));
        Some(moss_core::SummaryContent::Text { text: format!("{} of {}", request.kind.name(), c.subject) })
    }
}

#[tokio::test(start_paused = true)]
async fn a_summary_model_that_panics_starts_again_by_itself_after_a_pause() {
    let s = setup();
    s.fake.add(message("INBOX", 1, raw("q1@x", "Quote", NOW - 2 * DAY), NOW - 2 * DAY));
    s.fake.add(message("INBOX", 2, reply_to(&["q1@x"], raw("q2@x", "Re: Quote", NOW - DAY)), NOW - DAY));
    let summarizer = Arc::new(FakeSummarizer::default());
    summarizer.panics.store(1, std::sync::atomic::Ordering::SeqCst);
    s.engine.set_summarizer(summarizer.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    assert_eq!(s.engine.threads(None).unwrap()[0].ai_summary, None);
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(s.engine.threads(None).unwrap()[0].ai_summary.as_deref(), Some("2 messages about Quote."));
}

#[tokio::test(start_paused = true)]
async fn conversations_that_matter_get_a_summary_and_a_new_one_after_new_mail() {
    let s = setup();
    let first = raw("q1@x", "Quote", NOW - 2 * DAY);
    s.fake.add(message("INBOX", 1, first, NOW - 2 * DAY));
    s.fake.add(message("INBOX", 2, reply_to(&["q1@x"], raw("q2@x", "Re: Quote", NOW - DAY)), NOW - DAY));
    let summarizer = Arc::new(FakeSummarizer::default());
    s.engine.set_summarizer(summarizer.clone());
    s.gmail().await;
    s.synced().await;
    settle().await;
    let thread = s.engine.threads(None).unwrap().remove(0);
    assert_eq!(thread.ai_summary.as_deref(), Some("2 messages about Quote."));
    s.fake.add(message("INBOX", 3, reply_to(&["q1@x", "q2@x"], raw("q3@x", "Re: Quote", NOW)), NOW));
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    assert_eq!(s.engine.threads(None).unwrap()[0].ai_summary.as_deref(), Some("3 messages about Quote."));
    // While it can't run, nothing changes until it's woken again.
    summarizer.down.store(true, std::sync::atomic::Ordering::Relaxed);
    s.engine.wake_summarizer();
    settle().await;
    assert_eq!(summarizer.read.lock().unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn opening_mail_fetches_the_images_its_html_shows() {
    let s = setup();
    s.fake.add(message("INBOX", 5, with_files("q4@x", "Q4 signed", NOW), NOW));
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Q4 signed");
    assert!(m.images_missing, "files are left on the server");
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    assert!(!s.engine.load_message(&m.id).await.unwrap(), "offline: it stays missing");
    assert!(s.engine.load_message(&m.id).await.unwrap());
    let m = s.message_with_subject("Q4 signed");
    assert!(!m.images_missing);
    assert!(m.body_html.unwrap().contains("data:image/png;base64,"));
    // Nothing left to fetch.
    assert!(!s.engine.load_message(&m.id).await.unwrap());
    // Preloading fetches them too.
    s.fake.add(message("INBOX", 6, with_files("q5@x", "Q5 signed", NOW), NOW));
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    let thread = s.engine.search("\"Q5 signed\"", None, 1).unwrap().remove(0).thread.id;
    s.engine.preload(&[thread]).await.unwrap();
    assert!(!s.message_with_subject("Q5 signed").images_missing);
}

#[tokio::test(start_paused = true)]
async fn indexing_by_meaning_and_summaries_stop_when_the_database_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("moss.db");
    let s = setup_with(dir, FakeProvider::new(), Arc::new(MemorySecretStore::new()));
    s.fake.add(message("INBOX", 1, raw("q1@x", "Quote", NOW - 2 * DAY), NOW - 2 * DAY));
    s.fake.add(message("INBOX", 2, reply_to(&["q1@x"], raw("q2@x", "Re: Quote", NOW - DAY)), NOW - DAY));
    s.gmail().await;
    s.synced().await;
    let other = rusqlite::Connection::open(&db).unwrap();
    other
        .execute_batch(
            "CREATE TRIGGER full_e BEFORE INSERT ON embeddings BEGIN SELECT RAISE(FAIL, 'disk full'); END;
             CREATE TRIGGER full_s BEFORE INSERT ON summaries BEGIN SELECT RAISE(FAIL, 'disk full'); END;",
        )
        .unwrap();
    let embedder = FakeEmbedder::default();
    s.engine.shared.embed_waiting(&embedder).await;
    assert_eq!(*embedder.calls.lock().unwrap(), 1, "stops at the first failed save");
    let summarizer = FakeSummarizer::default();
    s.engine.shared.summarize_waiting(&summarizer).await;
    assert_eq!(summarizer.read.lock().unwrap().len(), 1);
}

// Addresses: the other addresses one login sends from.

const HI: &str = "hi@rossstudio.art";
const LEAK: &str = "bookshop@rossstudio.art";

fn icloud_sign_in() -> PasswordSignIn {
    PasswordSignIn {
        service: MailService::ICloud,
        email: "maya@icloud.com".into(),
        password: "pw".into(),
        servers: None,
        login: None,
    }
}

/// An iCloud login whose custom domain's mail goes to iCloud; other domains go elsewhere.
fn icloud_setup(mx: Arc<FakeMx>, clock: fn() -> i64) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    setup_all(dir, FakeProvider::new(), secrets, Endpoints::default(), mx, clock)
}

fn icloud_mx() -> Arc<FakeMx> {
    FakeMx::new(&[("rossstudio.art", "mx01.mail.icloud.com"), ("acme.co", "aspmx.l.google.com")])
}

impl Setup {
    fn to(&self, uid: u32, id: &str, to: &str, subject: &str) {
        let at = NOW - uid as i64 * 60_000;
        let raw = raw_from(id, "Dana <dana@acme.co>", to, subject, at);
        self.fake.add(message("INBOX", uid, raw, at));
    }

    async fn icloud(&self) -> AccountInfo {
        self.engine.add_password_account(icloud_sign_in()).await.unwrap()
    }

    fn addresses(&self) -> Vec<moss_core::AccountAddress> {
        self.engine.accounts().unwrap().remove(0).addresses
    }
}

fn row(email: &str, state: moss_core::AddressState, source: moss_core::AddressSource) -> moss_core::AccountAddress {
    moss_core::AccountAddress { email: email.into(), name: None, state, source, seen: 0, blocked: false }
}

#[tokio::test(start_paused = true)]
async fn addresses_are_added_renamed_and_removed() {
    use moss_core::{AddressSource as Source, AddressState as State};
    let s = setup();
    s.gmail().await;
    s.synced().await;
    s.events.clear();
    let info = s.engine.add_address("gmail", " Hi@RossStudio.art ", Some("Ross Studio")).unwrap();
    let hi = moss_core::AccountAddress { name: Some("Ross Studio".into()), ..row(HI, State::Added, Source::You) };
    assert_eq!(info.addresses, [hi.clone()]);
    assert_eq!(s.addresses(), [hi.clone()]);
    // Mail from it may have become yours, so the lists reload.
    assert!(s.events.all().contains(&Event::MailChanged { account_id: "gmail".into() }));
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["addresses"][0]["email"], HI);
    let info = s.engine.update_address("gmail", HI, Some(" ")).unwrap();
    assert_eq!(info.addresses[0].name, None);
    assert!(s.engine.remove_address("gmail", HI).unwrap().addresses.is_empty());
    assert_eq!(s.engine.add_address("gmail", HI, None).unwrap().addresses[0].state, State::Added);

    let err = s.engine.add_address("gmail", "nope", None).unwrap_err();
    assert_eq!(serde_json::to_value(&err).unwrap()["message"], "nope isn't an email address");
    let err = s.engine.add_address("gmail", "Maya.Ross@gmail.com", None).unwrap_err();
    assert!(matches!(err, Error::PrimaryAddress(_)), "{err}");
    assert!(matches!(s.engine.add_address("nobody", HI, None), Err(Error::UnknownAccount(_))));
    let err = s.engine.update_address("gmail", "x@rossstudio.art", None).unwrap_err();
    assert_eq!(err.to_string(), "x@rossstudio.art isn't one of Gmail's addresses");
    assert!(matches!(s.engine.remove_address("gmail", "x@rossstudio.art"), Err(Error::NotYourAddress { .. })));
    assert!(matches!(s.engine.update_address("nobody", HI, None), Err(Error::UnknownAccount(_))));
}

#[tokio::test(start_paused = true)]
async fn an_address_can_have_its_own_signatures() {
    let s = setup();
    s.gmail().await;
    s.engine.add_address("gmail", HI, None).unwrap();
    s.engine.save_signature(&moss_core::Signature { id: "s1".into(), name: "Studio".into(), lines: vec![] }).unwrap();
    let choice = moss_core::AccountSignatures { new_emails: Some("s1".into()), replies: None };
    s.engine.set_address_signatures("gmail", HI, Some(choice.clone())).unwrap();
    assert_eq!(s.engine.signatures().unwrap().by_address[HI], choice);
    s.engine.set_address_signatures("gmail", HI, None).unwrap();
    assert!(s.engine.signatures().unwrap().by_address.is_empty());
    let err = s.engine.set_address_signatures("gmail", "x@rossstudio.art", None).unwrap_err();
    assert!(matches!(err, Error::NotYourAddress { .. }), "{err}");
    assert!(matches!(s.engine.set_address_signatures("nobody", HI, None), Err(Error::UnknownAccount(_))));
    let gone = moss_core::AccountSignatures { new_emails: Some("gone".into()), replies: None };
    assert!(matches!(s.engine.set_address_signatures("gmail", HI, Some(gone)), Err(Error::Store(_))));
}

#[tokio::test(start_paused = true)]
async fn email_goes_from_the_address_you_choose() {
    let s = setup();
    s.inbox(1, "q4@x", "Q4 budget", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    s.engine.add_address("gmail", HI, Some("Ross Studio")).unwrap();
    s.engine.add_address("gmail", "shop@rossstudio.art", None).unwrap();
    let draft = |from: Option<&str>| OutgoingDraft {
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: "gmail".into(),
        to: vec![Person { name: "Dana".into(), email: "dana@acme.co".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Hello".into(),
        body: "Hi".into(),
        body_html: "<p>Hi</p>".into(),
        signature: vec![],
        from_address: from.map(String::from),
        remind_if_no_reply: false,
        attachments: vec![],
    };
    let thread = s.engine.send(draft(Some("Hi@RossStudio.art"))).await.unwrap();
    s.engine.send(draft(Some("shop@rossstudio.art"))).await.unwrap();
    s.engine.send(draft(Some("MAYA.ROSS@gmail.com"))).await.unwrap();
    s.engine.send(draft(Some(""))).await.unwrap();
    s.engine.send(draft(None)).await.unwrap();
    let from: Vec<_> = s.fake.sent().into_iter().map(|m| m.from).collect();
    use moss_providers::Address;
    assert_eq!(
        from,
        [
            Address::named("Ross Studio", HI),
            Address::named("Maya Ross", "shop@rossstudio.art"),
            Address::named("Maya Ross", "maya.ross@gmail.com"),
            Address::named("Maya Ross", "maya.ross@gmail.com"),
            Address::named("Maya Ross", "maya.ross@gmail.com"),
        ]
    );
    // The sent copy is yours, so it isn't waiting in the inbox.
    assert!(s.engine.thread(&thread).unwrap().messages[0].image_trust == moss_core::ImageTrust::Yours);
    assert_eq!(s.subjects(), ["Q4 budget"]);

    // Only an address you added and haven't blocked.
    s.engine.block_address("gmail", "shop@rossstudio.art", true).unwrap();
    s.engine.block_address("gmail", LEAK, true).unwrap();
    for refused in ["shop@rossstudio.art", LEAK, "new@rossstudio.art"] {
        let err = s.engine.send(draft(Some(refused))).await.unwrap_err();
        assert_eq!(err.to_string(), format!("{refused} isn't one of Gmail's addresses"));
    }
    assert_eq!(s.fake.sent().len(), 5);
}

#[tokio::test(start_paused = true)]
async fn blocking_an_address_moves_mail_to_it_to_the_blocked_folder_and_back() {
    let s = setup();
    s.to(1, "a@x", LEAK, "Lunch order");
    s.to(2, "b@x", "Maya Ross <maya.ross@gmail.com>", "Lunch");
    s.gmail().await;
    s.synced().await;
    s.events.clear();
    let info = s.engine.block_address("gmail", "Bookshop@RossStudio.art", true).unwrap();
    assert_eq!(info.addresses.len(), 1);
    assert!(info.addresses[0].blocked);
    assert_eq!(s.subjects(), ["Lunch"]);
    assert_eq!(s.blocked(), ["Lunch order"]);
    assert!(s.events.all().contains(&Event::MailChanged { account_id: "gmail".into() }));
    settle().await;
    let changes = s.fake.changes();
    assert_eq!(changes[changes.len() - 2..], ["create Blocked", "Move { to: \"Blocked\" } INBOX 1:1"]);
    s.from("Blocked", 7, "a@x", "Dana <dana@acme.co>", "Lunch order");
    s.sync_again().await;
    let info = s.engine.block_address("gmail", LEAK, false).unwrap();
    assert!(info.addresses.is_empty(), "dismissed and no longer blocked");
    assert_eq!(s.subjects(), ["Lunch", "Lunch order"]);
    settle().await;
    assert_eq!(s.fake.changes().last().unwrap(), "Move { to: \"INBOX\" } Blocked 1:7");
    assert!(matches!(s.engine.block_address("gmail", "maya.ross@gmail.com", true), Err(Error::PrimaryAddress(_))));
}

#[tokio::test(start_paused = true)]
async fn unblocking_mail_blocked_for_its_address_lifts_that_block() {
    let s = setup();
    s.to(1, "a@x", LEAK, "Lunch order");
    s.gmail().await;
    s.synced().await;
    s.engine.block_address("gmail", LEAK, true).unwrap();
    let id = s.engine.blocked_threads(None, 10).unwrap()[0].id.clone();
    s.engine.unblock_sender(&id).unwrap();
    assert!(s.blocked().is_empty());
    assert_eq!(s.subjects(), ["Lunch order"]);
    assert!(!s.engine.accounts().unwrap()[0].addresses.iter().any(|a| a.blocked));
}

#[tokio::test(start_paused = true)]
async fn each_conversation_says_which_address_it_came_to() {
    let s = setup();
    s.to(1, "a@x", HI, "Commission");
    s.gmail().await;
    s.synced().await;
    s.engine.add_address("gmail", HI, None).unwrap();
    let thread = &s.engine.threads(None).unwrap()[0];
    assert_eq!(thread.to_address.as_deref(), Some(HI));
}

#[tokio::test(start_paused = true)]
async fn after_syncing_an_icloud_account_suggests_the_custom_domain_addresses_mail_came_to() {
    use moss_core::{AddressSource as Source, AddressState as State};
    let mx = icloud_mx();
    let s = icloud_setup(mx.clone(), clock);
    s.to(1, "a@x", HI, "Commission");
    s.to(2, "b@x", "Hi <Hi@RossStudio.art>", "Prints");
    s.to(3, "c@x", "pat@acme.co", "Elsewhere");
    s.to(4, "d@x", "Maya <maya@icloud.com>, sam@team.co", "To you");
    s.icloud().await;
    let event = s.events.wait_for(|e| matches!(e, Event::AccountUpdated { account } if !account.addresses.is_empty())).await;
    let Event::AccountUpdated { account } = event else { unreachable!() };
    let hi = moss_core::AccountAddress { seen: 2, ..row(HI, State::Suggested, Source::Mail) };
    assert_eq!(account.addresses, [hi.clone()]);
    // Each domain is asked about once, and only for mail that came to no address Moss knew.
    let mut asked = mx.asked();
    asked.sort();
    assert_eq!(asked, ["acme.co", "rossstudio.art"]);
    // Looking again finds nothing new: no lookups, no event.
    s.events.clear();
    let info = s.engine.discover_addresses("icloud").await.unwrap();
    assert_eq!(info.addresses, [hi]);
    assert_eq!(mx.asked().len(), 2);
    assert!(!s.events.all().iter().any(|e| matches!(e, Event::AccountUpdated { .. })));
    assert!(matches!(s.engine.discover_addresses("nobody").await, Err(Error::UnknownAccount(_))));
}

#[tokio::test(start_paused = true)]
async fn only_twenty_domains_are_looked_up_each_time() {
    let mx = icloud_mx();
    let s = icloud_setup(mx.clone(), clock);
    for n in 0..25 {
        s.to(n + 1, &format!("{n}@x"), &format!("me@studio{n}.art"), &format!("Mail {n}"));
    }
    s.icloud().await;
    s.synced().await;
    settle().await;
    assert_eq!(mx.asked().len(), 20);
    s.engine.discover_addresses("icloud").await.unwrap();
    assert_eq!(mx.asked().len(), 25, "the other five when asked");
    s.engine.discover_addresses("icloud").await.unwrap();
    assert_eq!(mx.asked().len(), 25);
}

static LATER: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(NOW);

fn later() -> i64 {
    LATER.load(std::sync::atomic::Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn addresses_are_looked_for_again_after_six_hours_of_syncing() {
    let mx = icloud_mx();
    let s = icloud_setup(mx.clone(), later);
    s.to(1, "a@x", "me@one.art", "One");
    s.icloud().await;
    s.synced().await;
    settle().await;
    assert_eq!(mx.asked(), ["one.art"]);
    s.to(2, "b@x", "me@two.art", "Two");
    s.engine.sync_now(None);
    settle().await;
    settle().await;
    assert_eq!(mx.asked(), ["one.art"], "not again so soon");
    LATER.store(NOW + 6 * 3_600_000, std::sync::atomic::Ordering::SeqCst);
    s.to(3, "c@x", "me@three.art", "Three");
    s.engine.sync_now(None);
    settle().await;
    settle().await;
    let mut asked = mx.asked();
    asked.sort();
    assert_eq!(asked, ["one.art", "three.art", "two.art"]);
}

#[tokio::test(start_paused = true)]
async fn other_services_never_ask_the_dns() {
    let mx = icloud_mx();
    let dir = tempfile::tempdir().unwrap();
    let s = setup_all(dir, FakeProvider::new(), Arc::new(MemorySecretStore::new()), Endpoints::default(), mx.clone(), clock);
    s.to(1, "a@x", HI, "Commission");
    s.gmail().await;
    s.synced().await;
    let info = s.engine.discover_addresses("gmail").await.unwrap();
    assert!(info.addresses.is_empty());
    assert!(mx.asked().is_empty());
}

/// A Gmail account signed in with the browser, whose token works until 2100.
async fn gmail_oauth(server: &MockServer) -> Setup {
    let endpoints = Endpoints { gmail_api: server.uri(), ..Endpoints::default() };
    let s = setup_endpoints(tempfile::tempdir().unwrap(), FakeProvider::new(), Arc::new(MemorySecretStore::new()), endpoints);
    let account = naming::new_account(MailService::Gmail, "maya.ross@gmail.com", &[]);
    let connection = AccountConnection {
        email: "maya.ross@gmail.com".into(),
        kind: moss_core::ProviderKind::Gmail,
        service: MailService::Gmail,
        servers: MailService::Gmail.known_servers("maya.ross@gmail.com"),
        auth: AuthMethod::OAuth2 { issuer: moss_accounts::OAuthIssuer::Google, client_id: "c".into() },
        secret: SecretRef::for_account("maya.ross@gmail.com", "gmail"),
    };
    let tokens = moss_accounts::TokenSet {
        access_token: "tok".into(),
        refresh_token: None,
        expires_at: 4_102_444_800,
        refresh_issued_at: 0,
    };
    s.secrets.save(&connection.secret, &moss_accounts::Secret::OAuth { tokens, client_secret: None }).unwrap();
    let mut store = s.engine.shared.store();
    store.upsert_account(&account).unwrap();
    store.set_setting(&connection_key(&account.id), Some(&serde_json::to_string(&connection).unwrap())).unwrap();
    drop(store);
    s
}

/// An Outlook account signed in with the browser whose hour-long token has run out, so
/// the next connection refreshes it at `token_url`.
fn outlook_with_expired_token(token_url: String) -> Setup {
    let mut endpoints = Endpoints::default();
    endpoints.oauth.microsoft_token = token_url;
    let s = setup_endpoints(tempfile::tempdir().unwrap(), FakeProvider::new(), Arc::new(MemorySecretStore::new()), endpoints);
    let email = "maya.ross@outlook.com";
    let account = naming::new_account(MailService::Outlook, email, &[]);
    let connection = AccountConnection {
        email: email.into(),
        kind: moss_core::ProviderKind::Microsoft,
        service: MailService::Outlook,
        servers: MailService::Outlook.known_servers(email),
        auth: AuthMethod::OAuth2 { issuer: moss_accounts::OAuthIssuer::Microsoft, client_id: "c".into() },
        secret: SecretRef::for_account(email, "outlook"),
    };
    let tokens = moss_accounts::TokenSet {
        access_token: "old".into(),
        refresh_token: Some("r".into()),
        expires_at: 0,
        refresh_issued_at: 0,
    };
    s.secrets.save(&connection.secret, &moss_accounts::Secret::OAuth { tokens, client_secret: None }).unwrap();
    let mut store = s.engine.shared.store();
    store.upsert_account(&account).unwrap();
    store.set_setting(&connection_key(&account.id), Some(&serde_json::to_string(&connection).unwrap())).unwrap();
    drop(store);
    s
}

/// What connecting the Outlook account gives when Microsoft's token server answers `response`.
async fn refreshing_against(response: ResponseTemplate) -> (Setup, Result<Arc<dyn Provider>>, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/token")).respond_with(response).mount(&server).await;
    let s = outlook_with_expired_token(format!("{}/token", server.uri()));
    let result = s.engine.shared.provider("outlook").await;
    (s, result, server)
}

// Real time: the HTTP client's timeout would fire at once in paused time.
#[tokio::test]
async fn a_token_refresh_that_cant_reach_microsoft_is_offline_not_signed_out() {
    // A port nothing listens on: the refresh can't connect, as on a Wi-Fi drop.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let s = outlook_with_expired_token(format!("http://127.0.0.1:{port}/token"));
    let err = s.engine.shared.provider("outlook").await.err().unwrap();
    assert!(matches!(err, Error::Offline(_)), "{err:?}");
    assert_eq!(err.problem(), Problem::Offline);
}

#[tokio::test]
async fn a_token_refresh_that_microsoft_fumbles_is_retried_not_signed_out() {
    let (_s, result, _server) = refreshing_against(ResponseTemplate::new(503).set_body_string("<html>busy</html>")).await;
    let err = result.err().unwrap();
    assert!(matches!(err, Error::Server(_)), "{err:?}");
    let busy = serde_json::json!({"error": "temporarily_unavailable", "error_description": "try later"});
    let (_s, result, _server) = refreshing_against(ResponseTemplate::new(503).set_body_json(busy)).await;
    assert!(matches!(result.err().unwrap(), Error::Server(_)));
}

#[tokio::test]
async fn a_token_refresh_that_microsoft_refuses_is_signed_out() {
    let refused = serde_json::json!({"error": "invalid_grant", "error_description": "AADSTS70000: revoked"});
    let (_s, result, _server) = refreshing_against(ResponseTemplate::new(400).set_body_json(refused)).await;
    let err = result.err().unwrap();
    assert!(matches!(err, Error::SignedOut(_)), "{err:?}");
    assert!(err.to_string().contains("sign in again"), "{err}");
}

#[tokio::test]
async fn a_token_refresh_that_works_connects_with_the_new_token() {
    let fresh = serde_json::json!({"access_token": "new", "refresh_token": "r2", "expires_in": 3600});
    let (s, result, _server) = refreshing_against(ResponseTemplate::new(200).set_body_json(fresh)).await;
    result.unwrap();
    assert_eq!(s.connector.last_credential(), Some(Credential::Token("new".into())));
}

// Outlook drops idle connections; reconnecting with the hour-old token it was built with
// got "AUTHENTICATE failed" and showed the account as signed out every hour or so.
#[tokio::test]
async fn an_outlook_connection_signs_in_with_the_token_of_the_hour_not_the_one_it_was_built_with() {
    let server = MockServer::start().await;
    let token = |access: &str| serde_json::json!({"access_token": access, "refresh_token": "r2", "expires_in": 3600});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token("first")))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(token("second"))).mount(&server).await;
    let s = outlook_with_expired_token(format!("{}/token", server.uri()));
    s.engine.shared.provider("outlook").await.unwrap();
    assert_eq!(s.connector.last_credential(), Some(Credential::Token("first".into())));
    // While the token is good, the connection (and the mail server sessions in it) stays.
    s.engine.shared.provider("outlook").await.unwrap();
    assert_eq!(s.connector.built(), 1);
    // An hour on, the token has run out: the next use refreshes it and connects with it.
    let secret = SecretRef::for_account("maya.ross@outlook.com", "outlook");
    let run_out = moss_accounts::TokenSet { access_token: "first".into(), refresh_token: Some("r2".into()), expires_at: 0, refresh_issued_at: 0 };
    s.secrets.save(&secret, &moss_accounts::Secret::OAuth { tokens: run_out, client_secret: None }).unwrap();
    s.engine.shared.provider("outlook").await.unwrap();
    assert_eq!(s.connector.built(), 2);
    assert_eq!(s.connector.last_credential(), Some(Credential::Token("second".into())));
    // A token another part of Moss refreshed is picked up too.
    let refreshed = moss_accounts::TokenSet { access_token: "third".into(), refresh_token: Some("r3".into()), expires_at: 4_102_444_800, refresh_issued_at: 0 };
    s.secrets.save(&secret, &moss_accounts::Secret::OAuth { tokens: refreshed, client_secret: None }).unwrap();
    s.engine.shared.provider("outlook").await.unwrap();
    assert_eq!(s.connector.built(), 3);
    assert_eq!(s.connector.last_credential(), Some(Credential::Token("third".into())));
}

#[tokio::test]
async fn an_outlook_connection_whose_token_cant_be_refreshed_says_so_instead_of_using_the_old_one() {
    let server = MockServer::start().await;
    let fresh = serde_json::json!({"access_token": "first", "refresh_token": "r2", "expires_in": 3600});
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(fresh)).up_to_n_times(1).mount(&server).await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(503).set_body_string("busy")).mount(&server).await;
    let s = outlook_with_expired_token(format!("{}/token", server.uri()));
    s.engine.shared.provider("outlook").await.unwrap();
    let secret = SecretRef::for_account("maya.ross@outlook.com", "outlook");
    let run_out = moss_accounts::TokenSet { access_token: "first".into(), refresh_token: Some("r2".into()), expires_at: 0, refresh_issued_at: 0 };
    s.secrets.save(&secret, &moss_accounts::Secret::OAuth { tokens: run_out, client_secret: None }).unwrap();
    assert!(matches!(s.engine.shared.provider("outlook").await, Err(Error::Server(_))));
    assert_eq!(s.connector.built(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_password_account_keeps_its_connection_without_asking_the_keychain_again() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    let built = s.connector.built();
    // Its saved password going away doesn't matter to a connection already built.
    s.secrets.delete(&s.engine.shared.connection("gmail").unwrap().secret).unwrap();
    s.engine.shared.provider("gmail").await.unwrap();
    assert_eq!(s.connector.built(), built);
}

#[test]
fn sign_in_failures_while_syncing_say_whether_to_sign_in_again() {
    use moss_accounts::SignInError as S;
    let unreachable = S::Unreachable { host: "login.microsoftonline.com".into(), detail: "timed out".into() };
    let odd = S::UnexpectedResponse { host: "login.microsoftonline.com".into(), detail: "HTTP 502".into() };
    let cases = [
        (unreachable, "offline"),
        (S::Timeout, "offline"),
        (odd, "server"),
        (S::Keychain { detail: "locked".into() }, "keychain"),
        (S::SignInExpired, "signedOut"),
        (S::AccessDenied { detail: "no".into() }, "signedOut"),
        (S::GoogleTestingExpired, "signedOut"),
        (S::InvalidOAuthClient { detail: "bad".into() }, "signedOut"),
        (S::ClientIdNotConfigured, "signedOut"),
    ];
    for (error, kind) in cases {
        let message = error.to_string();
        let error = Error::syncing(error);
        assert_eq!(error.kind(), kind, "{message}");
        // The reason stays, for the window to show.
        let reason = message.trim_start_matches("couldn't use the keychain: ");
        assert!(error.to_string().contains(reason), "{error} keeps {reason}");
    }
}

// Real time: the HTTP client's timeout would fire at once in paused time.
#[tokio::test]
async fn gmail_signed_in_with_the_browser_adds_its_send_as_addresses() {
    use moss_core::{AddressSource as Source, AddressState as State};
    let server = MockServer::start().await;
    let body = serde_json::json!({"sendAs": [
        {"sendAsEmail": "maya.ross@gmail.com", "isPrimary": true},
        {"sendAsEmail": "Hi@RossStudio.art", "displayName": "Ross Studio", "verificationStatus": "accepted"},
        {"sendAsEmail": "not an address"}
    ]});
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/settings/sendAs"))
        .and(wiremock::matchers::header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let s = gmail_oauth(&server).await;
    let info = s.engine.discover_addresses("gmail").await.unwrap();
    let hi = moss_core::AccountAddress { name: Some("Ross Studio".into()), ..row(HI, State::Added, Source::Gmail) };
    assert_eq!(info.addresses, [hi]);
    let event = s.events.wait_for(|e| matches!(e, Event::AccountUpdated { .. })).await;
    assert!(matches!(event, Event::AccountUpdated { account } if account.addresses.len() == 1));
}

#[tokio::test]
async fn when_gmail_wont_list_its_addresses_asking_says_why() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let s = gmail_oauth(&server).await;
    let err = s.engine.discover_addresses("gmail").await.unwrap_err();
    assert!(matches!(err, Error::SignIn(moss_accounts::SignInError::SignInExpired)), "{err}");
    // A database that fails is an error too.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"sendAs": [{"sendAsEmail": "hi@rossstudio.art"}]})))
        .mount(&server)
        .await;
    let s = gmail_oauth(&server).await;
    let broken = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    broken.execute_batch("CREATE TRIGGER no_new BEFORE INSERT ON account_addresses BEGIN SELECT RAISE(FAIL, 'full'); END").unwrap();
    assert!(matches!(s.engine.discover_addresses("gmail").await, Err(Error::Store(_))));
}
mod drafts;
mod moves;
mod reading;
mod summaries;

#[tokio::test(start_paused = true)]
async fn ask_moss_finds_files_and_people() {
    let s = setup();
    s.fake.add(message("INBOX", 1, with_files("f@x", "Signed", NOW - DAY), NOW - DAY));
    s.fake.add(message("INBOX", 2, raw_from("p@x", "Priya Shah <priya@acme.co>", "maya.ross@gmail.com", "Hi", NOW), NOW));
    s.gmail().await;
    s.synced().await;

    let all = s.engine.find_files(&moss_core::FileQuery { limit: 5, ..Default::default() }).unwrap();
    let names: Vec<_> = all.iter().map(|f| f.file.name.as_str()).collect();
    assert_eq!(names, ["Q4.pdf"], "the inline logo isn't a file");
    assert_eq!(all[0].thread_id, s.thread_id("Signed"));
    assert_eq!(s.engine.file(&all[0].message_id, 1).unwrap(), all[0]);
    assert!(matches!(s.engine.file(&all[0].message_id, 9), Err(Error::Unavailable(_))));
    assert_eq!(s.engine.mail_count().unwrap(), 2);

    let people = |q: &str| -> Vec<String> { s.engine.find_people(q, 5).unwrap().into_iter().map(|c| c.person.email).collect() };
    assert_eq!(people("PRIYA"), ["priya@acme.co"]);
    assert_eq!(people("northwind"), ["daniel@northwind.co"]);
    assert_eq!(people("  ").len(), 2, "no words: everyone");
    assert!(people("nobody").is_empty());
    assert_eq!(s.engine.find_people("", 1).unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn your_corrections_stick_and_a_send_can_wait_for_an_answer() {
    let s = setup();
    s.inbox(1, "q4@x", "Q4 budget due 2026-11-13", NOW - DAY);
    s.gmail().await;
    s.synced().await;
    s.engine.set_sorter(FakeSorter::new(&[]));
    settle().await;
    let id = s.thread_id("Q4 budget due 2026-11-13");
    s.engine.set_section(&id, Section::Info).unwrap();
    s.engine.clear_deadline(&id).unwrap();
    s.engine.set_split(&id, SplitId::Other).unwrap();
    let row = s.engine.thread(&id).unwrap().summary;
    assert_eq!((row.section, row.split, row.due), (Section::Info, SplitId::Other, None));
    assert!(!s.engine.remind_if_no_reply(&id, true).unwrap(), "Daniel wrote last");

    let draft = OutgoingDraft {
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
        remind_if_no_reply: true,
        attachments: vec![],
    };
    s.engine.send(draft).await.unwrap();
    let waiting = s.engine.thread(&id).unwrap().summary;
    assert_eq!(waiting.section, Section::Later);
    assert_eq!(waiting.waiting.unwrap().why.by, moss_core::DecidedBy::You);
    assert!(s.engine.remind_if_no_reply(&id, false).unwrap());
    assert_eq!(s.engine.thread(&id).unwrap().summary.waiting, None);

    assert!(s.engine.set_section("x", Section::Info).is_err());
    assert!(s.engine.set_split("x", SplitId::Other).is_err());
    assert!(s.engine.clear_deadline("x").is_err());
    assert!(s.engine.remind_if_no_reply("x", true).is_err());
}

#[test]
#[cfg(unix)]
fn attachment_import_refuses_special_files() {
    let s = setup();
    let error = s.engine.attach_files("d1", &[std::path::PathBuf::from("/dev/null")]);
    assert!(error.is_err(), "devices are not attachments");
}

#[test]
fn attachment_import_bounds_a_batch_before_reading_or_storing_it() {
    let s = setup();
    let a = s._dir.path().join("a.bin");
    let b = s._dir.path().join("b.bin");
    for path in [&a, &b] {
        std::fs::File::create(path).unwrap().set_len(30 * 1024 * 1024).unwrap();
    }
    assert!(s.engine.attach_files("d1", &[a,b]).is_err(), "a batch must not allocate more than the largest provider accepts");
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM draft_files", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn attachment_import_is_atomic_when_a_later_database_insert_fails() {
    let s = setup();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_file BEFORE INSERT ON draft_files WHEN NEW.name = 'bad.txt' BEGIN SELECT RAISE(ABORT, 'fixture refusal'); END").unwrap();
    let a = s._dir.path().join("good.txt");
    let b = s._dir.path().join("bad.txt");
    std::fs::write(&a, "good").unwrap();
    std::fs::write(&b, "bad").unwrap();
    assert!(s.engine.attach_files("d1", &[a,b]).is_err());
    assert_eq!(db.query_row("SELECT count(*) FROM draft_files", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "the successful first insert must roll back too");
}

#[test]
fn attachment_import_rejects_direction_changing_filenames() {
    let s = setup();
    for control in ['\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'] {
        let result = s.engine.attach_contents("d1", &[(format!("invoice{control}fdp.exe"), b"fixture".to_vec())]);
        assert!(result.is_err(), "direction controls must not disguise an attachment");
    }
}

#[test]
fn attachment_import_and_draft_save_roll_back_together() {
    let s = setup();
    let d = moss_core::Draft {
        revision: None,
        id: "file-draft".into(), mode: ComposeMode::New, thread_id: None,
        from_account_id: "gmail".into(), to: vec![], cc: vec![], bcc: vec![],
        subject: "Fixture".into(), body: "Text".into(), body_html: "Text".into(),
        remind_if_no_reply: false, updated_at: NOW, signature_id: None,
        signature_chosen: false, from_address: None, attachments: vec![],
    };
    // No such account: a file must not be left behind when saving the draft fails.
    assert!(s.engine.attach_draft_contents(&d, &[("report.txt".into(), b"fixture".to_vec())]).is_err());
    assert_eq!(s.engine.shared.store().draft_file_usage(&d.id).unwrap(), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn optional_forward_flags_do_not_block_accepted_receipt_repair() {
    for scenario in ["invalid", "oversized", "corrupt", "resend", "removed"] {
        let s = setup();
        s.inbox(1, "original@example.test", "Original", NOW);
        s.gmail().await;
        s.synced().await;
        let id = s.thread_id("Original");
        let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_forward BEFORE INSERT ON forwards BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(matches!(s.engine.send(forward(&id)).await, Err(Error::DeliveryAccepted(_))));
        assert_eq!(s.fake.sent().len(), 1);
        let operation: String = db.query_row("SELECT id FROM deliveries WHERE state='accepted'", [], |r|r.get(0)).unwrap();
        db.execute_batch("DROP TRIGGER refuse_forward").unwrap();
        match scenario {
            "invalid" => { db.execute("UPDATE message_locations SET provider_id='opaque' WHERE message_id=(SELECT id FROM messages WHERE message_id='original@example.test')", []).unwrap(); }
            "oversized" => { db.execute("UPDATE message_locations SET provider_id=?1 WHERE message_id=(SELECT id FROM messages WHERE message_id='original@example.test')", ["1".repeat(1_048_577)]).unwrap(); }
            "corrupt" => s.engine.shared.store().set_setting(&connection_key("gmail"), Some("private malformed settings")).unwrap(),
            "removed" => { db.execute("DELETE FROM messages WHERE message_id='original@example.test'", []).unwrap(); }
            _ => {
                let mut connection = s.engine.shared.connection("gmail").unwrap();
                connection.service = MailService::Resend;
                s.engine.shared.store().set_setting(&connection_key("gmail"), Some(&serde_json::to_string(&connection).unwrap())).unwrap();
            }
        }
        let receipt = s.engine.repair_send(&operation).unwrap();
        assert!(!receipt.is_empty());
        assert_eq!(s.engine.shared.store().delivery(&operation).unwrap().unwrap().state, moss_store::DeliveryState::Complete);
        if scenario != "removed" { assert!(s.engine.thread(&id).unwrap().messages[0].forwarded); }
        assert!(s.pending().is_empty(), "{scenario}");
        assert_eq!(s.engine.repair_send(&operation).unwrap(), receipt);
        assert_eq!(s.fake.sent().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn cross_account_forward_keeps_flags_with_original_mailboxes() {
    let s = setup();
    s.inbox(1, "original@example.test", "Original", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Original");
    let mut account = s.engine.shared.account("gmail").unwrap();
    account.id = "original-account".into();
    account.address = "original@example.test".into();
    s.engine.shared.store().upsert_account(&account).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE messages SET account_id='original-account'", []).unwrap();
    db.execute("UPDATE threads SET account_id='original-account'", []).unwrap();
    db.execute("UPDATE mailboxes SET account_id='original-account' WHERE name='INBOX'", []).unwrap();
    db.execute("INSERT INTO mailboxes(account_id,name) VALUES('original-account','Other folder')", []).unwrap();
    db.execute("INSERT INTO message_locations(message_id,mailbox_id,provider_id) SELECT message_id,mailbox_id,'1:2' FROM message_locations LIMIT 1", []).unwrap();
    db.execute("INSERT INTO message_locations(message_id,mailbox_id,provider_id) SELECT message_id,(SELECT id FROM mailboxes WHERE account_id='original-account' AND name='Other folder'),'2:7' FROM message_locations LIMIT 1", []).unwrap();
    let receipt = s.engine.send(forward(&id)).await.unwrap();
    assert!(!receipt.is_empty());
    assert_eq!(s.fake.sent().len(), 1);
    assert!(s.pending().is_empty(), "sending account must receive no flags for another account's UIDs");
    let store = s.engine.shared.store();
    let pending = store.pending_changes("original-account").unwrap();
    let changes: Vec<ServerChange> = pending.iter().map(|(_,json)|serde_json::from_str(json).unwrap()).collect();
    assert_eq!(changes, vec![
        ServerChange::Forwarded { mailbox:"INBOX".into(), ids:vec!["1:1".into(),"1:2".into()] },
        ServerChange::Forwarded { mailbox:"Other folder".into(), ids:vec!["2:7".into()] },
    ]);
    let batch = store.batch_for_change("original-account", pending[0].0).unwrap().unwrap();
    assert_eq!(batch.kind, moss_store::BatchKind::Forwarded);
    assert_eq!(batch.parts.len(), 2);
    assert_eq!(store.batch_for_change("original-account", pending[1].0).unwrap().unwrap(), batch);
}

#[tokio::test(start_paused = true)]
async fn forwarded_receipt_preserves_storage_failure_but_needs_no_config_for_local_targets() {
    let s = setup();
    s.inbox(1, "original@example.test", "Original", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Original");
    let original = s.engine.thread(&id).unwrap().messages[0].id.clone();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("ALTER TABLE settings RENAME TO unavailable_settings;").unwrap();
    assert!(s.engine.forwarded(&original, &original).is_err());
    assert_eq!(db.query_row("SELECT forwarded FROM messages WHERE id=?1", [&original], |r|r.get::<_,bool>(0)).unwrap(), false);
    assert!(s.pending().is_empty());
    db.execute("UPDATE message_locations SET provider_id='local:fixture'", []).unwrap();
    s.engine.forwarded(&original, &original).unwrap();
    assert_eq!(db.query_row("SELECT forwarded FROM messages WHERE id=?1", [&original], |r|r.get::<_,bool>(0)).unwrap(), true);
    assert!(s.pending().is_empty());
}

#[tokio::test(start_paused = true)]
async fn accepted_forward_repair_after_completion_failure_keeps_one_batch() {
    let s = setup();
    s.inbox(1, "original@example.test", "Original", NOW);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Original");
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_completion BEFORE UPDATE OF state ON deliveries WHEN NEW.state='complete' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(matches!(s.engine.send(forward(&id)).await, Err(Error::DeliveryAccepted(_))));
    let operation: String = db.query_row("SELECT id FROM deliveries WHERE state='accepted'", [], |r|r.get(0)).unwrap();
    let pending = s.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM forwards", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
    db.execute_batch("DROP TRIGGER refuse_completion").unwrap();
    s.engine.repair_send(&operation).unwrap();
    assert_eq!(s.pending(), pending);
    assert_eq!(s.engine.shared.store().delivery(&operation).unwrap().unwrap().state, moss_store::DeliveryState::Complete);
    assert_eq!(s.fake.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn queue_recovery_retains_unknown_and_oversized_heads_without_later_mutations() {
    for payload in ["future action".to_string(), "x".repeat(moss_store::MAX_CHANGE_PAYLOAD_BYTES + 1),
        serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["invalid-uid".into()], seen:true }).unwrap(),
        serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec![], seen:true }).unwrap()] {
        let s = setup();
        s.gmail().await;
        s.synced().await;
        let before = s.fake.changes();
        let head = s.engine.shared.store().queue_change("gmail", &payload, NOW).unwrap();
        let next = serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["1:1".into()], seen:true }).unwrap();
        s.engine.shared.store().queue_change("gmail", &next, NOW).unwrap();
        assert!(s.engine.shared.send_all("gmail").await.is_err());
        let saved = s.engine.shared.store().earliest_change("gmail").unwrap().unwrap();
        assert_eq!(saved.id, head);
        assert_eq!(saved.state, moss_store::ChangeState::Blocked);
        assert_eq!(saved.failure, Some(if payload.len() > moss_store::MAX_CHANGE_PAYLOAD_BYTES { moss_store::ChangeFailure::TooLarge } else { moss_store::ChangeFailure::Unreadable }));
        assert_eq!(s.pending().len(), 2);
        assert!(s.engine.shared.send_all("gmail").await.is_err());
        assert_eq!(s.fake.changes(), before);
    }
}

#[tokio::test(start_paused = true)]
async fn interrupted_claims_never_reenter_the_provider_automatically() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    let json = serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["1:1".into()], seen:true }).unwrap();
    let id = s.engine.shared.store().queue_change("gmail", &json, NOW).unwrap();
    s.engine.shared.store().claim_change("gmail", id, 0).unwrap();
    let before = s.fake.changes();
    assert!(s.engine.shared.send_all("gmail").await.is_err());
    assert_eq!(s.fake.changes(), before);
    assert_eq!(s.engine.shared.store().earliest_change("gmail").unwrap().unwrap().state, moss_store::ChangeState::Claimed);
}

#[tokio::test(start_paused = true)]
async fn ignored_completion_keeps_a_claim_without_repeating_the_provider_attempt() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    let json = serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["1:1".into()], seen:true }).unwrap();
    let id = s.engine.shared.store().queue_change("gmail", &json, NOW).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER retain_attempt BEFORE DELETE ON outbox BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = s.fake.applies_started();
    assert!(s.engine.shared.send_all("gmail").await.is_err());
    assert_eq!(s.fake.applies_started(), before + 1);
    let saved = s.engine.shared.store().earliest_change("gmail").unwrap().unwrap();
    assert_eq!((saved.id, saved.state), (id, moss_store::ChangeState::Claimed));
    assert!(s.engine.shared.send_all("gmail").await.is_err());
    assert_eq!(s.fake.applies_started(), before + 1);
    assert_eq!(s.engine.shared.store().batch_for_change("gmail", id).unwrap().unwrap().parts[0].outcome, moss_store::BatchOutcome::Pending);
}

#[tokio::test(start_paused = true)]
async fn queue_recovery_defers_blocked_folder_moves_but_keeps_incoming_sync() {
    let s = setup();
    s.inbox(1, "blocked@example.test", "Block this", NOW);
    s.gmail().await;
    s.synced().await;
    let blocked_thread = s.thread_id("Block this");
    let queued = s.engine.shared.store().queue_change("gmail", "future action", NOW).unwrap();
    s.engine.shared.store().block_change("gmail", queued, 0, moss_store::ChangeFailure::Unreadable).unwrap();
    let before = s.fake.changes();
    s.engine.block_sender(&blocked_thread, false).unwrap();
    settle().await;
    assert_eq!(s.fake.changes(), before);
    s.fake.deliver(message("INBOX", 42, raw_from("incoming@example.test", "Reader <reader@example.test>", "Owner <owner@example.test>", "Incoming despite barrier", NOW), NOW));
    s.engine.shared.pass("gmail", false).await.unwrap();
    assert!(s.engine.thread(&s.thread_id("Incoming despite barrier")).is_ok());
    assert_eq!(s.fake.changes(), before);
    s.engine.unblock_sender(&blocked_thread).unwrap();
    settle().await;
    assert_eq!(s.fake.changes(), before);
    assert_eq!(s.pending().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn corrupt_batch_metadata_cannot_authorize_a_provider_attempt() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    let json = serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["1:1".into()], seen:true }).unwrap();
    s.engine.shared.store().queue_change("gmail", &json, NOW).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("DELETE FROM outbox_batch_parts", []).unwrap();
    let before = s.fake.applies_started();
    assert!(s.engine.shared.send_all("gmail").await.is_err());
    assert_eq!(s.fake.applies_started(), before);
    assert_eq!(s.engine.shared.store().earliest_change("gmail").unwrap().unwrap().state, moss_store::ChangeState::Queued);
}


#[tokio::test(start_paused = true)]
async fn server_change_status_survives_read_sync_and_reports_partial_batches() {
    let s = setup();
    s.inbox(1, "status@example.test", "Status fixture", NOW);
    s.gmail().await;
    s.synced().await;
    assert_eq!(s.engine.server_change_status("gmail").unwrap(), None);
    let thread = s.thread_id("Status fixture");
    s.engine.shared.store().change_thread_with_changes(&thread, "gmail", moss_store::ThreadChange::Read(true), NOW,
        |_,_| Ok(vec!["first".into(), "second".into(), "third".into()])).unwrap();
    let parts = s.pending();
    s.engine.shared.store().finish_unclaimed_change(parts[0].0, moss_store::UnclaimedOutcome::Applied).unwrap();
    s.engine.shared.store().block_change("gmail", parts[1].0, 0, moss_store::ChangeFailure::Uncertain).unwrap();
    let before = s.engine.server_change_status("gmail").unwrap().unwrap();
    assert_eq!(serde_json::to_value(&before).unwrap(), serde_json::json!({
        "accountId":"gmail", "change":parts[1].0.to_hex(), "revision":1, "state":"blocked", "failure":"uncertain", "kind":"read",
        "completed":1, "pending":2, "discarded":0, "dropped":0,
    }));
    s.engine.shared.pass("gmail", false).await.unwrap();
    s.engine.shared.recovered("gmail");
    assert_eq!(s.engine.server_change_status("gmail").unwrap(), Some(before));
    assert!(matches!(s.engine.server_change_status("removed"), Err(Error::UnknownAccount(_))));
}

#[tokio::test(start_paused = true)]
async fn server_change_events_refresh_status_when_claimed_and_finished() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    s.events.clear();
    let json = serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["1:1".into()], seen:true }).unwrap();
    s.engine.shared.store().queue_change("gmail", &json, NOW).unwrap();
    s.fake.slow(std::time::Duration::from_secs(1));
    let sending = s.engine.shared.send_all("gmail");
    tokio::pin!(sending);
    assert!(std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(sending.as_mut(), cx).is_pending())).await);
    let notifications = || s.events.all().iter().filter(|e| serde_json::to_value(e).unwrap() == serde_json::json!({"type":"serverChangesChanged", "accountId":"gmail"})).count();
    assert_eq!(notifications(), 1);
    assert_eq!(s.engine.server_change_status("gmail").unwrap().unwrap().state, moss_store::ChangeState::Claimed);
    sending.await.unwrap();
    assert_eq!(notifications(), 2);
    assert_eq!(s.engine.server_change_status("gmail").unwrap(), None);
}


#[test]
fn recovery_status_reopens_with_retained_work_and_honest_terminal_counts() {
    let s = setup();
    s.engine.shared.store().upsert_account(&moss_core::Account {
        id:"work".into(), name:"Work".into(), letter:"W".into(), address:"owner@example.test".into(),
        sender_name:"Owner".into(), color:moss_core::AccountColor::Work, provider:moss_core::ProviderKind::Imap,
        org:None, org_domain:None,
    }).unwrap();
    let thread = s.engine.shared.store().ingest(&raw("recovery@example.test", "Retained status", NOW), &moss_core::MessageSource { account_id:"work".into(), mailbox:"INBOX".into(), provider_id:"1:1".into(), ..Default::default() }).unwrap().thread_id;
    s.engine.shared.store().change_thread_with_changes(&thread, "work", moss_store::ThreadChange::Read(true), NOW,
        |_,_| Ok(vec!["one".into(), "two".into(), "three".into(), "four".into()])).unwrap();
    let parts = s.engine.shared.store().pending_changes("work").unwrap();
    s.engine.shared.store().finish_unclaimed_change(parts[0].0, moss_store::UnclaimedOutcome::Applied).unwrap();
    s.engine.shared.store().finish_unclaimed_change(parts[1].0, moss_store::UnclaimedOutcome::Dropped).unwrap();
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    // A historical partial dismissal remains distinct from successful completion.
    db.execute_batch("BEGIN; UPDATE outbox_batch_parts SET outcome='discarded' WHERE token=(SELECT token FROM outbox ORDER BY id LIMIT 1); DELETE FROM outbox WHERE id=(SELECT min(id) FROM outbox); COMMIT;").unwrap();
    s.engine.shared.store().claim_change("work", parts[3].0, 0).unwrap();
    let expected = s.engine.server_change_status("work").unwrap().unwrap();
    assert_eq!((expected.completed, expected.dropped, expected.discarded, expected.pending), (1,1,1,1));
    assert_eq!(expected.state, moss_store::ChangeState::Claimed);
    drop(db);
    let Setup { _dir: dir, fake, secrets, .. } = s;
    let reopened = setup_with(dir, fake, secrets);
    assert_eq!(reopened.engine.server_change_status("work").unwrap(), Some(expected));
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_server_task_notifies_the_retained_claim() {
    let s = setup(); s.gmail().await; s.synced().await; s.events.clear();
    let json = serde_json::to_string(&ServerChange::Seen { mailbox:"INBOX".into(), ids:vec!["1:1".into()], seen:true }).unwrap();
    s.engine.shared.store().queue_change("gmail", &json, NOW).unwrap();
    s.fake.slow(std::time::Duration::from_secs(10));
    let shared = s.engine.shared.clone();
    let task = tokio::spawn(async move { shared.send_all("gmail").await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(s.engine.server_change_status("gmail").unwrap().unwrap().state, moss_store::ChangeState::Claimed);
    task.abort(); assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(s.events.all().iter().filter(|e| matches!(e, Event::ServerChangesChanged { .. })).count(), 2);
    assert_eq!(s.engine.server_change_status("gmail").unwrap().unwrap().state, moss_store::ChangeState::Claimed);
}


#[tokio::test(start_paused = true)]
async fn refused_removal_preserves_credentials_connection_and_running_worker() {
    let s = setup(); s.gmail().await;
    let connection = s.engine.shared.connection("gmail").unwrap();
    let credential = s.secrets.load(&connection.secret).unwrap();
    assert!(credential.is_some());
    let worker = s.engine.shared.workers.lock().unwrap().get("gmail").unwrap().task.id();
    let action = s.engine.shared.store().queue_change("gmail", "retained", NOW).unwrap();
    assert!(s.engine.remove_account("gmail").is_err());
    assert_eq!(s.secrets.load(&connection.secret).unwrap(), credential);
    assert_eq!(s.engine.shared.connection("gmail").unwrap(), connection);
    assert_eq!(s.engine.shared.workers.lock().unwrap().get("gmail").unwrap().task.id(), worker);
    assert_eq!(s.engine.shared.store().pending_changes("gmail").unwrap(), [(action,"retained".into())]);
}

#[tokio::test(start_paused = true)]
async fn an_account_held_back_only_by_server_changes_is_removed_once_you_accept_losing_them() {
    let s = setup(); s.gmail().await;
    let connection = s.engine.shared.connection("gmail").unwrap();
    let first = s.engine.shared.store().queue_change("gmail", "refused", NOW).unwrap();
    s.engine.shared.store().block_change("gmail", first, 0, moss_store::ChangeFailure::Refused).unwrap();
    s.engine.shared.store().queue_change("gmail", "waiting", NOW).unwrap();
    let error = s.engine.remove_account("gmail").unwrap_err();
    assert!(matches!(error, Error::UnsentServerChanges(2)), "{error:?}");
    assert_eq!(serde_json::to_value(&error).unwrap(), serde_json::json!({
        "kind": "unsentServerChanges", "count": 2,
        "message": "2 changes haven't reached the server and will be lost.",
    }));
    assert_eq!(Error::UnsentServerChanges(1).to_string(), "1 change hasn't reached the server and will be lost.");
    assert!(s.engine.remove_account_discarding("gmail", 1).is_err(), "not what you were told");
    assert_eq!(s.engine.accounts().unwrap().len(), 1);
    assert!(s.secrets.load(&connection.secret).unwrap().is_some());
    s.engine.remove_account_discarding("gmail", 2).unwrap();
    assert!(s.engine.accounts().unwrap().is_empty());
    assert!(s.engine.shared.store().pending_changes("gmail").unwrap().is_empty());
    assert!(s.engine.shared.workers.lock().unwrap().get("gmail").is_none());
}

#[tokio::test(start_paused = true)]
async fn removing_an_account_deletes_the_copy_from_before_the_last_update() {
    let s = setup(); s.gmail().await;
    let copy = moss_store::safety_copy_path(&s._dir.path().join("moss.db"));
    std::fs::write(&copy, b"the removed account's mail").unwrap();
    s.engine.remove_account("gmail").unwrap();
    assert!(!copy.exists());
    // With no copy there, or one that can't be deleted, the account still goes.
    s.gmail().await;
    s.engine.remove_account("gmail").unwrap();
    s.gmail().await;
    s.engine.shared.store().queue_change("gmail", "waiting", NOW).unwrap();
    std::fs::create_dir(&copy).unwrap();
    s.engine.remove_account_discarding("gmail", 1).unwrap();
    assert!(s.engine.accounts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn accepting_lost_server_changes_still_never_removes_unsent_mail() {
    let s = setup(); s.gmail().await;
    s.engine.shared.store().queue_change("gmail", "waiting", NOW).unwrap();
    let unsent = saved_with("unsent", vec![]);
    s.engine.shared.store().prepare_delivery("op", &unsent, "{}", "unsent@example.test", NOW).unwrap();
    for result in [s.engine.remove_account("gmail"), s.engine.remove_account_discarding("gmail", 1)] {
        let error = result.unwrap_err();
        assert_eq!(error.to_string(), "This account has an unresolved delivery. Review its outcome before removing the account.");
    }
    assert_eq!(s.engine.accounts().unwrap().len(), 1);
    assert_eq!(s.pending().len(), 1);
}


#[tokio::test(start_paused = true)]
async fn unreadable_removal_preflight_does_not_touch_credentials_or_workers() {
    for corrupt_connection in [true, false] {
        let s = setup(); s.gmail().await;
        let connection = s.engine.shared.connection("gmail").unwrap();
        let credential = s.secrets.load(&connection.secret).unwrap();
        let worker = s.engine.shared.workers.lock().unwrap().get("gmail").unwrap().task.id();
        if corrupt_connection {
            s.engine.shared.store().set_setting(&connection_key("gmail"), Some("private unreadable fixture")).unwrap();
        } else {
            rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap()
                .execute_batch("ALTER TABLE outbox RENAME TO retained_outbox").unwrap();
        }
        let error = s.engine.remove_account("gmail").unwrap_err();
        assert!(!error.to_string().contains("private unreadable fixture"));
        assert_eq!(s.secrets.load(&connection.secret).unwrap(), credential);
        assert_eq!(s.engine.shared.workers.lock().unwrap().get("gmail").unwrap().task.id(), worker);
        assert_eq!(s.engine.shared.store().accounts().unwrap().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn database_failure_during_removal_keeps_the_credential_and_worker_usable() {
    for target in ["accounts", "settings"] {
        let s = setup(); s.gmail().await; s.synced().await;
        let connection = s.engine.shared.connection("gmail").unwrap();
        let credential = s.secrets.load(&connection.secret).unwrap();
        let worker = s.engine.shared.workers.lock().unwrap().get("gmail").unwrap().task.id();
        rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap().execute_batch(
            &format!("CREATE TRIGGER refuse_removal BEFORE DELETE ON {target} BEGIN SELECT RAISE(ABORT,'fixture'); END;")
        ).unwrap();
        assert!(s.engine.remove_account("gmail").is_err());
        assert_eq!(s.secrets.load(&connection.secret).unwrap(), credential);
        assert_eq!(s.engine.shared.connection("gmail").unwrap(), connection);
        assert_eq!(s.engine.shared.workers.lock().unwrap().get("gmail").unwrap().task.id(), worker);
        assert_eq!(s.engine.shared.store().accounts().unwrap().len(), 1);
    }
}

struct RetryKeychain {
    secrets: Arc<MemorySecretStore>,
    locked: std::sync::atomic::AtomicBool,
    ignore_delete: std::sync::atomic::AtomicBool,
    fail_read: std::sync::atomic::AtomicBool,
}
impl SecretStore for RetryKeychain {
    fn save(&self, key: &SecretRef, value: &Secret) -> Result<(), SecretStoreError> { self.secrets.save(key,value) }
    fn load(&self, key: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
        if self.fail_read.load(std::sync::atomic::Ordering::SeqCst) { return Err(SecretStoreError::Unavailable("read unavailable fixture".into())); }
        self.secrets.load(key)
    }
    fn delete(&self, key: &SecretRef) -> Result<(), SecretStoreError> {
        if self.locked.load(std::sync::atomic::Ordering::SeqCst) { return Err(SecretStoreError::Unavailable("locked fixture".into())); }
        if self.ignore_delete.load(std::sync::atomic::Ordering::SeqCst) { return Ok(()); }
        self.secrets.delete(key)
    }
}
fn cleanup_engine(path: &std::path::Path, secrets: Arc<RetryKeychain>) -> Engine {
    Engine::new(Parts {store:Store::open(path).unwrap(),secrets:secrets.clone(),accounts:Accounts::new(secrets),
        connector:Arc::new(FakeConnector::new(FakeProvider::new())),events:Arc::new(RecordedEvents::default()),clock})
}
fn seed_cleanup_account(engine: &Engine, secrets: &RetryKeychain) -> AccountConnection {
    let c = connection(AuthMethod::Password);
    secrets.save(&c.secret, &Secret::Password {password:"old-fixture".into()}).unwrap();
    let mut store = engine.shared.store();
    store.upsert_account(&naming::new_account(MailService::Imap, &c.email, &[])).unwrap();
    store.set_setting("connection:b", Some(&serde_json::to_string(&c).unwrap())).unwrap();
    c
}

#[tokio::test]
async fn failed_keychain_cleanup_survives_restart_and_finishes_without_restoring_the_account() {
    let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("mail.db");
    let secrets=Arc::new(RetryKeychain {secrets:Arc::new(MemorySecretStore::new()),locked:std::sync::atomic::AtomicBool::new(true),ignore_delete:std::sync::atomic::AtomicBool::new(false),fail_read:std::sync::atomic::AtomicBool::new(false)});
    let engine=cleanup_engine(&path,secrets.clone());
    let c=seed_cleanup_account(&engine,&secrets);
    assert!(engine.remove_account("b").unwrap().pending);
    assert!(engine.accounts().unwrap().is_empty());
    assert!(secrets.load(&c.secret).unwrap().is_some());
    drop(engine);
    let engine=cleanup_engine(&path,secrets.clone());
    assert!(engine.credential_cleanup_status().unwrap().pending);
    secrets.locked.store(false,std::sync::atomic::Ordering::SeqCst);
    engine.start().unwrap();
    assert!(!engine.credential_cleanup_status().unwrap().pending);
    assert_eq!(secrets.load(&c.secret).unwrap(),None);
    assert!(engine.accounts().unwrap().is_empty());
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
}

#[tokio::test]
async fn cleanup_never_deletes_a_readded_accounts_new_generation() {
    let dir=tempfile::tempdir().unwrap();
    let secrets=Arc::new(RetryKeychain {secrets:Arc::new(MemorySecretStore::new()),locked:std::sync::atomic::AtomicBool::new(true),ignore_delete:std::sync::atomic::AtomicBool::new(false),fail_read:std::sync::atomic::AtomicBool::new(false)});
    let engine=cleanup_engine(&dir.path().join("mail.db"),secrets.clone());
    let old=seed_cleanup_account(&engine,&secrets);
    assert!(engine.remove_account("b").unwrap().pending);
    let mut new=old.clone(); new.secret.account.push_str(" [0123456789abcdef0123456789abcdef]");
    secrets.save(&new.secret,&Secret::Password {password:"new-fixture".into()}).unwrap();
    engine.keep(new.clone()).await.unwrap();
    secrets.locked.store(false,std::sync::atomic::Ordering::SeqCst);
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
    assert_eq!(secrets.load(&old.secret).unwrap(),None);
    assert_eq!(secrets.load(&new.secret).unwrap(),Some(Secret::Password {password:"new-fixture".into()}));
    assert_eq!(engine.shared.connection("b").unwrap(),new);
}

#[tokio::test]
async fn cleanup_preserves_active_references_and_refuses_unreadable_settings() {
    let dir=tempfile::tempdir().unwrap();
    let secrets=Arc::new(RetryKeychain {secrets:Arc::new(MemorySecretStore::new()),locked:std::sync::atomic::AtomicBool::new(true),ignore_delete:std::sync::atomic::AtomicBool::new(false),fail_read:std::sync::atomic::AtomicBool::new(false)});
    let engine=cleanup_engine(&dir.path().join("mail.db"),secrets.clone());
    let c=seed_cleanup_account(&engine,&secrets);
    assert!(engine.remove_account("b").unwrap().pending);
    seed_cleanup_account(&engine,&secrets);
    secrets.locked.store(false,std::sync::atomic::Ordering::SeqCst);
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
    assert!(secrets.load(&c.secret).unwrap().is_some());
    engine.shared.store().retire_credential(&credential_cleanup::reference(&c).unwrap()).unwrap();
    engine.shared.store().set_setting("connection:b",Some("unreadable fixture")).unwrap();
    assert!(engine.retry_credential_cleanup().unwrap().pending);
    assert!(secrets.load(&c.secret).unwrap().is_some());
    engine.shared.store().set_setting("connection:b",None).unwrap();
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
    assert!(secrets.load(&c.secret).unwrap().is_none());
}

#[tokio::test]
async fn removal_and_cleanup_wait_for_an_inflight_oauth_refresh() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(200))
        .set_body_json(serde_json::json!({"access_token":"fresh","refresh_token":"r2","expires_in":3600}))).mount(&server).await;
    let s=outlook_with_expired_token(format!("{}/token",server.uri()));
    let shared=s.engine.shared.clone();
    let task=tokio::spawn(async move {shared.provider("outlook").await});
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        while server.received_requests().await.unwrap().is_empty() {tokio::task::yield_now().await;}
    }).await.unwrap();
    assert!(s.engine.remove_account("outlook").is_err());
    assert!(s.engine.retry_credential_cleanup().is_err());
    task.await.unwrap().unwrap();
    let key=s.engine.shared.connection("outlook").unwrap().secret;
    assert!(!s.engine.remove_account("outlook").unwrap().pending);
    assert_eq!(s.secrets.load(&key).unwrap(),None);
}

#[tokio::test]
async fn cleanup_never_deletes_another_applications_keychain_items() {
    let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("mail.db");
    let secrets=Arc::new(RetryKeychain {secrets:Arc::new(MemorySecretStore::new()),locked:std::sync::atomic::AtomicBool::new(false),ignore_delete:std::sync::atomic::AtomicBool::new(false),fail_read:std::sync::atomic::AtomicBool::new(false)});
    let engine=cleanup_engine(&path,secrets.clone());
    let key=SecretRef {service:"another.application".into(),account:"fixture".into()};
    secrets.save(&key,&Secret::Password {password:"fixture".into()}).unwrap();
    rusqlite::Connection::open(&path).unwrap().execute("INSERT INTO credential_cleanup(service,account) VALUES(?1,?2)",rusqlite::params![key.service,key.account]).unwrap();
    assert!(engine.retry_credential_cleanup().unwrap().pending);
    assert!(secrets.load(&key).unwrap().is_some());
}

#[tokio::test]
async fn cleanup_requires_readback_that_the_keychain_item_is_absent() {
    let dir=tempfile::tempdir().unwrap();
    let secrets=Arc::new(RetryKeychain {secrets:Arc::new(MemorySecretStore::new()),locked:std::sync::atomic::AtomicBool::new(false),ignore_delete:std::sync::atomic::AtomicBool::new(true),fail_read:std::sync::atomic::AtomicBool::new(false)});
    let engine=cleanup_engine(&dir.path().join("mail.db"),secrets.clone());
    let c=seed_cleanup_account(&engine,&secrets);
    assert!(engine.remove_account("b").unwrap().pending);
    assert!(engine.credential_cleanup_status().unwrap().pending);
    assert!(secrets.load(&c.secret).unwrap().is_some());
    secrets.ignore_delete.store(false,std::sync::atomic::Ordering::SeqCst);
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
    assert!(secrets.load(&c.secret).unwrap().is_none());
}

#[tokio::test]
async fn removal_cannot_overtake_address_discovery_using_the_account_sign_in() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/settings/sendAs"))
        .respond_with(ResponseTemplate::new(200)
            .set_delay(std::time::Duration::from_millis(200))
            .set_body_json(serde_json::json!({"sendAs":[]})))
        .mount(&server).await;
    let s = gmail_oauth(&server).await;
    let shared = s.engine.shared.clone();
    let discovery = tokio::spawn(async move {shared.discover_addresses("gmail").await});
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while server.received_requests().await.unwrap().is_empty() {tokio::task::yield_now().await;}
    }).await.unwrap();
    assert!(s.engine.remove_account("gmail").is_err());
    assert_eq!(s.engine.accounts().unwrap().len(), 1);
    discovery.await.unwrap().unwrap();
    assert!(!s.engine.remove_account("gmail").unwrap().pending);
    assert!(s.engine.accounts().unwrap().is_empty());
}

#[tokio::test]
async fn missing_connection_metadata_does_not_silently_abandon_a_saved_credential() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(RetryKeychain {
        secrets: Arc::new(MemorySecretStore::new()),
        locked: std::sync::atomic::AtomicBool::new(false),
        ignore_delete: std::sync::atomic::AtomicBool::new(false),
        fail_read: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = cleanup_engine(&dir.path().join("mail.db"), secrets.clone());
    let connection = seed_cleanup_account(&engine, &secrets);
    engine.shared.store().set_setting("connection:b", None).unwrap();
    assert!(engine.remove_account("b").is_err());
    assert_eq!(engine.accounts().unwrap().len(), 1);
    assert!(secrets.load(&connection.secret).unwrap().is_some());
    assert!(!engine.credential_cleanup_status().unwrap().pending);
}

#[tokio::test]
async fn removal_refuses_connection_metadata_bound_to_a_different_account() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(RetryKeychain {
        secrets: Arc::new(MemorySecretStore::new()),
        locked: std::sync::atomic::AtomicBool::new(false),
        ignore_delete: std::sync::atomic::AtomicBool::new(false),
        fail_read: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = cleanup_engine(&dir.path().join("mail.db"), secrets.clone());
    let original = seed_cleanup_account(&engine, &secrets);
    let mut wrong = original.clone();
    wrong.email = "another@example.com".into();
    wrong.secret = SecretRef::for_account(&wrong.email, "imap");
    secrets.save(&wrong.secret, &Secret::Password {password:"other-fixture".into()}).unwrap();
    engine.shared.store().set_setting("connection:b", Some(&serde_json::to_string(&wrong).unwrap())).unwrap();
    assert!(engine.remove_account("b").is_err());
    assert_eq!(engine.accounts().unwrap().len(), 1);
    assert!(secrets.load(&original.secret).unwrap().is_some());
    assert!(secrets.load(&wrong.secret).unwrap().is_some());
    assert!(!engine.credential_cleanup_status().unwrap().pending);
    let mut repaired = original.clone();
    repaired.email = repaired.email.to_ascii_uppercase();
    engine.shared.store().set_setting("connection:b", Some(&serde_json::to_string(&repaired).unwrap())).unwrap();
    assert!(!engine.remove_account("b").unwrap().pending);
    assert!(secrets.load(&original.secret).unwrap().is_none());
    assert!(secrets.load(&wrong.secret).unwrap().is_some());
}

#[tokio::test]
async fn unavailable_readback_keeps_the_cleanup_intent_even_after_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(RetryKeychain {
        secrets: Arc::new(MemorySecretStore::new()),
        locked: std::sync::atomic::AtomicBool::new(false),
        ignore_delete: std::sync::atomic::AtomicBool::new(false),
        fail_read: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = cleanup_engine(&dir.path().join("mail.db"), secrets.clone());
    let connection = seed_cleanup_account(&engine, &secrets);
    secrets.fail_read.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(engine.remove_account("b").unwrap().pending);
    assert!(engine.accounts().unwrap().is_empty());
    assert!(engine.credential_cleanup_status().unwrap().pending);
    assert!(secrets.secrets.load(&connection.secret).unwrap().is_none());
    assert!(engine.retry_credential_cleanup().unwrap().pending);
    secrets.fail_read.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
    assert!(!engine.credential_cleanup_status().unwrap().pending);
}

#[tokio::test]
async fn retained_cleanup_entries_cannot_starve_a_later_removable_credential() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let secrets = Arc::new(RetryKeychain {
        secrets: Arc::new(MemorySecretStore::new()),
        locked: std::sync::atomic::AtomicBool::new(false),
        ignore_delete: std::sync::atomic::AtomicBool::new(false),
        fail_read: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = cleanup_engine(&path, secrets.clone());
    let connection = seed_cleanup_account(&engine, &secrets);
    let db = rusqlite::Connection::open(&path).unwrap();
    for i in 0..128 {
        db.execute("INSERT INTO credential_cleanup(service,account) VALUES('aaa.unsupported',?1)", [format!("fixture-{i}")]).unwrap();
    }
    assert!(engine.remove_account("b").unwrap().pending);
    assert!(secrets.load(&connection.secret).unwrap().is_some());
    assert!(engine.retry_credential_cleanup().unwrap().pending);
    assert!(secrets.load(&connection.secret).unwrap().is_none());
    assert_eq!(db.query_row("SELECT count(*) FROM credential_cleanup", [], |r|r.get::<_,i64>(0)).unwrap(), 128);
}

#[tokio::test]
async fn cleanup_cursor_write_failure_retains_the_credential_for_a_later_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let secrets = Arc::new(RetryKeychain {
        secrets: Arc::new(MemorySecretStore::new()),
        locked: std::sync::atomic::AtomicBool::new(false),
        ignore_delete: std::sync::atomic::AtomicBool::new(false),
        fail_read: std::sync::atomic::AtomicBool::new(false),
    });
    let engine = cleanup_engine(&path, secrets.clone());
    let connection = seed_cleanup_account(&engine, &secrets);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER ignore_cleanup_position BEFORE INSERT ON settings WHEN NEW.key='credentialCleanupCursor' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(engine.remove_account("b").unwrap().pending);
    assert!(engine.accounts().unwrap().is_empty());
    assert!(secrets.load(&connection.secret).unwrap().is_some());
    assert!(engine.credential_cleanup_status().unwrap().pending);
    db.execute_batch("DROP TRIGGER ignore_cleanup_position").unwrap();
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
    assert!(secrets.load(&connection.secret).unwrap().is_none());
}

#[tokio::test]
async fn refused_connection_save_does_not_leave_a_partial_new_account() {
    for action in ["RAISE(ABORT,'fixture')", "RAISE(IGNORE)"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let secrets = Arc::new(RetryKeychain {secrets:Arc::new(MemorySecretStore::new()),locked:std::sync::atomic::AtomicBool::new(false),ignore_delete:std::sync::atomic::AtomicBool::new(false),fail_read:std::sync::atomic::AtomicBool::new(false)});
        let engine = cleanup_engine(&path, secrets.clone());
        let mut fresh = connection(AuthMethod::Password);
        fresh.secret.account.push_str(&format!(" [{}]", uuid::Uuid::new_v4().simple()));
        secrets.save(&fresh.secret, &Secret::Password {password:"unclaimed-fixture".into()}).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER refuse_connection BEFORE INSERT ON settings WHEN NEW.key LIKE 'connection:%' BEGIN SELECT {action}; END;")).unwrap();
        assert!(engine.keep(fresh.clone()).await.is_err());
        assert!(secrets.load(&fresh.secret).unwrap().is_none(), "failed sign-in must retire its unclaimed secret");
        assert!(engine.shared.store().accounts().unwrap().is_empty());
        assert!(engine.shared.store().setting("connection:b").unwrap().is_none());
        assert!(engine.shared.workers.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn backup_runs_off_the_runtime_without_holding_the_mail_store() {
    let s = setup();
    let output = s._dir.path().join("backup");
    let callback_engine = s.engine.clone();
    let runtime_thread = std::thread::current().id();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let recorded = calls.clone();
    let info = s.engine.create_backup(output.clone(), moss_store::BackupControl::default(), move |_| {
        assert_ne!(std::thread::current().id(), runtime_thread);
        let mut store = callback_engine.shared.store.try_lock().expect("backup must release the mail mutex");
        store.set_setting("fixture", Some("mail remains writable")).unwrap();
        recorded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }).await.unwrap();
    assert!(calls.load(std::sync::atomic::Ordering::Relaxed) >= 5);
    let verified = moss_store::verify_backup(&output, Some(&info.sha256), moss_store::BackupVerification::default()).unwrap();
    assert_eq!(verified.info().bytes, info.bytes);
    assert_eq!(s.engine.shared.store().setting("fixture").unwrap().as_deref(), Some("mail remains writable"));
}

#[tokio::test]
async fn backup_preserves_cancellation_and_worker_failures_without_outputs() {
    let s = setup();
    let output = s._dir.path().join("backup");
    let control = moss_store::BackupControl::default();
    control.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    let result = s.engine.create_backup(output.clone(), control, |_| {}).await;
    assert!(matches!(result, Err(moss_store::BackupError::Cancelled)));
    assert!(!output.exists());
    let result = s.engine.create_backup(output.clone(), moss_store::BackupControl::default(), |_| panic!("fixture worker failure")).await;
    assert!(matches!(result, Err(moss_store::BackupError::Invalid)));
    assert!(!output.exists());
    s.engine.shared.store().set_setting("fixture", Some("still available")).unwrap();
    assert_eq!(s.engine.shared.store().setting("fixture").unwrap().as_deref(), Some("still available"));
}

#[test]
fn credential_cleanup_does_not_hold_the_database_during_keychain_calls() {
    struct ProbedSecrets {
        shared: std::sync::Mutex<Option<std::sync::Weak<Shared>>>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl SecretStore for ProbedSecrets {
        fn save(&self, _: &SecretRef, _: &Secret) -> Result<(), SecretStoreError> { unreachable!() }
        fn load(&self, _: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
            let shared = self.shared.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
            assert!(shared.store.try_lock().is_ok(), "Keychain read holds database lock");
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(None)
        }
        fn delete(&self, _: &SecretRef) -> Result<(), SecretStoreError> {
            let shared = self.shared.lock().unwrap().as_ref().unwrap().upgrade().unwrap();
            assert!(shared.store.try_lock().is_ok(), "Keychain delete holds database lock");
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let secrets = Arc::new(ProbedSecrets {shared: std::sync::Mutex::new(None), calls: std::sync::atomic::AtomicUsize::new(0)});
    let engine = Engine::new(Parts {store: Store::open(&path).unwrap(), secrets: secrets.clone(), accounts: Accounts::new(secrets.clone()),
        connector: Arc::new(FakeConnector::new(FakeProvider::new())), events: Arc::new(RecordedEvents::default()), clock});
    *secrets.shared.lock().unwrap() = Some(Arc::downgrade(&engine.shared));
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute("INSERT INTO credential_cleanup(service,account) VALUES(?1,'fixture')", [moss_accounts::KEYCHAIN_SERVICE]).unwrap();
    assert!(!engine.retry_credential_cleanup().unwrap().pending);
    assert_eq!(secrets.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn signing_in_again_retires_only_the_previous_secret() {
    let s = setup();
    s.gmail().await;
    let old = s.engine.shared.connection("gmail").unwrap();
    assert!(s.secrets.load(&old.secret).unwrap().is_some());
    s.gmail().await;
    let current = s.engine.shared.connection("gmail").unwrap();
    assert_ne!(current.secret, old.secret);
    assert!(s.secrets.load(&current.secret).unwrap().is_some());
    assert!(s.secrets.load(&old.secret).unwrap().is_none());
    assert!(!s.engine.credential_cleanup_status().unwrap().pending);
}

#[tokio::test]
async fn failed_sign_in_never_deletes_an_unowned_or_unrecorded_secret() {
    for unowned in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let secrets = Arc::new(RetryKeychain {secrets:Arc::new(MemorySecretStore::new()),locked:std::sync::atomic::AtomicBool::new(false),ignore_delete:std::sync::atomic::AtomicBool::new(false),fail_read:std::sync::atomic::AtomicBool::new(false)});
        let engine = cleanup_engine(&path, secrets.clone());
        let mut fresh = connection(AuthMethod::Password);
        if unowned { fresh.secret.account = "unrelated-fixture-reference".into(); }
        else { fresh.secret.account.push_str(&format!(" [{}]", uuid::Uuid::new_v4().simple())); }
        secrets.save(&fresh.secret, &Secret::Password {password:"preserved-fixture".into()}).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_connection BEFORE INSERT ON settings WHEN NEW.key LIKE 'connection:%' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        if !unowned {
            db.execute_batch("CREATE TRIGGER refuse_retirement BEFORE INSERT ON credential_cleanup BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        }
        assert!(engine.keep(fresh.clone()).await.is_err());
        assert!(secrets.load(&fresh.secret).unwrap().is_some());
        assert!(!engine.credential_cleanup_status().unwrap().pending);
        assert!(engine.accounts().unwrap().is_empty());
    }
}

#[test]
fn attachment_filenames_cannot_hide_their_extension_with_direction_or_control_characters() {
    for control in ['\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\n', '\r', '\0', '\t'] {
        assert_eq!(safe_file_name(&format!("invoice{control}.fileloc")), "invoice.fileloc");
        assert_eq!(safe_file_name(&control.to_string()), "attachment");
    }
}

#[tokio::test(start_paused = true)]
async fn unreadable_previous_sign_in_cannot_be_replaced_or_retired_by_guessing() {
    let s=setup();s.gmail().await;s.synced().await;
    let old=s.engine.shared.connection("gmail").unwrap();
    let secret=s.secrets.load(&old.secret).unwrap().unwrap();
    let mut fresh=old.clone();fresh.secret.account=format!("{} [{}]",moss_accounts::SecretRef::for_account(&old.email,old.service.slug()).account,uuid::Uuid::new_v4().simple());
    s.secrets.save(&fresh.secret,&secret).unwrap();
    s.engine.shared.store().set_setting("connection:gmail",Some("unreadable private fixture")).unwrap();
    let error=s.engine.keep(fresh.clone()).await.unwrap_err();
    assert_eq!(error.to_string(),"The previous connection cannot be read. Its sign-in has been kept.");
    assert_eq!(s.engine.shared.store().setting("connection:gmail").unwrap().as_deref(),Some("unreadable private fixture"));
    assert_eq!(s.secrets.load(&old.secret).unwrap(),Some(secret.clone()));
    assert_eq!(s.secrets.load(&fresh.secret).unwrap(),Some(secret));
    assert!(s.engine.credential_cleanup_status().unwrap().pending);
    s.engine.shared.store().set_setting("connection:gmail",Some(&serde_json::to_string(&old).unwrap())).unwrap();
    assert!(!s.engine.retry_credential_cleanup().unwrap().pending);
    assert!(s.secrets.load(&old.secret).unwrap().is_some());
    assert!(s.secrets.load(&fresh.secret).unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_failed_message_index_query_does_not_masquerade_as_a_completed_pass() {
    for missing in ["embeddings","attachment_embeddings"] {
        let s=setup();
        s.fake.add(message("INBOX",1,with_files("index-query@example.test","Index query",NOW),NOW));
        s.gmail().await;s.synced().await;
        let id=s.message_with_subject("Index query").id;
        s.engine.shared.store().keep_attachment_text(&id,1,"attachment text").unwrap();
        let db=rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
        db.execute_batch(&format!("DROP TABLE {missing}")).unwrap();
        let embedder=FakeEmbedder::default();
        s.engine.shared.embed_waiting(&embedder).await;
        assert_eq!(*embedder.calls.lock().unwrap(),if missing=="embeddings" {0} else {1},"a failed query must stop the pass at its own stage");
    }
}

#[tokio::test(start_paused = true)]
async fn indexing_skips_changed_inputs_and_retries_them_on_the_next_pass() {
    struct ChangingEmbedder {
        shared: std::sync::Weak<Shared>,
        path: std::path::PathBuf,
        message_changed: std::sync::atomic::AtomicBool,
        file_changed: std::sync::atomic::AtomicBool,
    }
    #[async_trait::async_trait]
    impl Embedder for ChangingEmbedder {
        fn model(&self)->String {"changing-inputs".into()}
        async fn query(&self,_:&str)->Option<Vec<f32>> {None}
        async fn documents(&self,emails:&[moss_store::ToEmbed])->Option<Vec<Vec<f32>>> {
            if emails[0].subject=="Q4.pdf" {
                if !self.file_changed.swap(true,std::sync::atomic::Ordering::SeqCst) {
                    self.shared.upgrade().unwrap().store().keep_attachment_text(&emails[0].message_id.to_string(),1,"Updated attachment text").unwrap();
                }
            } else if !self.message_changed.swap(true,std::sync::atomic::Ordering::SeqCst) {
                rusqlite::Connection::open(&self.path).unwrap().execute("UPDATE messages SET subject='Changed during embedding' WHERE id=?1",[emails[0].message_id]).unwrap();
            }
            Some(emails.iter().map(|_|vec![1.0,0.0]).collect())
        }
    }
    let s=setup();
    s.fake.add(message("INBOX",1,with_files("changing-inputs@example.test","First input",NOW),NOW));
    s.inbox(2,"other-input@example.test","Other input",NOW-DAY);
    s.gmail().await;s.synced().await;
    let id=s.message_with_subject("First input").id;
    s.engine.shared.store().keep_attachment_text(&id,1,"Original attachment text").unwrap();
    let model=Arc::new(ChangingEmbedder{shared:Arc::downgrade(&s.engine.shared),path:s._dir.path().join("moss.db"),message_changed:false.into(),file_changed:false.into()});
    s.engine.shared.embed_waiting(model.as_ref()).await;
    assert_eq!(s.engine.shared.store().embedded("changing-inputs").unwrap(),(1,2));
    let pending=s.engine.shared.store().unembedded_files("changing-inputs",10).unwrap();
    assert_eq!(pending.len(),1);assert_eq!(pending[0].email.text,"Updated attachment text");
    s.engine.shared.embed_waiting(model.as_ref()).await;
    assert_eq!(s.engine.shared.store().embedded("changing-inputs").unwrap(),(2,2));
    assert!(s.engine.shared.store().unembedded_files("changing-inputs",10).unwrap().is_empty());
    s.engine.set_embedder(model);
    assert!(s.engine.find("unmatched semantic query",None,1).await.unwrap().is_empty(),"a temporarily unavailable query model keeps keyword fallback");
}

#[tokio::test]
async fn semantic_scan_releases_writer_lock_before_hydrating() {
    let s = setup();
    let shared = &s.engine.shared;
    shared.with_meaning_reader(|reader| {
        assert!(reader.is_some());
        let writer = shared.store.try_lock().expect("semantic scan must not hold the writer mutex");
        assert!(writer.accounts().unwrap().is_empty());
        Ok(())
    }).unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_panic_while_one_of_the_engines_locks_was_held_breaks_nothing_until_relaunch() {
    let s = setup();
    s.inbox(1, "a@x", "Budget", NOW);
    s.gmail().await;
    s.synced().await;
    let shared = &s.engine.shared;
    poison(&shared.problems);
    poison(&shared.providers);
    poison(&shared.workers);
    poison(&shared.on_server);
    poison(&shared.draft_timers);
    poison(&shared.browser_sign_in);
    poison(&shared.delivery_recovered);
    poison(&shared.embedder);
    poison(&shared.reader);
    poison(&shared.discovered);
    poison(&shared.hosts);
    shared.report("gmail", &Error::Offline("offline".into()));
    assert_eq!(s.engine.sync_problems().len(), 1);
    shared.recovered("gmail");
    assert!(s.engine.sync_problems().is_empty());
    s.engine.sync_now(None);
    s.engine.recover_deliveries().unwrap();
    assert!(matches!(s.engine.finish_browser_sign_in().await, Err(Error::Invalid(_))));
    let before = s.fake.changes().len();
    let budget = s.thread_id("Budget");
    s.engine.mark_done(&budget).unwrap();
    settle().await;
    assert_eq!(s.fake.changes()[before..], ["Archive INBOX 1:1"]);
    s.engine.save_draft(&saved_with("after-panic", vec![])).unwrap();
    assert!(crate::lock(&shared.reader).is_none());
    assert!(s.engine.find("Budget", None, 10).await.unwrap().iter().any(|hit| hit.thread.id == budget));
    crate::lock(&shared.discovered).clear();
    assert!(shared.discovery_due("gmail"));
    assert!(!shared.discovery_due("gmail"), "marked as started");
    s.engine.shared.stop_worker("gmail");
}
