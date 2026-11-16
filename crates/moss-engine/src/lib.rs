//! Moss's mail engine. The desktop app holds one [`Engine`] and turns each Tauri command
//! into one call here, so nearly everything the app does is tested in this crate.
//!
//! - Accounts: sign in ([`Engine::add_password_account`], [`Engine::add_resend_account`]),
//!   keep the connection in the database and the secret in the OS keychain.
//! - Sync: one background worker per account. Newest mail first, then older mail, then IMAP
//!   IDLE for new mail. Checkpoints are saved after every batch, so it resumes after a
//!   restart or a dropped connection. Progress, new mail and problems go out as [`Event`]s.
//! - Actions: done (archives on the server; not done brings it back), read, remind (local),
//!   send, search. Server changes for an account run in the order they were made.
//! - Addresses: the other addresses one login sends from, found in Gmail's settings and in
//!   the mail itself (see `addresses.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Serialize, Serializer};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use moss_accounts::{
    AccountConnection, Accounts, AuthMethod, Discovered, MailService, PasswordSignIn, PendingAuthorization,
    ResendDomain, Secret, SecretStore, SignInError,
};
use moss_core::{
    Account, AccountAddress, AccountColor, AccountSignatures, AddressState, Draft, Person, SearchHit, Signature, SignatureSettings, SignatureUse,
    SplitCounts, SplitId, Thread, ThreadSummary,
};
use moss_providers::Provider;
use moss_store::Store;

mod addresses;
mod credential_cleanup;
pub use credential_cleanup::CredentialCleanupStatus;
mod ask;
pub mod attach;
mod attachment_import;
pub mod compose;
pub mod connect;
mod drafts;
mod delivery;
pub use delivery::CancelSendOutcome;
mod server_queue;
pub use server_queue::ServerChangeStatus;
pub mod events;
pub mod meaning;
pub mod naming;
pub mod preview;
mod quarantine;
pub mod reading;
pub mod rules;
mod scheduled;
mod change_batches;
pub mod sort;
pub mod summarize;
mod storage;
mod backup;
mod sync;
#[cfg(test)]
mod testing;

pub use compose::OutgoingDraft;
pub use connect::{Connector, Credential, NetConnector};
pub use events::{Event, Events, Problem, SyncIssue, SyncProgress};
pub use meaning::Embedder;
pub use reading::Reader;
pub use sort::{Pitch, Sorter};
pub use summarize::{SummaryFailure, SummaryNeed, Summarizer};
pub use storage::StorageInfo;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    SignIn(#[from] SignInError),
    #[error("This draft changed or was removed elsewhere. Your edit must be kept separately before continuing.")]
    DraftConflict,
    #[error("{0}")]
    LocalRecoveryRejected(String),
    #[error("the mail database failed: {0}")]
    Store(moss_store::Error),
    #[error("no account with id {0}")]
    UnknownAccount(String),
    #[error("no thread with id {0}")]
    UnknownThread(String),
    #[error("the server no longer accepts the saved sign-in: {0}")]
    SignedOut(String),
    #[error("can't reach the server: {0}")]
    Offline(String),
    #[error("the server refused: {0}")]
    Server(String),
    #[error("This email already has a send in progress. Wait for its outcome or use Undo in Moss.")]
    DeliveryBusy,
    #[error("Delivery may have succeeded. Do not send this email again until you check Sent: {0}")]
    DeliveryUnknown(String),
    #[error("The email was sent, but its local copy needs recovery: {0}")]
    DeliveryAccepted(String),
    #[error("the keychain failed: {0}")]
    Keychain(String),
    #[error("add someone to send it to")]
    NoRecipients,
    #[error("{0} is a public mail service, so it can't be blocked or accepted as a whole")]
    PublicDomain(String),
    #[error("not on this device or the server: {0}")]
    Unavailable(String),
    #[error("{0} isn't an email address")]
    InvalidAddress(String),
    #[error("{0} is this account's own address")]
    PrimaryAddress(String),
    #[error("{address} isn't one of {account}'s addresses")]
    NotYourAddress { address: String, account: String },
    /// Something you typed that Moss can't use, saying why.
    #[error("{0}")]
    Invalid(String),
    /// More files than the mail service takes, saying how much: see [`attach::too_big`].
    #[error("{0}")]
    TooBig(String),
    /// Removing the account would lose this many server changes: ask first.
    #[error("{}", unsent_changes(*.0))]
    UnsentServerChanges(usize),
}

fn unsent_changes(count: usize) -> String {
    match count {
        1 => "1 change hasn't reached the server and will be lost.".into(),
        n => format!("{n} changes haven't reached the server and will be lost."),
    }
}

impl From<moss_store::Error> for Error {
    fn from(e: moss_store::Error) -> Self {
        match e {
            moss_store::Error::NotFound { kind: "thread", id } => Error::UnknownThread(id),
            // Removed while a sync was running: the worker stops.
            moss_store::Error::UnknownAccount(id) => Error::UnknownAccount(id),
            moss_store::Error::DraftConflict => Error::DraftConflict,
            moss_store::Error::PublicDomain(domain) => Error::PublicDomain(domain),
            moss_store::Error::InvalidAddress(email) => Error::InvalidAddress(email),
            moss_store::Error::PrimaryAddress(email) => Error::PrimaryAddress(email),
            moss_store::Error::Invalid(why) => Error::Invalid(why),
            e @ moss_store::Error::UploadReceiptConflict => Error::Invalid(e.to_string()),
            e => Error::Store(e),
        }
    }
}

impl From<moss_providers::Error> for Error {
    fn from(e: moss_providers::Error) -> Self {
        use moss_providers::Error as P;
        match e {
            P::Auth(m) => Error::SignedOut(m),
            P::DeliveryUnknown(m) => Error::DeliveryUnknown(m),
            P::Network(_) | P::RateLimited { .. } => Error::Offline(e.to_string()),
            e => Error::Server(e.to_string()),
        }
    }
}

impl Error {
    /// A sign-in that failed while syncing, such as the hourly token refresh. Only a refusal
    /// means signing in again: a dropped connection or a muddled answer is retried.
    fn syncing(e: SignInError) -> Self {
        match e {
            SignInError::Unreachable { .. } | SignInError::Timeout => Error::Offline(e.to_string()),
            SignInError::UnexpectedResponse { .. } => Error::Server(e.to_string()),
            SignInError::Keychain { detail } => Error::Keychain(detail),
            e => Error::SignedOut(e.to_string()),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Error::SignIn(_) => "signIn",
            Error::Store(_) => "store",
            Error::DraftConflict => "draftConflict",
            Error::LocalRecoveryRejected(_) => "localRecoveryRejected",
            Error::UnknownAccount(_) => "unknownAccount",
            Error::UnknownThread(_) => "unknownThread",
            Error::SignedOut(_) => "signedOut",
            Error::Offline(_) => "offline",
            Error::Server(_) => "server",
            Error::DeliveryBusy => "deliveryBusy",
            Error::DeliveryUnknown(_) => "deliveryUnknown",
            Error::DeliveryAccepted(_) => "deliveryAccepted",
            Error::Keychain(_) => "keychain",
            Error::NoRecipients => "noRecipients",
            Error::PublicDomain(_) => "publicDomain",
            Error::Unavailable(_) => "unavailable",
            Error::InvalidAddress(_) => "invalidAddress",
            Error::PrimaryAddress(_) => "primaryAddress",
            Error::NotYourAddress { .. } => "notYourAddress",
            Error::Invalid(_) => "invalid",
            Error::TooBig(_) => "tooBig",
            Error::UnsentServerChanges(_) => "unsentServerChanges",
        }
    }

    fn problem(&self) -> Problem {
        match self {
            Error::SignedOut(_) | Error::Keychain(_) => Problem::SignedOut,
            Error::Offline(_) => Problem::Offline,
            _ => Problem::Server,
        }
    }
}

/// Sign-in errors keep their own shape (`{kind: "wrongPassword", service}`); the rest
/// become `{kind, message}` for the UI.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if let Error::SignIn(e) = self {
            return e.serialize(serializer);
        }
        let count = match self { Error::UnsentServerChanges(count) => Some(*count), _ => None };
        let mut out = serializer.serialize_struct("Error", 2 + usize::from(count.is_some()))?;
        out.serialize_field("kind", self.kind())?;
        out.serialize_field("message", &self.to_string())?;
        if let Some(count) = count { out.serialize_field("count", &count)?; }
        out.end()
    }
}

/// A resolved journal identity must not be retried under a new identity automatically.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LocalDraftRecoveryWrite {
    Saved { recovery: moss_store::LocalDraftRecovery },
    Resolved { #[serde(rename = "adoptedId")] adopted_id: Option<String> },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A store result about one of `account`'s addresses: one it doesn't have is
/// [`Error::NotYourAddress`].
fn mine<T>(account: &Account, result: moss_store::Result<T>) -> Result<T> {
    result.map_err(|e| match e {
        moss_store::Error::NotFound { kind: "address", id } => {
            Error::NotYourAddress { address: id, account: account.name.clone() }
        }
        e => e.into(),
    })
}

/// A file name from an email, made safe to write: no folders, no drive letters, not hidden.
pub fn safe_file_name(name: &str) -> String {
    let last = name.rsplit('/').next().unwrap_or_default();
    let visible: String = last.chars().filter(|c| !c.is_control() && !matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect();
    let flat = visible.replace(['\\', ':'], "_");
    let shown = flat.trim_start_matches('.');
    if shown.is_empty() {
        "attachment".into()
    } else {
        shown.into()
    }
}

/// Messages re-indexed per store lock while the index is rebuilt.
const REBUILD_BATCH: usize = 50;

/// People offered as recipients: plenty to type against, small enough to send at once.
const CONTACTS: usize = 500;
/// Earlier emails of yours an AI reply reads for your voice.
const MY_WORDS: usize = 3;

/// How an account is connected, as the Settings screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub provider: MailService,
    /// e.g. "Gmail · App password".
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(flatten)]
    pub account: Account,
    pub connection: Option<ConnectionInfo>,
    /// The other addresses it sends from, in list order: added and suggested ones, and
    /// dismissed ones while they are blocked. Never the account's own.
    pub addresses: Vec<AccountAddress>,
}

/// Who a browser sign-in is with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserProvider {
    /// Outlook, Hotmail and Microsoft 365: Moss's own client ID, or yours.
    Outlook,
    /// Gmail with your own Google OAuth client.
    Gmail,
}

/// A sign-in in the system browser.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSignIn {
    pub provider: BrowserProvider,
    /// Pre-fills the sign-in page. The account is whichever one signs in.
    pub email: Option<String>,
    /// Your own client: needed for Gmail, optional for Outlook.
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

impl BrowserSignIn {
    /// Outlook with Moss's own client ID, asking the browser which account.
    #[cfg(test)]
    pub fn outlook() -> Self {
        BrowserSignIn { provider: BrowserProvider::Outlook, email: None, client_id: None, client_secret: None }
    }
}

/// A browser sign-in waiting for the browser to come back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStart {
    /// The provider's sign-in page, to open in the browser (and again, or copy).
    pub url: String,
    /// The client ID it signs in with, for the page an IT admin approves Moss on.
    pub client_id: String,
}

fn summary(connection: &AccountConnection) -> String {
    let name = match connection.service {
        MailService::Gmail => "Gmail",
        MailService::Outlook => "Microsoft 365",
        MailService::ICloud => "iCloud",
        MailService::Yahoo => "Yahoo",
        MailService::Fastmail => "Fastmail",
        MailService::Resend => return "Resend · API key".into(),
        MailService::Imap => {
            let host = connection.servers.as_ref().map(|s| s.imap.host.as_str()).unwrap_or("");
            return format!("IMAP · {host}");
        }
    };
    match connection.auth {
        AuthMethod::OAuth2 { .. } => format!("{name} · Signed in with browser"),
        _ => format!("{name} · App password"),
    }
}

/// Changes to an account from Settings or the Syncing screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPatch {
    pub name: Option<String>,
    pub letter: Option<String>,
    pub color: Option<AccountColor>,
    pub sender_name: Option<String>,
}

/// What the engine runs on. The app passes the real ones; tests pass fakes.
pub struct Parts {
    pub store: Store,
    pub secrets: Arc<dyn SecretStore>,
    pub accounts: Accounts,
    pub connector: Arc<dyn Connector>,
    pub events: Arc<dyn Events>,
    /// Milliseconds since the epoch.
    pub clock: fn() -> i64,
}

struct Worker {
    wake: Arc<Notify>,
    task: JoinHandle<()>,
}

/// Inbox, Sent and All Mail folder names, learned from the server.
#[derive(Debug, Clone)]
pub(crate) struct Folders {
    inbox: String,
    sent: Option<String>,
    /// Gmail's All Mail, where archived mail lives.
    all: Option<String>,
    /// Where blocked mail goes. Made the first time someone is blocked.
    blocked: Option<String>,
    /// Where drafts are kept, when the server has such a folder.
    drafts: Option<String>,
    count: u32,
}

pub(crate) struct Shared {
    store: Mutex<Store>,
    credential_lifecycle: tokio::sync::RwLock<()>,
    secrets: Arc<dyn SecretStore>,
    accounts: Accounts,
    connector: Arc<dyn Connector>,
    events: Arc<dyn Events>,
    clock: fn() -> i64,
    /// Each account's provider, with the OAuth token it signs in with (`None` for a password
    /// or key, which doesn't run out).
    providers: Mutex<HashMap<String, (Arc<dyn Provider>, Option<Credential>)>>,
    folders: Mutex<HashMap<String, Folders>>,
    progress: Mutex<HashMap<String, sync::Tally>>,
    /// Why each account isn't syncing, until a sync works again.
    problems: Mutex<HashMap<String, SyncIssue>>,
    workers: Mutex<HashMap<String, Worker>>,
    /// Per account, the last server change sent off, so the next one waits for it.
    on_server: Mutex<HashMap<String, JoinHandle<()>>>,
    server_mutations: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Per account, the pause before its drafts go up, restarted by every save.
    draft_timers: Mutex<HashMap<String, JoinHandle<()>>>,
    /// Wakes the decision model's sorting when mail changes.
    sort_wake: Notify,
    /// Wakes indexing by meaning, and summaries, when mail changes.
    meaning_wake: Notify,
    summary_wake: Notify,
    /// Wakes reading the text inside attachments, when mail changes.
    reading_wake: Notify,
    /// The embedding model, once set, for search by meaning.
    embedder: Mutex<Option<Arc<dyn Embedder>>>,
    /// At most one vector scan, including when its waiting UI request was cancelled.
    meaning_search_slot: Arc<tokio::sync::Semaphore>,
    /// What reads the text inside attachments, once set.
    reader: Mutex<Option<Arc<dyn Reader>>>,
    attachment_read_slots: Arc<tokio::sync::Semaphore>,
    /// When each account's addresses were last looked for (ms).
    discovered: Mutex<HashMap<String, i64>>,
    /// Which mail service each domain's mail goes to, from the DNS, once asked.
    hosts: Mutex<HashMap<String, Option<MailService>>>,
    /// Wakes the sending of emails set to send later, when one is due sooner.
    scheduled_wake: Notify,
    /// Emails set to send later that are on their way now, by draft id.
    // Lock order: scheduled_going, then store. Never acquire this while holding store.
    scheduled_going: Mutex<std::collections::HashSet<String>>,
    /// Last announced schedule revision during the current ownership-recovery failure.
    scheduled_recovery_announced: Mutex<HashMap<String,i64>>,
    /// Recovery finishes before this process can prepare or submit new mail.
    delivery_recovered: Mutex<bool>,
    /// The browser sign-in waiting to come back, with the address typed for it.
    browser_sign_in: Mutex<Option<(PendingAuthorization, Option<String>)>>,
}

fn connection_key(account_id: &str) -> String {
    format!("connection:{account_id}")
}

impl Shared {
    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn account(&self, id: &str) -> Result<Account> {
        self.store()
            .accounts()?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| Error::UnknownAccount(id.into()))
    }

    /// Who an email from this account is from: its own address, or another you added and
    /// haven't blocked, with that address's name if it has one.
    fn sender(&self, account: &Account, address: Option<&str>) -> Result<Person> {
        let own = Person { name: account.sender_name.clone(), email: account.address.clone() };
        let Some(address) =
            address.map(str::trim).filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case(&account.address))
        else {
            return Ok(own);
        };
        match self.store().address(&account.id, address)? {
            Some(a) if a.state == AddressState::Added && !a.blocked => {
                Ok(Person { name: a.name.unwrap_or(own.name), email: a.email })
            }
            _ => Err(Error::NotYourAddress {
                address: address.to_lowercase(),
                account: account.name.clone(),
            }),
        }
    }

    /// An account as the UI lists it: with how it connects and its other addresses.
    fn info(&self, account: Account) -> Result<AccountInfo> {
        let connection = self
            .connection(&account.id)
            .ok()
            .map(|c| ConnectionInfo { provider: c.service, summary: summary(&c) });
        let addresses = self.store().addresses(&account.id)?;
        Ok(AccountInfo { account, connection, addresses })
    }

    fn connection(&self, id: &str) -> Result<AccountConnection> {
        let json = self.store().setting(&connection_key(id))?;
        let json = json.ok_or_else(|| Error::UnknownAccount(id.into()))?;
        serde_json::from_str(&json).map_err(|e| Error::Server(e.to_string()))
    }

    async fn credential(&self, connection: &AccountConnection) -> Result<Credential> {
        if let AuthMethod::OAuth2 { .. } = connection.auth {
            let token = self.accounts.access_token(connection).await;
            return token.map(Credential::Token).map_err(Error::syncing);
        }
        match self.secrets.load(&connection.secret).map_err(|e| Error::Keychain(e.to_string()))? {
            Some(Secret::Password { password }) => Ok(Credential::Password(password)),
            Some(Secret::ApiKey { key }) => Ok(Credential::ApiKey(key)),
            Some(Secret::OAuth { tokens, .. }) => Ok(Credential::Token(tokens.access_token)),
            None => Err(Error::SignedOut("no saved password in the keychain".into())),
        }
    }

    /// The account's provider, kept so its connections stay open. One that signs in with an
    /// OAuth token is built again once the token is refreshed (hourly for Outlook): a mail
    /// server that drops the connection would refuse the old token when it reconnects.
    async fn provider(&self, id: &str) -> Result<Arc<dyn Provider>> {
        let _credentials = self.credential_lifecycle.read().await;
        let kept = crate::lock(&self.providers).get(id).cloned();
        if let Some((provider, None)) = &kept {
            return Ok(provider.clone());
        }
        let connection = self.connection(id)?;
        let credential = self.credential(&connection).await?;
        if let Some((provider, Some(token))) = kept {
            if token == credential {
                return Ok(provider);
            }
        }
        let provider = self.connector.provider(&connection, &credential)?;
        let token = matches!(connection.auth, AuthMethod::OAuth2 { .. }).then_some(credential);
        crate::lock(&self.providers).insert(id.into(), (provider.clone(), token));
        Ok(provider)
    }

    /// See [`Engine::attachment`].
    async fn attachment_bytes(&self, message_id: &str, index: u32) -> Result<Vec<u8>> {
        let unavailable = || Error::Unavailable(format!("attachment {index} of message {message_id}"));
        let source = {
            let store = self.store();
            if let Some(bytes) = store.attachment_content(message_id, index)? {
                return Ok(bytes);
            }
            store.attachment_source(message_id, index)?.ok_or_else(unavailable)?
        };
        let provider = self.provider(&source.account_id).await?;
        for l in &source.locations {
            match provider.fetch_part(&l.mailbox, &l.provider_id, &source.part).await {
                Ok(bytes) => {
                    self.store().keep_attachment(message_id, index, &bytes)?;
                    self.trim_cache()?;
                    return Ok(bytes);
                }
                Err(moss_providers::Error::NotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err(unavailable())
    }

    /// The bytes of the draft's files, ready to send, checked against what `service` takes.
    pub(crate) async fn files_to_send(
        &self,
        draft_id: &str,
        files: &[moss_core::DraftFile],
        service: MailService,
    ) -> Result<Vec<moss_providers::Attachment>> {
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            let data = match &f.forwarded {
                Some(r) => self.attachment_bytes(&r.message_id, r.index).await?,
                None => {
                    let kept = self.store().draft_file_for(draft_id, &f.id)?;
                    let missing = || Error::Unavailable(format!("{} is no longer on this Mac", f.name));
                    kept.ok_or_else(missing)?.2
                }
            };
            out.push(moss_providers::Attachment {
                filename: f.name.clone(),
                content_type: f.content_type.clone(),
                content_id: None,
                data,
            });
        }
        let total = out.iter().map(|a| a.data.len() as u64).sum();
        match attach::too_big(total, service) {
            Some(why) => Err(Error::TooBig(why)),
            None => Ok(out),
        }
    }

    fn forget_provider(&self, id: &str) {
        crate::lock(&self.providers).remove(id);
    }

    fn inbox_name(&self, id: &str) -> String {
        lock(&self.folders).get(id).map_or_else(|| "INBOX".into(), |f| f.inbox.clone())
    }

    fn changed(&self, account_id: &str) {
        self.events.emit(Event::MailChanged { account_id: account_id.into() });
        self.sort_wake.notify_one();
        self.meaning_wake.notify_one();
        self.summary_wake.notify_one();
        self.reading_wake.notify_one();
    }

    fn report(&self, account_id: &str, error: &Error) {
        let issue = SyncIssue {
            account_id: account_id.into(),
            problem: error.problem(),
            message: error.to_string(),
        };
        crate::lock(&self.problems).insert(account_id.into(), issue.clone());
        self.events.emit(Event::SyncError {
            account_id: issue.account_id,
            problem: issue.problem,
            message: issue.message,
        });
    }

    /// A sync worked: forgets the account's problem, and says so if there was one.
    fn recovered(&self, account_id: &str) {
        if crate::lock(&self.problems).remove(account_id).is_some() {
            self.events.emit(Event::SyncOk { account_id: account_id.into() });
        }
    }

    fn start_worker(self: &Arc<Self>, id: &str) {
        let mut workers = crate::lock(&self.workers);
        if let Some(w) = workers.get(id).filter(|w| !w.task.is_finished()) {
            w.wake.notify_one();
            return;
        }
        let wake = Arc::new(Notify::new());
        let task = tokio::spawn(self.clone().supervise(id.to_string(), wake.clone()));
        workers.insert(id.into(), Worker { wake, task });
    }

    /// Runs the account's worker under [`supervised`]. It stops when the account goes (the
    /// worker returns) or when it is stopped, which stops the worker too.
    async fn supervise(self: Arc<Self>, id: String, wake: Arc<Notify>) {
        supervised(move || self.clone().run(id.clone(), wake.clone())).await
    }

    fn stop_worker(&self, id: &str) {
        if let Some(w) = crate::lock(&self.workers).remove(id) {
            w.task.abort();
        }
    }

    /// Runs `job` against the account's server in the background, after the changes already
    /// sent off for it, so an Undo never overtakes the change it undoes.
    fn queue(
        self: &Arc<Self>,
        account_id: &str,
        job: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let mut queue = crate::lock(&self.on_server);
        let before = queue.remove(account_id);
        let task = tokio::spawn(async move {
            if let Some(before) = before {
                // A change that failed has said so already.
                let _ = before.await;
            }
            job.await;
        });
        queue.insert(account_id.into(), task);
    }

    /// Sends the account's waiting changes, oldest first, after any already on their way.
    pub(crate) fn send_waiting(self: &Arc<Self>, account_id: &str) {
        let shared = self.clone();
        let id = account_id.to_string();
        self.queue(account_id, async move {
            if let Err(e) = shared.send_all(&id).await {
                shared.report(&id, &e);
            }
        });
    }

    /// Moves the account's blocked mail out of the Inbox on the server, in the background.
    fn block_on_server(self: &Arc<Self>, account_id: String) {
        let shared = self.clone();
        self.queue(&account_id.clone(), async move {
            let result = match shared.provider(&account_id).await {
                Ok(provider) => shared.move_blocked(&account_id, provider.as_ref()).await,
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                shared.report(&account_id, &e);
            }
        });
    }

    /// Moves mail that's no longer blocked from the Blocked folder back to the Inbox, in the
    /// background.
    fn unblock_on_server(self: &Arc<Self>, account_id: String) {
        let shared = self.clone();
        self.queue(&account_id.clone(), async move {
            let result = match shared.provider(&account_id).await {
                Ok(provider) => shared.move_unblocked(&account_id, provider.as_ref()).await,
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                shared.report(&account_id, &e);
            }
        });
    }

}

/// A change waiting to reach the server, as saved in the store's outbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ServerChange {
    /// Out of the Inbox (Gmail: into All Mail), by the messages' ids in `mailbox`.
    Archive { mailbox: String, ids: Vec<String> },
    Seen { mailbox: String, ids: Vec<String>, seen: bool },
    /// Starred (IMAP `\Flagged`): you moved it to Important. Unstarred: you moved it out.
    Flagged { mailbox: String, ids: Vec<String>, flagged: bool },
    /// Marked forwarded (`$Forwarded`), for other mail apps to show.
    Forwarded { mailbox: String, ids: Vec<String> },
    /// Back in the Inbox, found by Message-ID wherever it was archived to.
    #[serde(rename_all = "camelCase")]
    Unarchive { message_ids: Vec<String> },
    /// Into Trash, found by Message-ID wherever it is.
    #[serde(rename_all = "camelCase")]
    Trash { message_ids: Vec<String> },
    /// Back out of Trash, to the Inbox or the archive.
    #[serde(rename_all = "camelCase")]
    Untrash { message_ids: Vec<String>, to_inbox: bool },
    /// Out of the Drafts folder for good, by Message-ID: sent or discarded.
    #[serde(rename_all = "camelCase")]
    DeleteDrafts { message_ids: Vec<String> },
}

#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
}

impl Engine {
    pub fn new(parts: Parts) -> Self {
        Engine {
            shared: Arc::new(Shared {
                store: Mutex::new(parts.store),
                credential_lifecycle: tokio::sync::RwLock::new(()),
                secrets: parts.secrets,
                accounts: parts.accounts,
                connector: parts.connector,
                events: parts.events,
                clock: parts.clock,
                providers: Mutex::default(),
                folders: Mutex::default(),
                progress: Mutex::default(),
                problems: Mutex::default(),
                workers: Mutex::default(),
                on_server: Mutex::default(),
                server_mutations: Mutex::default(),
                draft_timers: Mutex::default(),
                sort_wake: Notify::new(),
                meaning_wake: Notify::new(),
                summary_wake: Notify::new(),
                reading_wake: Notify::new(),
                embedder: Mutex::default(),
                meaning_search_slot: Arc::new(tokio::sync::Semaphore::new(1)),
                reader: Mutex::default(),
                attachment_read_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                discovered: Mutex::default(),
                hosts: Mutex::default(),
                scheduled_wake: Notify::new(),
                scheduled_going: Mutex::default(),
                scheduled_recovery_announced: Mutex::default(),
                delivery_recovered: Mutex::new(false),
                browser_sign_in: Mutex::default(),
            }),
        }
    }

    /// Lets the decision model sort new mail, from now on, in the background.
    pub fn set_sorter(&self, sorter: Arc<dyn Sorter>) {
        let shared = self.shared.clone();
        keep_running(move || {
            let (shared, sorter) = (shared.clone(), sorter.clone());
            async move {
                loop {
                    shared.sort_waiting(sorter.as_ref()).await;
                    shared.sort_wake.notified().await;
                }
            }
        });
    }

    /// Sorts again now, e.g. once the decision model has downloaded.
    pub fn wake_sorter(&self) {
        self.shared.sort_wake.notify_one();
    }

    /// Starts syncing every connected account, and tidies the database in the background.
    /// Call once the async runtime is up.
    pub fn start(&self) -> Result<()> {
        self.recover_deliveries()?;
        self.retry_credential_cleanup()?;
        let engine = self.clone();
        tokio::spawn(async move { engine.tidy().await });
        self.send_later();
        let accounts = self.shared.store().accounts()?;
        for account in accounts {
            self.shared.recover_server_changes(&account.id)?;
            if self.shared.connection(&account.id).is_ok() {
                self.shared.start_worker(&account.id);
            }
        }
        Ok(())
    }

    pub fn accounts(&self) -> Result<Vec<AccountInfo>> {
        let accounts = self.shared.store().accounts()?;
        accounts.into_iter().map(|account| self.shared.info(account)).collect()
    }

    fn info(&self, id: &str) -> Result<AccountInfo> {
        self.accounts()?
            .into_iter()
            .find(|a| a.account.id == id)
            .ok_or_else(|| Error::UnknownAccount(id.into()))
    }

    /// Saves a signed-in connection as an account (or signs an existing one in again) and
    /// starts syncing it.
    async fn keep(&self, connection: AccountConnection) -> Result<AccountInfo> {
        let _credentials = self.shared.credential_lifecycle.write().await;
        let saved = (|| -> Result<Account> {
            let existing = self.shared.store().accounts()?;
            let account = existing
                .iter()
                .find(|a| a.address.eq_ignore_ascii_case(&connection.email))
                .cloned()
                .unwrap_or_else(|| {
                    naming::new_account(connection.service, &connection.email, &existing)
                });
            let json = serde_json::to_string(&connection).expect("connections serialise");
            {
                let mut store = self.shared.store();
                let previous = store.setting(&connection_key(&account.id))?
                    .map(|json| serde_json::from_str::<AccountConnection>(&json)
                        .map_err(|_| Error::Invalid("The previous connection cannot be read. Its sign-in has been kept.".into())))
                    .transpose()?;
                let retired = previous.filter(|previous| previous.secret != connection.secret)
                    .as_ref().map(credential_cleanup::reference).transpose()?;
                store.save_account_connection_retiring(&account, &json, retired.as_ref())?;
            }
            Ok(account)
        })();
        let account = match saved {
            Ok(account) => account,
            Err(error) => {
                // A successful Keychain save may precede a failed database commit.
                // Cleanup rechecks all active references before deleting anything.
                if let Ok(reference) = credential_cleanup::reference(&connection) {
                    let queued = self.shared.store().retire_credential(&reference).is_ok();
                    if queued { self.shared.clean_credentials(); }
                }
                return Err(error);
            }
        };
        self.shared.clean_credentials();
        self.shared.forget_provider(&account.id);
        self.shared.stop_worker(&account.id);
        self.shared.start_worker(&account.id);
        self.info(&account.id)
    }

    /// App password or IMAP password. Checks it with the server before saving anything.
    pub async fn add_password_account(&self, request: PasswordSignIn) -> Result<AccountInfo> {
        let login = connect::ProviderLogin { connector: self.shared.connector.clone() };
        let connection = self.shared.accounts.sign_in_with_password(request, &login).await?;
        self.keep(connection).await
    }

    /// Starts a sign-in in the browser: open the returned page, then call
    /// [`finish_browser_sign_in`](Self::finish_browser_sign_in). Starting again replaces one still waiting.
    pub async fn start_browser_sign_in(&self, request: BrowserSignIn) -> Result<BrowserStart> {
        let accounts = &self.shared.accounts;
        let email = request.email.as_deref();
        let client_id = request.client_id.as_deref();
        let pending = match request.provider {
            BrowserProvider::Outlook => accounts.start_outlook(email, client_id).await?,
            BrowserProvider::Gmail => {
                accounts
                    .start_gmail_oauth(email, client_id.unwrap_or(""), request.client_secret.as_deref())
                    .await?
            }
        };
        let started = BrowserStart { url: pending.authorize_url().into(), client_id: pending.config().client_id.clone() };
        *crate::lock(&self.shared.browser_sign_in) = Some((pending, request.email));
        Ok(started)
    }

    /// Waits for the browser to come back from the sign-in, then saves the account that
    /// signed in and starts syncing it.
    pub async fn finish_browser_sign_in(&self) -> Result<AccountInfo> {
        let waiting = crate::lock(&self.shared.browser_sign_in).take();
        let (pending, email) = waiting.ok_or_else(|| Error::Invalid("No browser sign-in is waiting. Start again.".into()))?;
        let connection = self.shared.accounts.finish_oauth(pending, email.as_deref()).await?;
        self.keep(connection).await
    }

    pub async fn check_resend_key(&self, key: &str) -> Result<Vec<ResendDomain>> {
        Ok(self.shared.accounts.check_resend_key(key).await?)
    }

    pub async fn add_resend_account(&self, key: &str, email: &str) -> Result<AccountInfo> {
        let connection = self.shared.accounts.sign_in_resend(key, email).await?;
        self.keep(connection).await
    }

    /// Server settings for an address, from its provider's published configuration.
    pub async fn discover(&self, email: &str) -> Result<Discovered> {
        Ok(self.shared.accounts.discover(email).await?)
    }

    pub fn update_account(&self, id: &str, patch: AccountPatch) -> Result<AccountInfo> {
        let mut account = self.shared.account(id)?;
        if let Some(name) = patch.name {
            account.name = name;
        }
        if let Some(letter) = patch.letter {
            account.letter = letter;
        }
        if let Some(color) = patch.color {
            account.color = color;
        }
        if let Some(sender) = patch.sender_name {
            account.sender_name = sender;
        }
        self.shared.store().upsert_account(&account)?;
        self.info(id)
    }

    pub fn reorder_accounts(&self, ids: &[String]) -> Result<()> {
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        Ok(self.shared.store().reorder_accounts(&ids)?)
    }

    /// Forgets an account and its mail on this device, and its secret in the keychain.
    /// Nothing changes on the server.
    /// While server changes haven't reached the server, this is [`Error::UnsentServerChanges`]
    /// saying how many: ask, then [`Engine::remove_account_discarding`].
    pub fn remove_account(&self, id: &str) -> Result<CredentialCleanupStatus> {
        self.remove_account_with(id, None)
    }

    /// Removes the account and the `changes` server changes you were told will be lost.
    /// Unsent or scheduled mail still stops it.
    pub fn remove_account_discarding(&self, id: &str, changes: usize) -> Result<CredentialCleanupStatus> {
        self.remove_account_with(id, Some(changes))
    }

    fn remove_account_with(&self, id: &str, discard: Option<usize>) -> Result<CredentialCleanupStatus> {
        let _credentials = self.shared.credential_lifecycle.try_write()
            .map_err(|_| Error::Invalid("A sign-in is still in use. Try removing the account again when it finishes.".into()))?;
        let mut store = self.shared.store();
        let waiting = store.account_removal_check(id)?;
        if discard.is_none() && waiting > 0 { return Err(Error::UnsentServerChanges(waiting)); }
        let connection: Option<AccountConnection> = store.setting(&connection_key(id))?
            .map(|json| serde_json::from_str(&json).map_err(|_| Error::Invalid("This account's connection cannot be read. Restore it before removing the account.".into())))
            .transpose()?;
        let account = store.accounts()?.into_iter().find(|account| account.id == id);
        if connection.is_none() && account.is_some() {
            return Err(Error::Invalid("This account's connection is missing. Restore it before removing the account so its saved sign-in can be retired safely.".into()));
        }
        if let (Some(account), Some(connection)) = (&account, &connection) {
            if !account.address.eq_ignore_ascii_case(&connection.email) {
                return Err(Error::Invalid("This account's connection belongs to another address. Restore the matching connection before removing it.".into()));
            }
        }
        let cleanup = connection.as_ref().map(credential_cleanup::reference).transpose()?;
        match discard {
            None => store.remove_account_with_cleanup(id, cleanup.as_ref())?,
            Some(changes) => store.remove_account_discarding_changes(id, cleanup.as_ref(), changes)?,
        };
        // The copy from before the last update still holds this account's mail.
        if store.forget_safety_copy().is_err() {
            eprintln!("Moss removed the account but could not delete the copy of the mail from before the last update.");
        }
        // No worker or credential side effect precedes the durable database commit.
        self.shared.stop_worker(id);
        self.shared.forget_provider(id);
        drop(store);
        // The removal commit is already authoritative. Legacy payload cleanup is
        // retryable maintenance and must not make the account appear to survive it.
        let shared=self.shared.clone();
        std::thread::spawn(move || shared.compact_deliveries());
        Ok(self.shared.clean_credentials())
    }

    /// Every open thread (all splits), for one account or all of them.
    pub fn threads(&self, account: Option<&str>) -> Result<Vec<ThreadSummary>> {
        Ok(self.shared.store().list_open(account, self.shared.now())?)
    }

    pub fn split_counts(&self, account: Option<&str>) -> Result<SplitCounts> {
        Ok(self.shared.store().split_counts(account, self.shared.now())?)
    }

    pub fn thread(&self, id: &str) -> Result<Thread> {
        let thread = self.shared.store().thread(id, self.shared.now())?;
        thread.ok_or_else(|| Error::UnknownThread(id.into()))
    }

    fn change_thread_state(&self, thread_id: &str, action: moss_store::ThreadChange) -> Result<()> {
        use moss_store::ThreadChange;
        let account = self.thread(thread_id)?.summary.account_id;
        let inbox = self.shared.inbox_name(&account);
        let changed = self.shared.store().change_thread_with_changes(thread_id, &account, action, self.shared.now(), |store, targets| {
            let changes: Vec<ServerChange> = match action {
                ThreadChange::Done(true) | ThreadChange::Read(_) => {
                    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
                    for l in store.thread_locations(thread_id)? {
                        if l.provider_id.starts_with("local:") || (action == ThreadChange::Done(true) && l.mailbox != inbox) { continue; }
                        match groups.iter_mut().find(|(mailbox, _)| *mailbox == l.mailbox) {
                            Some((_, ids)) => ids.push(l.provider_id),
                            None => groups.push((l.mailbox, vec![l.provider_id])),
                        }
                    }
                    groups.into_iter().map(|(mailbox, ids)| match action {
                        ThreadChange::Read(seen) => ServerChange::Seen { mailbox, ids, seen },
                        _ => ServerChange::Archive { mailbox, ids },
                    }).collect()
                }
                ThreadChange::Done(false) => {
                    let message_ids = store.message_ids(thread_id)?;
                    if message_ids.is_empty() { vec![] } else { vec![ServerChange::Unarchive { message_ids }] }
                }
                ThreadChange::Delete => {
                    if targets.trash.is_empty() { vec![] } else { vec![ServerChange::Trash { message_ids: targets.trash }] }
                }
                ThreadChange::Restore => {
                    let mut changes = Vec::new();
                    if !targets.untrash_inbox.is_empty() { changes.push(ServerChange::Untrash { message_ids: targets.untrash_inbox, to_inbox: true }); }
                    if !targets.untrash_archive.is_empty() { changes.push(ServerChange::Untrash { message_ids: targets.untrash_archive, to_inbox: false }); }
                    changes
                }
            };
            if changes.is_empty() || !server_folders_mutable(store, &account)? { return Ok(vec![]); }
            change_batches::encode(changes)
        })?;
        if let Some(account) = changed { self.shared.send_waiting(&account); }
        Ok(())
    }

    /// Done here and archived on the server, committed as one local action.
    pub fn mark_done(&self, thread_id: &str) -> Result<()> {
        self.change_thread_state(thread_id, moss_store::ThreadChange::Done(true))
    }

    /// Return a done thread to the Inbox here and on the server.
    pub fn mark_not_done(&self, thread_id: &str) -> Result<()> {
        self.change_thread_state(thread_id, moss_store::ThreadChange::Done(false))
    }

    /// Retain local deletion and its server intent in the same commit.
    pub fn delete_thread(&self, thread_id: &str) -> Result<()> {
        self.change_thread_state(thread_id, moss_store::ThreadChange::Delete)
    }

    /// Undo deletion, preserving the original Inbox or archive destination.
    pub fn restore_thread(&self, thread_id: &str) -> Result<()> {
        self.change_thread_state(thread_id, moss_store::ThreadChange::Restore)
    }

    /// Done threads, most recently done first, for one account or all of them.
    pub fn done_threads(&self, account: Option<&str>, limit: usize) -> Result<Vec<ThreadSummary>> {
        Ok(self.shared.store().list_done(account, limit, self.shared.now())?)
    }

    /// Blocks this thread's sender (with `whole_domain`, everyone at their domain) in its
    /// account: their mail leaves the inbox here and moves to the Blocked folder on the server.
    pub fn block_sender(&self, thread_id: &str, whole_domain: bool) -> Result<()> {
        let thread = self.thread(thread_id)?.summary;
        let now = self.shared.now();
        self.shared.store().block_sender(
            &thread.account_id,
            &thread.from.email,
            whole_domain,
            now,
        )?;
        self.shared.block_on_server(thread.account_id);
        Ok(())
    }

    /// Never asks about this thread's sender (with `whole_domain`, anyone at their domain)
    /// again, and shows their images. Only on this device, unless accepting a domain lifts a
    /// block on it: that mail moves back to the inbox on the server too.
    pub fn accept_sender(&self, thread_id: &str, whole_domain: bool) -> Result<()> {
        let thread = self.thread(thread_id)?.summary;
        let now = self.shared.now();
        self.shared.store().accept_sender(
            &thread.account_id,
            &thread.from.email,
            whole_domain,
            now,
        )?;
        if whole_domain {
            self.shared.unblock_on_server(thread.account_id);
        }
        Ok(())
    }

    /// Unblocks (and accepts) this thread's sender, and lifts any block on the address it was
    /// sent to: the mail comes back to the inbox, here and on the server.
    pub fn unblock_sender(&self, thread_id: &str) -> Result<()> {
        let thread = self.thread(thread_id)?.summary;
        let now = self.shared.now();
        {
            let mut store = self.shared.store();
            store.unblock_sender(&thread.account_id, &thread.from.email, now)?;
            // Blocked for the address it was sent to: that block goes too.
            store.unblock_addresses_in(thread_id)?;
        }
        self.shared.unblock_on_server(thread.account_id);
        Ok(())
    }

    /// Everyone you blocked or accepted, by address or domain, in every account, newest first.
    pub fn sender_rules(&self) -> Result<Vec<moss_core::SenderRule>> {
        Ok(self.shared.store().sender_rules()?)
    }

    /// Forgets what you decided about `sender` in this account, so Moss asks about them again.
    /// Mail a forgotten block kept out comes back to the inbox, here and on the server.
    pub fn forget_sender_rule(&self, account_id: &str, sender: &str) -> Result<()> {
        self.shared.store().forget_sender_rule(account_id, sender)?;
        self.shared.unblock_on_server(account_id.to_string());
        Ok(())
    }

    /// Blocked threads, newest first, for one account or all of them.
    pub fn blocked_threads(
        &self,
        account: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ThreadSummary>> {
        Ok(self.shared.store().list_blocked(account, limit, self.shared.now())?)
    }

    /// Read here and on the server.
    pub fn mark_read(&self, thread_id: &str, read: bool) -> Result<()> {
        self.change_thread_state(thread_id, moss_store::ThreadChange::Read(read))
    }

    /// Moves a thread to another split tab, for good; `None` gives it back to Moss's sorting.
    /// To Important stars its latest email, here and on the server (Gmail shows a star, other
    /// apps a flag); anywhere else takes every star away. See `moss_store`'s `moves.rs`.
    pub fn move_thread(&self, thread_id: &str, to: Option<SplitId>) -> Result<()> {
        let expected_account = self.thread(thread_id)?.summary.account_id;
        let flagged = to == Some(SplitId::Important);
        let account = self.shared.store().move_thread_with_changes(thread_id, &expected_account, to, self.shared.now(), |store, messages| {
            let mut groups: Vec<(String, Vec<String>)> = Vec::new();
            for message in messages {
                for l in store.message_locations(message)? {
                    if l.provider_id.starts_with("local:") { continue; }
                    match groups.iter_mut().find(|(m, _)| *m == l.mailbox) {
                        Some((_, ids)) => ids.push(l.provider_id),
                        None => groups.push((l.mailbox, vec![l.provider_id])),
                    }
                }
            }
            if groups.is_empty() || !server_folders_mutable(store, &expected_account)? { return Ok(vec![]); }
            change_batches::encode(groups.into_iter().map(|(mailbox, ids)|
                ServerChange::Flagged { mailbox, ids, flagged }
            ).collect())
        })?;
        self.shared.send_waiting(&account);
        self.shared.changed(&account);
        Ok(())
    }

    /// Waiting for an answer to your latest email in a thread: out of sight until `until`
    /// (ms), back sooner if someone replies. Only on this device. False when the latest email
    /// isn't yours.
    pub fn wait_for_reply(&self, thread_id: &str, until: i64) -> Result<bool> {
        self.thread(thread_id)?;
        Ok(self.shared.store().wait_for_reply(thread_id, until, self.shared.now())?)
    }

    /// Show images from this sender from now on, in this account. Only on this device.
    pub fn allow_images(&self, account_id: &str, email: &str) -> Result<()> {
        Ok(self.shared.store().allow_images(account_id, email)?)
    }

    /// You send from `email` too, with `name` on mail from it when given. Mail here from
    /// it becomes yours, so the lists reload.
    pub fn add_address(&self, account_id: &str, email: &str, name: Option<&str>) -> Result<AccountInfo> {
        self.shared.store().add_address(account_id, email, name)?;
        self.shared.changed(account_id);
        self.info(account_id)
    }

    /// The sender name on mail from an address; blank for the account's own.
    pub fn update_address(&self, account_id: &str, email: &str, name: Option<&str>) -> Result<AccountInfo> {
        let account = self.shared.account(account_id)?;
        mine(&account, self.shared.store().update_address(account_id, email, name))?;
        self.info(account_id)
    }

    /// Removes an address, or says no to one Moss suggested. It isn't suggested again.
    pub fn remove_address(&self, account_id: &str, email: &str) -> Result<AccountInfo> {
        let account = self.shared.account(account_id)?;
        mine(&account, self.shared.store().remove_address(account_id, email))?;
        self.info(account_id)
    }

    /// Blocks mail sent to an address of this account (a catch-all address that leaked to
    /// spammers): it leaves the inbox here and moves to the Blocked folder on the server.
    /// Unblocking brings it back.
    pub fn block_address(&self, account_id: &str, email: &str, blocked: bool) -> Result<AccountInfo> {
        self.shared.store().block_address(account_id, email, blocked)?;
        if blocked {
            self.shared.block_on_server(account_id.into());
        } else {
            self.shared.unblock_on_server(account_id.into());
        }
        self.shared.changed(account_id);
        self.info(account_id)
    }

    /// Looks for the account's other addresses now: Gmail's "Send mail as" list, and the
    /// addresses its mail came to.
    pub async fn discover_addresses(&self, account_id: &str) -> Result<AccountInfo> {
        self.shared.account(account_id)?;
        self.shared.discover_addresses(account_id).await?;
        self.info(account_id)
    }

    /// The signatures mail from an address adds; `None` uses its account's again.
    pub fn set_address_signatures(
        &self,
        account_id: &str,
        email: &str,
        choice: Option<AccountSignatures>,
    ) -> Result<()> {
        let account = self.shared.account(account_id)?;
        let mut store = self.shared.store();
        mine(&account, store.set_address_signatures(account_id, email, choice.as_ref()))
    }

    /// Your signatures and which one each account adds. Only on this device.
    pub fn signatures(&self) -> Result<SignatureSettings> {
        Ok(self.shared.store().signatures()?)
    }

    pub fn save_signature(&self, signature: &Signature) -> Result<()> {
        Ok(self.shared.store().save_signature(signature)?)
    }

    pub fn delete_signature(&self, id: &str) -> Result<()> {
        Ok(self.shared.store().delete_signature(id)?)
    }

    pub fn set_signature_choice(
        &self,
        account_id: &str,
        kind: SignatureUse,
        signature_id: Option<&str>,
    ) -> Result<()> {
        Ok(self.shared.store().set_signature_choice(account_id, kind, signature_id)?)
    }

    pub fn set_let_moss_choose(&self, on: bool) -> Result<()> {
        Ok(self.shared.store().set_let_moss_choose(on)?)
    }

    pub fn mark_signature_used(&self, id: &str) -> Result<()> {
        Ok(self.shared.store().mark_signature_used(id)?)
    }

    /// Remembers that you chose `email` from the recipient suggestions after typing `typed`.
    pub fn pick_recipient(&self, typed: &str, email: &str) -> Result<()> {
        let now = self.shared.now();
        Ok(self.shared.store().pick_recipient(typed, email, now)?)
    }

    /// Who you chose from the recipient suggestions for what you typed, newest first.
    pub fn recipient_picks(&self) -> Result<Vec<moss_core::RecipientPick>> {
        Ok(self.shared.store().recipient_picks()?)
    }

    /// People to suggest when you type a recipient, the ones you write to most first.
    pub fn contacts(&self) -> Result<Vec<Person>> {
        Ok(self.shared.store().contacts(CONTACTS)?.into_iter().map(|c| c.person).collect())
    }

    /// The account, and your address in it, you last wrote `email` from; if you never wrote
    /// them, the account their mail comes to (see [`moss_store::Store::writes_from`]).
    pub fn writes_from(&self, email: &str) -> Result<Option<(String, Option<String>)>> {
        Ok(self.shared.store().writes_from(email)?)
    }

    /// What you wrote `email` before from this account (a few emails, newest first, outside
    /// the conversation `except`), so an AI reply sounds like you.
    pub fn my_words_to(&self, account_id: &str, email: &str, except: &str) -> Result<Vec<String>> {
        Ok(self.shared.store().my_words_to(account_id, email, except, MY_WORDS)?)
    }

    /// Unsent emails, most recently edited first: this Mac's, and those in each account's
    /// Drafts folder (see [`drafts`]).
    pub fn drafts(&self) -> Result<Vec<Draft>> {
        Ok(self.shared.store().editable_drafts()?)
    }

    /// Read one editable draft and its current content identity.
    pub fn editable_draft(&self, id: &str) -> Result<Option<Draft>> {
        Ok(self.shared.store().editable_draft(id)?)
    }

    /// Read an acknowledgement for identical saved content after a lost IPC reply.
    pub fn draft_save_receipt(&self, attempted: &Draft) -> Result<Option<Draft>> {
        Ok(self.shared.store().draft_save_receipt(attempted)?)
    }

    /// Desktop-only local recovery. It is never exposed as an AI attachment source.
    pub fn keep_local_draft_recovery(&self,id:&str,previous:Option<&str>,draft:&Draft)->Result<LocalDraftRecoveryWrite> {
        match self.shared.store().keep_local_draft_recovery(id,previous,draft,self.shared.now()) {
            Ok(recovery)=>Ok(LocalDraftRecoveryWrite::Saved {recovery}),
            Err(moss_store::Error::LocalDraftRecoveryResolved {adopted_id})=>Ok(LocalDraftRecoveryWrite::Resolved {adopted_id}),
            Err(moss_store::Error::Invalid(message))=>Err(Error::LocalRecoveryRejected(message)),
            Err(error)=>Err(error.into()),
        }
    }

    pub fn local_draft_recovery_index(&self)->Result<Vec<moss_store::LocalDraftRecoveryEntry>> {
        Ok(self.shared.store().local_draft_recovery_index()?)
    }

    pub fn local_draft_recovery(&self,id:&str)->Result<Option<moss_store::LocalDraftRecovery>> {
        Ok(self.shared.store().local_draft_recovery(id)?)
    }

    pub fn discard_local_draft_recovery(&self,id:&str,version:&str)->Result<()> {
        Ok(self.shared.store().discard_local_draft_recovery(id,version)?)
    }

    pub fn adopt_local_draft_recovery(&self,id:&str,version:&str,new_id:&str,account:&str,missing:&[String])->Result<Option<Draft>> {
        let draft=self.shared.store().adopt_local_draft_recovery(id,version,new_id,account,missing,self.shared.now()).map_err(|error| match error {
            moss_store::Error::RequestConflict => Error::LocalRecoveryRejected("This recovery changed or was resolved elsewhere. Review it again.".into()),
            moss_store::Error::UnknownAccount(_) => Error::LocalRecoveryRejected("The selected account is no longer available. Choose an account again.".into()),
            moss_store::Error::DraftConflict => Error::LocalRecoveryRejected("The new draft identity is no longer available. Review this recovery again.".into()),
            moss_store::Error::Invalid(message) => Error::LocalRecoveryRejected(message),
            error => error.into(),
        })?;
        if let Some(draft)=&draft {self.shared.push_soon(&draft.from_account_id);}
        Ok(draft)
    }

    /// Saves a draft, replacing the one with the same id. It goes up to the account's
    /// Drafts folder once you stop typing; moved to another account, it leaves the first
    /// one's.
    pub fn save_draft(&self, draft: &Draft) -> Result<Draft> {
        self.save_draft_with(draft, |store| {
            Ok(store.edit_draft(draft, &[], self.shared.now(), drafts::cleanup_changes)?)
        })
    }

    /// Keep account-move cleanup identical for ordinary saves and atomic file imports.
    fn save_draft_with(&self, draft: &Draft, save: impl FnOnce(&mut Store) -> Result<(Draft, Option<String>)>) -> Result<Draft> {
        let (saved, moved) = save(&mut self.shared.store())?;
        if let Some(account) = moved {
            self.shared.send_waiting(&account);
        }
        self.shared.push_soon(&draft.from_account_id);
        Ok(saved)
    }

    /// Deletes a draft, sent or discarded, here and from the server. Returns whether there
    /// was such a draft. Its files go too, unless `keep_files`: it is being sent, and they go
    /// once the email has gone.
    #[cfg(test)]
    pub fn delete_draft(&self, id: &str, keep_files: bool) -> Result<bool> {
        let (removed, account) = self.shared.store().discard_draft(id, keep_files, self.shared.now(), drafts::cleanup_changes)?;
        if let Some(account) = account {
            self.shared.send_waiting(&account);
        }
        Ok(removed)
    }

    /// Discard only the editable version the caller observed.
    pub fn delete_editable_draft(&self, id: &str, revision: Option<&str>, keep_files: bool) -> Result<bool> {
        let (removed, account) = self.shared.store().discard_editable_draft(id, revision, keep_files, self.shared.now(), drafts::cleanup_changes)?;
        if let Some(account) = account { self.shared.send_waiting(&account); }
        Ok(removed)
    }

    /// A small JPEG of a file you attached, like [`Engine::attachment_preview`].
    pub async fn draft_preview(&self, draft_id: &str, id: &str, dir: &std::path::Path) -> Result<Option<Vec<u8>>> {
        self.draft_preview_with(draft_id, id, dir, preview::thumbnail).await
    }

    /// [`Engine::draft_preview`], drawn by `make`.
    pub(crate) async fn draft_preview_with(
        &self,
        draft_id: &str,
        id: &str,
        dir: &std::path::Path,
        make: fn(&[u8], &str) -> Option<Vec<u8>>,
    ) -> Result<Option<Vec<u8>>> {
        let file = self.shared.store().draft_file_for(draft_id, id)?;
        let (name, content_type, data) = file.ok_or_else(|| Error::Unavailable(format!("attached file {id}")))?;
        if !preview::previewable(&name, &content_type) {
            return Ok(None);
        }
        let folder = dir.join(preview::PREVIEWS);
        // Named by what's in the file too, so a thumbnail left by another file with this id
        // (ids were reused before V24) is never shown for it.
        let content = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (&name, &data).hash(&mut h);
            h.finish()
        };
        let path = folder.join("drafts").join(format!("{}-{content:016x}.jpg", safe_file_name(id)));
        if let Ok(kept) = std::fs::read(&path) {
            return Ok((!kept.is_empty()).then_some(kept));
        }
        let made = tokio::task::spawn_blocking(move || make(&data, &name))
            .await
            .expect("making a thumbnail never panics");
        std::fs::create_dir_all(path.parent().expect("in a folder"))
            .and_then(|()| std::fs::write(&path, made.as_deref().unwrap_or_default()))
            .map_err(|e| Error::Unavailable(format!("can't save {}: {e}", path.display())))?;
        Ok(made)
    }

    /// Remind me: hidden until `until` (ms), then back at the top. Only on this device.
    pub fn snooze(&self, thread_id: &str, until: Option<i64>) -> Result<()> {
        Ok(self.shared.store().snooze(thread_id, until)?)
    }

    /// Moves a thread between "needs your reply" and "for your information", for good.
    pub fn set_section(&self, thread_id: &str, section: moss_core::Section) -> Result<()> {
        Ok(self.shared.store().set_section(thread_id, section, self.shared.now())?)
    }

    /// Moves a thread to another split tab, for good.
    pub fn set_split(&self, thread_id: &str, split: moss_core::SplitId) -> Result<()> {
        Ok(self.shared.store().set_split(thread_id, split, self.shared.now())?)
    }

    /// Takes a thread's due day away, for good.
    pub fn clear_deadline(&self, thread_id: &str) -> Result<()> {
        Ok(self.shared.store().clear_deadline(thread_id, self.shared.now())?)
    }

    /// Brings your latest email in a thread back if no one answers, or stops waiting.
    /// Returns whether the latest email was yours.
    pub fn remind_if_no_reply(&self, thread_id: &str, on: bool) -> Result<bool> {
        Ok(self.shared.store().remind_if_no_reply(thread_id, on, sort::WAIT, self.shared.now())?)
    }

    pub fn search(
        &self,
        query: &str,
        account: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        Ok(self.shared.store().search(query, account, limit, self.shared.now())?)
    }

    /// Sends a draft. On success the sent copy is in the thread; returns the thread id.
    pub async fn send(&self, draft: OutgoingDraft) -> Result<String> {
        // Older callers have no draft ID or signature choice. Preserve their exact
        // signature in the recoverable body; current callers use send_saved instead.
        let mut body = draft.body.clone();
        let mut html = draft.body_html.clone();
        if let Some((text, markup)) = compose::signature(&draft.signature) {
            body = format!("{body}\n\n{text}");
            html = format!("{html}<br>{markup}");
        }
        let recovery = Draft {
        revision: None,
            id: uuid::Uuid::new_v4().simple().to_string(), mode: draft.mode,
            thread_id: draft.thread_id.clone(), from_account_id: draft.from_account_id.clone(),
            to: draft.to.clone(), cc: draft.cc.clone(), bcc: draft.bcc.clone(),
            subject: draft.subject.clone(), body, body_html: html,
            remind_if_no_reply: draft.remind_if_no_reply, updated_at: self.shared.now(),
            signature_id: None, signature_chosen: true, from_address: draft.from_address.clone(),
            attachments: draft.attachments.clone(),
        };
        self.send_saved(&recovery, &draft).await
    }

    /// Remembers that `message_id` went out as `sent_id`, and marks it forwarded on the server.
    #[cfg(test)]
    fn forwarded(&self, message_id: &str, sent_id: &str) -> Result<()> {
        let queued = self.record_forwarded(&mut self.shared.store(), message_id, sent_id)?;
        if let Some(account) = queued { self.shared.send_waiting(&account); }
        Ok(())
    }

    fn record_forwarded(&self, store: &mut Store, message_id: &str, sent_id: &str) -> Result<Option<String>> {
        Ok(store.record_forward_with_changes(message_id, sent_id, self.shared.now(), |store, account, locations| {
            let mut groups: Vec<(String, Vec<String>)> = Vec::new();
            for location in locations.into_iter().filter(|l| !l.provider_id.starts_with("local:")) {
                match groups.last_mut() {
                    Some((mailbox, ids)) if *mailbox == location.mailbox => ids.push(location.provider_id),
                    _ => groups.push((location.mailbox, vec![location.provider_id])),
                }
            }
            if groups.is_empty() { return Ok(vec![]); }
            match account_server_mode(store, account)? {
                ServerMode::Mutable => {}
                ServerMode::LocalOnly => return Ok(vec![]),
                ServerMode::Unreadable => {
                    eprintln!("Moss omitted an optional forwarded flag because connection settings were unreadable.");
                    return Ok(vec![]);
                }
            }
            // A courtesy flag must not stall an already accepted email's local receipt.
            // Storage failures still propagate, keeping acceptance available for repair.
            match change_batches::encode(groups.into_iter().map(|(mailbox, ids)| ServerChange::Forwarded { mailbox, ids }).collect()) {
                Ok(changes) => Ok(changes),
                Err(_) => {
                    eprintln!("Moss omitted an optional forwarded flag because its targets could not be encoded safely.");
                    Ok(vec![])
                }
            }
        })?)
    }

    /// The bytes of attachment `index` of a message, to open or save it. Bytes on this
    /// device come from here; the rest from the server, kept as cache for next time.
    pub async fn attachment(&self, message_id: &str, index: u32) -> Result<Vec<u8>> {
        self.shared.attachment_bytes(message_id, index).await
    }

    /// Attachment `index` as a file in `dir`, for the system to open with its default app.
    /// Each attachment gets its own folder, so its file keeps the sender's name. Only the
    /// newest few files stay in `dir`: the bytes are in the cache anyway.
    pub async fn attachment_file(
        &self,
        message_id: &str,
        index: u32,
        dir: &std::path::Path,
    ) -> Result<std::path::PathBuf> {
        let bytes = self.attachment(message_id, index).await?;
        let name = self.shared.store().attachment_name(message_id, index)?.unwrap_or_default();
        let folder = dir.join(safe_file_name(message_id)).join(index.to_string());
        let path = folder.join(safe_file_name(&name));
        std::fs::create_dir_all(&folder)
            .and_then(|()| quarantine::save(&path, &bytes))
            .map_err(|_| Error::Unavailable("The attachment could not be saved safely.".into()))?;
        storage::keep_newest_files(dir, storage::FILES_KEPT);
        Ok(path)
    }

    /// A small JPEG of what's in attachment `index`, made by Quick Look and kept in `dir`
    /// (see [`preview`]). `None` for a file that can't have one, or is too big to fetch
    /// just for this: it shows its icon.
    pub async fn attachment_preview(
        &self,
        message_id: &str,
        index: u32,
        dir: &std::path::Path,
    ) -> Result<Option<Vec<u8>>> {
        self.preview_with(message_id, index, dir, preview::PREVIEW_MAX_BYTES, preview::thumbnail).await
    }

    /// [`Engine::attachment_preview`], for files up to `max_bytes`, drawn by `make`.
    pub(crate) async fn preview_with(
        &self,
        message_id: &str,
        index: u32,
        dir: &std::path::Path,
        max_bytes: u64,
        make: fn(&[u8], &str) -> Option<Vec<u8>>,
    ) -> Result<Option<Vec<u8>>> {
        let unavailable = || Error::Unavailable(format!("attachment {index} of message {message_id}"));
        let file = self.shared.store().attachment(message_id, index)?.ok_or_else(unavailable)?;
        if file.bytes > max_bytes || !preview::previewable(&file.name, &file.content_type) {
            return Ok(None);
        }
        let folder = dir.join(preview::PREVIEWS);
        let path = folder.join(safe_file_name(message_id)).join(format!("{index}.jpg"));
        if let Ok(kept) = std::fs::read(&path) {
            return Ok((!kept.is_empty()).then_some(kept));
        }
        let bytes = self.attachment(message_id, index).await?;
        let made = tokio::task::spawn_blocking(move || make(&bytes, &file.name))
            .await
            .expect("making a thumbnail never panics");
        // An empty file when there's none, so the attachment isn't fetched again for it.
        std::fs::create_dir_all(path.parent().expect("in a folder"))
            .and_then(|()| std::fs::write(&path, made.as_deref().unwrap_or_default()))
            .map_err(|e| Error::Unavailable(format!("can't save {}: {e}", path.display())))?;
        storage::keep_newest_files(&folder, preview::PREVIEWS_KEPT);
        Ok(made)
    }

    /// Fetches what an opened message shows that isn't on this device: its body (stored
    /// with headers only, or not kept by the offline setting), or the images its HTML
    /// shows. Returns whether it changed anything.
    pub async fn load_message(&self, message_id: &str) -> Result<bool> {
        let unavailable = || Error::Unavailable(format!("message {message_id}"));
        let copies = self.shared.store().server_copies(message_id)?.ok_or_else(unavailable)?;
        if copies.has_body {
            let missing = self.shared.store().missing_images(message_id)?;
            let mut fetched = false;
            for index in missing {
                // An image the server no longer has just stays missing.
                fetched |= self.attachment(message_id, index).await.is_ok();
            }
            if fetched {
                self.shared.changed(&copies.account_id);
            }
            return Ok(fetched);
        }
        let provider = self.shared.provider(&copies.account_id).await?;
        for l in &copies.locations {
            match provider.fetch_message(&l.mailbox, &l.provider_id).await {
                Ok(whole) => {
                    let filled = self.shared.store().fill_in(message_id, &whole.raw)?;
                    self.shared.trim_cache()?;
                    self.shared.changed(&copies.account_id);
                    return Ok(filled);
                }
                Err(moss_providers::Error::NotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err(unavailable())
    }

    /// Keeps the database small and its index whole: finishes rebuilding the index if one
    /// is under way, trims the cache to its limit, tidies the keyword index, then gives freed
    /// space back to the disk (the first time, with one VACUUM). Runs off the async threads
    /// and holds the store for one batch or step at a time, letting others have it in
    /// between, so the app stays usable. If it fails, the next start tries again.
    pub async fn tidy(&self) -> Result<()> {
        let shared = self.shared.clone();
        tokio::task::spawn_blocking(move || {
            shared.compact_deliveries();
            // No composer is open yet, so files of drafts that are gone aren't needed.
            shared.store().forget_lost_draft_files()?;
            while shared.store().rebuild_step(REBUILD_BATCH)? > 0 {
                std::thread::yield_now();
            }
            shared.trim_cache()?;
            let mut first = true;
            while shared.store().tidy_index_step(moss_store::INDEX_PAGES, first)? {
                first = false;
                std::thread::yield_now();
            }
            while shared.store().free_space_step(moss_store::FREE_PAGES)? {
                std::thread::yield_now();
            }
            shared.store().empty_wal()?;
            Ok(())
        })
        .await
        .expect("tidying never panics")
    }

    /// Syncs now instead of waiting for IDLE: one account, or all of them.
    pub fn sync_now(&self, account: Option<&str>) {
        for (id, worker) in crate::lock(&self.shared.workers).iter() {
            if account.is_none_or(|a| a == id) {
                worker.wake.notify_one();
            }
        }
    }

    /// The latest sync progress, for screens that open after the events went out.
    pub fn sync_progress(&self, account: &str) -> Option<SyncProgress> {
        let tally = lock(&self.shared.progress).get(account).cloned()?;
        let folders = lock(&self.shared.folders).get(account).map_or(0, |f| f.count);
        Some(tally.progress(account, folders, self.shared.now()))
    }

    /// Why accounts aren't syncing right now, by account id.
    pub fn sync_problems(&self) -> Vec<SyncIssue> {
        let mut all: Vec<_> = crate::lock(&self.shared.problems).values().cloned().collect();
        all.sort_by(|a, b| a.account_id.cmp(&b.account_id));
        all
    }
}

/// Runs the task `start` makes, and starts it again if it panics: after a pause that grows
/// with each crash in a row ([`sync::backoff`]), so a bug can't spin. A task that ran longer
/// than the longest pause starts counting again. It ends when the task returns; stopping it
/// stops the task too.
pub(crate) async fn supervised<F, T>(mut start: F)
where
    F: FnMut() -> T,
    T: std::future::Future<Output = ()> + Send + 'static,
{
    /// Stops the task when its supervisor is stopped.
    struct Running(JoinHandle<()>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let mut crashes = 0u32;
    loop {
        let started = tokio::time::Instant::now();
        let mut task = Running(tokio::spawn(start()));
        match (&mut task.0).await {
            Err(e) if e.is_panic() => {
                crashes = sync::crashes_in_a_row(crashes, started.elapsed());
                tokio::time::sleep(sync::backoff(crashes)).await;
            }
            _ => return,
        }
    }
}

/// Runs a background loop for as long as Moss runs, [`supervised`]: a bug in it stops it only
/// for a pause, not until Moss is opened again.
pub(crate) fn keep_running<F, T>(start: F)
where
    F: FnMut() -> T + Send + 'static,
    T: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(supervised(start));
}

/// Locks `mutex` even after a panic while it was held. Only for data a panic can't leave
/// half changed: each change replaces a value or adds or removes one entry at a time.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;

/// Resolve capabilities inside the same transaction as local state and queue intent.
/// Missing settings retain waiting intent. Malformed settings are distinct from storage errors.
enum ServerMode { Mutable, LocalOnly, Unreadable }

fn account_server_mode(store: &moss_store::Store, account: &str) -> moss_store::Result<ServerMode> {
    let Some(json) = store.setting(&connection_key(account))? else { return Ok(ServerMode::Mutable); };
    Ok(match serde_json::from_str::<AccountConnection>(&json) {
        Ok(connection) if connection.service == MailService::Resend => ServerMode::LocalOnly,
        Ok(_) => ServerMode::Mutable,
        Err(_) => ServerMode::Unreadable,
    })
}

fn server_folders_mutable(store: &moss_store::Store, account: &str) -> moss_store::Result<bool> {
    match account_server_mode(store, account)? {
        ServerMode::Mutable => Ok(true),
        ServerMode::LocalOnly => Ok(false),
        ServerMode::Unreadable => Err(moss_store::Error::Invalid("This account's connection settings could not be read. No local changes were saved.".into())),
    }
}
