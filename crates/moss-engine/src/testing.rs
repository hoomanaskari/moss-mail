//! Fakes for the engine's tests: a mail server that lives in memory, a connector that hands
//! it out, and an event sink that records what the UI would hear.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use moss_accounts::{AccountConnection, BoxFuture, MxLookup};
use moss_providers::{
    Address, Capabilities, Change, Checkpoint, ConversationMessage, Error, FlagUpdate, Flags,
    Mailbox, MailboxRole, OutgoingMessage, Provider, RawMessage, Result, SavedDraft, SentMessage,
    SyncBatch, Wakeup,
};
use tokio::sync::Notify;

use crate::connect::{Connector, Credential};
use crate::events::{Event, Events};

/// The fake server's Drafts folder, once [`FakeProvider::with_drafts`].
pub const DRAFTS: &str = "[Gmail]/Drafts";

/// A raw email from Daniel, `id` is its Message-ID.
pub fn raw(id: &str, subject: &str, date_ms: i64) -> Vec<u8> {
    raw_from(
        id,
        "Daniel Okafor <daniel@northwind.co>",
        "Maya Ross <maya.ross@gmail.com>",
        subject,
        date_ms,
    )
}

pub fn raw_from(id: &str, from: &str, to: &str, subject: &str, date_ms: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(date_ms).unwrap().to_rfc2822();
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nHello about {subject}\r\n"
    )
    .into_bytes()
}

/// `raw_from` with a PDF attached and a small logo shown in the body.
pub fn with_files(id: &str, subject: &str, date_ms: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(date_ms).unwrap().to_rfc2822();
    format!(
        "From: Daniel Okafor <daniel@northwind.co>\r\nTo: Maya Ross <maya.ross@gmail.com>\r\n\
         Subject: {subject}\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n\
         Content-Type: text/html\r\n\r\n<p>Signed copy attached</p><img src=\"cid:logo\">\r\n--m\r\n\
         Content-Type: image/png\r\nContent-ID: <logo>\r\nContent-Disposition: inline\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nUE5H\r\n--m\r\n\
         Content-Type: application/pdf\r\nContent-Disposition: attachment; filename=Q4.pdf\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nUERGREFUQQ==\r\n--m--\r\n"
    )
    .into_bytes()
}

/// `raw` as a reply in a conversation: `chain` is its Message-IDs, oldest first.
pub fn reply_to(chain: &[&str], raw: Vec<u8>) -> Vec<u8> {
    let refs: Vec<String> = chain.iter().map(|id| format!("<{id}>")).collect();
    let parent = refs.last().expect("a reply has a parent");
    let mut out =
        format!("In-Reply-To: {parent}\r\nReferences: {}\r\n", refs.join(" ")).into_bytes();
    out.extend(raw);
    out
}

pub fn message(mailbox: &str, uid: u32, raw: Vec<u8>, date_ms: i64) -> RawMessage {
    RawMessage {
        provider_id: format!("1:{uid}"),
        mailbox: mailbox.into(),
        flags: Flags::default(),
        received_at: chrono::DateTime::from_timestamp_millis(date_ms),
        raw,
        labels: vec![],
        gmail_msg_id: None,
        headers_only: false,
    }
}

fn checkpoint_at(position: usize) -> Checkpoint {
    serde_json::from_value(serde_json::json!({
        "provider": "imap", "uid_validity": 1, "uid_next": position, "highest_modseq": null, "backfill_below": 1
    }))
    .unwrap()
}

fn position(checkpoint: Option<&Checkpoint>) -> usize {
    checkpoint
        .map_or(0, |c| serde_json::to_value(c).unwrap()["uid_next"].as_u64().unwrap() as usize)
}

#[derive(Default)]
struct State {
    mailboxes: Vec<Mailbox>,
    /// Per mailbox, in the order the server hands them out (newest first).
    mail: HashMap<String, Vec<RawMessage>>,
    failures: VecDeque<Error>,
    /// Fetches still to panic, like a bug in the worker would.
    panics: usize,
    /// Sends still to panic once the server has the email.
    send_panics: usize,
    wait_failures: VecDeque<Error>,
    draft_delete_failures: VecDeque<Error>,
    flag_updates: Vec<FlagUpdate>,
    vanished: Vec<String>,
    reset: bool,
    all_ids: HashMap<String, Vec<String>>,
    applied: Vec<(String, Vec<String>, Change)>,
    /// Every change, unarchive, trash and untrash, in the order they reached the server.
    changes: Vec<String>,
    apply_delay: Option<Duration>,
    applies_started: usize,
    send_delay: Option<Duration>,
    fetch_delay: Option<Duration>,
    omitted_headers: Vec<String>,
    sent: Vec<OutgoingMessage>,
    fetches: usize,
    /// Gmail conversation of each message, by mailbox and provider id.
    conversations: HashMap<(String, String), u64>,
    not_gmail: bool,
    /// Like Resend: no folders or flags on the server.
    local_only: bool,
    /// Full messages handed out, to catch downloading one twice.
    downloads: usize,
    /// What sync asked for, in order: "fetch INBOX more", "conversations_of INBOX 3"…
    calls: Vec<String>,
    /// Drafts saved so far, for their Message-IDs and UIDs.
    drafts_saved: u32,
    /// The Date drafts are saved with.
    draft_date: i64,
    /// `save_draft` doesn't say where the new copy is, like a server that can't find it.
    draft_ids_unknown: bool,
    /// Has a Drafts folder but keeps no drafts in it, and can't list it.
    drafts_unsupported: bool,
}

/// The Message-ID (without <>) in a raw message.
pub fn message_id_of(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let line = text.lines().find(|l| l.to_ascii_lowercase().starts_with("message-id:"))?;
    Some(line[11..].trim().trim_start_matches('<').trim_end_matches('>').to_string())
}

fn addresses(list: &[Address]) -> String {
    let one = |a: &Address| match &a.name {
        Some(name) => format!("{name} <{}>", a.email),
        None => a.email.clone(),
    };
    list.iter().map(one).collect::<Vec<_>>().join(", ")
}

/// A draft as the fake server keeps it.
fn draft_raw(m: &OutgoingMessage, id: &str, date_ms: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(date_ms).unwrap().to_rfc2822();
    let mut out = format!("From: {}\r\n", addresses(std::slice::from_ref(&m.from)));
    for (name, list) in [("To", &m.to), ("Cc", &m.cc), ("Bcc", &m.bcc)] {
        if !list.is_empty() {
            out += &format!("{name}: {}\r\n", addresses(list));
        }
    }
    out += &format!("Subject: {}\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n", m.subject);
    if let Some(parent) = &m.in_reply_to {
        out += &format!("In-Reply-To: <{parent}>\r\n");
    }
    if !m.references.is_empty() {
        let refs: Vec<String> = m.references.iter().map(|r| format!("<{r}>")).collect();
        out += &format!("References: {}\r\n", refs.join(" "));
    }
    for (name, value) in &m.headers {
        out += &format!("{name}: {value}\r\n");
    }
    let text = m.text.clone().unwrap_or_default();
    if m.attachments.is_empty() {
        out += &format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n");
        return out.into_bytes();
    }
    out += &format!("Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n");
    // Test files are plain text, so they go as they are.
    for a in &m.attachments {
        out += &format!(
            "--m\r\nContent-Type: {}\r\nContent-Disposition: attachment; filename=\"{}\"\r\n\r\n{}\r\n",
            a.content_type,
            a.filename,
            String::from_utf8_lossy(&a.data)
        );
    }
    out += "--m--\r\n";
    out.into_bytes()
}

/// A mail server in memory. Hands out mail in the order it was added.
pub struct FakeProvider {
    state: Mutex<State>,
    changed: Notify,
}

impl FakeProvider {
    pub fn new() -> Arc<Self> {
        let mailbox = |id: &str, role| Mailbox {
            id: id.into(),
            name: id.into(),
            delimiter: Some("/".into()),
            role,
            selectable: true,
        };
        let state = State {
            mailboxes: vec![
                mailbox("INBOX", Some(MailboxRole::Inbox)),
                mailbox("[Gmail]/Sent Mail", Some(MailboxRole::Sent)),
                mailbox("[Gmail]/All Mail", Some(MailboxRole::All)),
                mailbox("[Gmail]/Starred", Some(MailboxRole::Flagged)),
            ],
            ..Default::default()
        };
        Arc::new(FakeProvider { state: Mutex::new(state), changed: Notify::new() })
    }

    /// The server has a Drafts folder, as every IMAP account does.
    pub fn with_drafts(&self) {
        let drafts = Mailbox {
            id: DRAFTS.into(),
            name: "Drafts".into(),
            delimiter: Some("/".into()),
            role: Some(MailboxRole::Drafts),
            selectable: true,
        };
        let mut s = self.state.lock().unwrap();
        s.mailboxes.push(drafts);
        s.draft_date = 1_793_524_320_000;
    }

    /// Drafts on the server, oldest first.
    pub fn drafts(&self) -> Vec<RawMessage> {
        self.state.lock().unwrap().mail.get(DRAFTS).cloned().unwrap_or_default()
    }

    /// The text of each draft on the server, oldest first.
    pub fn draft_texts(&self) -> Vec<String> {
        self.drafts().iter().map(|m| String::from_utf8_lossy(&m.raw).into_owned()).collect()
    }

    /// `save_draft` stops saying where the new copy is.
    pub fn hide_draft_ids(&self) {
        self.state.lock().unwrap().draft_ids_unknown = true;
    }

    /// A partial successful FETCH response, without removing the server message.
    pub fn omit_headers(&self, ids: Vec<String>) {
        self.state.lock().unwrap().omitted_headers=ids;
    }

    /// Lists a Drafts folder but keeps nothing there, like a provider without drafts.
    pub fn drafts_unsupported(&self) {
        self.state.lock().unwrap().drafts_unsupported = true;
    }

    pub fn add(&self, message: RawMessage) {
        let mut s = self.state.lock().unwrap();
        s.mail.entry(message.mailbox.clone()).or_default().push(message);
    }

    /// New mail arrives and IDLE wakes up.
    pub fn deliver(&self, message: RawMessage) {
        self.add(message);
        self.changed.notify_one();
    }

    /// The message is no longer on the server.
    pub fn forget(&self, mailbox: &str, provider_id: &str) {
        let mut s = self.state.lock().unwrap();
        s.mail.entry(mailbox.into()).or_default().retain(|m| m.provider_id != provider_id);
    }

    /// The next `count` sends panic after the server took the email.
    pub fn panic_after_send(&self, count: usize) {
        self.state.lock().unwrap().send_panics = count;
    }

    /// The next `count` fetches panic.
    pub fn panic_next(&self, count: usize) {
        self.state.lock().unwrap().panics = count;
    }

    pub fn panics_left(&self) -> usize {
        self.state.lock().unwrap().panics
    }

    pub fn fail_next(&self, error: Error) {
        self.state.lock().unwrap().failures.push_back(error);
    }

    /// Fail only a nonempty draft deletion, after an upload can have succeeded.
    pub fn fail_next_draft_delete(&self, error: Error) {
        self.state.lock().unwrap().draft_delete_failures.push_back(error);
    }

    /// Only the next IDLE fails, like a dropped IDLE connection.
    pub fn fail_next_wait(&self, error: Error) {
        self.state.lock().unwrap().wait_failures.push_back(error);
    }

    pub fn update_flags(&self, update: FlagUpdate) {
        self.state.lock().unwrap().flag_updates.push(update);
    }

    pub fn vanish(&self, provider_id: &str) {
        self.state.lock().unwrap().vanished.push(provider_id.into());
    }

    pub fn reset_next(&self) {
        self.state.lock().unwrap().reset = true;
    }

    pub fn set_all_ids(&self, mailbox: &str, ids: &[&str]) {
        let ids = ids.iter().map(|s| s.to_string()).collect();
        self.state.lock().unwrap().all_ids.insert(mailbox.into(), ids);
    }

    /// Behave like a server that keeps no folders or flags (Resend).
    pub fn local_only(&self) {
        self.state.lock().unwrap().local_only = true;
    }

    /// A folder the server has already, such as the Blocked folder Spark made.
    pub fn add_mailbox(&self, name: &str) {
        let mailbox = Mailbox {
            id: name.into(),
            name: name.into(),
            delimiter: Some("/".into()),
            role: None,
            selectable: true,
        };
        self.state.lock().unwrap().mailboxes.push(mailbox);
    }

    pub fn without_mailboxes(&self) {
        self.state.lock().unwrap().mailboxes.clear();
    }

    pub fn applied(&self) -> Vec<(String, Vec<String>, Change)> {
        self.state.lock().unwrap().applied.clone()
    }

    /// e.g. "Archive INBOX 1:1", "unarchive a@x" or "trash a@x", oldest first.
    pub fn changes(&self) -> Vec<String> {
        self.state.lock().unwrap().changes.clone()
    }

    /// Every later change takes this long to reach the server.
    pub fn slow(&self, delay: Duration) {
        self.state.lock().unwrap().apply_delay = Some(delay);
    }

    pub fn applies_started(&self) -> usize {
        self.state.lock().unwrap().applies_started
    }

    /// Every later send takes this long.
    pub fn delay_sends(&self, delay: Duration) {
        self.state.lock().unwrap().send_delay = Some(delay);
    }

    /// Delay a message download so preparation races can be exercised without a live server.
    pub fn delay_fetches(&self, delay: Duration) {
        self.state.lock().unwrap().fetch_delay = Some(delay);
    }

    pub fn sent(&self) -> Vec<OutgoingMessage> {
        self.state.lock().unwrap().sent.clone()
    }

    pub fn fetches(&self) -> usize {
        self.state.lock().unwrap().fetches
    }

    /// Puts a message already added to `mailbox` in a Gmail conversation.
    pub fn in_conversation(&self, mailbox: &str, uid: u32, conversation: u64) {
        let key = (mailbox.to_string(), format!("1:{uid}"));
        self.state.lock().unwrap().conversations.insert(key, conversation);
    }

    /// Behave like an IMAP server that isn't Gmail, or like Gmail again.
    pub fn set_gmail(&self, gmail: bool) {
        self.state.lock().unwrap().not_gmail = !gmail;
    }

    pub fn downloads(&self) -> usize {
        self.state.lock().unwrap().downloads
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    fn failure(&self) -> Result<()> {
        match self.state.lock().unwrap().failures.pop_front() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities { server_flags: !self.state.lock().unwrap().local_only }
    }

    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
        self.failure()?;
        Ok(self.state.lock().unwrap().mailboxes.clone())
    }

    async fn fetch_since(
        &self,
        mailbox: &str,
        checkpoint: Option<&Checkpoint>,
        limit: usize,
    ) -> Result<SyncBatch> {
        let panics = {
            let mut s = self.state.lock().unwrap();
            let panics = s.panics > 0;
            s.panics = s.panics.saturating_sub(1);
            panics
        };
        assert!(!panics, "a bug in the sync worker");
        self.failure()?;
        let mut s = self.state.lock().unwrap();
        s.fetches += 1;
        let reset = std::mem::take(&mut s.reset);
        let start = if reset { 0 } else { position(checkpoint) };
        let all = s.mail.get(mailbox).cloned().unwrap_or_default();
        let end = (start + limit).min(all.len());
        let inbox = mailbox == "INBOX";
        s.downloads += end - start.min(end);
        let more = end < all.len();
        s.calls.push(format!("fetch {mailbox} {}", if more { "more" } else { "done" }));
        Ok(SyncBatch {
            reset,
            messages: all[start.min(end)..end].to_vec(),
            flag_updates: if inbox { std::mem::take(&mut s.flag_updates) } else { vec![] },
            vanished: if inbox { std::mem::take(&mut s.vanished) } else { vec![] },
            checkpoint: checkpoint_at(end),
            more,
        })
    }

    /// Older mail as headers only. (A real server sends new mail whole; this one doesn't
    /// tell them apart.)
    async fn fetch_since_shallow(
        &self,
        mailbox: &str,
        checkpoint: Option<&Checkpoint>,
        limit: usize,
    ) -> Result<SyncBatch> {
        let mut batch = self.fetch_since(mailbox, checkpoint, limit).await?;
        for m in &mut batch.messages {
            let end = m.raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(m.raw.len(), |i| i + 4);
            m.raw.truncate(end);
            m.headers_only = true;
        }
        self.state.lock().unwrap().calls.push(format!("headers {mailbox}"));
        Ok(batch)
    }

    async fn fetch_headers(&self, mailbox: &str, provider_ids: &[String]) -> Result<Vec<RawMessage>> {
        let delay=self.state.lock().unwrap().fetch_delay;
        if let Some(delay)=delay {tokio::time::sleep(delay).await;}
        let mut messages=self.fetch_messages(mailbox,provider_ids).await?;
        let state=self.state.lock().unwrap();
        messages.retain(|message|!state.omitted_headers.contains(&message.provider_id));
        for message in &mut messages {
            let end=message.raw.windows(4).position(|window|window==b"\r\n\r\n").map_or(message.raw.len(),|i|i+4);
            message.raw.truncate(end);
            message.headers_only=true;
        }
        Ok(messages)
    }

    async fn fetch_message(&self, mailbox: &str, provider_id: &str) -> Result<RawMessage> {
        let delay = self.state.lock().unwrap().fetch_delay;
        if let Some(delay) = delay { tokio::time::sleep(delay).await; }
        self.failure()?;
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("fetch_message {mailbox} {provider_id}"));
        let found = s.mail.get(mailbox).into_iter().flatten().find(|m| m.provider_id == provider_id);
        let found = found.cloned().ok_or_else(|| Error::NotFound(provider_id.into()))?;
        s.downloads += 1;
        Ok(found)
    }

    async fn apply(&self, mailbox: &str, provider_ids: &[String], change: &Change) -> Result<()> {
        let delay = {
            let mut state = self.state.lock().unwrap();
            state.applies_started += 1;
            state.apply_delay
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        self.failure()?;
        let mut s = self.state.lock().unwrap();
        s.changes.push(format!("{change:?} {mailbox} {}", provider_ids.join(",")));
        s.applied.push((mailbox.into(), provider_ids.to_vec(), change.clone()));
        Ok(())
    }

    async fn unarchive(&self, message_ids: &[String]) -> Result<()> {
        self.failure()?;
        self.state.lock().unwrap().changes.push(format!("unarchive {}", message_ids.join(",")));
        Ok(())
    }

    async fn trash(&self, message_ids: &[String]) -> Result<()> {
        self.failure()?;
        self.state.lock().unwrap().changes.push(format!("trash {}", message_ids.join(",")));
        Ok(())
    }

    async fn untrash(&self, message_ids: &[String], to_inbox: bool) -> Result<()> {
        self.failure()?;
        let to = if to_inbox { "inbox" } else { "archive" };
        self.state.lock().unwrap().changes.push(format!("untrash {} to {to}", message_ids.join(",")));
        Ok(())
    }

    async fn create_mailbox(&self, name: &str) -> Result<()> {
        self.failure()?;
        self.state.lock().unwrap().changes.push(format!("create {name}"));
        self.add_mailbox(name);
        Ok(())
    }

    async fn send(&self, message: &OutgoingMessage) -> Result<SentMessage> {
        let delay = self.state.lock().unwrap().send_delay;
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        self.failure()?;
        let mut s = self.state.lock().unwrap();
        s.sent.push(message.clone());
        let id = message.message_id.as_deref().map(|id| id.trim_matches(['<', '>']).to_owned())
            .unwrap_or_else(|| format!("sent-{}@moss.test", s.sent.len()));
        let mut headers = format!(
            "From: {} <{}>\r\nTo: {}\r\nSubject: {}\r\nMessage-ID: <{id}>\r\nDate: Mon, 2 Nov 2026 09:00:00 +0000\r\n",
            message.from.name.clone().unwrap_or_default(),
            message.from.email,
            message.to.first().map(|a| a.email.clone()).unwrap_or_default(),
            message.subject,
        );
        if let Some(parent) = &message.in_reply_to {
            headers.push_str(&format!("In-Reply-To: <{parent}>\r\n"));
        }
        let raw = format!("{headers}\r\n{}\r\n", message.text.clone().unwrap_or_default());
        let panics = s.send_panics > 0;
        s.send_panics = s.send_panics.saturating_sub(1);
        drop(s);
        assert!(!panics, "a bug after the server took the email");
        Ok(SentMessage {
            provider_id: None,
            message_id: format!("<{id}>"),
            raw: raw.into_bytes(),
            saved_on_server: false,
        })
    }

    async fn save_draft(
        &self,
        mailbox: &str,
        draft: &OutgoingMessage,
    ) -> Result<Option<SavedDraft>> {
        let delay = self.state.lock().unwrap().apply_delay;
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let keeps = { let s = self.state.lock().unwrap(); !s.local_only && !s.drafts_unsupported };
        if !keeps {
            return Ok(None);
        }
        self.failure()?;
        let (id, uid, raw, known) = {
            let mut s = self.state.lock().unwrap();
            s.drafts_saved += 1;
            let n = s.drafts_saved;
            let id = format!("draft-{n}@moss.test");
            (id.clone(), 100 + n, draft_raw(draft, &id, s.draft_date), !s.draft_ids_unknown)
        };
        self.add(message(mailbox, uid, raw, 0));
        let mut s = self.state.lock().unwrap();
        s.changes.push(format!("save_draft {mailbox}"));
        Ok(Some(SavedDraft { message_id: id, provider_id: known.then(|| format!("1:{uid}")) }))
    }

    async fn delete_drafts(&self, mailbox: &str, message_ids: &[String]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        self.failure()?;
        if let Some(error) = self.state.lock().unwrap().draft_delete_failures.pop_front() {
            return Err(error);
        }
        let mut s = self.state.lock().unwrap();
        let gone = |m: &RawMessage| message_id_of(&m.raw).is_some_and(|id| message_ids.contains(&id));
        s.mail.entry(mailbox.into()).or_default().retain(|m| !gone(m));
        s.changes.push(format!("delete_drafts {mailbox} {}", message_ids.join(",")));
        Ok(())
    }

    async fn wait_for_change(&self, _mailbox: &str, timeout: Duration) -> Result<Wakeup> {
        self.failure()?;
        if let Some(e) = self.state.lock().unwrap().wait_failures.pop_front() {
            return Err(e);
        }
        tokio::select! {
            _ = self.changed.notified() => Ok(Wakeup::Changed),
            _ = tokio::time::sleep(timeout) => Ok(Wakeup::Timeout),
        }
    }

    async fn all_ids(&self, mailbox: &str) -> Result<Option<Vec<String>>> {
        let s = self.state.lock().unwrap();
        if s.local_only || (mailbox == DRAFTS && s.drafts_unsupported) {
            return Ok(None);
        }
        if let Some(ids) = s.all_ids.get(mailbox) {
            return Ok(Some(ids.clone()));
        }
        if mailbox == DRAFTS {
            let ids = s.mail.get(mailbox).into_iter().flatten().map(|m| m.provider_id.clone());
            return Ok(Some(ids.collect()));
        }
        Ok(None)
    }

    async fn fetch_messages(
        &self,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<Vec<RawMessage>> {
        self.failure()?;
        let mut s = self.state.lock().unwrap();
        let found: Vec<RawMessage> = s
            .mail
            .get(mailbox)
            .into_iter()
            .flatten()
            .filter(|m| provider_ids.contains(&m.provider_id))
            .cloned()
            .collect();
        s.downloads += found.len();
        s.calls.push(format!("fetch_messages {mailbox} {}", provider_ids.len()));
        Ok(found)
    }

    async fn conversations_of(
        &self,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<Option<Vec<(String, u64)>>> {
        self.failure()?;
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("conversations_of {mailbox} {}", provider_ids.len()));
        if s.not_gmail {
            return Ok(None);
        }
        let found = provider_ids
            .iter()
            .filter_map(|id| {
                let key = (mailbox.to_string(), id.clone());
                s.conversations.get(&key).map(|c| (id.clone(), *c))
            })
            .collect();
        Ok(Some(found))
    }

    async fn conversation_messages(
        &self,
        mailbox: &str,
        conversations: &[u64],
    ) -> Result<Vec<ConversationMessage>> {
        self.failure()?;
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("conversation_messages {mailbox} {}", conversations.len()));
        let members = s
            .mail
            .get(mailbox)
            .into_iter()
            .flatten()
            .filter(|m| {
                let key = (mailbox.to_string(), m.provider_id.clone());
                s.conversations.get(&key).is_some_and(|c| conversations.contains(c))
            })
            .map(|m| ConversationMessage {
                provider_id: m.provider_id.clone(),
                flags: m.flags.clone(),
                labels: m.labels.clone(),
                headers: m.raw.clone(),
            })
            .collect();
        Ok(members)
    }
}

/// Hands out the same fake server for every account.
pub struct FakeConnector {
    provider: Arc<FakeProvider>,
    last: Mutex<Option<Credential>>,
    refuse: Mutex<bool>,
    built: Mutex<usize>,
}

impl FakeConnector {
    pub fn new(provider: Arc<FakeProvider>) -> Self {
        FakeConnector {
            provider,
            last: Mutex::new(None),
            refuse: Mutex::new(false),
            built: Mutex::new(0),
        }
    }

    pub fn last_credential(&self) -> Option<Credential> {
        self.last.lock().unwrap().clone()
    }

    /// Every later build fails, like a connection with broken settings.
    pub fn refuse(&self) {
        *self.refuse.lock().unwrap() = true;
    }

    pub fn built(&self) -> usize {
        *self.built.lock().unwrap()
    }
}

impl Connector for FakeConnector {
    fn provider(
        &self,
        _connection: &AccountConnection,
        credential: &Credential,
    ) -> std::result::Result<Arc<dyn Provider>, crate::Error> {
        *self.last.lock().unwrap() = Some(credential.clone());
        if *self.refuse.lock().unwrap() {
            return Err(crate::Error::Server("broken settings".into()));
        }
        *self.built.lock().unwrap() += 1;
        Ok(self.provider.clone())
    }
}

/// Keeps every event, for the tests to look through.
#[derive(Default)]
pub struct RecordedEvents(Mutex<Vec<Event>>);

impl RecordedEvents {
    pub fn all(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }

    /// Waits (in paused test time) until an event matches.
    pub async fn wait_for(&self, matches: impl Fn(&Event) -> bool) -> Event {
        let mut found = None;
        for _ in 0..10_000 {
            found = self.all().into_iter().find(&matches);
            if found.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        found.unwrap_or_else(|| panic!("no matching event in {:?}", self.all()))
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

impl Events for RecordedEvents {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

/// MX answers from a table of domain → mail host, keeping the domains asked about.
#[derive(Default)]
pub struct FakeMx {
    hosts: HashMap<String, String>,
    asked: Mutex<Vec<String>>,
}

impl FakeMx {
    pub fn new(hosts: &[(&str, &str)]) -> Arc<Self> {
        let hosts = hosts.iter().map(|(d, h)| (d.to_string(), h.to_string())).collect();
        Arc::new(FakeMx { hosts, asked: Mutex::default() })
    }

    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl MxLookup for FakeMx {
    fn mail_hosts<'a>(&'a self, domain: &'a str) -> BoxFuture<'a, Vec<String>> {
        self.asked.lock().unwrap().push(domain.into());
        Box::pin(async move { self.hosts.get(domain).cloned().into_iter().collect() })
    }
}
