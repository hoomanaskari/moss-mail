//! IMAP over TLS, with SMTP for sending. Built on `async-imap` (the client
//! Delta Chat ships, maintained, tokio-native, with IDLE, CONDSTORE and
//! Gmail's extensions) and `lettre` for SMTP.
//!
//! Sync per mailbox, one batch per call:
//! 1. Flag changes since the checkpoint's MODSEQ (CONDSTORE), and on QRESYNC
//!    servers the UIDs expunged since then (VANISHED).
//! 2. New mail above the checkpoint's UIDNEXT, oldest first.
//! 3. Older mail below everything fetched so far, newest first, until the
//!    mailbox is all here. The first sync starts here, so recent mail comes
//!    first.
//!
//! A UIDVALIDITY change resets the mailbox (`SyncBatch::reset`).
//!
//! Gmail shows labels as folders: one message is in INBOX, in each label's
//! folder and in "[Gmail]/All Mail". Every copy has the same `gmail_msg_id`,
//! and its labels come in `labels`.

mod connect;
mod convert;
mod slim;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use async_imap::imap_proto::{AttributeValue, MailboxDatum, MessageSection, Response, ResponseCode, SectionPath, Status};
use async_imap::types::{Fetch, Name, UnsolicitedResponse};
use async_trait::async_trait;
use chrono::Utc;
use futures::TryStreamExt as _;
use serde::{Deserialize, Serialize};

#[cfg(test)]
pub(crate) use connect::greeted;
use connect::TcpConnector;
pub(crate) use connect::{keep_alive, Connector, Io};
use convert::{parse_provider_id, provider_id, uid_set};

use crate::stall::Watch;
use crate::types::CheckpointState;
use crate::{
    Capabilities, Change, Checkpoint, ConversationMessage, Credentials, DraftListing, Error, FlagUpdate, Mailbox,
    MailboxRole, OutgoingMessage, Provider, RawMessage, Result, SavedDraft, Security, SentMessage,
    SmtpConfig, SyncBatch, TlsRoots, Wakeup,
};

/// Where the IMAP server is and how to sign in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub credentials: Credentials,
    pub tls_roots: TlsRoots,
}

/// IMAP sync position for one mailbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapCheckpoint {
    uid_validity: u32,
    /// Every UID below this has been seen.
    uid_next: u32,
    /// CONDSTORE position for flag changes.
    highest_modseq: Option<u64>,
    /// Older mail below this UID is still to fetch; 1 when it's all here.
    backfill_below: u32,
}

#[cfg(test)]
impl ImapCheckpoint {
    pub(crate) fn for_tests() -> Self {
        ImapCheckpoint {
            uid_validity: 1,
            uid_next: 1,
            highest_modseq: None,
            backfill_below: 1,
        }
    }
}

type Session = async_imap::Session<Io>;

/// How much of each message to fetch.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Content {
    /// All of it, attachments too: for opening one message.
    Whole,
    /// What Moss shows, leaving big attachments on the server: for sync.
    Shown,
    /// The header block: for mail only needed for threading.
    Headers,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Signing in: how long the server may take to answer LOGIN, CAPABILITY and ENABLE.
const SIGN_IN_STALL: Duration = Duration::from_secs(30);
/// After that, how long any command may wait with nothing arriving before the connection
/// counts as stuck and is dropped. Generous, as a SEARCH or MOVE on a big mailbox can take a
/// while to start answering; a FETCH that keeps arriving is never cut, however big. A dead
/// connection fails sooner, through TCP keepalive (see `connect`).
const COMMAND_STALL: Duration = Duration::from_secs(120);

pub struct ImapProvider {
    connector: Arc<dyn Connector>,
    credentials: std::sync::Mutex<Credentials>,
    smtp: std::sync::Mutex<SmtpConfig>,
    save_sent: bool,
    /// For everything but IDLE.
    main: tokio::sync::Mutex<Option<Conn>>,
    /// IDLE blocks its connection, so it gets its own.
    idle: tokio::sync::Mutex<Option<Conn>>,
}

struct Caps {
    idle: bool,
    condstore: bool,
    qresync: bool,
    mv: bool,
    uidplus: bool,
    gmail: bool,
}

struct Conn {
    session: Session,
    /// How long its reads and writes may wait; off while IDLE waits for mail.
    watch: Watch,
    caps: Caps,
    mailboxes: Option<Vec<Mailbox>>,
    /// Per mailbox: its UIDVALIDITY and the older UIDs still to fetch.
    backfill: HashMap<String, (u32, Vec<u32>)>,
    /// Without CONDSTORE: per mailbox, its UIDVALIDITY and the flags last seen on its newest
    /// messages, to tell what changed in another app.
    flags_seen: HashMap<String, (u32, HashMap<String, FlagUpdate>)>,
    /// VANISHED notifications read by the checked SEARCH response loop.
    search_vanished: Vec<std::ops::RangeInclusive<u32>>,
}

/// Without CONDSTORE, how many of a mailbox's newest messages each sync looks at for flags
/// changed elsewhere (a star, read on the phone). Older mail keeps what it had.
const FLAG_WINDOW: u32 = 1000;

/// Runs `$body` on a signed-in connection from `$slot`, connecting first if
/// needed. A network error (a server that stopped answering too) drops the connection, so the
/// next call reconnects instead of reusing one in an unknown state.
macro_rules! with_conn {
    ($provider:expr, $slot:ident, |$conn:ident| $body:expr) => {{
        let mut slot = $provider.$slot.lock().await;
        if slot.is_none() {
            *slot = Some($provider.open().await?);
        }
        let $conn = slot.as_mut().expect("connected above");
        let result = $body.await;
        if matches!(result, Err(Error::Network(_))) {
            *slot = None;
        }
        result
    }};
}

impl ImapProvider {
    pub fn new(imap: ImapConfig, smtp: SmtpConfig) -> Self {
        let connector = TcpConnector {
            host: imap.host,
            port: imap.port,
            security: imap.security,
            roots: imap.tls_roots,
            timeout: CONNECT_TIMEOUT,
        };
        Self::with_connector(Arc::new(connector), imap.credentials, smtp)
    }

    pub(crate) fn with_connector(
        connector: Arc<dyn Connector>,
        credentials: Credentials,
        smtp: SmtpConfig,
    ) -> Self {
        ImapProvider {
            connector,
            credentials: std::sync::Mutex::new(credentials),
            smtp: std::sync::Mutex::new(smtp),
            save_sent: true,
            main: tokio::sync::Mutex::new(None),
            idle: tokio::sync::Mutex::new(None),
        }
    }

    /// Whether `send` appends a copy to the Sent folder (default: yes). Turn
    /// it off for servers that file sent mail themselves, like Gmail.
    pub fn save_sent_copies(mut self, save: bool) -> Self {
        self.save_sent = save;
        self
    }

    /// New credentials (say, a refreshed OAuth token), used from the next
    /// connection on.
    pub fn set_credentials(&self, imap: Credentials, smtp: Credentials) {
        *self.credentials.lock().unwrap() = imap;
        self.smtp.lock().unwrap().credentials = smtp;
    }

    /// The ids of every message in `mailbox`. On servers without QRESYNC this
    /// is how to find mail deleted elsewhere: compare with what's stored.
    pub async fn message_ids(&self, mailbox: &str) -> Result<Vec<String>> {
        with_conn!(self, main, |conn| conn.message_ids(mailbox))
    }

    async fn append_sent(&self, raw: &[u8]) -> Result<bool> {
        with_conn!(self, main, |conn| conn.append_sent(raw))
    }

    async fn open(&self) -> Result<Conn> {
        let watch = Watch::new(SIGN_IN_STALL);
        let client = self.connector.connect(&watch).await?;
        let credentials = self.credentials.lock().unwrap().clone();
        let mut session = login(client, &credentials).await?;
        let caps = session.capabilities().await.map_err(imap_error)?;
        let caps = Caps {
            idle: caps.has_str("IDLE"),
            condstore: caps.has_str("CONDSTORE"),
            qresync: caps.has_str("QRESYNC"),
            mv: caps.has_str("MOVE"),
            uidplus: caps.has_str("UIDPLUS"),
            gmail: caps.has_str("X-GM-EXT-1"),
        };
        if caps.qresync {
            session
                .run_command_and_check_ok("ENABLE QRESYNC")
                .await
                .map_err(imap_error)?;
        }
        watch.set(COMMAND_STALL);
        Ok(Conn {
            session,
            watch,
            caps,
            mailboxes: None,
            backfill: HashMap::new(),
            flags_seen: HashMap::new(),
            search_vanished: Vec::new(),
        })
    }
}

async fn login(client: async_imap::Client<Io>, credentials: &Credentials) -> Result<Session> {
    let result = match credentials {
        Credentials::Password { username, password } => {
            client.login(username, password.expose()).await
        }
        Credentials::OAuth2 {
            username,
            access_token,
        } => {
            let auth = XOAuth2 {
                username: username.clone(),
                token: access_token.expose().to_string(),
                sent: false,
            };
            client.authenticate("XOAUTH2", auth).await
        }
    };
    result.map_err(|(e, _)| match e {
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => Error::Auth("The server refused sign-in.".into()),
        e => imap_error(e),
    })
}

struct XOAuth2 {
    username: String,
    token: String,
    sent: bool,
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        // A second challenge carries the server's error; an empty reply ends it.
        if std::mem::replace(&mut self.sent, true) {
            return String::new();
        }
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.username, self.token
        )
    }
}

pub(crate) fn imap_error(e: async_imap::error::Error) -> Error {
    use async_imap::error::Error as E;
    match e {
        E::Io(e) => imap_io_error(e),
        E::ConnectionLost => Error::Network("The server closed the connection.".into()),
        E::Parse(_) => Error::Network("The server sent an unreadable IMAP response.".into()),
        E::No(_) | E::Bad(_) => Error::Protocol("The server refused the IMAP command.".into()),
        _ => Error::Protocol("The IMAP operation could not be completed.".into()),
    }
}

/// Parser I/O errors may embed the raw response buffer, including mail content.
fn imap_io_error(e: std::io::Error) -> Error {
    if crate::stall::is_stall(&e) {
        return Error::Network("The server stopped answering.".into());
    }
    Error::Network("The IMAP connection could not read or write a response.".into())
}

impl Conn {
    async fn select(&mut self, mailbox: &str) -> Result<async_imap::types::Mailbox> {
        // A selected mailbox is authoritative only after its matching tagged OK.
        // The dependency's SELECT parser also returns metadata on EOF.
        if mailbox.contains(['\r', '\n', '\0']) {
            return Err(Error::Protocol("The mailbox name contains an invalid character.".into()));
        }
        self.search_vanished.clear();
        while self.session.unsolicited_responses.try_recv().is_ok() {}
        let quoted = mailbox.replace('\\', "\\\\").replace('"', "\\\"");
        let modifier = if self.caps.condstore || self.caps.qresync { " (CONDSTORE)" } else { "" };
        let expected = self.session.run_command(format!("SELECT \"{quoted}\"{modifier}"))
            .await.map_err(imap_error)?;
        let mut info = async_imap::types::Mailbox::default();
        let mut exists = false;
        loop {
            let response = self.session.read_response().await
                .map_err(|_| Error::Network("The mailbox selection response could not be read.".into()))?
                .ok_or_else(|| Error::Network("The server closed before selecting the mailbox.".into()))?;
            if let Response::Data { status: Status::Ok, outcome }
                | Response::Done { status: Status::Ok, outcome, .. } = response.parsed() {
                // imap-proto 0.17 leaves unknown response codes in information.
                // CLOSED separates the old mailbox's metadata from the new one's.
                if outcome.code.is_none() && outcome.information.as_deref()
                    .and_then(|text| text.split_ascii_whitespace().next())
                    .is_some_and(|code| code.eq_ignore_ascii_case("[CLOSED]")) {
                    info = async_imap::types::Mailbox::default();
                    exists = false;
                }
                match &outcome.code {
                    Some(ResponseCode::UidValidity(value)) => {
                        if info.uid_validity.is_some_and(|old| old != *value) {
                            return Err(Error::Network("The mailbox returned conflicting UIDVALIDITY values.".into()));
                        }
                        info.uid_validity = Some(*value);
                    }
                    Some(ResponseCode::UidNext(value)) => info.uid_next = Some(*value),
                    Some(ResponseCode::HighestModSeq(value)) => info.highest_modseq = Some(*value),
                    _ => {}
                }
            }
            match response.parsed() {
                Response::MailboxData(MailboxDatum::Exists(value)) => { info.exists = *value; exists = true; }
                Response::Done { tag, status, .. } => {
                    if tag != &expected {
                        return Err(Error::Network("The server returned an unexpected mailbox selection completion.".into()));
                    }
                    match status {
                        Status::No => return Err(Error::NotFound("The server refused to select the mailbox.".into())),
                        Status::Ok => {}
                        _ => return Err(Error::Protocol("The server rejected mailbox selection.".into())),
                    }
                    if info.uid_validity.unwrap_or(0) == 0 {
                        return Err(Error::Network("The mailbox has no valid UIDVALIDITY.".into()));
                    }
                    if !exists || info.uid_next == Some(0) {
                        return Err(Error::Network("The mailbox selection metadata is incomplete or invalid.".into()));
                    }
                    return Ok(info);
                }
                Response::Data { status: Status::Bye, .. } | Response::Continue(_) => {
                    return Err(Error::Network("The server interrupted mailbox selection.".into()));
                }
                // No QRESYNC parameters are sent by SELECT. Expunge recovery is
                // requested later with FETCH; these may belong to the old mailbox.
                _ => {}
            }
        }
    }

    async fn search(&mut self, query: &str) -> Result<Vec<u32>> {
        // async-imap 0.12's uid_search stops at a tagged response without checking
        // its status, and accepts EOF. Neither may establish an empty mailbox.
        // A tagged OK with no result line is "nothing matched": iCloud sends no line then.
        let expected = self.session.run_command(format!("UID SEARCH {query}")).await.map_err(imap_error)?;
        let mut ids = Vec::new();
        loop {
            let response = self.session.read_response().await.map_err(imap_io_error)?
                .ok_or_else(|| Error::Network("The server closed before completing the search.".into()))?;
            match response.parsed() {
                Response::MailboxData(MailboxDatum::Search(found)) => ids.extend(found),
                Response::Done { tag, status, .. } => {
                    if tag != &expected {
                        return Err(Error::Network("The server returned an unexpected search completion.".into()));
                    }
                    if *status != Status::Ok {
                        return Err(Error::Protocol("The server refused the search.".into()));
                    }
                    ids.sort_unstable();
                    ids.dedup();
                    return Ok(ids);
                }
                Response::Data { status: Status::Bye, .. } | Response::Continue(_) => {
                    return Err(Error::Network("The server interrupted the search.".into()));
                }
                Response::Vanished { uids, .. } => self.search_vanished.extend(uids.iter().cloned()),
                _ => {}
            }
        }
    }

    /// FETCH by message sequence numbers rather than UIDs.
    async fn fetch_seq(&mut self, set: &str, query: &str) -> Result<Vec<Fetch>> {
        let stream = self.session.fetch(set, query).await.map_err(imap_error)?;
        stream.try_collect().await.map_err(imap_error)
    }

    async fn fetch(&mut self, set: &str, query: &str) -> Result<Vec<Fetch>> {
        let stream = self
            .session
            .uid_fetch(set, query)
            .await
            .map_err(imap_error)?;
        stream.try_collect().await.map_err(imap_error)
    }

    fn message_items(&self, content: Content) -> String {
        let body = match content {
            Content::Whole => "BODY.PEEK[]".to_string(),
            Content::Shown => "RFC822.SIZE BODYSTRUCTURE".to_string(),
            Content::Headers => "BODY.PEEK[HEADER]".to_string(),
        };
        let gmail = if self.caps.gmail {
            " X-GM-MSGID X-GM-LABELS"
        } else {
            ""
        };
        format!("(UID FLAGS INTERNALDATE {body}{gmail})")
    }

    fn flag_items(&self) -> &'static str {
        if self.caps.gmail {
            "UID FLAGS X-GM-LABELS"
        } else {
            "UID FLAGS"
        }
    }

    async fn messages(
        &mut self,
        mailbox: &str,
        validity: u32,
        uids: &[u32],
        newest_first: bool,
        content: Content,
    ) -> Result<Vec<RawMessage>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let query = self.message_items(content);
        let mut fetches = self.fetch(&uid_set(uids), &query).await?;
        fetches.sort_by_key(|f| f.uid);
        if newest_first {
            fetches.reverse();
        }
        if content != Content::Shown {
            let headers_only = content == Content::Headers;
            return Ok(fetches
                .iter()
                .filter_map(|f| convert::raw_message(f, mailbox, validity, headers_only))
                .collect());
        }
        // The first command brought sizes and structures: small messages come whole in one
        // more, big ones without their attachments.
        let big = |f: &Fetch| f.size.is_some_and(|size| size > slim::WHOLE_BELOW);
        let small: Vec<u32> = fetches.iter().filter(|f| !big(f)).filter_map(|f| f.uid).collect();
        let mut bodies = HashMap::new();
        if !small.is_empty() {
            for f in self.fetch(&uid_set(&small), "(UID BODY.PEEK[])").await? {
                if let (Some(uid), Some(body)) = (f.uid, f.body()) {
                    bodies.insert(uid, body.to_vec());
                }
            }
        }
        let mut out = Vec::new();
        for f in &fetches {
            let raw = if big(f) {
                Some(self.shown_parts(f).await?)
            } else {
                f.uid.and_then(|uid| bodies.remove(&uid))
            };
            // A message deleted between the two commands is left for the next sync.
            if let Some(message) = raw.and_then(|raw| convert::message(f, mailbox, validity, raw)) {
                out.push(message);
            }
        }
        Ok(out)
    }

    /// A big message without its attachments (see `slim.rs`), or whole when it can't be.
    async fn shown_parts(&mut self, first: &Fetch) -> Result<Vec<u8>> {
        let uid = first.uid.unwrap_or_default().to_string();
        if let Some((structure, sections)) =
            first.bodystructure().and_then(|s| Some((s, slim::wanted(s)?)))
        {
            let mut query = "(UID BODY.PEEK[HEADER]".to_string();
            for section in &sections {
                let numbers: Vec<String> = section.iter().map(u32::to_string).collect();
                query += &format!(" BODY.PEEK[{}]", numbers.join("."));
            }
            query.push(')');
            let got = self.fetch(&uid, &query).await?;
            let rebuilt = got.first().and_then(|f| {
                let part = |path: &[u32]| {
                    f.section(&SectionPath::Part(path.to_vec(), None))
                        .map(<[u8]>::to_vec)
                };
                slim::rebuild(f.header()?, structure, &part)
            });
            if let Some(raw) = rebuilt {
                return Ok(raw);
            }
        }
        let query = self.message_items(Content::Whole);
        let whole = self.fetch(&uid, &query).await?;
        Ok(whole.first().and_then(Fetch::body).unwrap_or_default().to_vec())
    }

    async fn mailboxes(&mut self) -> Result<Vec<Mailbox>> {
        if let Some(list) = &self.mailboxes {
            return Ok(list.clone());
        }
        let names: Vec<Name> = self
            .session
            .list(Some(""), Some("*"))
            .await
            .map_err(imap_error)?
            .try_collect()
            .await
            .map_err(imap_error)?;
        let list = convert::mailboxes(&names);
        self.mailboxes = Some(list.clone());
        Ok(list)
    }

    async fn role_mailbox(&mut self, role: MailboxRole) -> Result<Option<String>> {
        Ok(self
            .mailboxes()
            .await?
            .into_iter()
            .find(|m| m.role == Some(role))
            .map(|m| m.id))
    }

    /// The highest UID in the selected mailbox, for servers that don't say UIDNEXT.
    async fn highest_uid(&mut self, exists: u32) -> Result<u32> {
        if exists == 0 {
            return Ok(0);
        }
        Ok(self
            .fetch("*", "(UID)")
            .await?
            .iter()
            .filter_map(|f| f.uid)
            .max()
            .unwrap_or(0))
    }

    /// VANISHED responses the server sent since the last call.
    fn take_vanished(&mut self, validity: u32) -> Vec<String> {
        let mut out: Vec<_> = self.search_vanished.drain(..).flatten().map(|uid| provider_id(validity, uid)).collect();
        while let Ok(response) = self.session.unsolicited_responses.try_recv() {
            if let UnsolicitedResponse::Other(data) = response {
                if let Response::Vanished { uids, .. } = data.parsed() {
                    out.extend(
                        uids.iter()
                            .flat_map(|r| r.clone())
                            .map(|uid| provider_id(validity, uid)),
                    );
                }
            }
        }
        out
    }

    async fn fetch_since(
        &mut self,
        mailbox: &str,
        checkpoint: Option<ImapCheckpoint>,
        limit: usize,
        older_headers_only: bool,
    ) -> Result<SyncBatch> {
        let limit = limit.max(1);
        let info = self.select(mailbox).await?;
        let validity = info.uid_validity.unwrap_or(0);
        let server_next = match info.uid_next {
            Some(next) => next,
            None => self.highest_uid(info.exists).await? + 1,
        };
        let server_modseq = info.highest_modseq;

        let (mut cp, reset, fresh) = match checkpoint {
            Some(cp) if cp.uid_validity == validity => (cp, false, false),
            earlier => (
                ImapCheckpoint {
                    uid_validity: validity,
                    uid_next: server_next,
                    highest_modseq: server_modseq,
                    backfill_below: server_next,
                },
                earlier.is_some(),
                true,
            ),
        };

        // 1. Flag changes and expunges among the mail we already have.
        let mut flag_updates = Vec::new();
        let mut vanished = Vec::new();
        if let (false, Some(since), Some(now)) = (fresh, cp.highest_modseq, server_modseq) {
            if now > since && cp.uid_next > 1 {
                let vanished_too = if self.caps.qresync { " VANISHED" } else { "" };
                let query = format!(
                    "({}) (CHANGEDSINCE {since}{vanished_too})",
                    self.flag_items()
                );
                let known = cp.uid_next;
                let fetches = self.fetch(&format!("1:{}", known - 1), &query).await?;
                flag_updates = fetches
                    .iter()
                    .filter(|f| f.uid.is_some_and(|uid| uid < known))
                    .filter_map(|f| convert::flag_update(f, validity))
                    .collect();
                vanished = self.take_vanished(validity);
            }
        }
        // Without CONDSTORE the server can't say what changed: look at the newest messages'
        // flags and compare them with the last look. The first look reports them all.
        if !self.caps.condstore && !fresh && info.exists > 0 {
            let from = info.exists.saturating_sub(FLAG_WINDOW - 1).max(1);
            let query = format!("({})", self.flag_items());
            let fetches = self.fetch_seq(&format!("{from}:{}", info.exists), &query).await?;
            let known = cp.uid_next;
            let (seen_validity, seen) = self
                .flags_seen
                .entry(mailbox.to_string())
                .or_insert_with(|| (validity, HashMap::new()));
            if *seen_validity != validity {
                *seen_validity = validity;
                seen.clear();
            }
            for update in fetches
                .iter()
                .filter(|f| f.uid.is_some_and(|uid| uid < known))
                .filter_map(|f| convert::flag_update(f, validity))
            {
                if seen.get(&update.provider_id) != Some(&update) {
                    seen.insert(update.provider_id.clone(), update.clone());
                    flag_updates.push(update);
                }
            }
        }
        cp.highest_modseq = server_modseq;

        // 2. New mail, oldest first.
        let mut more = false;
        let mut new_uids = Vec::new();
        if server_next > cp.uid_next {
            // "n:*" always matches the highest UID, even below n.
            let from = cp.uid_next;
            new_uids = self.search(&format!("UID {from}:*")).await?;
            new_uids.retain(|uid| *uid >= from);
            if new_uids.len() > limit {
                new_uids.truncate(limit);
                cp.uid_next = new_uids[limit - 1] + 1;
                more = true;
            } else {
                cp.uid_next = server_next;
            }
        }
        let mut messages = self
            .messages(mailbox, validity, &new_uids, false, Content::Shown)
            .await?;

        // 3. Older mail, newest first.
        let older_content = if older_headers_only {
            Content::Headers
        } else {
            Content::Shown
        };
        let room = limit - new_uids.len();
        if cp.backfill_below > 1 && room > 0 {
            let below = cp.backfill_below;
            if self
                .backfill
                .get(mailbox)
                .is_none_or(|(v, _)| *v != validity)
            {
                let uids = self.search(&format!("UID 1:{}", below - 1)).await?;
                self.backfill.insert(mailbox.to_string(), (validity, uids));
            }
            let (_, pending) = self.backfill.get_mut(mailbox).expect("filled above");
            pending.retain(|uid| *uid < below);
            let older = pending.split_off(pending.len().saturating_sub(room));
            cp.backfill_below = if pending.is_empty() { 1 } else { older[0] };
            messages.extend(
                self.messages(mailbox, validity, &older, true, older_content)
                    .await?,
            );
        }
        more |= cp.backfill_below > 1;

        Ok(SyncBatch {
            reset,
            messages,
            flag_updates,
            vanished,
            checkpoint: Checkpoint(CheckpointState::Imap(cp)),
            more,
        })
    }

    async fn fetch_message(&mut self, mailbox: &str, id: &str) -> Result<RawMessage> {
        let (validity, uid) = parse_provider_id(id)?;
        let info = self.select(mailbox).await?;
        let missing = || Error::NotFound(format!("message {id} is no longer in {mailbox}"));
        if info.uid_validity.unwrap_or(0) != validity {
            return Err(missing());
        }
        self.messages(mailbox, validity, &[uid], false, Content::Whole)
            .await?
            .pop()
            .ok_or_else(missing)
    }

    /// One part of a message: its MIME header and body, decoded. Servers that won't give a
    /// part's MIME header (some, for a message that is a single part) send the whole message.
    async fn fetch_part(&mut self, mailbox: &str, id: &str, section: &str) -> Result<Vec<u8>> {
        let missing = || Error::NotFound(format!("part {section} of message {id} in {mailbox}"));
        let path = crate::parts::valid_section(section).ok_or_else(missing)?;
        let (validity, uid) = parse_provider_id(id)?;
        let info = self.select(mailbox).await?;
        if info.uid_validity.unwrap_or(0) != validity {
            return Err(missing());
        }
        let query = format!("(UID BODY.PEEK[{section}.MIME] BODY.PEEK[{section}])");
        let fetches = self.fetch(&uid.to_string(), &query).await?;
        let fetch = fetches.iter().find(|f| f.uid == Some(uid)).ok_or_else(missing)?;
        let mime = fetch.section(&SectionPath::Part(path.clone(), Some(MessageSection::Mime)));
        let body = fetch.section(&SectionPath::Part(path, None));
        match (mime, body) {
            (Some(mime), Some(body)) => Ok(crate::parts::decode(mime, body)),
            _ => {
                let whole = self.fetch_message(mailbox, id).await?;
                crate::parts::part(&whole.raw, section).ok_or_else(missing)
            }
        }
    }

    /// Selects `mailbox` and keeps the UIDs of `ids` that are from its current UIDVALIDITY.
    /// Ids that aren't IMAP ids (say, `local:` ones) are skipped.
    async fn current_uids(&mut self, mailbox: &str, ids: &[String]) -> Result<(u32, Vec<u32>)> {
        let validity = self.select(mailbox).await?.uid_validity.unwrap_or(0);
        let uids = ids
            .iter()
            .filter_map(|id| parse_provider_id(id).ok())
            .filter(|(v, _)| *v == validity)
            .map(|(_, uid)| uid)
            .collect();
        Ok((validity, uids))
    }

    async fn fetch_messages(&mut self, mailbox: &str, ids: &[String]) -> Result<Vec<RawMessage>> {
        let (validity, uids) = self.current_uids(mailbox, ids).await?;
        self.messages(mailbox, validity, &uids, true, Content::Shown)
            .await
    }

    async fn fetch_headers(&mut self, mailbox: &str, ids: &[String]) -> Result<Vec<RawMessage>> {
        let (validity, uids) = self.current_uids(mailbox, ids).await?;
        self.messages(mailbox, validity, &uids, true, Content::Headers)
            .await
    }

    async fn conversations_of(
        &mut self,
        mailbox: &str,
        ids: &[String],
    ) -> Result<Option<Vec<(String, u64)>>> {
        if !self.caps.gmail {
            return Ok(None);
        }
        if ids.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let (validity, uids) = self.current_uids(mailbox, ids).await?;
        if uids.is_empty() {
            return Ok(Some(Vec::new()));
        }
        // async-imap reads X-GM-THRID but doesn't hand it out, so read the replies directly.
        let tag = self
            .session
            .run_command(format!("UID FETCH {} (UID X-GM-THRID)", uid_set(&uids)))
            .await
            .map_err(imap_error)?;
        let mut found = Vec::new();
        loop {
            let response = self
                .session
                .read_response()
                .await
                .map_err(imap_io_error)?
                .ok_or_else(|| imap_error(async_imap::error::Error::ConnectionLost))?;
            match response.parsed() {
                Response::Fetch(_, attributes) => {
                    let uid = attributes.iter().find_map(|a| match a {
                        AttributeValue::Uid(uid) => Some(*uid),
                        _ => None,
                    });
                    let thread = attributes.iter().find_map(|a| match a {
                        AttributeValue::GmailThrId(id) => Some(*id),
                        _ => None,
                    });
                    if let (Some(uid), Some(thread)) = (uid, thread) {
                        found.push((provider_id(validity, uid), thread));
                    }
                }
                Response::Done {
                    tag: done,
                    status,
                    ..
                } if *done == tag => {
                    return match status {
                        Status::Ok => Ok(Some(found)),
                        _ => Err(Error::Protocol("The server refused the conversation lookup.".into())),
                    };
                }
                _ => {}
            }
        }
    }

    async fn conversation_messages(
        &mut self,
        mailbox: &str,
        conversations: &[u64],
    ) -> Result<Vec<ConversationMessage>> {
        if !self.caps.gmail || conversations.is_empty() {
            return Ok(Vec::new());
        }
        let validity = self.select(mailbox).await?.uid_validity.unwrap_or(0);
        // One search for them all: "OR OR X-GM-THRID 1 X-GM-THRID 2 X-GM-THRID 3".
        let keys: Vec<String> = conversations
            .iter()
            .map(|id| format!("X-GM-THRID {id}"))
            .collect();
        let query = format!("{}{}", "OR ".repeat(keys.len() - 1), keys.join(" "));
        let uids = self.search(&query).await?;
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let items = "(UID FLAGS X-GM-LABELS BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])";
        let mut fetches = self.fetch(&uid_set(&uids), items).await?;
        fetches.sort_by_key(|f| std::cmp::Reverse(f.uid));
        Ok(fetches
            .iter()
            .filter_map(|f| convert::conversation_message(f, validity))
            .collect())
    }

    async fn draft_listing(&mut self, mailbox: &str) -> Result<DraftListing> {
        let info = self.select(mailbox).await?;
        // The pinned client's SELECT parser also returns a default mailbox on EOF.
        // Missing identity therefore retires the connection instead of reusing it.
        let validity = info.uid_validity.filter(|value| *value != 0)
            .ok_or_else(|| Error::Network("The mailbox has no valid UIDVALIDITY.".into()))?;
        let uids = self.search("ALL").await?;
        // Listing nothing in a mailbox the server just said holds mail would read as
        // "all of it is gone": refuse that rather than believe it.
        if uids.is_empty() && info.exists > 0 {
            return Err(Error::Protocol("The server omitted the search result.".into()));
        }
        let provider_ids = uids.into_iter().map(|uid| provider_id(validity, uid)).collect();
        Ok(DraftListing { mailbox: mailbox.into(), generation: validity.to_string(), provider_ids })
    }

    async fn message_ids(&mut self, mailbox: &str) -> Result<Vec<String>> {
        self.draft_listing(mailbox).await.map(|listing| listing.provider_ids)
    }

    async fn apply(&mut self, mailbox: &str, ids: &[String], change: &Change) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let parsed: Vec<(u32, u32)> = ids
            .iter()
            .map(|id| parse_provider_id(id))
            .collect::<Result<_>>()?;
        let info = self.select(mailbox).await?;
        let validity = info.uid_validity.unwrap_or(0);
        if parsed.iter().any(|(v, _)| *v != validity) {
            return Err(Error::NotFound(format!(
                "{mailbox} was reset on the server; sync it again"
            )));
        }
        let set = uid_set(&parsed.iter().map(|(_, uid)| *uid).collect::<Vec<_>>());
        match change {
            Change::Seen(on) => self.store(&set, *on, "\\Seen").await,
            Change::Flagged(on) => self.store(&set, *on, "\\Flagged").await,
            Change::Answered(on) => self.store(&set, *on, "\\Answered").await,
            Change::Forwarded(on) => self.store(&set, *on, "$Forwarded").await,
            Change::Move { to } => self.move_to(mailbox, &set, to).await,
            Change::Archive => {
                let to = self.role_mailbox(self.archive_role()).await?;
                let to = to.ok_or_else(|| {
                    Error::Unsupported("this account has no Archive folder".into())
                })?;
                self.move_to(mailbox, &set, &to).await
            }
            Change::Trash => {
                let to = self.role_mailbox(MailboxRole::Trash).await?;
                let to = to
                    .ok_or_else(|| Error::Unsupported("this account has no Trash folder".into()))?;
                self.move_to(mailbox, &set, &to).await
            }
        }
    }

    /// Where archived mail goes: All Mail on Gmail, the Archive folder elsewhere.
    fn archive_role(&self) -> MailboxRole {
        if self.caps.gmail {
            MailboxRole::All
        } else {
            MailboxRole::Archive
        }
    }

    /// Finds the messages in the archive by Message-ID (their old Inbox UIDs are
    /// gone) and puts them back in the Inbox.
    async fn unarchive(&mut self, message_ids: &[String]) -> Result<()> {
        let ids = searchable(message_ids);
        if ids.is_empty() {
            return Ok(());
        }
        // Without an archive folder, done couldn't have moved anything.
        let Some(archive) = self.role_mailbox(self.archive_role()).await? else {
            return Ok(());
        };
        let uids = self.find(&archive, &ids).await?;
        if uids.is_empty() {
            return Ok(());
        }
        let set = uid_set(&uids);
        if self.caps.gmail {
            // Mail never leaves All Mail: a copy in INBOX puts the Inbox label back.
            return self
                .session
                .uid_copy(&set, "INBOX")
                .await
                .map_err(imap_error);
        }
        self.move_to(&archive, &set, "INBOX").await
    }

    /// Moves the messages to Trash by Message-ID, from wherever they are: the Inbox or the
    /// archive, or on Gmail All Mail, which holds everything, so they leave every label.
    async fn trash(&mut self, message_ids: &[String]) -> Result<()> {
        let ids = searchable(message_ids);
        if ids.is_empty() {
            return Ok(());
        }
        let boxes = self.mailboxes().await?;
        let role = |r| boxes.iter().find(|m| m.role == Some(r)).map(|m| m.id.clone());
        let trash = role(MailboxRole::Trash)
            .ok_or_else(|| Error::Unsupported("this account has no Trash folder".into()))?;
        let inbox = role(MailboxRole::Inbox).unwrap_or_else(|| "INBOX".into());
        let from: Vec<String> = match (self.caps.gmail, role(MailboxRole::All)) {
            (true, Some(all)) => vec![all],
            _ => std::iter::once(inbox).chain(role(MailboxRole::Archive)).collect(),
        };
        for mailbox in from {
            let uids = self.find(&mailbox, &ids).await?;
            if !uids.is_empty() {
                self.move_to(&mailbox, &uid_set(&uids), &trash).await?;
            }
        }
        Ok(())
    }

    /// Finds the messages in Trash by Message-ID and puts them back in the Inbox, or the
    /// archive (the Inbox if there's none, since done couldn't move them).
    ///
    /// On Gmail they always go back to All Mail, which takes the Trash label off; a message
    /// out of Trash has no Inbox label, so inbox mail is then copied to INBOX as well, the
    /// same way undoing Done puts the label back.
    async fn untrash(&mut self, message_ids: &[String], to_inbox: bool) -> Result<()> {
        let ids = searchable(message_ids);
        if ids.is_empty() {
            return Ok(());
        }
        let boxes = self.mailboxes().await?;
        let role = |r| boxes.iter().find(|m| m.role == Some(r)).map(|m| m.id.clone());
        // Without a Trash folder, delete couldn't have moved anything.
        let Some(trash) = role(MailboxRole::Trash) else {
            return Ok(());
        };
        let inbox = role(MailboxRole::Inbox).unwrap_or_else(|| "INBOX".into());
        let all_mail = role(MailboxRole::All).filter(|_| self.caps.gmail);
        let to = match &all_mail {
            Some(all) => all.clone(),
            None => (!to_inbox)
                .then(|| role(self.archive_role()))
                .flatten()
                .unwrap_or(inbox),
        };
        let uids = self.find(&trash, &ids).await?;
        if uids.is_empty() {
            return Ok(());
        }
        self.move_to(&trash, &uid_set(&uids), &to).await?;
        if to_inbox && all_mail.is_some() {
            self.unarchive(message_ids).await?;
        }
        Ok(())
    }

    /// The UIDs in `mailbox` of the messages with these Message-IDs.
    async fn find(&mut self, mailbox: &str, message_ids: &[&String]) -> Result<Vec<u32>> {
        self.select(mailbox).await?;
        let mut uids = Vec::new();
        for id in message_ids {
            uids.extend(
                self.search(&format!("HEADER Message-ID \"<{id}>\""))
                    .await?,
            );
        }
        Ok(uids)
    }

    async fn create(&mut self, name: &str) -> Result<()> {
        match self.session.create(name).await {
            Err(async_imap::error::Error::No(m)) if m.contains("ALREADYEXISTS") => Ok(()),
            result => result.map_err(imap_error),
        }
    }

    async fn store(&mut self, set: &str, on: bool, flag: &str) -> Result<()> {
        let sign = if on { '+' } else { '-' };
        let stream = self
            .session
            .uid_store(set, format!("{sign}FLAGS.SILENT ({flag})"))
            .await
            .map_err(imap_error)?;
        let _: Vec<Fetch> = stream.try_collect().await.map_err(imap_error)?;
        Ok(())
    }

    async fn move_to(&mut self, from: &str, set: &str, to: &str) -> Result<()> {
        if from == to {
            return Ok(());
        }
        if self.caps.mv {
            return self.session.uid_mv(set, to).await.map_err(imap_error);
        }
        self.session.uid_copy(set, to).await.map_err(imap_error)?;
        self.remove(set).await
    }

    /// Deletes these messages from the selected mailbox for good.
    async fn remove(&mut self, set: &str) -> Result<()> {
        self.store(set, true, "\\Deleted").await?;
        if self.caps.uidplus {
            let _: Vec<u32> = self
                .session
                .uid_expunge(set)
                .await
                .map_err(imap_error)?
                .try_collect()
                .await
                .map_err(imap_error)?;
        } else {
            // Without UIDPLUS this also removes anything else marked \Deleted.
            let _: Vec<u32> = self
                .session
                .expunge()
                .await
                .map_err(imap_error)?
                .try_collect()
                .await
                .map_err(imap_error)?;
        }
        Ok(())
    }

    /// See [`Provider::save_draft`].
    async fn save_draft(&mut self, mailbox: &str, raw: &[u8], message_id: &str) -> Result<SavedDraft> {
        self.session
            .append(mailbox, Some("(\\Seen \\Draft)"), None, raw)
            .await
            .map_err(imap_error)?;
        Ok(SavedDraft { message_id: message_id.into(), provider_id: None })
    }

    /// See [`Provider::delete_drafts`]. Gmail keeps a message expunged from Drafts in All
    /// Mail, so there it goes through the Bin, and out of that.
    async fn delete_drafts(&mut self, mailbox: &str, message_ids: &[String]) -> Result<()> {
        let ids = searchable(message_ids);
        if ids.is_empty() {
            return Ok(());
        }
        let bin = match self.caps.gmail {
            true => self.role_mailbox(MailboxRole::Trash).await?,
            false => None,
        };
        let uids = self.find(mailbox, &ids).await?;
        if uids.is_empty() {
            return Ok(());
        }
        let Some(bin) = bin else {
            return self.remove(&uid_set(&uids)).await;
        };
        self.move_to(mailbox, &uid_set(&uids), &bin).await?;
        let binned = self.find(&bin, &ids).await?;
        if binned.is_empty() {
            return Ok(());
        }
        self.remove(&uid_set(&binned)).await
    }

    /// Appends to the Sent folder. Returns false if there isn't one.
    async fn append_sent(&mut self, raw: &[u8]) -> Result<bool> {
        let Some(sent) = self.role_mailbox(MailboxRole::Sent).await? else {
            return Ok(false);
        };
        self.session
            .append(&sent, Some("(\\Seen)"), None, raw)
            .await
            .map_err(imap_error)?;
        Ok(true)
    }

    /// IDLE until the mailbox changes or `timeout`. Hands the connection back
    /// unless it broke.
    async fn idle(mut self, mailbox: &str, timeout: Duration) -> (Result<Wakeup>, Option<Conn>) {
        if let Err(e) = self.select(mailbox).await {
            let keep = !matches!(e, Error::Network(_));
            return (Err(e), keep.then_some(self));
        }
        let Conn {
            session,
            watch,
            caps,
            mailboxes,
            backfill,
            flags_seen,
            search_vanished,
        } = self;
        let mut handle = session.idle();
        if let Err(e) = handle.init().await {
            return (Err(imap_error(e)), None);
        }
        // Waiting for mail is what IDLE is for: only `timeout` ends it (and TCP keepalive, if
        // the connection died). DONE and its answer are watched again.
        watch.off();
        let response = {
            let (wait, _stop) = handle.wait_with_timeout(timeout);
            wait.await
        };
        watch.set(COMMAND_STALL);
        match (response, handle.done().await) {
            (Ok(response), Ok(session)) => {
                let wakeup = match response {
                    IdleResponse::NewData(_) => Wakeup::Changed,
                    IdleResponse::Timeout | IdleResponse::ManualInterrupt => Wakeup::Timeout,
                };
                (
                    Ok(wakeup),
                    Some(Conn {
                        session,
                        watch,
                        caps,
                        mailboxes,
                        backfill,
                        flags_seen,
                        search_vanished,
                    }),
                )
            }
            (Err(e), _) | (_, Err(e)) => (Err(imap_error(e)), None),
        }
    }
}

fn imap_checkpoint(checkpoint: Option<&Checkpoint>) -> Result<Option<ImapCheckpoint>> {
    match checkpoint.map(|c| &c.0) {
        None => Ok(None),
        Some(CheckpointState::Imap(cp)) => Ok(Some(cp.clone())),
        Some(_) => Err(Error::InvalidCheckpoint),
    }
}

#[async_trait]
impl Provider for ImapProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities { server_flags: true }
    }

    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
        with_conn!(self, main, |conn| {
            conn.mailboxes = None;
            conn.mailboxes()
        })
    }

    async fn fetch_since(
        &self,
        mailbox: &str,
        checkpoint: Option<&Checkpoint>,
        limit: usize,
    ) -> Result<SyncBatch> {
        let checkpoint = imap_checkpoint(checkpoint)?;
        with_conn!(self, main, |conn| conn
            .fetch_since(mailbox, checkpoint, limit, false))
    }

    async fn fetch_since_shallow(
        &self,
        mailbox: &str,
        checkpoint: Option<&Checkpoint>,
        limit: usize,
    ) -> Result<SyncBatch> {
        let checkpoint = imap_checkpoint(checkpoint)?;
        with_conn!(self, main, |conn| conn
            .fetch_since(mailbox, checkpoint, limit, true))
    }

    async fn fetch_message(&self, mailbox: &str, provider_id: &str) -> Result<RawMessage> {
        with_conn!(self, main, |conn| conn.fetch_message(mailbox, provider_id))
    }

    async fn fetch_part(&self, mailbox: &str, provider_id: &str, section: &str) -> Result<Vec<u8>> {
        with_conn!(self, main, |conn| conn.fetch_part(mailbox, provider_id, section))
    }

    async fn apply(&self, mailbox: &str, provider_ids: &[String], change: &Change) -> Result<()> {
        with_conn!(self, main, |conn| conn.apply(mailbox, provider_ids, change))
    }

    async fn unarchive(&self, message_ids: &[String]) -> Result<()> {
        with_conn!(self, main, |conn| conn.unarchive(message_ids))
    }

    async fn trash(&self, message_ids: &[String]) -> Result<()> {
        with_conn!(self, main, |conn| conn.trash(message_ids))
    }

    async fn untrash(&self, message_ids: &[String], to_inbox: bool) -> Result<()> {
        with_conn!(self, main, |conn| conn.untrash(message_ids, to_inbox))
    }

    async fn create_mailbox(&self, name: &str) -> Result<()> {
        with_conn!(self, main, |conn| conn.create(name))
    }

    async fn save_draft(
        &self,
        mailbox: &str,
        draft: &OutgoingMessage,
    ) -> Result<Option<SavedDraft>> {
        let built = crate::mime::build_draft(draft, Utc::now(), &mut crate::mime::new_boundary)?;
        let id = built.message_id.trim_start_matches('<').trim_end_matches('>').to_string();
        with_conn!(self, main, |conn| conn.save_draft(mailbox, &built.archive, &id)).map(Some)
    }

    async fn delete_drafts(&self, mailbox: &str, message_ids: &[String]) -> Result<()> {
        with_conn!(self, main, |conn| conn.delete_drafts(mailbox, message_ids))
    }

    async fn send(&self, message: &OutgoingMessage) -> Result<SentMessage> {
        let built = crate::mime::build(message, Utc::now(), &mut crate::mime::new_boundary)?;
        let smtp = self.smtp.lock().unwrap().clone();
        crate::smtp::send(&smtp, message, &built.wire).await?;
        // The mail has gone: failing to file a copy mustn't look like a failed send.
        let saved_on_server =
            self.save_sent && matches!(self.append_sent(&built.archive).await, Ok(true));
        Ok(SentMessage {
            provider_id: None,
            message_id: built.message_id,
            raw: built.archive,
            saved_on_server,
        })
    }

    async fn list_drafts(&self, mailbox: &str) -> Result<Option<DraftListing>> {
        with_conn!(self, main, |conn| conn.draft_listing(mailbox)).map(Some)
    }

    async fn all_ids(&self, mailbox: &str) -> Result<Option<Vec<String>>> {
        self.message_ids(mailbox).await.map(Some)
    }

    async fn fetch_messages(
        &self,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<Vec<RawMessage>> {
        if provider_ids.is_empty() {
            return Ok(Vec::new());
        }
        with_conn!(self, main, |conn| conn
            .fetch_messages(mailbox, provider_ids))
    }

    async fn fetch_headers(
        &self,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<Vec<RawMessage>> {
        if provider_ids.is_empty() {
            return Ok(Vec::new());
        }
        with_conn!(self, main, |conn| conn
            .fetch_headers(mailbox, provider_ids))
    }

    async fn conversations_of(
        &self,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<Option<Vec<(String, u64)>>> {
        with_conn!(self, main, |conn| conn
            .conversations_of(mailbox, provider_ids))
    }

    async fn conversation_messages(
        &self,
        mailbox: &str,
        conversations: &[u64],
    ) -> Result<Vec<ConversationMessage>> {
        with_conn!(self, main, |conn| conn
            .conversation_messages(mailbox, conversations))
    }

    async fn wait_for_change(&self, mailbox: &str, timeout: Duration) -> Result<Wakeup> {
        let mut slot = self.idle.lock().await;
        if slot.is_none() {
            *slot = Some(self.open().await?);
        }
        let conn = slot.take().expect("connected above");
        if !conn.caps.idle {
            *slot = Some(conn);
            drop(slot);
            tokio::time::sleep(timeout).await;
            return Ok(Wakeup::Timeout);
        }
        let (result, conn) = conn.idle(mailbox, timeout).await;
        *slot = conn;
        result
    }
}

/// Message-IDs (without <>) that can go in a quoted IMAP search.
fn searchable(message_ids: &[String]) -> Vec<&String> {
    message_ids
        .iter()
        .filter(|id| crate::searchable_message_id(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{smtp_server, step, FakeImap, SmtpMode, Step, ANY, HANG_UP};
    use crate::{Address, Secret};
    use base64::Engine as _;

    const LOGIN: &str = r#"{tag} LOGIN "ada@x.io" "pw""#;
    const BODY_ITEMS: &str = "(UID FLAGS INTERNALDATE BODY.PEEK[])";
    const SYNC_ITEMS: &str = "(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE)";

    fn signed_in(caps: &str) -> Vec<Step> {
        let mut steps = vec![
            step(LOGIN, "{tag} OK\r\n"),
            step(
                "{tag} CAPABILITY",
                &format!("* CAPABILITY IMAP4rev1 {caps}\r\n{{tag}} OK\r\n"),
            ),
        ];
        if caps.contains("QRESYNC") {
            steps.push(step(
                "{tag} ENABLE QRESYNC",
                "* ENABLED QRESYNC\r\n{tag} OK\r\n",
            ));
        }
        steps
    }

    fn script(caps: &str, rest: Vec<Step>) -> Vec<Step> {
        let mut steps = signed_in(caps);
        steps.extend(rest);
        steps
    }

    fn select(
        mailbox: &str,
        condstore: bool,
        exists: u32,
        validity: u32,
        next: Option<u32>,
        modseq: Option<u64>,
    ) -> Step {
        let command = if condstore {
            format!("{{tag}} SELECT \"{mailbox}\" (CONDSTORE)")
        } else {
            format!("{{tag}} SELECT \"{mailbox}\"")
        };
        let mut reply = format!("* {exists} EXISTS\r\n* OK [UIDVALIDITY {validity}] ok\r\n");
        if let Some(next) = next {
            reply += &format!("* OK [UIDNEXT {next}] ok\r\n");
        }
        if let Some(modseq) = modseq {
            reply += &format!("* OK [HIGHESTMODSEQ {modseq}] ok\r\n");
        }
        reply += "{tag} OK [READ-WRITE] selected\r\n";
        (command, reply)
    }

    /// INBOX, validity 7, UIDNEXT 10, no CONDSTORE.
    fn inbox() -> Step {
        select("INBOX", false, 3, 7, Some(10), None)
    }

    fn search(query: &str, uids: &str) -> Step {
        step(
            &format!("{{tag}} UID SEARCH {query}"),
            &format!("* SEARCH {uids}\r\n{{tag}} OK\r\n"),
        )
    }

    fn body(uid: u32) -> String {
        format!("Subject: {uid}\r\n\r\nbody\r\n")
    }

    /// Sync's first command: flags, dates, sizes and structures, no bodies.
    fn sizes(set: &str, uids: &[u32], gmail: bool) -> Step {
        let items = if gmail {
            "(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE X-GM-MSGID X-GM-LABELS)"
        } else {
            SYNC_ITEMS
        };
        let mut reply = String::new();
        for (i, uid) in uids.iter().enumerate() {
            let gm = if gmail {
                format!(" X-GM-MSGID {} X-GM-LABELS (\\Inbox)", 1000 + uid)
            } else {
                String::new()
            };
            reply += &format!(
                "* {} FETCH (UID {uid} FLAGS (\\Seen) INTERNALDATE \"01-Nov-2026 09:30:00 +0000\" RFC822.SIZE {} BODYSTRUCTURE (\"text\" \"plain\" NIL NIL NIL \"7bit\" 6 1 NIL NIL NIL NIL){gm})\r\n",
                i + 1,
                body(*uid).len(),
            );
        }
        reply += "{tag} OK\r\n";
        step(&format!("{{tag}} UID FETCH {set} {items}"), &reply)
    }

    /// Then the small messages, whole.
    fn bodies(set: &str, uids: &[u32]) -> Step {
        let mut reply = String::new();
        for (i, uid) in uids.iter().enumerate() {
            let b = body(*uid);
            reply += &format!("* {} FETCH (UID {uid} BODY[] {{{}}}\r\n{b})\r\n", i + 1, b.len());
        }
        reply += "{tag} OK\r\n";
        step(&format!("{{tag}} UID FETCH {set} (UID BODY.PEEK[])"), &reply)
    }

    /// One whole message, attachments and all.
    fn fetch_whole(uid: u32) -> Step {
        let b = body(uid);
        step(
            &format!("{{tag}} UID FETCH {uid} {BODY_ITEMS}"),
            &format!(
                "* 1 FETCH (UID {uid} FLAGS (\\Seen) INTERNALDATE \"01-Nov-2026 09:30:00 +0000\" BODY[] {{{}}}\r\n{b})\r\n{{tag}} OK\r\n",
                b.len()
            ),
        )
    }

    fn list(lines: &[&str]) -> Step {
        step(
            "{tag} LIST \"\" *",
            &format!("{}\r\n{{tag}} OK\r\n", lines.join("\r\n")),
        )
    }

    fn ok(command: &str) -> Step {
        step(command, "{tag} OK\r\n")
    }

    fn password(pw: &str) -> Credentials {
        Credentials::Password {
            username: "ada@x.io".into(),
            password: Secret::from(pw),
        }
    }

    fn smtp_config(port: u16, root: Vec<u8>) -> SmtpConfig {
        SmtpConfig {
            host: "localhost".into(),
            port,
            security: Security::Tls,
            credentials: password("smtp-pw"),
            tls_roots: TlsRoots::Custom(vec![root]),
        }
    }

    fn provider(fake: &Arc<FakeImap>) -> ImapProvider {
        ImapProvider::with_connector(fake.clone(), password("pw"), smtp_config(1, vec![]))
    }

    fn ids(batch: &SyncBatch) -> Vec<&str> {
        batch
            .messages
            .iter()
            .map(|m| m.provider_id.as_str())
            .collect()
    }

    fn state(batch: &SyncBatch) -> ImapCheckpoint {
        serde_json::from_value(serde_json::to_value(&batch.checkpoint).unwrap()).unwrap()
    }

    fn checkpoint(
        uid_validity: u32,
        uid_next: u32,
        highest_modseq: Option<u64>,
        backfill_below: u32,
    ) -> Checkpoint {
        Checkpoint(CheckpointState::Imap(ImapCheckpoint {
            uid_validity,
            uid_next,
            highest_modseq,
            backfill_below,
        }))
    }

    fn round_trip(cp: &Checkpoint) -> Checkpoint {
        serde_json::from_str(&serde_json::to_string(cp).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn first_sync_is_newest_first_then_new_mail_and_flag_changes() {
        let fake = FakeImap::new(vec![script(
            "IDLE CONDSTORE",
            vec![
                select("INBOX", true, 4, 7, Some(11), Some(100)),
                search("UID 1:10", "3 5 9 10"),
                sizes("5,9:10", &[5, 9, 10], false),
                bodies("5,9:10", &[5, 9, 10]),
                select("INBOX", true, 4, 7, Some(11), Some(100)),
                sizes("3", &[3], false),
                bodies("3", &[3]),
                select("INBOX", true, 5, 7, Some(13), Some(105)),
                step(
                    "{tag} UID FETCH 1:10 (UID FLAGS) (CHANGEDSINCE 100)",
                    "* 2 FETCH (UID 5 FLAGS (\\Flagged) MODSEQ (104))\r\n{tag} OK\r\n",
                ),
                search("UID 11:*", "12"),
                sizes("12", &[12], false),
                bodies("12", &[12]),
            ],
        )]);
        let p = provider(&fake);

        let first = p.fetch_since("INBOX", None, 3).await.unwrap();
        assert_eq!(ids(&first), ["7:10", "7:9", "7:5"]);
        assert!(
            first.more
                && !first.reset
                && first.flag_updates.is_empty()
                && first.vanished.is_empty()
        );
        let m = &first.messages[0];
        assert_eq!(
            (m.mailbox.as_str(), m.raw.as_slice(), m.flags.seen),
            ("INBOX", body(10).as_bytes(), true)
        );
        assert_eq!(
            m.received_at.unwrap().to_rfc3339(),
            "2026-11-01T09:30:00+00:00"
        );

        let second = p
            .fetch_since("INBOX", Some(&round_trip(&first.checkpoint)), 3)
            .await
            .unwrap();
        assert_eq!(ids(&second), ["7:3"]);
        assert!(!second.more);

        let third = p
            .fetch_since("INBOX", Some(&second.checkpoint), 3)
            .await
            .unwrap();
        assert_eq!(ids(&third), ["7:12"]);
        assert_eq!(third.flag_updates.len(), 1);
        assert_eq!(third.flag_updates[0].provider_id, "7:5");
        assert!(third.flag_updates[0].flags.flagged);
        assert!(!third.more);
        assert_eq!(
            state(&third),
            ImapCheckpoint {
                uid_validity: 7,
                uid_next: 13,
                highest_modseq: Some(105),
                backfill_below: 1
            }
        );

        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn backfill_follows_the_checkpoint_it_is_given() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 4, 7, Some(11), None),
                search("UID 1:10", "3 5 9 10"),
                sizes("10", &[10], false),
                bodies("10", &[10]),
                select("INBOX", false, 4, 7, Some(11), None),
                step("{tag} FETCH 1:4 (UID FLAGS)", "{tag} OK\r\n"),
                sizes("5", &[5], false),
                bodies("5", &[5]),
            ],
        )]);
        let p = provider(&fake);
        let first = p.fetch_since("INBOX", None, 1).await.unwrap();
        assert_eq!(ids(&first), ["7:10"]);
        // A checkpoint from another run, further down than this connection got.
        let further = p
            .fetch_since("INBOX", Some(&checkpoint(7, 11, None, 6)), 1)
            .await
            .unwrap();
        assert_eq!(ids(&further), ["7:5"]);
        assert_eq!(state(&further).backfill_below, 5);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn without_condstore_flags_changed_elsewhere_are_found_by_looking() {
        let flags = |list: &str| {
            format!(
                "* 1 FETCH (UID 3 FLAGS (\\Seen))\r\n* 2 FETCH (UID 5 FLAGS ({list}))\r\n\
                 * 3 FETCH (UID 12 FLAGS ())\r\n{{tag}} OK\r\n"
            )
        };
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 3, 7, Some(11), None),
                step("{tag} FETCH 1:3 (UID FLAGS)", &flags("")),
                select("INBOX", false, 3, 7, Some(11), None),
                step("{tag} FETCH 1:3 (UID FLAGS)", &flags("\\Flagged")),
                select("INBOX", false, 3, 7, Some(11), None),
                step("{tag} FETCH 1:3 (UID FLAGS)", &flags("\\Flagged")),
                // Only the newest thousand are looked at.
                select("INBOX", false, 1500, 7, Some(11), None),
                step("{tag} FETCH 501:1500 (UID FLAGS)", "{tag} OK\r\n"),
                // A new UIDVALIDITY starts the mailbox afresh; once Moss has the new ids,
                // their flags are new to it too.
                select("INBOX", false, 3, 8, Some(11), None),
                step("{tag} FETCH 1:3 (UID FLAGS)", &flags("\\Flagged").replace("UID 12", "UID 99")),
            ],
        )]);
        let p = provider(&fake);
        let known = checkpoint(7, 11, None, 1);
        let updated = |batch: &SyncBatch| -> Vec<(String, bool, bool)> {
            batch
                .flag_updates
                .iter()
                .map(|u| (u.provider_id.clone(), u.flags.seen, u.flags.flagged))
                .collect()
        };
        // The first look reports them all; UID 12 isn't stored yet, so it's left out.
        let first = p.fetch_since("INBOX", Some(&known), 50).await.unwrap();
        assert_eq!(
            updated(&first),
            [("7:3".into(), true, false), ("7:5".into(), false, false)]
        );
        // Then only what changed: starred on the phone.
        let second = p.fetch_since("INBOX", Some(&known), 50).await.unwrap();
        assert_eq!(updated(&second), [("7:5".into(), false, true)]);
        let third = p.fetch_since("INBOX", Some(&known), 50).await.unwrap();
        assert!(third.flag_updates.is_empty());
        p.fetch_since("INBOX", Some(&known), 50).await.unwrap();
        let renewed = p.fetch_since("INBOX", Some(&checkpoint(8, 11, None, 1)), 50).await.unwrap();
        assert_eq!(renewed.flag_updates.len(), 2);
        assert_eq!(renewed.flag_updates[0].provider_id, "8:3");
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn qresync_reports_expunged_mail() {
        let fake = FakeImap::new(vec![script(
            "CONDSTORE QRESYNC",
            vec![
                select("INBOX", true, 3, 7, Some(11), Some(120)),
                step(
                    "{tag} UID FETCH 1:10 (UID FLAGS) (CHANGEDSINCE 100 VANISHED)",
                    "* VANISHED (EARLIER) 3:4,8\r\n* FLAGS (\\Seen)\r\n* 5 EXISTS\r\n\
                     * 1 FETCH (UID 5 FLAGS () MODSEQ (110))\r\n{tag} OK\r\n",
                ),
            ],
        )]);
        let p = provider(&fake);
        let batch = p
            .fetch_since("INBOX", Some(&checkpoint(7, 11, Some(100), 1)), 50)
            .await
            .unwrap();
        assert_eq!(batch.vanished, ["7:3", "7:4", "7:8"]);
        assert_eq!(batch.flag_updates.len(), 1);
        assert!(batch.messages.is_empty() && !batch.more);
        assert_eq!(state(&batch).highest_modseq, Some(120));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_new_uidvalidity_resets_the_mailbox() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 1, 9, Some(3), None),
                search("UID 1:2", "2"),
                sizes("2", &[2], false),
                bodies("2", &[2]),
            ],
        )]);
        let p = provider(&fake);
        let batch = p
            .fetch_since("INBOX", Some(&checkpoint(6, 11, Some(5), 1)), 50)
            .await
            .unwrap();
        assert!(batch.reset);
        assert_eq!(ids(&batch), ["9:2"]);
        assert_eq!(
            state(&batch),
            ImapCheckpoint {
                uid_validity: 9,
                uid_next: 3,
                highest_modseq: None,
                backfill_below: 1
            }
        );
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn new_mail_comes_in_batches_and_ignores_the_star_quirk() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 9, 7, Some(11), None),
                step("{tag} FETCH 1:9 (UID FLAGS)", "{tag} OK\r\n"),
                search("UID 5:*", "5 6 7 9"),
                sizes("5:6", &[5, 6], false),
                bodies("5:6", &[5, 6]),
                select("INBOX", false, 9, 7, Some(11), None),
                step("{tag} FETCH 1:9 (UID FLAGS)", "{tag} OK\r\n"),
                search("UID 7:*", "7 9"),
                sizes("7,9", &[7, 9], false),
                bodies("7,9", &[7, 9]),
                select("INBOX", false, 9, 7, Some(12), None),
                step("{tag} FETCH 1:9 (UID FLAGS)", "{tag} OK\r\n"),
                search("UID 11:*", "9"),
            ],
        )]);
        let p = provider(&fake);
        let first = p
            .fetch_since("INBOX", Some(&checkpoint(7, 5, None, 1)), 2)
            .await
            .unwrap();
        assert_eq!(ids(&first), ["7:5", "7:6"]);
        assert!(first.more);
        assert_eq!(state(&first).uid_next, 7);
        let second = p
            .fetch_since("INBOX", Some(&first.checkpoint), 2)
            .await
            .unwrap();
        assert_eq!(ids(&second), ["7:7", "7:9"]);
        assert!(!second.more);
        assert_eq!(state(&second).uid_next, 11);
        let third = p
            .fetch_since("INBOX", Some(&second.checkpoint), 0)
            .await
            .unwrap();
        assert!(third.messages.is_empty() && !third.more);
        assert_eq!(state(&third).uid_next, 12);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn gmail_messages_carry_their_id_and_labels() {
        let fake = FakeImap::new(vec![script(
            "X-GM-EXT-1 CONDSTORE",
            vec![
                select("INBOX", true, 1, 3, Some(2), Some(50)),
                search("UID 1:1", "1"),
                sizes("1", &[1], true),
                bodies("1", &[1]),
                select("INBOX", true, 1, 3, Some(2), Some(60)),
                step(
                    "{tag} UID FETCH 1:1 (UID FLAGS X-GM-LABELS) (CHANGEDSINCE 50)",
                    "* 1 FETCH (UID 1 FLAGS () X-GM-LABELS (Work) MODSEQ (60))\r\n\
                     * 2 FETCH (UID 2 FLAGS () X-GM-LABELS () MODSEQ (60))\r\n{tag} OK\r\n",
                ),
            ],
        )]);
        let p = provider(&fake);
        let first = p.fetch_since("INBOX", None, 10).await.unwrap();
        assert_eq!(first.messages[0].gmail_msg_id, Some(1001));
        assert_eq!(first.messages[0].labels, ["\\Inbox"]);
        let second = p
            .fetch_since("INBOX", Some(&first.checkpoint), 10)
            .await
            .unwrap();
        assert_eq!(
            second.flag_updates.len(),
            1,
            "UIDs not synced yet are left out"
        );
        assert_eq!(second.flag_updates[0].labels, ["Work"]);
        drop(p);
        fake.finish().await;
    }

    const ALL_MAIL: &str = "[Gmail]/All Mail";
    const HEADER_ITEMS: &str = "(UID FLAGS X-GM-LABELS BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])";

    fn message_id(id: &str) -> String {
        format!("Message-ID: <{id}>\r\n\r\n")
    }

    /// A FETCH response line for a conversation member: labels and Message-ID, no body.
    fn member(n: u32, uid: u32, labels: &str, id: &str) -> String {
        let h = message_id(id);
        format!(
            "* {n} FETCH (UID {uid} FLAGS (\\Seen) X-GM-LABELS ({labels}) \
             BODY[HEADER.FIELDS (MESSAGE-ID)] {{{}}}\r\n{h})\r\n",
            h.len()
        )
    }

    #[tokio::test]
    async fn gmail_finds_conversations_and_the_archived_mail_in_them() {
        let fake = FakeImap::new(vec![script(
            "X-GM-EXT-1",
            vec![
                inbox(),
                step(
                    "{tag} UID FETCH 5,9 (UID X-GM-THRID)",
                    "* 1 FETCH (X-GM-THRID 111 UID 5)\r\n* 4 EXISTS\r\n\
                     * 2 FETCH (UID 9 X-GM-THRID 18446744073709551614)\r\n\
                     * 3 FETCH (UID 8)\r\n{tag} OK\r\n",
                ),
                inbox(),
                select(ALL_MAIL, false, 9, 4, Some(50), None),
                search(
                    "OR OR X-GM-THRID 111 X-GM-THRID 222 X-GM-THRID 333",
                    "12 30 31",
                ),
                step(
                    &format!("{{tag}} UID FETCH 12,30:31 {HEADER_ITEMS}"),
                    &format!(
                        "{}{}{}{{tag}} OK\r\n",
                        member(1, 12, "\\Inbox", "a@x"),
                        member(2, 30, "\\Sent", "b@x"),
                        member(3, 31, "", "c@x")
                    ),
                ),
                select(ALL_MAIL, false, 9, 4, Some(50), None),
                step(
                    "{tag} UID SEARCH X-GM-THRID 444",
                    "* SEARCH\r\n{tag} OK\r\n",
                ),
                select(ALL_MAIL, false, 9, 4, Some(50), None),
                sizes("12,31", &[12, 31], true),
                bodies("12,31", &[12, 31]),
            ],
        )]);
        let p = provider(&fake);
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        let found = p
            .conversations_of("INBOX", &ids(&["7:9", "7:5", "local:<x@y>"]))
            .await
            .unwrap();
        assert_eq!(
            found,
            Some(vec![("7:5".into(), 111), ("7:9".into(), u64::MAX - 1)])
        );
        // Ids from before a UIDVALIDITY change aren't there to ask about.
        assert_eq!(
            p.conversations_of("INBOX", &ids(&["6:5"])).await.unwrap(),
            Some(vec![])
        );

        let members = p
            .conversation_messages(ALL_MAIL, &[111, 222, 333])
            .await
            .unwrap();
        let got: Vec<_> = members.iter().map(|m| m.provider_id.as_str()).collect();
        assert_eq!(got, ["4:31", "4:30", "4:12"], "newest first");
        assert_eq!(members[2].labels, ["\\Inbox"]);
        assert_eq!(members[1].labels, ["\\Sent"]);
        assert!(members[0].labels.is_empty() && members[0].flags.seen);
        assert_eq!(members[2].headers, message_id("a@x").as_bytes());
        assert!(p
            .conversation_messages(ALL_MAIL, &[444])
            .await
            .unwrap()
            .is_empty());

        let bodies = p
            .fetch_messages(ALL_MAIL, &ids(&["4:12", "4:31", "3:7", "bad"]))
            .await
            .unwrap();
        let got: Vec<_> = bodies.iter().map(|m| m.provider_id.as_str()).collect();
        assert_eq!(got, ["4:31", "4:12"]);
        assert_eq!(bodies[0].mailbox, ALL_MAIL);

        // Nothing to ask: nothing is sent.
        assert_eq!(
            p.conversations_of("INBOX", &[]).await.unwrap(),
            Some(vec![])
        );
        assert!(p
            .conversation_messages(ALL_MAIL, &[])
            .await
            .unwrap()
            .is_empty());
        assert!(p.fetch_messages(ALL_MAIL, &[]).await.unwrap().is_empty());
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn other_servers_have_no_conversations() {
        let fake = FakeImap::new(vec![script("IDLE", vec![])]);
        let p = provider(&fake);
        let asked = ["7:5".to_string()];
        assert_eq!(p.conversations_of("INBOX", &asked).await.unwrap(), None);
        assert!(p
            .conversation_messages("Archive", &[1])
            .await
            .unwrap()
            .is_empty());
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_refused_or_dropped_conversation_lookup_is_an_error() {
        let fake = FakeImap::new(vec![
            script(
                "X-GM-EXT-1",
                vec![
                    inbox(),
                    step(
                        "{tag} UID FETCH 5 (UID X-GM-THRID)",
                        "{tag} NO [LIMIT] too many\r\n",
                    ),
                    inbox(),
                    step("{tag} UID FETCH 5 (UID X-GM-THRID)", HANG_UP),
                ],
            ),
            script(
                "X-GM-EXT-1",
                vec![
                    inbox(),
                    step("{tag} UID FETCH 5 (UID X-GM-THRID)", "!! not IMAP\r\n"),
                ],
            ),
        ]);
        let p = provider(&fake);
        let asked = ["7:5".to_string()];
        let refused = p.conversations_of("INBOX", &asked).await.unwrap_err();
        assert!(
            matches!(&refused, Error::Protocol(m) if m == "The server refused the conversation lookup."),
            "{refused:?}"
        );
        assert!(matches!(
            p.conversations_of("INBOX", &asked).await,
            Err(Error::Network(_))
        ));
        // A reply that can't be read is a broken connection too.
        let garbled = p.conversations_of("INBOX", &asked).await;
        assert!(matches!(garbled, Err(Error::Network(_))), "{garbled:?}");
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn works_out_uidnext_when_the_server_doesnt_say() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 2, 7, None, None),
                step(
                    "{tag} UID FETCH * (UID)",
                    "* 2 FETCH (UID 8)\r\n{tag} OK\r\n",
                ),
                search("UID 1:8", "4 8"),
                sizes("4,8", &[4, 8], false),
                bodies("4,8", &[4, 8]),
                select("Empty", false, 0, 7, None, None),
            ],
        )]);
        let p = provider(&fake);
        let batch = p.fetch_since("INBOX", None, 10).await.unwrap();
        assert_eq!(ids(&batch), ["7:8", "7:4"]);
        assert_eq!(state(&batch).uid_next, 9);
        let empty = p.fetch_since("Empty", None, 10).await.unwrap();
        assert!(empty.messages.is_empty() && !empty.more);
        assert_eq!(
            state(&empty),
            ImapCheckpoint {
                uid_validity: 7,
                uid_next: 1,
                highest_modseq: None,
                backfill_below: 1
            }
        );
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn fetches_one_message_by_id() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                inbox(),
                fetch_whole(9),
                inbox(),
                ok(&format!("{{tag}} UID FETCH 8 {BODY_ITEMS}")),
                inbox(),
                step("{tag} SELECT \"Nope\"", "{tag} NO no such mailbox\r\n"),
            ],
        )]);
        let p = provider(&fake);
        let m = p.fetch_message("INBOX", "7:9").await.unwrap();
        assert_eq!(m.raw, body(9).as_bytes());
        let gone = p.fetch_message("INBOX", "7:8").await;
        assert!(matches!(gone, Err(Error::NotFound(_))), "gone");
        let older = p.fetch_message("INBOX", "6:9").await;
        assert!(
            matches!(older, Err(Error::NotFound(_))),
            "older UIDVALIDITY"
        );
        assert!(matches!(
            p.fetch_message("INBOX", "bad").await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            p.fetch_message("Nope", "7:1").await,
            Err(Error::NotFound(_))
        ));
        drop(p);
        fake.finish().await;
    }

    fn fetch_part(uid: u32, section: &str, reply: &str) -> Step {
        step(
            &format!("{{tag}} UID FETCH {uid} (UID BODY.PEEK[{section}.MIME] BODY.PEEK[{section}])"),
            &format!("{reply}{{tag}} OK\r\n"),
        )
    }

    #[tokio::test]
    async fn fetches_one_attachment_by_its_section() {
        let mime = "Content-Type: image/png\r\nContent-Transfer-Encoding: base64\r\n\r\n";
        let whole = "Subject: 9\r\nContent-Type: application/pdf\r\n\
                     Content-Transfer-Encoding: base64\r\n\r\nUERG\r\n";
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                inbox(),
                fetch_part(
                    9,
                    "1.2",
                    &format!(
                        "* 1 FETCH (UID 9 BODY[1.2.MIME] {{{}}}\r\n{mime} BODY[1.2] {{4}}\r\nUE5H)\r\n",
                        mime.len()
                    ),
                ),
                // A server that won't give a single part's MIME header: the whole message.
                inbox(),
                fetch_part(9, "1", "* 1 FETCH (UID 9 BODY[1] {4}\r\nUERG)\r\n"),
                inbox(),
                step(
                    &format!("{{tag}} UID FETCH 9 {BODY_ITEMS}"),
                    &format!(
                        "* 1 FETCH (UID 9 FLAGS () BODY[] {{{}}}\r\n{whole})\r\n{{tag}} OK\r\n",
                        whole.len()
                    ),
                ),
                // Gone from the mailbox.
                inbox(),
                fetch_part(8, "2", ""),
            ],
        )]);
        let p = provider(&fake);
        assert_eq!(p.fetch_part("INBOX", "7:9", "1.2").await.unwrap(), b"PNG");
        assert_eq!(p.fetch_part("INBOX", "7:9", "1").await.unwrap(), b"PDF");
        assert!(matches!(p.fetch_part("INBOX", "7:8", "2").await, Err(Error::NotFound(_))));
        // Never sent to the server.
        assert!(matches!(p.fetch_part("INBOX", "7:9", "2] BODY[").await, Err(Error::NotFound(_))));
        assert!(matches!(p.fetch_part("INBOX", "bad", "2").await, Err(Error::NotFound(_))));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_part_from_an_older_uidvalidity_is_gone() {
        let fake = FakeImap::new(vec![script("", vec![inbox()])]);
        let p = provider(&fake);
        assert!(matches!(p.fetch_part("INBOX", "6:9", "2").await, Err(Error::NotFound(_))));
        drop(p);
        fake.finish().await;
    }

    fn fetch_headers(set: &str, uids: &[u32]) -> Step {
        let mut reply = String::new();
        for (i, uid) in uids.iter().enumerate() {
            let h = format!("Subject: {uid}\r\n\r\n");
            reply += &format!(
                "* {} FETCH (UID {uid} FLAGS (\\Seen) INTERNALDATE \"01-Nov-2026 09:30:00 +0000\" BODY[HEADER] {{{}}}\r\n{h})\r\n",
                i + 1,
                h.len()
            );
        }
        reply += "{tag} OK\r\n";
        step(
            &format!("{{tag}} UID FETCH {set} (UID FLAGS INTERNALDATE BODY.PEEK[HEADER])"),
            &reply,
        )
    }

    #[tokio::test]
    async fn older_mail_can_come_as_headers_only() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("Sent", false, 4, 7, Some(11), None),
                search("UID 1:10", "3 5 9 10"),
                fetch_headers("9:10", &[9, 10]),
                // New mail still comes whole.
                select("Sent", false, 5, 7, Some(12), None),
                step("{tag} FETCH 1:5 (UID FLAGS)", "{tag} OK\r\n"),
                search("UID 11:*", "11"),
                sizes("11", &[11], false),
                bodies("11", &[11]),
                fetch_headers("5", &[5]),
            ],
        )]);
        let p = provider(&fake);
        let first = p.fetch_since_shallow("Sent", None, 2).await.unwrap();
        assert_eq!(ids(&first), ["7:10", "7:9"]);
        assert!(first.messages.iter().all(|m| m.headers_only && m.flags.seen));
        assert_eq!(first.messages[0].raw, b"Subject: 10\r\n\r\n");
        let second = p.fetch_since_shallow("Sent", Some(&first.checkpoint), 2).await.unwrap();
        assert_eq!(ids(&second), ["7:11", "7:5"]);
        assert!(!second.messages[0].headers_only && second.messages[1].headers_only);
        assert!(second.more);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn several_messages_can_come_as_headers_only() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![select("Archive", false, 4, 7, Some(11), None), fetch_headers("5,9", &[5, 9])],
        )]);
        let p = provider(&fake);
        let list = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let got = p.fetch_headers("Archive", &list(&["7:9", "7:5", "6:1"])).await.unwrap();
        let found: Vec<_> = got.iter().map(|m| m.provider_id.as_str()).collect();
        assert_eq!(found, ["7:9", "7:5"]);
        assert!(got.iter().all(|m| m.headers_only));
        assert!(p.fetch_headers("Archive", &[]).await.unwrap().is_empty());
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn gmail_headers_carry_labels_too() {
        let fake = FakeImap::new(vec![script(
            "X-GM-EXT-1",
            vec![
                select("Sent", false, 1, 7, Some(4), None),
                search("UID 1:3", "3"),
                step(
                    "{tag} UID FETCH 3 (UID FLAGS INTERNALDATE BODY.PEEK[HEADER] X-GM-MSGID X-GM-LABELS)",
                    "* 1 FETCH (UID 3 FLAGS () X-GM-MSGID 1003 X-GM-LABELS (\\Sent) BODY[HEADER] {4}\r\n\r\n\r\n)\r\n{tag} OK\r\n",
                ),
            ],
        )]);
        let p = provider(&fake);
        let batch = p.fetch_since_shallow("Sent", None, 5).await.unwrap();
        assert_eq!(batch.messages[0].labels, ["\\Sent"]);
        assert_eq!(batch.messages[0].gmail_msg_id, Some(1003));
        assert!(batch.messages[0].headers_only);
        drop(p);
        fake.finish().await;
    }

    /// A big message's first fetch: its size and structure.
    fn fetch_big(uid: u32, structure: &str) -> Step {
        step(
            &format!("{{tag}} UID FETCH {uid} {SYNC_ITEMS}"),
            &format!(
                "* 1 FETCH (UID {uid} FLAGS (\\Seen) INTERNALDATE \"01-Nov-2026 09:30:00 +0000\" RFC822.SIZE 900000 BODYSTRUCTURE {structure})\r\n{{tag}} OK\r\n"
            ),
        )
    }

    #[tokio::test]
    async fn big_messages_sync_without_their_attachments() {
        let structure = r#"(("text" "plain" ("charset" "utf-8") NIL NIL "7bit" 5 1 NIL NIL NIL NIL)("application" "pdf" ("name" "Q4.pdf") NIL NIL "base64" 780000 NIL ("attachment" ("filename" "Q4.pdf")) NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        let header = "From: a@x.co\r\nSubject: Q4\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n";
        let forwarded = r#"(("text" "plain" NIL NIL NIL "7bit" 5 1 NIL NIL NIL NIL)("message" "rfc822" NIL NIL NIL "7bit" 300 ("Mon, 2 Nov 2026 09:00:00 +0000" "Hi" NIL NIL NIL NIL NIL NIL NIL NIL) ("text" "plain" NIL NIL NIL "7bit" 2 1 NIL NIL NIL NIL) 8 NIL NIL NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        let whole = "Subject: 8\r\n\r\nwhole\r\n";
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 2, 7, Some(10), None),
                search("UID 1:9", "8 9"),
                fetch_big(9, structure),
                step(
                    "{tag} UID FETCH 9 (UID BODY.PEEK[HEADER] BODY.PEEK[1])",
                    &format!(
                        "* 1 FETCH (UID 9 BODY[HEADER] {{{}}}\r\n{header} BODY[1] {{5}}\r\nHello)\r\n{{tag}} OK\r\n",
                        header.len()
                    ),
                ),
                // An attached email: Moss takes it whole.
                select("INBOX", false, 2, 7, Some(10), None),
                step("{tag} FETCH 1:2 (UID FLAGS)", "{tag} OK\r\n"),
                fetch_big(8, forwarded),
                step(
                    &format!("{{tag}} UID FETCH 8 {BODY_ITEMS}"),
                    &format!(
                        "* 1 FETCH (UID 8 FLAGS () BODY[] {{{}}}\r\n{whole})\r\n{{tag}} OK\r\n",
                        whole.len()
                    ),
                ),
            ],
        )]);
        let p = provider(&fake);
        let first = p.fetch_since("INBOX", None, 1).await.unwrap();
        let m = &first.messages[0];
        assert_eq!((m.provider_id.as_str(), m.flags.seen), ("7:9", true));
        let raw = String::from_utf8(m.raw.clone()).unwrap();
        assert!(raw.starts_with(header), "{raw}");
        assert!(raw.contains("Hello") && raw.contains("Q4.pdf") && raw.contains("X-Moss-Omitted: 570000"));
        let second = p.fetch_since("INBOX", Some(&first.checkpoint), 1).await.unwrap();
        assert_eq!(second.messages[0].raw, whole.as_bytes());
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn sync_asks_for_sizes_before_bodies() {
        let structure = r#"(("text" "plain" NIL NIL NIL "7bit" 5 1 NIL NIL NIL NIL)("application" "pdf" NIL NIL NIL "base64" 780000 NIL ("attachment" NIL) NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        let header = "Subject: 4\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n";
        let small = r#"("text" "plain" NIL NIL NIL "7bit" 6 1 NIL NIL NIL NIL)"#;
        let date = "INTERNALDATE \"01-Nov-2026 09:30:00 +0000\"";
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 3, 7, Some(10), None),
                search("UID 1:9", "4 5 6"),
                step(
                    &format!("{{tag}} UID FETCH 4:6 {SYNC_ITEMS}"),
                    &format!(
                        "* 1 FETCH (UID 4 FLAGS () {date} RFC822.SIZE 900000 BODYSTRUCTURE {structure})\r\n\
                         * 2 FETCH (UID 5 FLAGS () {date} RFC822.SIZE 22 BODYSTRUCTURE {small})\r\n\
                         * 3 FETCH (UID 6 FLAGS () {date} RFC822.SIZE 22 BODYSTRUCTURE {small})\r\n\
                         {{tag}} OK\r\n"
                    ),
                ),
                // Only the small ones come whole; 6 was deleted in between.
                bodies("5:6", &[5]),
                step(
                    "{tag} UID FETCH 4 (UID BODY.PEEK[HEADER] BODY.PEEK[1])",
                    &format!(
                        "* 1 FETCH (UID 4 BODY[HEADER] {{{}}}\r\n{header} BODY[1] {{5}}\r\nHello)\r\n{{tag}} OK\r\n",
                        header.len()
                    ),
                ),
            ],
        )]);
        let p = provider(&fake);
        let batch = p.fetch_since("INBOX", None, 10).await.unwrap();
        let ids: Vec<&str> = batch.messages.iter().map(|m| m.provider_id.as_str()).collect();
        assert_eq!(ids, ["7:5", "7:4"]);
        assert_eq!(batch.messages[0].raw, body(5).as_bytes());
        let big = String::from_utf8(batch.messages[1].raw.clone()).unwrap();
        assert!(big.starts_with(header) && big.contains("Hello") && big.contains("X-Moss-Omitted"), "{big}");
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_big_message_comes_whole_when_its_parts_dont() {
        let structure = r#"(("text" "plain" NIL NIL NIL "7bit" 5 1 NIL NIL NIL NIL)("application" "pdf" NIL NIL NIL "base64" 780000 NIL ("attachment" NIL) NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                select("INBOX", false, 1, 7, Some(10), None),
                search("UID 1:9", "9"),
                fetch_big(9, structure),
                // The server leaves the text out.
                step(
                    "{tag} UID FETCH 9 (UID BODY.PEEK[HEADER] BODY.PEEK[1])",
                    "* 1 FETCH (UID 9 BODY[HEADER] {2}\r\n\r\n)\r\n{tag} OK\r\n",
                ),
                fetch_whole(9),
            ],
        )]);
        let p = provider(&fake);
        let batch = p.fetch_since("INBOX", None, 1).await.unwrap();
        assert_eq!(batch.messages[0].raw, body(9).as_bytes());
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn sets_and_clears_flags() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                inbox(),
                ok("{tag} UID STORE 3:4 +FLAGS.SILENT (\\Seen)"),
                inbox(),
                ok("{tag} UID STORE 3 -FLAGS.SILENT (\\Flagged)"),
                inbox(),
                ok("{tag} UID STORE 3 +FLAGS.SILENT (\\Answered)"),
                inbox(),
                ok("{tag} UID STORE 3 +FLAGS.SILENT ($Forwarded)"),
                inbox(),
            ],
        )]);
        let p = provider(&fake);
        p.apply("INBOX", &["7:4".into(), "7:3".into()], &Change::Seen(true))
            .await
            .unwrap();
        p.apply("INBOX", &["7:3".into()], &Change::Flagged(false))
            .await
            .unwrap();
        p.apply("INBOX", &["7:3".into()], &Change::Answered(true))
            .await
            .unwrap();
        p.apply("INBOX", &["7:3".into()], &Change::Forwarded(true))
            .await
            .unwrap();
        let stale = p.apply("INBOX", &["6:3".into()], &Change::Seen(true)).await;
        assert!(matches!(stale, Err(Error::NotFound(m)) if m.contains("reset")));
        p.apply("INBOX", &[], &Change::Seen(true)).await.unwrap();
        assert!(matches!(
            p.apply("INBOX", &["x".into()], &Change::Seen(true)).await,
            Err(Error::NotFound(_))
        ));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn creates_a_folder_once() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                ok("{tag} CREATE \"Blocked\""),
                step("{tag} CREATE \"Blocked\"", "{tag} NO [ALREADYEXISTS] Mailbox exists\r\n"),
                step("{tag} CREATE \"Blocked\"", "{tag} NO [NOPERM] Not allowed\r\n"),
            ],
        )]);
        let p = provider(&fake);
        p.create_mailbox("Blocked").await.unwrap();
        // Already there (made by Spark, say) is fine.
        p.create_mailbox("Blocked").await.unwrap();
        assert!(matches!(
            p.create_mailbox("Blocked").await,
            Err(Error::Protocol(m)) if m == "The server refused the IMAP command."
        ));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn moves_with_move_or_copy_and_expunge() {
        let ids = ["7:3".to_string()];
        let to_work = Change::Move { to: "Work".into() };
        let fake = FakeImap::new(vec![
            script(
                "MOVE",
                vec![inbox(), ok("{tag} UID MOVE 3 \"Work\""), inbox()],
            ),
            script(
                "UIDPLUS",
                vec![
                    inbox(),
                    ok("{tag} UID COPY 3 \"Work\""),
                    ok("{tag} UID STORE 3 +FLAGS.SILENT (\\Deleted)"),
                    step("{tag} UID EXPUNGE 3", "* 1 EXPUNGE\r\n{tag} OK\r\n"),
                ],
            ),
            script(
                "",
                vec![
                    inbox(),
                    ok("{tag} UID COPY 3 \"Work\""),
                    ok("{tag} UID STORE 3 +FLAGS.SILENT (\\Deleted)"),
                    step("{tag} EXPUNGE", "* 1 EXPUNGE\r\n{tag} OK\r\n"),
                ],
            ),
        ]);
        for _ in 0..3 {
            let p = provider(&fake);
            p.apply("INBOX", &ids, &to_work).await.unwrap();
            if p.main.lock().await.as_ref().unwrap().caps.mv {
                // Moving a message to where it already is does nothing.
                p.apply("INBOX", &ids, &Change::Move { to: "INBOX".into() })
                    .await
                    .unwrap();
            }
        }
        fake.finish().await;
    }

    #[tokio::test]
    async fn archives_and_trashes_by_folder_role() {
        let ids = ["7:3".to_string()];
        let fake = FakeImap::new(vec![
            script(
                "MOVE",
                vec![
                    inbox(),
                    list(&[
                        r#"* LIST () "/" "INBOX""#,
                        r#"* LIST (\Archive) "/" "Archive""#,
                        r#"* LIST (\Trash) "/" "Bin""#,
                    ]),
                    ok("{tag} UID MOVE 3 \"Archive\""),
                    inbox(),
                    ok("{tag} UID MOVE 3 \"Bin\""),
                ],
            ),
            script(
                "MOVE X-GM-EXT-1",
                vec![
                    inbox(),
                    list(&[
                        r#"* LIST () "/" "INBOX""#,
                        r#"* LIST (\All) "/" "[Gmail]/All Mail""#,
                    ]),
                    ok("{tag} UID MOVE 3 \"[Gmail]/All Mail\""),
                    select("[Gmail]/All Mail", false, 3, 7, Some(10), None),
                ],
            ),
            script(
                "MOVE",
                vec![inbox(), list(&[r#"* LIST () "/" "INBOX""#]), inbox()],
            ),
        ]);
        let p = provider(&fake);
        p.apply("INBOX", &ids, &Change::Archive).await.unwrap();
        p.apply("INBOX", &ids, &Change::Trash).await.unwrap();
        let gmail = provider(&fake);
        gmail.apply("INBOX", &ids, &Change::Archive).await.unwrap();
        gmail
            .apply("[Gmail]/All Mail", &ids, &Change::Archive)
            .await
            .unwrap();
        let bare = provider(&fake);
        assert!(matches!(
            bare.apply("INBOX", &ids, &Change::Archive).await,
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            bare.apply("INBOX", &ids, &Change::Trash).await,
            Err(Error::Unsupported(_))
        ));
        drop((p, gmail, bare));
        fake.finish().await;
    }

    #[tokio::test]
    async fn unarchives_by_message_id_back_to_the_inbox() {
        let by_id = |id: &str, uids: &str| {
            step(
                &format!("{{tag}} UID SEARCH HEADER Message-ID \"<{id}>\""),
                &format!("* SEARCH{uids}\r\n{{tag}} OK\r\n"),
            )
        };
        let fake = FakeImap::new(vec![
            script(
                "MOVE",
                vec![
                    list(&[
                        r#"* LIST () "/" "INBOX""#,
                        r#"* LIST (\Archive) "/" "Archive""#,
                    ]),
                    select("Archive", false, 3, 7, Some(10), None),
                    by_id("a@x", " 5"),
                    by_id("b@x", " 3 5"),
                    ok("{tag} UID MOVE 3,5 \"INBOX\""),
                    // Never archived: nothing to move.
                    select("Archive", false, 3, 7, Some(10), None),
                    by_id("c@x", ""),
                ],
            ),
            // Gmail keeps everything in All Mail: copying there puts the Inbox label back.
            script(
                "MOVE X-GM-EXT-1",
                vec![
                    list(&[
                        r#"* LIST () "/" "INBOX""#,
                        r#"* LIST (\All) "/" "[Gmail]/All Mail""#,
                    ]),
                    select("[Gmail]/All Mail", false, 3, 7, Some(10), None),
                    by_id("a@x", " 4"),
                    ok("{tag} UID COPY 4 \"INBOX\""),
                ],
            ),
            // No Archive folder, so done never moved anything.
            script("MOVE", vec![list(&[r#"* LIST () "/" "INBOX""#])]),
        ]);
        let p = provider(&fake);
        p.unarchive(&["a@x".into(), "b@x".into()]).await.unwrap();
        p.unarchive(&["c@x".into()]).await.unwrap();
        // Ids that can't go in a search are skipped, so these need no server at all.
        p.unarchive(&[]).await.unwrap();
        p.unarchive(&[
            "quote\"d@x".into(),
            "back\\slash@x".into(),
            "ünï@x".into(),
            "sp ace@x".into(),
        ])
        .await
        .unwrap();
        let gmail = provider(&fake);
        gmail.unarchive(&["a@x".into()]).await.unwrap();
        let bare = provider(&fake);
        bare.unarchive(&["a@x".into()]).await.unwrap();
        drop((p, gmail, bare));
        fake.finish().await;
    }

    #[tokio::test]
    async fn trashes_and_untrashes_by_message_id() {
        let by_id = |id: &str, uids: &str| {
            step(
                &format!("{{tag}} UID SEARCH HEADER Message-ID \"<{id}>\""),
                &format!("* SEARCH{uids}\r\n{{tag}} OK\r\n"),
            )
        };
        let at = |name: &str| select(name, false, 3, 7, Some(10), None);
        let boxes = [
            r#"* LIST () "/" "INBOX""#,
            r#"* LIST (\Archive) "/" "Archive""#,
            r#"* LIST (\Trash) "/" "Bin""#,
        ];
        let gmail_boxes = [
            r#"* LIST () "/" "INBOX""#,
            r#"* LIST (\All) "/" "[Gmail]/All Mail""#,
            r#"* LIST (\Trash) "/" "[Gmail]/Bin""#,
        ];
        let no_archive = [r#"* LIST () "/" "INBOX""#, r#"* LIST (\Trash) "/" "Trash""#];
        let fake = FakeImap::new(vec![
            script(
                "MOVE",
                vec![
                    // From the Inbox or the archive, wherever each one is.
                    list(&boxes),
                    at("INBOX"),
                    by_id("a@x", " 3"),
                    by_id("b@x", ""),
                    ok("{tag} UID MOVE 3 \"Bin\""),
                    at("Archive"),
                    by_id("a@x", ""),
                    by_id("b@x", " 6"),
                    ok("{tag} UID MOVE 6 \"Bin\""),
                    // Undone (the folders are known by now): back to the Inbox, or the archive.
                    at("Bin"),
                    by_id("a@x", " 8"),
                    ok("{tag} UID MOVE 8 \"INBOX\""),
                    at("Bin"),
                    by_id("b@x", " 9"),
                    ok("{tag} UID MOVE 9 \"Archive\""),
                    // Emptied from the Bin meanwhile: nothing to bring back.
                    at("Bin"),
                    by_id("c@x", ""),
                ],
            ),
            // Gmail keeps every message in All Mail: moving it to the Bin deletes it everywhere.
            script(
                "MOVE X-GM-EXT-1",
                vec![
                    list(&gmail_boxes),
                    at("[Gmail]/All Mail"),
                    by_id("a@x", " 4"),
                    ok("{tag} UID MOVE 4 \"[Gmail]/Bin\""),
                    // Done mail goes back to All Mail only.
                    at("[Gmail]/Bin"),
                    by_id("a@x", " 5"),
                    ok("{tag} UID MOVE 5 \"[Gmail]/All Mail\""),
                    // Out of the Bin a message lands in All Mail without the Inbox label,
                    // so inbox mail is also copied to INBOX, which puts the label back.
                    at("[Gmail]/Bin"),
                    by_id("b@x", " 6"),
                    ok("{tag} UID MOVE 6 \"[Gmail]/All Mail\""),
                    at("[Gmail]/All Mail"),
                    by_id("b@x", " 12"),
                    ok("{tag} UID COPY 12 \"INBOX\""),
                    // Emptied from the Bin meanwhile: nothing to bring back.
                    at("[Gmail]/Bin"),
                    by_id("c@x", ""),
                ],
            ),
            // No archive folder, so done mail was still in the Inbox.
            script(
                "MOVE",
                vec![
                    list(&no_archive),
                    at("INBOX"),
                    by_id("a@x", ""),
                    at("Trash"),
                    by_id("a@x", " 2"),
                    ok("{tag} UID MOVE 2 \"INBOX\""),
                ],
            ),
            // No Trash folder: nothing can be deleted, so nothing to bring back.
            script("MOVE", vec![list(&[r#"* LIST () "/" "INBOX""#])]),
        ]);
        let p = provider(&fake);
        p.trash(&["a@x".into(), "b@x".into()]).await.unwrap();
        p.untrash(&["a@x".into()], true).await.unwrap();
        p.untrash(&["b@x".into()], false).await.unwrap();
        p.untrash(&["c@x".into()], true).await.unwrap();
        // Ids that can't go in a search are skipped, so these need no server at all.
        p.trash(&["quote\"d@x".into()]).await.unwrap();
        p.untrash(&[], true).await.unwrap();
        let gmail = provider(&fake);
        gmail.trash(&["a@x".into()]).await.unwrap();
        gmail.untrash(&["a@x".into()], false).await.unwrap();
        gmail.untrash(&["b@x".into()], true).await.unwrap();
        gmail.untrash(&["c@x".into()], true).await.unwrap();
        let plain = provider(&fake);
        plain.trash(&["a@x".into()]).await.unwrap();
        plain.untrash(&["a@x".into()], false).await.unwrap();
        let bare = provider(&fake);
        assert!(matches!(bare.trash(&["a@x".into()]).await, Err(Error::Unsupported(_))));
        bare.untrash(&["a@x".into()], true).await.unwrap();
        drop((p, gmail, plain, bare));
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_listing_without_the_inbox_still_trashes_from_and_restores_to_inbox() {
        // Some servers leave INBOX out of LIST "" *; it is there all the same (RFC 3501).
        let by_id = |uids: &str| {
            step("{tag} UID SEARCH HEADER Message-ID \"<a@x>\"", &format!("* SEARCH{uids}\r\n{{tag}} OK\r\n"))
        };
        let at = |name: &str| select(name, false, 3, 7, Some(10), None);
        let fake = FakeImap::new(vec![script("MOVE", vec![
            list(&[r#"* LIST (\Archive) "/" "Archive""#, r#"* LIST (\Trash) "/" "Bin""#]),
            at("INBOX"),
            by_id(" 3"),
            ok("{tag} UID MOVE 3 \"Bin\""),
            at("Archive"),
            by_id(""),
            at("Bin"),
            by_id(" 8"),
            ok("{tag} UID MOVE 8 \"INBOX\""),
        ])]);
        let p = provider(&fake);
        p.trash(&["a@x".into()]).await.unwrap();
        p.untrash(&["a@x".into()], true).await.unwrap();
        drop(p);
        fake.finish().await;
    }

    fn draft_message(id: &str) -> OutgoingMessage {
        OutgoingMessage {
            to: vec![],
            bcc: vec![],
            message_id: Some(format!("<{id}>")),
            headers: vec![("X-Moss-Draft".into(), "d1".into())],
            ..outgoing()
        }
    }

    #[tokio::test]
    async fn draft_append_and_explicit_cleanup_have_separate_receipts() {
        let by_id = |id: &str, uids: &str| {
            step(
                &format!("{{tag}} UID SEARCH HEADER Message-ID \"<{id}>\""),
                &format!("* SEARCH{uids}\r\n{{tag}} OK\r\n"),
            )
        };
        let drafts = || select("Drafts", false, 3, 7, Some(13), None);
        let fake = FakeImap::new(vec![
            script(
                "UIDPLUS",
                vec![
                    ok(&format!("{{tag}} APPEND \"Drafts\" (\\Seen \\Draft) {{{ANY}")),
                    // Cleanup is a separate operation after the receipt is returned.
                    drafts(),
                    by_id("old@x.io", " 11"),
                    ok("{tag} UID STORE 11 +FLAGS.SILENT (\\Deleted)"),
                    ok("{tag} UID EXPUNGE 11"),
                    // A server that doesn't find what it just took, and nothing to replace.
                    ok(&format!("{{tag}} APPEND \"Drafts\" (\\Seen \\Draft) {{{ANY}")),
                    // Discarded separately, without UIDPLUS too.
                ],
            ),
            script(
                "",
                vec![
                    select("Drafts", false, 3, 7, Some(13), None),
                    by_id("n1@x.io", " 12"),
                    ok("{tag} UID STORE 12 +FLAGS.SILENT (\\Deleted)"),
                    ok("{tag} EXPUNGE"),
                    // Already gone elsewhere: nothing to do.
                    select("Drafts", false, 3, 7, Some(13), None),
                    by_id("n2@x.io", ""),
                ],
            ),
        ]);
        let p = provider(&fake);
        let saved = p.save_draft("Drafts", &draft_message("n1@x.io")).await.unwrap();
        assert_eq!(
            saved,
            Some(SavedDraft { message_id: "n1@x.io".into(), provider_id: None })
        );
        p.delete_drafts("Drafts", &["old@x.io".into()]).await.unwrap();
        let saved = p.save_draft("Drafts", &draft_message("n2@x.io")).await.unwrap();
        assert_eq!(saved, Some(SavedDraft { message_id: "n2@x.io".into(), provider_id: None }));
        // Ids that can't go in a search need no server.
        p.delete_drafts("Drafts", &["quote\"d@x".into()]).await.unwrap();
        let plain = provider(&fake);
        plain.delete_drafts("Drafts", &["n1@x.io".into()]).await.unwrap();
        plain.delete_drafts("Drafts", &["n2@x.io".into()]).await.unwrap();
        // A draft with no sender isn't sent to the server at all.
        let nobody = OutgoingMessage { from: Address::new(""), ..draft_message("n3@x.io") };
        assert!(matches!(plain.save_draft("Drafts", &nobody).await, Err(Error::InvalidMessage(_))));
        drop((p, plain));
        fake.finish().await;
    }

    #[tokio::test]
    async fn gmail_deletes_a_draft_for_good_through_the_bin() {
        let by_id = |id: &str, uids: &str| {
            step(
                &format!("{{tag}} UID SEARCH HEADER Message-ID \"<{id}>\""),
                &format!("* SEARCH{uids}\r\n{{tag}} OK\r\n"),
            )
        };
        let at = |name: &str| select(name, false, 3, 7, Some(50), None);
        let boxes = [
            r#"* LIST () "/" "INBOX""#,
            r#"* LIST (\Drafts) "/" "[Gmail]/Drafts""#,
            r#"* LIST (\Trash) "/" "[Gmail]/Bin""#,
        ];
        let fake = FakeImap::new(vec![
            script(
                "MOVE UIDPLUS X-GM-EXT-1",
                vec![
                    list(&boxes),
                    at("[Gmail]/Drafts"),
                    by_id("old@x.io", " 11"),
                    // Expunged from Drafts alone, Gmail would keep it in All Mail.
                    ok("{tag} UID MOVE 11 \"[Gmail]/Bin\""),
                    at("[Gmail]/Bin"),
                    by_id("old@x.io", " 40"),
                    ok("{tag} UID STORE 40 +FLAGS.SILENT (\\Deleted)"),
                    ok("{tag} UID EXPUNGE 40"),
                    // Not there any more: nothing to move.
                    at("[Gmail]/Drafts"),
                    by_id("gone@x.io", ""),
                    // Gone from the Bin as soon as it got there: nothing left to delete.
                    at("[Gmail]/Drafts"),
                    by_id("quick@x.io", " 12"),
                    ok("{tag} UID MOVE 12 \"[Gmail]/Bin\""),
                    at("[Gmail]/Bin"),
                    by_id("quick@x.io", ""),
                ],
            ),
            // No Bin to go through: removed from Drafts.
            script(
                "MOVE UIDPLUS X-GM-EXT-1",
                vec![
                    list(&[r#"* LIST (\Drafts) "/" "[Gmail]/Drafts""#]),
                    at("[Gmail]/Drafts"),
                    by_id("old@x.io", " 11"),
                    ok("{tag} UID STORE 11 +FLAGS.SILENT (\\Deleted)"),
                    ok("{tag} UID EXPUNGE 11"),
                ],
            ),
        ]);
        let gmail = provider(&fake);
        gmail.delete_drafts("[Gmail]/Drafts", &["old@x.io".into()]).await.unwrap();
        gmail.delete_drafts("[Gmail]/Drafts", &["gone@x.io".into()]).await.unwrap();
        gmail.delete_drafts("[Gmail]/Drafts", &["quick@x.io".into()]).await.unwrap();
        let no_bin = provider(&fake);
        no_bin.delete_drafts("[Gmail]/Drafts", &["old@x.io".into()]).await.unwrap();
        drop((gmail, no_bin));
        fake.finish().await;
    }

    #[tokio::test]
    async fn lists_mailboxes_fresh_each_time() {
        let lines = [r#"* LIST () "/" "INBOX""#, r#"* LIST (\Sent) "/" "Sent""#];
        let fake = FakeImap::new(vec![script("", vec![list(&lines), list(&lines[..1])])]);
        let p = provider(&fake);
        assert_eq!(p.list_mailboxes().await.unwrap().len(), 2);
        assert_eq!(p.list_mailboxes().await.unwrap().len(), 1);
        assert!(p.capabilities().server_flags);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn lists_message_ids_for_servers_without_qresync() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![inbox(), search("ALL", "1 2"), inbox(), search("ALL", "1 2")],
        )]);
        let p = provider(&fake);
        assert_eq!(p.message_ids("INBOX").await.unwrap(), ["7:1", "7:2"]);
        assert_eq!(
            Provider::all_ids(&p, "INBOX").await.unwrap(),
            Some(vec!["7:1".to_string(), "7:2".to_string()])
        );
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn draft_listing_keeps_generation_even_when_the_folder_is_empty() {
        let fake = FakeImap::new(vec![script("", vec![
            select("Drafts", false, 2, 7, Some(3), None), search("ALL", "2 1"),
            select("Drafts", false, 0, 9, Some(1), None), search("ALL", ""),
        ])]);
        let p = provider(&fake);
        assert_eq!(p.list_drafts("Drafts").await.unwrap(), Some(crate::DraftListing {
            mailbox: "Drafts".into(), generation: "7".into(), provider_ids: vec!["7:1".into(), "7:2".into()],
        }));
        assert_eq!(p.list_drafts("Drafts").await.unwrap(), Some(crate::DraftListing {
            mailbox: "Drafts".into(), generation: "9".into(), provider_ids: vec![],
        }));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn draft_listing_refuses_missing_or_zero_generation() {
        for generation in ["", "* OK [UIDVALIDITY 0] ok\r\n"] {
            let selected = format!("* 0 EXISTS\r\n{generation}{{tag}} OK [READ-WRITE] selected\r\n");
            let fake = FakeImap::new(vec![script("", vec![step("{tag} SELECT \"Drafts\"", &selected)])]);
            let p = provider(&fake);
            assert!(matches!(p.list_drafts("Drafts").await, Err(Error::Network(message)) if message == "The mailbox has no valid UIDVALIDITY."));
            drop(p);
            fake.finish().await;
        }
    }

    #[tokio::test]
    async fn legacy_listing_refuses_missing_generation_without_searching() {
        for generation in ["", "* OK [UIDVALIDITY 0] ok\r\n"] {
            let selected = format!("* 0 EXISTS\r\n{generation}{{tag}} OK [READ-WRITE] selected\r\n");
            let fake = FakeImap::new(vec![script("", vec![step("{tag} SELECT \"INBOX\"", &selected)])]);
            let p = provider(&fake);
            let result = p.all_ids("INBOX").await;
            assert!(matches!(&result, Err(Error::Network(message)) if message == "The mailbox has no valid UIDVALIDITY."), "{result:?}");
            drop(p);
            fake.finish().await;
        }
    }

    #[tokio::test]
    async fn draft_listing_does_not_turn_select_or_search_errors_into_absence() {
        for steps in [
            vec![step("{tag} SELECT \"Drafts\"", "{tag} NO unavailable\r\n")],
            vec![select("Drafts", false, 0, 7, Some(1), None), step("{tag} UID SEARCH ALL", "{tag} BAD refused\r\n")],
        ] {
            let fake = FakeImap::new(vec![script("", steps)]);
            let p = provider(&fake);
            assert!(p.list_drafts("Drafts").await.is_err());
            drop(p);
            fake.finish().await;
        }
    }

    #[tokio::test]
    async fn checked_select_quotes_names_rejects_controls_and_discards_old_notifications() {
        let fake = FakeImap::new(vec![script("CONDSTORE", vec![step(
            r#"{tag} SELECT "a\\b\"c" (CONDSTORE)"#,
            "* FLAGS (\\Seen)\r\n* 0 EXISTS\r\n* OK [UIDNEXT 1] next\r\n* OK [HIGHESTMODSEQ 42] modseq\r\n* VANISHED 4:5\r\n{tag} OK [UIDVALIDITY 7] selected\r\n",
        )])]);
        let p = provider(&fake);
        let mut conn = p.open().await.unwrap();
        for name in ["a\r", "a\n", "a\0"] {
            assert!(matches!(conn.select(name).await, Err(Error::Protocol(_))));
        }
        let info = conn.select("a\\b\"c").await.unwrap();
        assert_eq!((info.exists, info.uid_validity, info.uid_next, info.highest_modseq), (0, Some(7), Some(1), Some(42)));
        assert!(conn.take_vanished(7).is_empty());
        drop(conn); drop(p); fake.finish().await;
    }

    #[tokio::test]
    async fn checked_select_failure_is_safe_and_reconnects_when_needed() {
        for (reply, reconnect, kind) in [
            ("{tag} NO private-server-detail\r\n", false, "notFound"),
            ("{tag} BAD private-server-detail\r\n", false, "protocol"),
            ("* BYE private-server-detail\r\n", true, "network"),

            ("* nonsense private-server-detail\r\n", true, "network"),
            ("* 0 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n<hang up>", true, "network"),
        ] {
            let first = vec![step("{tag} SELECT \"INBOX\"", reply)];
            let next = vec![inbox(), search("ALL", "9")];
            let scripts = if reconnect { vec![script("", first), script("", next)] }
                else { vec![script("", [first, next].concat())] };
            let fake = FakeImap::new(scripts);
            let p = provider(&fake);
            let error = p.message_ids("INBOX").await.unwrap_err();
            let actual = match error { Error::NotFound(_) => "notFound", Error::Protocol(_) => "protocol", Error::Network(_) => "network", _ => "other" };
            assert_eq!(actual, kind);
            assert!(!error.to_string().contains("private-server-detail"));
            assert_eq!(p.message_ids("INBOX").await.unwrap(), vec!["7:9"]);
            drop(p); fake.finish().await;
        }
    }

    #[tokio::test]
    async fn checked_select_accepts_warnings_and_resets_metadata_at_closed() {
        let fake = FakeImap::new(vec![script("QRESYNC", vec![step(
            "{tag} SELECT \"INBOX\" (CONDSTORE)",
            "* 99 EXISTS\r\n* OK [UIDVALIDITY 4] old\r\n* OK [UIDNEXT 100] old\r\n* OK [HIGHESTMODSEQ 50] old\r\n* VANISHED 4:5\r\n* OK [cLoSeD] old closed\r\n* NO [ALERT] warning\r\n* BAD warning\r\n* 0 EXISTS\r\n* OK [UIDVALIDITY 7] valid\r\n* OK [UIDVALIDITY 7] repeated\r\n{tag} OK selected\r\n",
        )])]);
        let p = provider(&fake);
        let mut conn = p.open().await.unwrap();
        conn.search_vanished.push(100..=200);
        let info = conn.select("INBOX").await.unwrap();
        assert_eq!((info.exists, info.uid_validity, info.uid_next, info.highest_modseq), (0, Some(7), None, None));
        assert!(conn.take_vanished(7).is_empty());
        drop(conn); drop(p); fake.finish().await;
    }

    #[tokio::test]
    async fn checked_select_rejects_conflicting_generations_and_missing_new_metadata() {
        for reply in [
            "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] valid\r\n* OK [UIDVALIDITY 8] different\r\n{tag} OK done\r\n",
            "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] old\r\n* OK [CLOSED] old closed\r\n{tag} OK done\r\n",
        ] {
            let fake = FakeImap::new(vec![script("", vec![step("{tag} SELECT \"INBOX\"", reply)])]);
            let p = provider(&fake);
            let mut conn = p.open().await.unwrap();
            assert!(matches!(conn.select("INBOX").await, Err(Error::Network(_))));
            drop(conn); drop(p); fake.finish().await;
        }
    }

    #[tokio::test]
    async fn select_requires_complete_metadata_and_matching_success() {
        let mut accepted = Vec::new();
        for (case, reply) in [
            ("eof", HANG_UP),
            ("partial metadata", "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] valid\r\n* OK [UIDNEXT 1] next\r\n<hang up>"),
            ("missing generation", "* 0 EXISTS\r\n{tag} OK done\r\n"),
            ("zero generation", "* 0 EXISTS\r\n* OK [UIDVALIDITY 0] valid\r\n{tag} OK done\r\n"),
            ("missing exists", "* OK [UIDVALIDITY 7] valid\r\n{tag} OK done\r\n"),
            ("zero next", "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] valid\r\n* OK [UIDNEXT 0] next\r\n{tag} OK done\r\n"),
            ("wrong tag", "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] valid\r\nwrong OK done\r\n{tag} OK done\r\n"),
            ("continuation", "+ unexpected\r\n* 0 EXISTS\r\n* OK [UIDVALIDITY 7] valid\r\n{tag} OK done\r\n"),
        ] {
            let fake = FakeImap::new(vec![script("", vec![step("{tag} SELECT \"INBOX\"", reply)])]);
            let p = provider(&fake);
            let mut conn = p.open().await.unwrap();
            if conn.select("INBOX").await.is_ok() { accepted.push(case); }
            drop(conn); drop(p); fake.finish().await;
        }
        assert!(accepted.is_empty(), "accepted incomplete selections: {accepted:?}");
    }

    #[tokio::test]
    async fn search_requires_a_matching_successful_completion_and_result() {
        let mut accepted = Vec::new();
        for (case, reply) in [
            ("rejected", "{tag} NO refused\r\n"),
            ("bad", "{tag} BAD refused\r\n"),
            ("partial", "* SEARCH 1 2\r\n{tag} NO refused\r\n"),
            ("missing result", "{tag} OK done\r\n"),
            ("wrong tag", "* SEARCH 1\r\nwrong OK done\r\n{tag} OK done\r\n"),
            ("bye", "* BYE closing\r\n* SEARCH 1\r\n{tag} OK done\r\n"),
            ("continuation", "+ unexpected\r\n* SEARCH 1\r\n{tag} OK done\r\n"),
            ("disconnect", HANG_UP),
        ] {
            let fake = FakeImap::new(vec![script("", vec![inbox(), step("{tag} UID SEARCH ALL", reply)])]);
            let p = provider(&fake);
            if p.message_ids("INBOX").await.is_ok() { accepted.push(case); }
            drop(p);
            fake.finish().await;
        }
        assert!(accepted.is_empty(), "accepted incomplete searches: {accepted:?}");
    }

    #[tokio::test]
    async fn a_search_that_finds_nothing_may_come_without_a_result_line() {
        // iCloud answers a search with no matches with the tagged OK alone.
        let fake = FakeImap::new(vec![script("", vec![
            inbox(),
            step("{tag} UID SEARCH UID 10:*", "{tag} OK done\r\n"),
            select("Junk", false, 0, 7, Some(1), None),
            step("{tag} UID SEARCH ALL", "{tag} OK done\r\n"),
        ])]);
        let p = provider(&fake);
        let mut conn = p.open().await.unwrap();
        conn.select("INBOX").await.unwrap();
        assert!(conn.search("UID 10:*").await.unwrap().is_empty());
        // An empty folder lists as empty; one the server says holds mail does not (see
        // `search_requires_a_matching_successful_completion_and_result`).
        assert!(conn.message_ids("Junk").await.unwrap().is_empty());
        drop(conn);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn checked_search_preserves_vanished_and_sorts_unique_ids() {
        let fake = FakeImap::new(vec![script("", vec![inbox(), step("{tag} UID SEARCH ALL",
            "* 2 EXISTS\r\n* VANISHED 4:5\r\n* SEARCH 2 1 2\r\n{tag} OK done\r\n")])]);
        let p = provider(&fake);
        let mut conn = p.open().await.unwrap();
        conn.select("INBOX").await.unwrap();
        assert_eq!(conn.search("ALL").await.unwrap(), vec![1, 2]);
        assert_eq!(conn.take_vanished(7), vec!["7:4", "7:5"]);
        assert!(conn.take_vanished(7).is_empty());
        drop(conn);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn failed_search_uses_only_a_synchronized_connection() {
        for (reply, reconnect) in [
            ("{tag} NO private-server-detail\r\n", false),
            ("{tag} OK done\r\n", false),
            ("* SEARCH 1\r\nwrong OK done\r\n{tag} OK done\r\n", true),
            ("* BYE private-server-detail\r\n{tag} OK done\r\n", true),
            ("+ unexpected\r\n{tag} OK done\r\n", true),
            (HANG_UP, true),
        ] {
            let first = vec![inbox(), step("{tag} UID SEARCH ALL", reply)];
            let next = vec![inbox(), search("ALL", "9")];
            let scripts = if reconnect { vec![script("", first), script("", next)] }
                else { vec![script("", first.into_iter().chain(next).collect())] };
            let fake = FakeImap::new(scripts);
            let p = provider(&fake);
            let error = p.message_ids("INBOX").await.unwrap_err();
            assert_eq!(matches!(error, Error::Network(_)), reconnect);
            assert!(!error.to_string().contains("private-server-detail"));
            assert_eq!(p.message_ids("INBOX").await.unwrap(), vec!["7:9"]);
            drop(p);
            fake.finish().await;
        }
    }

    #[tokio::test]
    async fn acknowledged_draft_append_returns_before_any_followup_command() {
        let fake = FakeImap::new(vec![script("", vec![
            ok(&format!("{{tag}} APPEND \"Drafts\" (\\Seen \\Draft) {{{ANY}")),
        ])]);
        let p = provider(&fake);
        let saved = p.save_draft("Drafts", &draft_message("acknowledged@fixture")).await;
        assert_eq!(saved.unwrap(), Some(SavedDraft { message_id:"acknowledged@fixture".into(),provider_id:None }));
        drop(p);
        fake.finish().await;
    }

    fn outgoing() -> OutgoingMessage {
        OutgoingMessage {
            from: Address::new("ada@x.io"),
            to: vec![Address::new("bob@example.com")],
            bcc: vec![Address::new("dan@example.com")],
            subject: "Hi".into(),
            text: Some("Hello".into()),
            message_id: Some("<m1@x.io>".into()),
            ..Default::default()
        }
    }

    async fn sending_provider(
        fake: &Arc<FakeImap>,
    ) -> (ImapProvider, Arc<std::sync::Mutex<crate::testing::SmtpLog>>) {
        let auth = base64::engine::general_purpose::STANDARD.encode("\0ada@x.io\0smtp-pw");
        let (port, root, log) = smtp_server(SmtpMode::Tls, &auth).await;
        (
            ImapProvider::with_connector(fake.clone(), password("pw"), smtp_config(port, root)),
            log,
        )
    }

    #[tokio::test]
    async fn sends_then_files_a_copy_in_sent() {
        let fake = FakeImap::new(vec![script(
            "",
            vec![
                list(&[r#"* LIST () "/" "INBOX""#, r#"* LIST (\Sent) "/" "Sent""#]),
                ok(&format!("{{tag}} APPEND \"Sent\" (\\Seen) {{{ANY}")),
            ],
        )]);
        let (p, log) = sending_provider(&fake).await;
        let sent = p.send(&outgoing()).await.unwrap();
        assert!(sent.saved_on_server);
        assert_eq!(sent.message_id, "<m1@x.io>");
        assert_eq!(sent.provider_id, None);
        let raw = String::from_utf8(sent.raw).unwrap();
        assert!(raw.contains("Bcc: dan@example.com"));
        let wire = String::from_utf8(log.lock().unwrap().data.clone()).unwrap();
        assert!(wire.contains("Message-ID: <m1@x.io>") && !wire.contains("Bcc"));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_failed_copy_to_sent_doesnt_fail_the_send() {
        let fake = FakeImap::new(vec![
            script("", vec![list(&[r#"* LIST () "/" "INBOX""#])]),
            script(
                "",
                vec![
                    list(&[r#"* LIST (\Sent) "/" "Sent""#]),
                    step(
                        &format!("{{tag}} APPEND \"Sent\" (\\Seen) {{{ANY}"),
                        "{tag} NO [OVERQUOTA] full\r\n",
                    ),
                ],
            ),
        ]);
        for _ in 0..2 {
            let (p, _log) = sending_provider(&fake).await;
            assert!(!p.send(&outgoing()).await.unwrap().saved_on_server);
        }
        fake.finish().await;
    }

    #[tokio::test]
    async fn gmail_style_accounts_skip_the_copy_and_smtp_errors_stop_the_send() {
        let fake = FakeImap::new(vec![]);
        let (p, log) = sending_provider(&fake).await;
        let p = p.save_sent_copies(false);
        assert!(!p.send(&outgoing()).await.unwrap().saved_on_server);
        assert!(!log.lock().unwrap().data.is_empty());
        let unreachable = provider(&fake);
        assert!(matches!(
            unreachable.send(&outgoing()).await,
            Err(Error::Network(_))
        ));
        assert!(matches!(
            unreachable.send(&OutgoingMessage::default()).await,
            Err(Error::InvalidMessage(_))
        ));
        fake.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn idle_wakes_on_new_mail_and_times_out_on_quiet() {
        let fake = FakeImap::new(vec![
            script(
                "IDLE",
                vec![
                    inbox(),
                    step("{tag} IDLE", "+ idling\r\n* 4 EXISTS\r\n"),
                    step("DONE", "{tag} OK IDLE terminated\r\n"),
                    inbox(),
                    step("{tag} IDLE", "+ idling\r\n"),
                    step("DONE", "{tag} OK IDLE terminated\r\n"),
                    step("{tag} SELECT \"Nope\"", "{tag} NO no such mailbox\r\n"),
                    inbox(),
                    step("{tag} IDLE", "+ idling\r\n* 5 EXISTS\r\n"),
                    step("DONE", HANG_UP),
                ],
            ),
            script("IDLE", vec![inbox(), step("{tag} IDLE", HANG_UP)]),
        ]);
        let p = provider(&fake);
        let minute = Duration::from_secs(60);
        assert_eq!(
            p.wait_for_change("INBOX", minute).await.unwrap(),
            Wakeup::Changed
        );
        assert_eq!(
            p.wait_for_change("INBOX", minute).await.unwrap(),
            Wakeup::Timeout
        );
        assert!(matches!(
            p.wait_for_change("Nope", minute).await,
            Err(Error::NotFound(_))
        ));
        let done_unanswered = p.wait_for_change("INBOX", minute).await;
        assert!(matches!(done_unanswered, Err(Error::Network(_))));
        assert!(
            p.idle.lock().await.is_none(),
            "a broken connection is dropped"
        );
        let idle_unanswered = p.wait_for_change("INBOX", minute).await;
        assert!(matches!(idle_unanswered, Err(Error::Network(_))));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_stops_answering_is_dropped_and_the_next_caller_reconnects() {
        let fake = FakeImap::new(vec![
            // Starts answering, then says nothing more, with the connection still open.
            script("", vec![step("{tag} LIST \"\" *", "* LIST () \"/\" \"INBOX\"\r\n")]),
            script("", vec![list(&[r#"* LIST () "/" "INBOX""#])]),
        ]);
        let p = provider(&fake);
        let started = tokio::time::Instant::now();
        // The second call waits for the connection the first one holds.
        let (stalled, queued) = tokio::join!(p.list_mailboxes(), p.list_mailboxes());
        assert!(matches!(&stalled, Err(Error::Network(m)) if m == "The server stopped answering."), "{stalled:?}");
        assert_eq!(started.elapsed(), COMMAND_STALL);
        assert_eq!(queued.unwrap().len(), 1, "the stuck connection was dropped, not reused");
        drop(p);
        fake.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn signing_in_to_a_silent_server_gives_up_sooner() {
        let fake = FakeImap::new(vec![vec![step(LOGIN, "")]]);
        let p = provider(&fake);
        let started = tokio::time::Instant::now();
        assert!(matches!(p.list_mailboxes().await, Err(Error::Network(_))));
        assert_eq!(started.elapsed(), SIGN_IN_STALL);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn idle_waits_past_the_stall_limit_but_its_commands_do_not() {
        let quiet = Duration::from_secs(29 * 60);
        let fake = FakeImap::new(vec![script(
            "IDLE",
            vec![
                inbox(),
                step("{tag} IDLE", "+ idling\r\n"),
                step("DONE", "{tag} OK IDLE terminated\r\n"),
                inbox(),
                step("{tag} IDLE", "+ idling\r\n"),
                step("DONE", ""),
            ],
        )]);
        let p = provider(&fake);
        let started = tokio::time::Instant::now();
        assert_eq!(p.wait_for_change("INBOX", quiet).await.unwrap(), Wakeup::Timeout);
        assert_eq!(started.elapsed(), quiet, "a quiet mailbox is not a stalled server");
        let started = tokio::time::Instant::now();
        assert!(matches!(p.wait_for_change("INBOX", quiet).await, Err(Error::Network(_))));
        assert_eq!(started.elapsed(), quiet + COMMAND_STALL, "an unanswered DONE is");
        assert!(p.idle.lock().await.is_none());
        drop(p);
        fake.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn without_idle_waiting_is_just_a_timeout() {
        let fake = FakeImap::new(vec![script("", vec![])]);
        let p = provider(&fake);
        let started = tokio::time::Instant::now();
        for _ in 0..2 {
            assert_eq!(
                p.wait_for_change("INBOX", Duration::from_secs(30))
                    .await
                    .unwrap(),
                Wakeup::Timeout
            );
        }
        assert!(started.elapsed() >= Duration::from_secs(60));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn reconnects_after_the_connection_drops_with_fresh_credentials() {
        let new_login = r#"{tag} LOGIN "ada@x.io" "new-pw""#;
        let fake = FakeImap::new(vec![
            script("", vec![step("{tag} SELECT \"INBOX\"", HANG_UP)]),
            vec![
                step(new_login, "{tag} OK\r\n"),
                step("{tag} CAPABILITY", "* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n"),
                inbox(),
                search("ALL", "1"),
            ],
        ]);
        let p = provider(&fake);
        assert!(matches!(
            p.message_ids("INBOX").await,
            Err(Error::Network(_))
        ));
        p.set_credentials(password("new-pw"), password("new-smtp"));
        assert_eq!(p.smtp.lock().unwrap().credentials, password("new-smtp"));
        assert_eq!(p.message_ids("INBOX").await.unwrap(), ["7:1"]);
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_refused_password_is_an_auth_error() {
        let fake = FakeImap::new(vec![vec![step(
            LOGIN,
            "{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n",
        )]]);
        let p = provider(&fake);
        assert!(matches!(p.list_mailboxes().await, Err(Error::Auth(_))));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn a_hang_up_while_signing_in_is_a_network_error() {
        let fake = FakeImap::new(vec![vec![step(LOGIN, HANG_UP)]]);
        let p = provider(&fake);
        assert!(matches!(p.list_mailboxes().await, Err(Error::Network(_))));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn signs_in_with_an_oauth_token() {
        let token = base64::engine::general_purpose::STANDARD
            .encode("user=ada@x.io\x01auth=Bearer ya29\x01\x01");
        let fake = FakeImap::new(vec![
            vec![
                step("{tag} AUTHENTICATE XOAUTH2", "+ \r\n"),
                step(&token, "{tag} OK\r\n"),
                step("{tag} CAPABILITY", "* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n"),
                list(&[r#"* LIST () "/" "INBOX""#]),
            ],
            vec![
                step("{tag} AUTHENTICATE XOAUTH2", "+ \r\n"),
                step(&token, "+ eyJzdGF0dXMiOiI0MDEifQ==\r\n"),
                step(
                    "",
                    "{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n",
                ),
            ],
        ]);
        let oauth = Credentials::OAuth2 {
            username: "ada@x.io".into(),
            access_token: Secret::from("ya29"),
        };
        for works in [true, false] {
            let p =
                ImapProvider::with_connector(fake.clone(), oauth.clone(), smtp_config(1, vec![]));
            let result = p.list_mailboxes().await;
            assert_eq!(result.is_ok(), works);
            if !works {
                assert!(matches!(result, Err(Error::Auth(_))));
            }
        }
        fake.finish().await;
    }

    #[tokio::test]
    async fn bad_input_fails_without_touching_the_server() {
        let fake = FakeImap::new(vec![script("", vec![])]);
        let p = provider(&fake);
        let resend = Checkpoint(CheckpointState::Resend(Default::default()));
        assert!(matches!(
            p.fetch_since("INBOX", Some(&resend), 10).await,
            Err(Error::InvalidCheckpoint)
        ));
        assert!(matches!(
            p.fetch_since("bad\nname", None, 10).await,
            Err(Error::Protocol(_))
        ));
        drop(p);
        fake.finish().await;
    }

    #[tokio::test]
    async fn no_server_at_all_is_a_network_error() {
        let fake = FakeImap::new(vec![]);
        assert!(matches!(
            provider(&fake).list_mailboxes().await,
            Err(Error::Network(_))
        ));
    }

    #[tokio::test]
    async fn the_public_constructor_connects_over_tls() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let tls = crate::testing::test_tls();
        let root = tls.root.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let stream = tls.acceptor.accept(tcp).await.unwrap();
            crate::testing::serve_imap(
                stream,
                Some("* OK hi\r\n"),
                script("", vec![list(&[r#"* LIST () "/" "INBOX""#])]),
            )
            .await
        });
        let config = ImapConfig {
            host: "localhost".into(),
            port,
            security: Security::Tls,
            credentials: password("pw"),
            tls_roots: TlsRoots::Custom(vec![root]),
        };
        let p = ImapProvider::new(config, smtp_config(1, vec![]));
        assert_eq!(
            p.list_mailboxes().await.unwrap()[0].role,
            Some(MailboxRole::Inbox)
        );
        drop(p);
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn server_diagnostics_never_echo_response_content() {
        let marker = "synthetic-private-message-marker";
        let malformed = format!("!! not IMAP {marker}\r\n");
        let refused = format!("{{tag}} NO {marker}\r\n");
        let mut leaks = Vec::new();
        for (case, command, reply, caps) in [
            ("search", "{tag} UID SEARCH ALL", malformed.as_str(), ""),
            ("fetch", "{tag} UID FETCH 5 (UID FLAGS INTERNALDATE BODY.PEEK[])", malformed.as_str(), ""),
            ("conversation malformed", "{tag} UID FETCH 5 (UID X-GM-THRID)", malformed.as_str(), "X-GM-EXT-1"),
            ("conversation refused", "{tag} UID FETCH 5 (UID X-GM-THRID)", refused.as_str(), "X-GM-EXT-1"),
        ] {
            let fake = FakeImap::new(vec![script(caps, vec![inbox(), step(command, reply)])]);
            let p = provider(&fake);
            let error = match case {
                "search" => p.message_ids("INBOX").await.unwrap_err(),
                "fetch" => p.fetch_message("INBOX", "7:5").await.unwrap_err(),
                _ => p.conversations_of("INBOX", &["7:5".into()]).await.unwrap_err(),
            };
            if format!("{error:?} {error}").contains(marker) { leaks.push(case); }
            drop(p); fake.finish().await;
        }
        let fake = FakeImap::new(vec![vec![step(LOGIN, &refused)]]);
        let p = provider(&fake);
        let error = p.list_mailboxes().await.unwrap_err();
        assert!(matches!(error, Error::Auth(_)));
        if format!("{error:?} {error}").contains(marker) { leaks.push("login"); }
        drop(p); fake.finish().await;
        assert!(leaks.is_empty(), "server response content exposed by {leaks:?}");
    }

    #[test]
    fn imap_errors_map_to_safe_provider_errors() {
        use async_imap::error::{Error as E, ParseError};
        let marker = "synthetic-private-parser-marker";
        for e in [
            E::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, marker)),
            E::ConnectionLost,
            E::Parse(ParseError::Unexpected(marker.into())),
            E::Parse(ParseError::Authentication(marker.into(), None)),
        ] {
            let error = imap_error(e);
            assert!(matches!(error, Error::Network(_)));
            assert!(!format!("{error:?} {error}").contains(marker));
        }
        for e in [E::Bad(marker.into()), E::No(marker.into()), E::Append, E::Validate(async_imap::error::ValidateError('x'))] {
            let error = imap_error(e);
            assert!(matches!(error, Error::Protocol(_)));
            assert!(!format!("{error:?} {error}").contains(marker));
        }
    }
}
