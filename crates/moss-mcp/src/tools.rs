//! The tools an agent sees: read the mail, manage it (done, remind, move, read, delete,
//! block), write drafts and send. Each one is a call into [`Engine`], the same ones the
//! window makes, so the window shows what an agent did.
//!
//! Results are compact JSON text, without attachments' bytes (only their names), so an agent
//! reads a lot of mail in a few tokens.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use moss_core::{
    ComposeMode, Draft, DraftFile, EmailKind, FileRef, Message, Person, Signature, SignatureEmail,
    SplitId, Thread, ThreadSummary,
};
use moss_engine::AccountInfo;
use moss_engine::{Engine, Event, Events, OutgoingDraft};
use moss_store::{DeliveryReceipt, ScheduleOutcome};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;
use serde_json::{json, Value};

mod chat;
pub use chat::{Ran, CHAT_LEAVES_OUT};

/// At most this many threads in a list, whatever an agent asks for.
pub const MOST: usize = 100;
/// Threads in a list when an agent doesn't say.
pub const SOME: usize = 25;
/// The longest an email's text gets in a result, in characters.
pub const LONGEST_TEXT: usize = 20_000;

/// What the server tells an agent about itself when it connects.
pub const INSTRUCTIONS: &str = "Moss is the user's email app. These tools read, manage and send their mail. \
Thread ids come from list_inbox and search_mail; account ids from list_accounts; times are RFC 3339. \
Moss adds the user's signature, as it does in the app, so don't sign emails off with their details. \
Only call send when the user clearly asked to send that email. If they asked for a draft, use create_draft. \
If it isn't clear, ask the user before sending.";

/// What sending goes through: the engine, in the app. Tests pass their own.
#[async_trait]
pub trait Outbox: Send + Sync {
    /// Sends now. Returns the thread it's in.
    async fn send(&self, draft: &Draft, email: OutgoingDraft) -> moss_engine::Result<String>;
    /// Send later, as schedule `operation`: `email` goes at `at` (ms), and `draft` leaves
    /// Drafts. The same operation again answers what became of it, and schedules nothing.
    fn schedule(&self, operation: &str, draft: &Draft, email: &OutgoingDraft, at: i64) -> moss_engine::Result<ScheduleOutcome>;
}

/// Sends with the engine, as the window does.
pub struct EngineOutbox(pub Engine);

#[async_trait]
impl Outbox for EngineOutbox {
    async fn send(&self, draft: &Draft, email: OutgoingDraft) -> moss_engine::Result<String> {
        self.0.send_saved(draft, &email).await
    }
    fn schedule(&self, operation: &str, draft: &Draft, email: &OutgoingDraft, at: i64) -> moss_engine::Result<ScheduleOutcome> {
        self.0.schedule_with_operation(operation, draft, email, at)
    }
}

/// Moss's tools over one engine. One per connection; they share the engine.
#[derive(Clone)]
pub struct MossTools {
    engine: Engine,
    events: Arc<dyn Events>,
    outbox: Arc<dyn Outbox>,
    clock: fn() -> i64,
    /// Where files from mail are saved for an agent to open.
    files: PathBuf,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct InboxArgs {
    /// Which split: important, other, calendar, newsletters or receipts. All of them when left out.
    pub split: Option<String>,
    /// One account's id, from list_accounts. Every account when left out.
    pub account: Option<String>,
    /// How many threads, newest first. 25 when left out, at most 100.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ThreadArgs {
    /// The thread's id, from list_inbox or search_mail.
    pub thread_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// Words to find, or what the email is about: Moss also finds mail by meaning when its search model is on this Mac.
    pub query: String,
    /// One account's id, from list_accounts. Every account when left out.
    pub account: Option<String>,
    /// How many threads. 25 when left out, at most 100.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DoneArgs {
    /// The thread's id.
    pub thread_id: String,
    /// True (the default) marks it done: out of the inbox, archived on the server. False brings it back.
    pub done: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    /// The thread's id.
    pub thread_id: String,
    /// True (the default) marks it read; false, unread.
    pub read: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RemindArgs {
    /// The thread's id.
    pub thread_id: String,
    /// When it comes back to the top of the inbox, as an RFC 3339 time (2026-11-09T09:00:00+01:00). Leave out to stop reminding.
    pub until: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NoReplyArgs {
    /// The thread's id. The user's email must be the latest in it.
    pub thread_id: String,
    /// True (the default) brings it back if no one answers in a few days; false stops waiting.
    pub on: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MoveArgs {
    /// The thread's id.
    pub thread_id: String,
    /// important, other, calendar, newsletters or receipts, for good. "auto" gives it back to Moss's sorting.
    pub to: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SenderArgs {
    /// A thread from the sender.
    pub thread_id: String,
    /// True for everyone at the sender's domain, not only this address. Public mail services (gmail.com…) can't be blocked as a whole.
    pub whole_domain: Option<bool>,
}

/// Bytes the external agent already holds. Moss never reads an agent-supplied path.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppliedAttachment {
    pub name: String,
    pub content_base64: String,
}

/// Keep inline tool payloads small. Larger files can be staged in Moss's composer.
pub(crate) const MAX_SUPPLIED_BYTES: usize = 1024 * 1024;

fn supplied(files: &[SuppliedAttachment]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let limit = "Supply at most 1 MB in 100 attachments, or attach the files in Moss and use draft_id.";
    if files.len() > 100 { return Err(limit.into()); }
    let mut total = 0usize;
    let mut decoded = Vec::with_capacity(files.len());
    for file in files {
        // Check encoded length before decoding, including room for base64 padding.
        if file.content_base64.len() > (MAX_SUPPLIED_BYTES - total).div_ceil(3) * 4 { return Err(limit.into()); }
        let bytes = base64::engine::general_purpose::STANDARD.decode(&file.content_base64).map_err(|_| "Attachment contents must be valid base64.".to_string())?;
        total += bytes.len();
        if total > MAX_SUPPLIED_BYTES { return Err(limit.into()); }
        decoded.push((file.name.clone(), bytes));
    }
    Ok(decoded)
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct CreateArgs {
    /// For a reply: the thread it answers. It goes to the person who wrote, with "Re:" and the thread's subject.
    pub reply_to_thread: Option<String>,
    /// With reply_to_thread: reply to everyone on its latest email, not only who wrote.
    pub reply_all: Option<bool>,
    /// For a forward: the thread whose latest email it forwards, with its files. Say who it goes to.
    pub forward_thread: Option<String>,
    /// The account it goes from, by id from list_accounts. For a reply or forward, the thread's account; otherwise the one the user writes that person from.
    pub account: Option<String>,
    /// Who it goes to: "name@example.com", "Name <name@example.com>", or just their name, which Moss looks up in the user's mail. Never make up an address.
    pub to: Option<Vec<String>>,
    /// Who gets a copy, written the same way as to.
    pub cc: Option<Vec<String>>,
    /// Who gets a hidden copy, written the same way as to.
    pub bcc: Option<Vec<String>>,
    /// The user's own address it goes from (an account's, or one in its alsoSendsFrom), never the address of who it goes to. Best left out: a reply goes from the address the email came to, a new email from the one the user writes that person from.
    pub from: Option<String>,
    /// The signature, by name, or "none". Left out, Moss picks the user's own as it does in the app.
    pub signature: Option<String>,
    /// Legacy paths are rejected. Attach files in Moss and use draft_id instead.
    #[schemars(skip)]
    pub files: Option<Vec<String>>,
    /// File contents already held by the external agent, at most 1 MB total. Never local paths.
    pub attachments: Option<Vec<SuppliedAttachment>>,
    /// The subject. A reply has "Re:" and the thread's subject when left out, a forward "Fwd:".
    pub subject: Option<String>,
    /// The email's text, in plain text. Blank lines start new paragraphs. No signature: Moss adds the user's.
    pub body: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct UpdateArgs {
    /// The revision returned by list_drafts or the last successful draft edit.
    pub revision: String,
    /// The draft's id, from list_drafts or create_draft.
    pub draft_id: String,
    /// Another account to send it from, by id.
    pub account: Option<String>,
    /// Who it goes to, replacing who it went to.
    pub to: Option<Vec<String>>,
    /// Who gets a copy, replacing who did.
    pub cc: Option<Vec<String>>,
    /// Who gets a hidden copy, replacing who did.
    pub bcc: Option<Vec<String>>,
    /// Which of the account's addresses it goes from.
    pub from: Option<String>,
    /// The signature, by name, or "none".
    pub signature: Option<String>,
    /// Legacy paths are rejected. Attach files in Moss and use draft_id instead.
    #[schemars(skip)]
    pub files: Option<Vec<String>>,
    /// More file contents already held by the external agent, at most 1 MB total.
    pub attachments: Option<Vec<SuppliedAttachment>>,
    /// A new subject.
    pub subject: Option<String>,
    /// New text, in plain text, replacing the old.
    pub body: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct SendArgs {
    /// Required with draft_id: the revision you observed before deciding to send.
    pub revision: Option<String>,
    /// Sends this draft, from list_drafts or create_draft, with any changes below. Leave out to send a new email, reply or forward.
    pub draft_id: Option<String>,
    /// For a reply: the thread it answers. It goes to the person who wrote, with "Re:" and the thread's subject.
    pub reply_to_thread: Option<String>,
    /// With reply_to_thread: reply to everyone on its latest email, not only who wrote.
    pub reply_all: Option<bool>,
    /// For a forward: the thread whose latest email it forwards, with its files. Say who it goes to.
    pub forward_thread: Option<String>,
    /// The account it goes from, by id from list_accounts. For a reply or forward, the thread's account; otherwise the one the user writes that person from.
    pub account: Option<String>,
    /// Who it goes to: "name@example.com", "Name <name@example.com>", or just their name, which Moss looks up in the user's mail. Never make up an address.
    pub to: Option<Vec<String>>,
    /// Who gets a copy, written the same way as to.
    pub cc: Option<Vec<String>>,
    /// Who gets a hidden copy, written the same way as to.
    pub bcc: Option<Vec<String>>,
    /// The user's own address it goes from (an account's, or one in its alsoSendsFrom), never the address of who it goes to. Best left out: a reply goes from the address the email came to, a new email from the one the user writes that person from.
    pub from: Option<String>,
    /// The signature, by name, or "none". Left out, Moss picks the user's own as it does in the app.
    pub signature: Option<String>,
    /// Legacy paths are rejected. Attach files in Moss and use draft_id instead.
    #[schemars(skip)]
    pub files: Option<Vec<String>>,
    /// File contents already held by the external agent, at most 1 MB total. Never local paths.
    pub attachments: Option<Vec<SuppliedAttachment>>,
    /// The subject. A reply has "Re:" and the thread's subject when left out, a forward "Fwd:".
    pub subject: Option<String>,
    /// The email's text, in plain text. Blank lines start new paragraphs. No signature: Moss adds the user's.
    pub body: Option<String>,
    /// Send later: when it goes, as an RFC 3339 time. Leave out to send now.
    pub send_at: Option<String>,
    /// Your own id for this email, the same on every try of it: a UUID you make, say. Give it,
    /// and when a send times out or fails, retry with the same request_id: Moss answers with what
    /// became of that email (sent, still sending, may have been sent, set to send later, or kept
    /// as a draft) and never sends it twice. A new email needs a new request_id.
    pub request_id: Option<String>,
}

/// What a draft is made or changed from, whichever tool it came in by.
#[derive(Debug, Default)]
struct Fields {
    /// The id a new email gets, when it must be this one: from a send's request_id.
    fresh_id: Option<String>,
    revision: Option<String>,
    draft_id: Option<String>,
    reply_to_thread: Option<String>,
    reply_all: bool,
    forward_thread: Option<String>,
    account: Option<String>,
    to: Option<Vec<String>>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    from: Option<String>,
    signature: Option<String>,
    files: Option<Vec<String>>,
    attachments: Option<Vec<SuppliedAttachment>>,
    subject: Option<String>,
    body: Option<String>,
}

impl Fields {
    fn is_pure_replay(&self) -> bool {
        // Exhaustive: a newly added content field must explicitly choose replay semantics.
        let Self { fresh_id: _, revision: _, draft_id: _, reply_to_thread, reply_all, forward_thread,
            account, to, cc, bcc, from, signature, files, attachments, subject, body } = self;
        reply_to_thread.is_none() && !reply_all && forward_thread.is_none()
            && account.is_none() && to.is_none() && cc.is_none() && bcc.is_none()
            && from.is_none() && signature.is_none() && files.is_none()
            && attachments.is_none() && subject.is_none() && body.is_none()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DraftIdArgs {
    /// The draft revision returned by list_drafts or a successful edit.
    pub revision: String,
    /// The draft's id, from list_drafts or create_draft.
    pub draft_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListArgs {
    /// One account's id, from list_accounts. Every account when left out.
    pub account: Option<String>,
    /// How many threads, newest first. 25 when left out, at most 100.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScheduledArgs {
    /// The email's id, from list_scheduled.
    pub scheduled_id: String,
    /// The immutable operationId returned by list_scheduled. Never substitute a draft id.
    pub operation_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FileArgs {
    /// The thread the file came in, from list_inbox or search_mail.
    pub thread_id: String,
    /// The file's name, as get_thread lists it. When more than one email has a file by this name, the newest one's.
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ContactArgs {
    /// Part of a name or an email address.
    pub query: String,
    /// How many people. 25 when left out, at most 100.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SyncArgs {
    /// One account's id, from list_accounts. Every account when left out.
    pub account: Option<String>,
}

impl MossTools {
    pub fn new(
        engine: Engine,
        events: Arc<dyn Events>,
        clock: fn() -> i64,
        files: PathBuf,
    ) -> Self {
        let outbox = Arc::new(EngineOutbox(engine.clone()));
        Self::with_outbox(engine, events, outbox, clock, files)
    }

    pub fn with_outbox(
        engine: Engine,
        events: Arc<dyn Events>,
        outbox: Arc<dyn Outbox>,
        clock: fn() -> i64,
        files: PathBuf,
    ) -> Self {
        Self {
            engine,
            events,
            outbox,
            clock,
            files,
            tool_router: Self::tool_router(),
        }
    }

    fn signatures(&self) -> Result<Vec<Signature>, String> {
        Ok(self.engine.signatures().map_err(text)?.signatures)
    }

    /// A draft as an agent sees it, with its signature by name.
    fn view(&self, d: &Draft) -> Result<Value, String> {
        Ok(draft_view(d, &self.signatures()?))
    }

    fn account(&self, id: &str) -> Result<AccountInfo, String> {
        let known = self.engine.accounts().map_err(text)?;
        known
            .into_iter()
            .find(|a| a.account.id == id)
            .ok_or_else(|| format!("There's no account with id {id}. list_accounts has them."))
    }

    /// The thread's account, checking it exists.
    fn account_of(&self, thread_id: &str) -> Result<String, String> {
        Ok(self
            .engine
            .thread(thread_id)
            .map_err(text)?
            .summary
            .account_id)
    }

    /// Tells the window to show what changed in `account`.
    fn changed(&self, account: &str) {
        self.events.emit(Event::MailChanged {
            account_id: account.to_string(),
        });
    }

    fn drafts_changed(&self, account: &str) {
        self.events.emit(Event::DraftsChanged {
            account_id: account.to_string(),
        });
    }

    /// Runs `action` on a thread, tells the window, and says what happened.
    fn on_thread(
        &self,
        thread_id: &str,
        action: impl FnOnce() -> moss_engine::Result<()>,
        done: &str,
    ) -> Result<String, String> {
        let account = self.account_of(thread_id)?;
        action().map_err(text)?;
        self.changed(&account);
        Ok(compact(&json!({ "threadId": thread_id, "done": done })))
    }
}

#[tool_router]
impl MossTools {
    #[tool(
        description = "The user's email accounts in Moss: each one's id, address and name, and the other addresses it sends from."
    )]
    async fn list_accounts(&self) -> Result<String, String> {
        let accounts = self.engine.accounts().map_err(text)?;
        let list: Vec<Value> = accounts
            .iter()
            .map(|a| {
                let also: Vec<&str> = a
                    .addresses
                    .iter()
                    .filter(|x| x.state == moss_core::AddressState::Added && !x.blocked)
                    .map(|x| x.email.as_str())
                    .collect();
                json!({ "id": a.account.id, "email": a.account.address, "name": a.account.name, "senderName": a.account.sender_name, "alsoSendsFrom": also })
            })
            .collect();
        Ok(compact(&json!({ "accounts": list })))
    }

    #[tool(
        description = "The open threads in the inbox, newest first: who wrote, the subject, a short summary and whether it needs a reply. Done mail and reminders not due yet aren't here."
    )]
    async fn list_inbox(&self, Parameters(args): Parameters<InboxArgs>) -> Result<String, String> {
        let split = args.split.as_deref().map(split).transpose()?;
        let mut threads = self.engine.threads(args.account.as_deref()).map_err(text)?;
        let now = (self.clock)();
        // Remind me hides a thread until its time, as in the window.
        threads.retain(|t| {
            split.is_none_or(|s| t.split == s) && t.snoozed_until.is_none_or(|u| u <= now)
        });
        threads.sort_by_key(|t| std::cmp::Reverse(t.last_message_at));
        let total = threads.len();
        threads.truncate(limit(args.limit));
        let list: Vec<Value> = threads.iter().map(summary).collect();
        Ok(compact(
            &json!({ "threads": list, "shown": list.len(), "total": total, "now": when(now) }),
        ))
    }

    #[tool(
        description = "One thread with every email in it: who sent it, to whom, when, and the text. Attachments are listed by name."
    )]
    async fn get_thread(&self, Parameters(args): Parameters<ThreadArgs>) -> Result<String, String> {
        let mut thread = self.engine.thread(&args.thread_id).map_err(text)?;
        // Older emails can be headers only: fetch their text, if the server is reachable.
        let missing: Vec<String> = thread
            .messages
            .iter()
            .filter(|m| m.headers_only)
            .map(|m| m.id.clone())
            .collect();
        if !missing.is_empty() {
            for id in &missing {
                let _ = self.engine.load_message(id).await;
            }
            thread = self.engine.thread(&args.thread_id).map_err(text)?;
        }
        Ok(compact(&full(&thread)))
    }

    #[tool(
        description = "Searches the mail by words, and by meaning when Moss's search model is on this Mac. Best matches first."
    )]
    async fn search_mail(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<String, String> {
        if args.query.trim().is_empty() {
            return Err("Say what to search for.".into());
        }
        let hits = self
            .engine
            .find(&args.query, args.account.as_deref(), limit(args.limit))
            .await
            .map_err(text)?;
        let list: Vec<Value> = hits
            .iter()
            .map(|h| {
                let mut v = summary(&h.thread);
                v["match"] = json!(h
                    .snippet
                    .iter()
                    .map(|p| p.text.as_str())
                    .collect::<String>());
                if let Some(f) = &h.file {
                    v["matchedFile"] = json!(f.name);
                }
                v
            })
            .collect();
        Ok(compact(&json!({ "threads": list })))
    }

    #[tool(description = "The drafts in Moss that haven't been sent, most recently changed first.")]
    async fn list_drafts(&self) -> Result<String, String> {
        let drafts = self.engine.drafts().map_err(text)?;
        let signatures = self.signatures()?;
        let list: Vec<Value> = drafts.iter().map(|d| draft_view(d, &signatures)).collect();
        Ok(compact(&json!({ "drafts": list })))
    }

    #[tool(
        description = "Done: out of the inbox, and archived on the server (for Gmail, out of the Inbox). done=false brings it back."
    )]
    async fn mark_done(&self, Parameters(args): Parameters<DoneArgs>) -> Result<String, String> {
        let id = &args.thread_id;
        match args.done.unwrap_or(true) {
            true => self.on_thread(id, || self.engine.mark_done(id), "marked done"),
            false => self.on_thread(id, || self.engine.mark_not_done(id), "back in the inbox"),
        }
    }

    #[tool(description = "Marks a thread read or unread, here and on the server.")]
    async fn mark_read(&self, Parameters(args): Parameters<ReadArgs>) -> Result<String, String> {
        let read = args.read.unwrap_or(true);
        let id = &args.thread_id;
        self.on_thread(
            id,
            || self.engine.mark_read(id, read),
            if read { "marked read" } else { "marked unread" },
        )
    }

    #[tool(
        description = "Remind me: hides a thread until a time, then brings it back to the top of the inbox. Leave out until to stop reminding."
    )]
    async fn remind_me(&self, Parameters(args): Parameters<RemindArgs>) -> Result<String, String> {
        let until = args.until.as_deref().map(time).transpose()?;
        let id = &args.thread_id;
        let said = until.map_or("no reminder".to_string(), |ms| {
            format!("back at {}", when(ms))
        });
        self.on_thread(id, || self.engine.snooze(id, until), &said)
    }

    #[tool(
        description = "Remind me if no reply: brings the thread back if no one answers the user's latest email in a few days. on=false stops waiting."
    )]
    async fn remind_if_no_reply(
        &self,
        Parameters(args): Parameters<NoReplyArgs>,
    ) -> Result<String, String> {
        let on = args.on.unwrap_or(true);
        let account = self.account_of(&args.thread_id)?;
        let theirs = !self
            .engine
            .remind_if_no_reply(&args.thread_id, on)
            .map_err(text)?;
        if on && theirs {
            return Err("The latest email in this thread isn't the user's, so there's no reply to wait for.".into());
        }
        self.changed(&account);
        Ok(compact(
            &json!({ "threadId": args.thread_id, "done": if on { "waiting for a reply" } else { "not waiting" } }),
        ))
    }

    #[tool(
        description = "Moves a thread to another split for good (to Important stars it), or back to Moss's own sorting with \"auto\"."
    )]
    async fn move_thread(&self, Parameters(args): Parameters<MoveArgs>) -> Result<String, String> {
        let to = match args.to.trim().to_ascii_lowercase().as_str() {
            "auto" => None,
            name => Some(split(name)?),
        };
        let id = &args.thread_id;
        self.on_thread(
            id,
            || self.engine.move_thread(id, to),
            &format!("moved to {}", args.to.trim()),
        )
    }

    #[tool(
        description = "Deletes a thread: to Trash, here and on the server. restore_thread brings it back."
    )]
    async fn delete_thread(
        &self,
        Parameters(args): Parameters<ThreadArgs>,
    ) -> Result<String, String> {
        let id = &args.thread_id;
        self.on_thread(id, || self.engine.delete_thread(id), "moved to Trash")
    }

    #[tool(description = "Brings a deleted thread back from Trash.")]
    async fn restore_thread(
        &self,
        Parameters(args): Parameters<ThreadArgs>,
    ) -> Result<String, String> {
        let id = &args.thread_id;
        self.on_thread(id, || self.engine.restore_thread(id), "restored")
    }

    #[tool(
        description = "Blocks the sender of a thread (or their whole domain): their mail goes out of sight, here and on the server."
    )]
    async fn block_sender(
        &self,
        Parameters(args): Parameters<SenderArgs>,
    ) -> Result<String, String> {
        let id = &args.thread_id;
        let domain = args.whole_domain.unwrap_or(false);
        self.on_thread(
            id,
            || self.engine.block_sender(id, domain),
            "sender blocked",
        )
    }

    #[tool(
        description = "Accepts the sender of a thread (or their whole domain), so Moss stops asking about them."
    )]
    async fn accept_sender(
        &self,
        Parameters(args): Parameters<SenderArgs>,
    ) -> Result<String, String> {
        let id = &args.thread_id;
        let domain = args.whole_domain.unwrap_or(false);
        self.on_thread(
            id,
            || self.engine.accept_sender(id, domain),
            "sender accepted",
        )
    }

    #[tool(description = "Unblocks the sender of a thread: their mail comes back to the inbox.")]
    async fn unblock_sender(
        &self,
        Parameters(args): Parameters<ThreadArgs>,
    ) -> Result<String, String> {
        let id = &args.thread_id;
        self.on_thread(id, || self.engine.unblock_sender(id), "sender unblocked")
    }

    #[tool(
        description = "Writes a draft in Moss, a new email or a reply to a thread, without sending it. Use this when the user asked for a draft, or didn't clearly ask to send."
    )]
    pub(crate) async fn create_draft(
        &self,
        Parameters(args): Parameters<CreateArgs>,
    ) -> Result<String, String> {
        let fields = Fields {
            reply_to_thread: args.reply_to_thread,
            reply_all: args.reply_all.unwrap_or(false),
            forward_thread: args.forward_thread,
            account: args.account,
            to: args.to,
            cc: args.cc,
            bcc: args.bcc,
            from: args.from,
            signature: args.signature,
            files: args.files,
            attachments: args.attachments,
            subject: args.subject,
            body: args.body,
            ..Default::default()
        };
        self.keep(self.drafted(fields)?)
    }

    #[tool(
        description = "Changes a draft in Moss, by draft_id, without sending it. Only what's given changes."
    )]
    pub(crate) async fn update_draft(
        &self,
        Parameters(args): Parameters<UpdateArgs>,
    ) -> Result<String, String> {
        let fields = Fields {
            draft_id: Some(args.draft_id),
            revision: Some(args.revision),
            account: args.account,
            to: args.to,
            cc: args.cc,
            bcc: args.bcc,
            from: args.from,
            signature: args.signature,
            files: args.files,
            attachments: args.attachments,
            subject: args.subject,
            body: args.body,
            ..Default::default()
        };
        self.keep(self.drafted(fields)?)
    }

    #[tool(
        description = "Sends an email: a new one, a reply to a thread, or a draft by draft_id; or sets it to send later with send_at. Only call send when the user clearly asked to send this email. If they asked for a draft, use create_draft. If it isn't clear, ask the user before sending. Give each email a request_id, and use the same value when retrying the same email: a retry then says what became of it instead of sending it again. If the result has draftCleanupFailed, the email was sent; do not send it again."
    )]
    pub(crate) async fn send(
        &self,
        Parameters(args): Parameters<SendArgs>,
    ) -> Result<String, String> {
        let send_at = args.send_at.as_deref().map(time).transpose()?;
        let request = args.request_id.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(request_ids);
        let mut fields = send_fields(args);
        if let (None, Some((id, _))) = (&fields.draft_id, &request) {
            // A retry: what the first try did, never a second email.
            if let Some(answer) = self.earlier(id)? { return answer; }
            fields.fresh_id = Some(id.clone());
        }
        if let Some(id) = &fields.draft_id {
            if let Some(receipt) = self.engine.delivery_receipt(id).map_err(text)? {
                if fields.revision.is_none() || fields.revision != receipt.source_revision {
                    return Err("This email already has a delivery record, but this request does not identify its original revision. Do not resend it. Review its delivery status in Moss.".into());
                }
                if send_at.is_some() || !fields.is_pure_replay() {
                    return Err("This email already has a delivery record. Do not resend it or change its content. Start a new email only if you intend a separate delivery.".into());
                }
                return self.delivered(id, receipt);
            }
            if let Some(waiting) = self.waiting(id)? { return Ok(waiting); }
        }
        let retried = request.is_some();
        let operation = request.map_or_else(new_id, |(_, operation)| operation);
        let draft = self.drafted(fields)?;
        let email = outgoing(&draft, &self.signatures()?);
        if let Some(at) = send_at {
            let outcome = self.outbox.schedule(&operation, &draft, &email, at).map_err(|why| refused(why, retried))?;
            if outcome != ScheduleOutcome::Scheduled {
                return Ok(compact(&json!({ "draftId": draft.id, "operationId": operation, "outcome": outcome,
                    "note": "Moss had this request already. Nothing new was sent or set to send later." })));
            }
            self.used(&draft);
            self.drafts_changed(&draft.from_account_id);
            self.changed(&draft.from_account_id);
            return Ok(compact(
                &json!({ "scheduled": when(at.max((self.clock)())), "scheduledId": draft.id, "draftId": draft.id, "operationId": operation, "subject": draft.subject, "to": whom(&draft.to), "note": "Set to send later. The user can change or cancel it in Moss." }),
            ));
        }
        match self.outbox.send(&draft, email).await {
            Ok(thread) => {
                self.used(&draft);
                let cleanup = (|| {
                    if self.engine.editable_draft(&draft.id)?.is_none() { return Ok(false); }
                    self.engine.delete_editable_draft(&draft.id, draft.revision.as_deref(), true)
                })();
                let cleanup_failed = match cleanup {
                    Ok(removed) => {
                        if removed { self.drafts_changed(&draft.from_account_id); }
                        false
                    }
                    Err(_) => {
                        eprintln!("Moss: a sent email's draft cleanup could not be confirmed. Do not resend it.");
                        true
                    }
                };
                self.changed(&draft.from_account_id);
                let note = if cleanup_failed {
                    "Sent. Draft cleanup could not be confirmed. Do not resend."
                } else {
                    "Sent. It's in Sent in Moss."
                };
                let delivery = self.engine.delivery_receipt(&draft.id).ok().flatten().map(|r| r.id);
                Ok(compact(
                    &json!({ "sent": true, "threadId": thread, "draftId": draft.id, "deliveryId": delivery, "subject": draft.subject, "to": whom(&draft.to), "draftCleanupFailed": cleanup_failed, "note": note }),
                ))
            }
            Err(moss_engine::Error::DeliveryBusy) => Err(moss_engine::Error::DeliveryBusy.to_string()),
            Err(why @ moss_engine::Error::DraftConflict) if retried => Err(refused(why, retried)),
            Err(why @ (moss_engine::Error::DeliveryUnknown(_) | moss_engine::Error::DeliveryAccepted(_))) => {
                self.changed(&draft.from_account_id);
                Err(format!("{}. Do not resend this email. Its delivery record is retained under draft id {}.", why.to_string().trim_end_matches('.'), draft.id))
            }
            Err(why) => {
                // A definitive rejection may already have restored a fresh revision.
                // Recognize that exact content instead of re-saving a retired precondition.
                let preserved: moss_engine::Result<()> = (|| {
                    let current = self.engine.editable_draft(&draft.id)?;
                    let matches = current.is_some_and(|current| {
                        let observed = Draft { revision: None, updated_at: draft.updated_at, ..current };
                        let intended = Draft { revision: None, ..draft.clone() };
                        observed == intended
                    });
                    if matches { Ok(()) } else { self.engine.save_draft(&draft).map(|_| ()) }
                })();
                if preserved.is_err() {
                    return Err(format!("Couldn't send: {why}. The draft could not be restored. Keep the text and check Moss before retrying. Draft id {}.", draft.id));
                }
                self.drafts_changed(&draft.from_account_id);
                Err(format!(
                    "Couldn't send: {why}. It's kept as a draft, id {}.",
                    draft.id
                ))
            }
        }
    }

    #[tool(description = "Deletes a draft in Moss, by draft_id. It's gone for good.")]
    async fn delete_draft(
        &self,
        Parameters(args): Parameters<DraftIdArgs>,
    ) -> Result<String, String> {
        let drafts = self.engine.drafts().map_err(text)?;
        let draft = drafts
            .iter()
            .find(|d| d.id == args.draft_id)
            .ok_or_else(|| format!("There's no draft with id {}.", args.draft_id))?;
        self.engine.delete_editable_draft(&draft.id, Some(&args.revision), false).map_err(text)?;
        self.drafts_changed(&draft.from_account_id);
        Ok(compact(
            &json!({ "draftId": draft.id, "done": "draft deleted" }),
        ))
    }

    #[tool(
        description = "Threads marked Done, newest first: out of the inbox, archived. mark_done with done=false brings one back."
    )]
    async fn list_done(&self, Parameters(args): Parameters<ListArgs>) -> Result<String, String> {
        let threads = self
            .engine
            .done_threads(args.account.as_deref(), limit(args.limit))
            .map_err(text)?;
        Ok(compact(
            &json!({ "threads": threads.iter().map(summary).collect::<Vec<_>>() }),
        ))
    }

    #[tool(
        description = "Mail from blocked senders, newest first. unblock_sender brings a sender's mail back."
    )]
    async fn list_blocked(&self, Parameters(args): Parameters<ListArgs>) -> Result<String, String> {
        let threads = self
            .engine
            .blocked_threads(args.account.as_deref(), limit(args.limit))
            .map_err(text)?;
        Ok(compact(
            &json!({ "threads": threads.iter().map(summary).collect::<Vec<_>>() }),
        ))
    }

    #[tool(
        description = "Emails set to send later, soonest first: when each goes, to whom, and why a try failed if one did. Use operationId with cancel_scheduled. Revision is informational; cancellation requires no revision."
    )]
    async fn list_scheduled(&self) -> Result<String, String> {
        let mut later = self.engine.scheduled().map_err(text)?;
        later.sort_by_key(|e| e.send_at);
        let list: Vec<Value> = later
            .iter()
            .map(|e| {
                json!({
                    "scheduledId": e.draft.id,
                    "operationId": e.operation_id,
                    "revision": e.revision,
                    "account": e.draft.from_account_id,
                    "sendAt": when(e.send_at),
                    "to": whom(&e.draft.to),
                    "subject": e.draft.subject,
                    "problem": e.error,
                })
            })
            .collect();
        Ok(compact(&json!({ "scheduled": list })))
    }

    #[tool(description = "Only when the user asks, cancels the exact operation from list_scheduled. A restored draft is returned only when cancellation takes it back to Drafts. Sending, completed and inDelivery outcomes do not mean the email was cancelled.")]
    async fn cancel_scheduled(
        &self,
        Parameters(args): Parameters<ScheduledArgs>,
    ) -> Result<String, String> {
        let outcome = self.engine.cancel_known_schedule(&args.operation_id, &args.scheduled_id).map_err(text)?;
        if let moss_store::ScheduleOutcome::Cancelled { early: false, restored: Some(draft) } = outcome {
            self.drafts_changed(&draft.from_account_id);
            self.changed(&draft.from_account_id);
            return Ok(compact(&json!({ "status": "cancelled", "draftId": draft.id, "revision": draft.revision, "done": "back in Drafts, not sent" })));
        }
        // A retained cancellation says nothing about a replacement schedule on the same draft.
        if matches!(outcome, moss_store::ScheduleOutcome::Cancelled { restored: None, .. }) {
            return Ok(compact(&json!({ "status": "cancelled", "alreadyCancelled": true,
                "note": "This operation was already cancelled. Any other schedule for this email is unchanged." })));
        }
        // In-flight and completed delivery are not a newly restored draft.
        Ok(compact(&json!(outcome)))
    }

    #[tool(
        description = "Saves a file from an email on this Mac and says where, so you can open it. Files are named in get_thread."
    )]
    async fn get_attachment(
        &self,
        Parameters(args): Parameters<FileArgs>,
    ) -> Result<String, String> {
        let thread = self.engine.thread(&args.thread_id).map_err(text)?;
        let (message, file) = thread
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                m.attachments
                    .iter()
                    .find(|a| a.name == args.name)
                    .map(|a| (m, a))
            })
            .ok_or_else(|| format!("There's no file called {} in this thread.", args.name))?;
        let path = self
            .engine
            .attachment_file(&message.id, file.index, &self.files)
            .await
            .map_err(text)?;
        Ok(compact(&json!({
            "name": file.name,
            "contentType": file.content_type,
            "bytes": file.bytes,
            "path": path.display().to_string(),
        })))
    }

    #[tool(
        description = "People the user has written to or heard from, by part of a name or address: most written to first."
    )]
    async fn find_contacts(
        &self,
        Parameters(args): Parameters<ContactArgs>,
    ) -> Result<String, String> {
        let query = args.query.trim().to_lowercase();
        if query.is_empty() {
            return Err("Say who to look for: part of a name or an address.".into());
        }
        let people = self.engine.contacts().map_err(text)?;
        let found: Vec<String> = people
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query) || p.email.to_lowercase().contains(&query)
            })
            .take(limit(args.limit))
            .map(who)
            .collect();
        Ok(compact(&json!({ "contacts": found })))
    }

    #[tool(description = "Checks for new mail now, in every account or one.")]
    async fn sync_now(&self, Parameters(args): Parameters<SyncArgs>) -> Result<String, String> {
        if let Some(id) = &args.account {
            self.account(id)?;
        }
        self.engine.sync_now(args.account.as_deref());
        Ok(compact(&json!({ "done": "checking for new mail" })))
    }
}

impl MossTools {
    /// What became of the email with draft id `id`, for a retry, which never sends it again:
    /// sent, being sent, maybe sent, set to send later, or still in Drafts. `None` when it is none
    /// of those.
    fn earlier(&self, id: &str) -> Result<Option<Result<String, String>>, String> {
        if let Some(receipt) = self.engine.delivery_receipt(id).map_err(text)? {
            return Ok(Some(self.delivered(id, receipt)));
        }
        if let Some(waiting) = self.waiting(id)? {
            return Ok(Some(Ok(waiting)));
        }
        let Some(draft) = self.engine.editable_draft(id).map_err(text)? else { return Ok(None) };
        Ok(Some(Err(format!(
            "This email wasn't sent: it's in Drafts, id {id}, revision {}. Send it with that draft_id and revision, or give a new request_id for a new email.",
            draft.revision.unwrap_or_default()
        ))))
    }

    /// What a send already recorded for draft `id` says happened, without sending again.
    fn delivered(&self, id: &str, mut receipt: DeliveryReceipt) -> Result<String, String> {
        use moss_store::DeliveryState;
        if matches!(receipt.state,DeliveryState::Held | DeliveryState::Claimed) {
            self.engine.recover_deliveries().map_err(|_| "Delivery recovery needs attention in Moss. Do not resend this email.".to_string())?;
            receipt.state=self.engine.delivery_state(&receipt.id).map_err(text)?.unwrap_or(DeliveryState::Uncertain);
        }
        if matches!(receipt.state,DeliveryState::Held | DeliveryState::Uncertain) && self.engine.send_cancelled(&receipt.id).map_err(text)? {
            return Err("This send was cancelled. Recover its unsent draft in Moss before starting a new delivery.".into());
        }
        match receipt.state {
            DeliveryState::Cancelled => Err("This send was cancelled and its unsent draft was recovered into Drafts. Use list_drafts for its new revision.".into()),
            DeliveryState::Held | DeliveryState::Claimed => Err(moss_engine::Error::DeliveryBusy.to_string()),
            _ => self.engine.repair_send(&receipt.id)
                .map(|thread| compact(&json!({ "sent": true, "alreadySent": true, "threadId": thread, "draftId": id, "deliveryId": receipt.id, "note": "Already sent. Do not resend this email." })))
                .map_err(|why| format!("{}. Do not resend this email. Its delivery record is retained under draft id {id}.", why.to_string().trim_end_matches('.'))),
        }
    }

    /// When draft `id` is set to send later: says so, as a retry's answer.
    fn waiting(&self, id: &str) -> Result<Option<String>, String> {
        let later = self.engine.scheduled().map_err(text)?;
        Ok(later.into_iter().find(|e| e.draft.id == id).map(|e| compact(&json!({
            "scheduled": when(e.send_at), "alreadyScheduled": true, "scheduledId": e.draft.id, "draftId": e.draft.id,
            "operationId": e.operation_id, "subject": e.draft.subject, "to": whom(&e.draft.to),
            "note": "Already set to send later. Nothing new was sent or set to send later. cancel_scheduled changes it.",
        }))))
    }

    /// The draft `args` describe: an existing one changed, or a new one.
    fn drafted(&self, args: Fields) -> Result<Draft, String> {
        if args.files.as_ref().is_some_and(|files| !files.is_empty()) {
            return Err("Local paths are not authorized. Attach files in Moss and use draft_id, or supply attachment contents.".into());
        }
        let contents = args.attachments.as_deref().map(supplied).transpose()?;
        if args.reply_to_thread.is_some() && args.forward_thread.is_some() {
            return Err("Give either reply_to_thread or forward_thread, not both.".into());
        }
        if args.reply_all && args.reply_to_thread.is_none() {
            return Err("reply_all needs reply_to_thread: the thread to reply to.".into());
        }
        let now = (self.clock)();
        let mut draft = match &args.draft_id {
            Some(id) => {
                if self.engine.delivery_receipt(id).map_err(text)?.is_some() {
                    return Err("This email already has a delivery record. Do not edit or resend it. Start a new email only if you intend a separate delivery.".into());
                }
                let drafts = self.engine.drafts().map_err(text)?;
                let expected = args.revision.as_deref().ok_or_else(|| "Give the draft revision returned by list_drafts before changing or sending it.".to_string())?;
                let observed = drafts.into_iter().find(|d| &d.id == id)
                    .ok_or_else(|| format!("There's no draft with id {id}."))?;
                if observed.revision.as_deref() != Some(expected) { return Err(moss_engine::Error::DraftConflict.to_string()); }
                observed
            }
            None => self.new_draft(&args)?,
        };
        if let Some(account) = &args.account {
            self.account(account)?;
            if &draft.from_account_id != account {
                draft.from_account_id = account.clone();
                draft.from_address = None;
            }
        }
        // Only a new email chooses its account; a reply or forward goes from the thread's.
        let choosing = args.draft_id.is_none() && draft.mode == ComposeMode::New && args.account.is_none();
        if let Some(from) = &args.from {
            let accounts = self.engine.accounts().map_err(text)?;
            let Some(owner) = accounts.iter().find(|a| sends_from(a, from).is_ok()) else {
                return Err(format!(
                    "{} isn't one of the user's addresses: from is the user's own address it goes from. Put the people it goes to in to, and leave from out.",
                    from.trim()
                ));
            };
            if choosing {
                draft.from_account_id = owner.account.id.clone();
            }
            draft.from_address = sends_from(&self.account(&draft.from_account_id)?, from)?;
        }
        if let Some(to) = &args.to {
            draft.to = self.recipients(to)?;
            // From where the user writes them: the account and address they last wrote them
            // from, or the account their mail comes to.
            if let (true, None, Some(first)) = (choosing, &args.from, draft.to.first()) {
                if let Some((account, address)) = self.engine.writes_from(&first.email).map_err(text)? {
                    let info = self.account(&account)?;
                    draft.from_address = address.and_then(|a| sends_from(&info, &a).ok().flatten());
                    draft.from_account_id = account;
                }
            }
        }
        if let Some(cc) = &args.cc {
            draft.cc = self.recipients(cc)?;
        }
        if let Some(bcc) = &args.bcc {
            draft.bcc = self.recipients(bcc)?;
        }
        if let Some(subject) = args.subject {
            draft.subject = subject;
        }
        if let Some(body) = args.body {
            draft.body_html = moss_engine::compose::text_to_html(&body);
            draft.body = body;
        }
        self.sign(&mut draft, args.signature.as_deref())?;
        draft.updated_at = now;
        if let Some(contents) = contents {
            draft = self.engine.attach_draft_contents(&draft, &contents).map_err(text)?;
        }
        Ok(draft)
    }

    /// Who an email goes to: addresses as written, and names looked up in the user's mail (the
    /// person they write to most), so an agent needn't know or make up an address.
    fn recipients(&self, list: &[String]) -> Result<Vec<Person>, String> {
        list.iter()
            .map(|s| match s.contains('@') {
                true => person(s),
                false => {
                    let name = s.trim();
                    let found = self.engine.find_people(name, 1).map_err(text)?.into_iter().next().map(|c| c.person);
                    found.ok_or_else(|| format!("There's no one called {name} in the user's mail. Use the address the user gave, or ask them for it."))
                }
            })
            .collect()
    }

    /// The signature: the one asked for (by name, or "none"), or else Moss's pick, as the window
    /// picks it, unless one was chosen before.
    fn sign(&self, draft: &mut Draft, asked: Option<&str>) -> Result<(), String> {
        let settings = self.engine.signatures().map_err(text)?;
        if let Some(name) = asked {
            draft.signature_chosen = true;
            draft.signature_id = match name.trim() {
                n if n.eq_ignore_ascii_case("none") => None,
                n => Some(
                    settings
                        .signatures
                        .iter()
                        .find(|s| s.name.eq_ignore_ascii_case(n) || s.id == n)
                        .ok_or_else(|| format!("There's no signature called {n}."))?
                        .id
                        .clone(),
                ),
            };
            return Ok(());
        }
        if draft.signature_chosen {
            return Ok(());
        }
        let kind = match draft.mode {
            ComposeMode::New => EmailKind::New,
            ComposeMode::Forward => EmailKind::Forward,
            _ => EmailKind::Reply,
        };
        let signed_before = match &draft.thread_id {
            Some(id) => self.signed_before(id)?,
            None => 0,
        };
        let email = SignatureEmail {
            account_id: draft.from_account_id.clone(),
            address: draft.from_address.clone(),
            kind,
            signed_before,
        };
        draft.signature_id = moss_core::pick_signature(&settings, &email).id;
        Ok(())
    }

    /// How many emails in the thread the user sent, from any of their addresses.
    fn signed_before(&self, thread_id: &str) -> Result<u32, String> {
        let accounts = self.engine.accounts().map_err(text)?;
        let thread = self.engine.thread(thread_id).map_err(text)?;
        let mine = |email: &str| accounts.iter().any(|a| owns(a, email));
        Ok(thread
            .messages
            .iter()
            .filter(|m| mine(&m.from.email))
            .count() as u32)
    }

    /// Notes the signature as just used, so Moss offers it first.
    fn used(&self, draft: &Draft) {
        if let Some(id) = &draft.signature_id {
            let _ = self.engine.mark_signature_used(id);
        }
    }

    /// Saves a draft and says so.
    fn keep(&self, draft: Draft) -> Result<String, String> {
        let draft = self.engine.save_draft(&draft).map_err(text)?;
        self.drafts_changed(&draft.from_account_id);
        let mut v = self.view(&draft)?;
        v["note"] = json!("Saved as a draft in Moss. It hasn't been sent.");
        Ok(compact(&v))
    }

    fn new_draft(&self, args: &Fields) -> Result<Draft, String> {
        let blank = |account: String| Draft {
            revision: None,
            id: args.fresh_id.clone().unwrap_or_else(new_id),
            mode: ComposeMode::New,
            thread_id: None,
            from_account_id: account,
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: String::new(),
            body: String::new(),
            body_html: String::new(),
            remind_if_no_reply: false,
            updated_at: 0,
            signature_id: None,
            signature_chosen: false,
            from_address: None,
            attachments: vec![],
        };
        if let Some(thread_id) = args
            .reply_to_thread
            .as_ref()
            .or(args.forward_thread.as_ref())
        {
            let thread = self.engine.thread(thread_id).map_err(text)?;
            let account = self.account(&thread.summary.account_id)?;
            let latest = thread.messages.last();
            let mut d = blank(thread.summary.account_id.clone());
            d.thread_id = Some(thread.summary.id.clone());
            let subject = strip_re(&thread.summary.subject);
            if args.forward_thread.is_some() {
                d.mode = ComposeMode::Forward;
                d.subject = format!("Fwd: {subject}");
                d.attachments = latest.map(forwarded_files).unwrap_or_default();
                return Ok(d);
            }
            // Replying goes from the address the email came to, when the account sends from it.
            d.from_address = thread
                .summary
                .to_address
                .as_deref()
                .and_then(|to| sends_from(&account, to).ok().flatten());
            d.subject = format!("Re: {subject}");
            d.to = vec![thread.summary.from.clone()];
            if args.reply_all {
                d.mode = ComposeMode::ReplyAll;
                let others = |list: &[Person]| -> Vec<Person> {
                    list.iter()
                        .filter(|p| !owns(&account, &p.email))
                        .cloned()
                        .collect()
                };
                let (to, cc) = latest
                    .map(|m| (others(&m.to), others(&m.cc)))
                    .unwrap_or_default();
                d.to.extend(to);
                d.cc = cc;
            } else {
                d.mode = ComposeMode::Reply;
            }
            return Ok(d);
        }
        let first = self.engine.accounts().map_err(text)?.into_iter().next();
        let account = first.ok_or("There are no accounts in Moss yet.")?;
        Ok(blank(account.account.id))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MossTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("moss", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// What `send` makes its email from.
fn send_fields(args: SendArgs) -> Fields {
    Fields {
        fresh_id: None,
        draft_id: args.draft_id,
        revision: args.revision,
        reply_to_thread: args.reply_to_thread,
        reply_all: args.reply_all.unwrap_or(false),
        forward_thread: args.forward_thread,
        account: args.account,
        to: args.to,
        cc: args.cc,
        bcc: args.bcc,
        from: args.from,
        signature: args.signature,
        files: args.files,
            attachments: args.attachments,
        subject: args.subject,
        body: args.body,
    }
}

fn text(e: moss_engine::Error) -> String {
    e.to_string()
}

/// Why sending or setting it to send later was refused. A request_id whose draft id is taken
/// (its email was sent, set to send later or kept as a draft, then removed) says so.
fn refused(why: moss_engine::Error, reused: bool) -> String {
    match why {
        moss_engine::Error::DraftConflict if reused => "This request_id was used before, for an email that isn't sent, waiting or in Drafts now. Nothing was sent. Give a new request_id for a new email.".into(),
        why => text(why),
    }
}

/// A result as an agent reads it: no null, false or empty fields, which say nothing and cost
/// a small model its attention.
/// A list left empty at the top says "none found", so it stays.
pub(crate) fn compact(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let kept = map.iter().filter_map(|(k, v)| match v {
                Value::Array(items) if items.is_empty() => Some((k.clone(), v.clone())),
                v => pruned(v).map(|v| (k.clone(), v)),
            });
            Value::Object(kept.collect()).to_string()
        }
        v => pruned(v).unwrap_or(Value::Null).to_string(),
    }
}

fn pruned(v: &Value) -> Option<Value> {
    match v {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) if s.is_empty() => None,
        Value::Array(items) => {
            let items: Vec<Value> = items.iter().filter_map(pruned).collect();
            (!items.is_empty()).then_some(Value::Array(items))
        }
        Value::Object(map) => {
            let map: serde_json::Map<String, Value> = map
                .iter()
                .filter_map(|(k, v)| pruned(v).map(|v| (k.clone(), v)))
                .collect();
            (!map.is_empty()).then_some(Value::Object(map))
        }
        other => Some(other.clone()),
    }
}

fn limit(asked: Option<usize>) -> usize {
    asked.unwrap_or(SOME).clamp(1, MOST)
}

fn split(name: &str) -> Result<SplitId, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "important" => Ok(SplitId::Important),
        "other" => Ok(SplitId::Other),
        "calendar" => Ok(SplitId::Calendar),
        "newsletters" => Ok(SplitId::Newsletters),
        "receipts" => Ok(SplitId::Receipts),
        _ => Err(format!("There's no split called {name}. Use important, other, calendar, newsletters or receipts.")),
    }
}

/// `"Re: Re: Lunch"` → `"Lunch"`, as the window does.
fn strip_re(subject: &str) -> &str {
    let mut s = subject.trim();
    while s.get(..3).is_some_and(|head| head.eq_ignore_ascii_case("re:")) {
        s = s[3..].trim_start();
    }
    s
}

/// `"Name <a@b.c>"` or `"a@b.c"` → a person.
fn person(s: &str) -> Result<Person, String> {
    let s = s.trim();
    let (name, email) = match (s.rfind('<'), s.ends_with('>')) {
        (Some(open), true) => (
            s[..open].trim().trim_matches('"').to_string(),
            s[open + 1..s.len() - 1].trim().to_string(),
        ),
        _ => (String::new(), s.to_string()),
    };
    let ok = email.split_once('@').is_some_and(|(user, host)| {
        !user.is_empty() && host.contains('.') && !email.contains(char::is_whitespace)
    });
    if !ok {
        return Err(format!("{s} isn't an email address."));
    }
    Ok(Person { name, email })
}

pub(crate) fn who(p: &Person) -> String {
    if p.name.is_empty() {
        p.email.clone()
    } else {
        format!("{} <{}>", p.name, p.email)
    }
}

pub(crate) fn whom(list: &[Person]) -> Vec<String> {
    list.iter().map(who).collect()
}

/// Milliseconds since the epoch as an RFC 3339 time, in UTC.
fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn name<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn summary(t: &ThreadSummary) -> Value {
    json!({
        "id": t.id,
        "account": t.account_id,
        "split": name(t.split),
        "section": name(t.section),
        "from": who(&t.from),
        "subject": t.subject,
        "summary": t.summary,
        "date": when(t.last_message_at),
        "unread": t.unread,
        "messages": t.message_count,
        "hasFiles": t.has_attachments,
    })
}

fn cut(s: &str) -> String {
    match s.char_indices().nth(LONGEST_TEXT) {
        Some((at, _)) => format!("{}… (cut, the rest is in Moss)", &s[..at]),
        None => s.to_string(),
    }
}

fn message(m: &Message) -> Value {
    let files: Vec<&str> = m
        .attachments
        .iter()
        .filter(|a| !a.inline)
        .map(|a| a.name.as_str())
        .collect();
    let mut v = json!({
        "id": m.id,
        "from": who(&m.from),
        "to": whom(&m.to),
        "date": when(m.sent_at),
        "text": cut(&m.body_text),
    });
    if !m.cc.is_empty() {
        v["cc"] = json!(whom(&m.cc));
    }
    if !files.is_empty() {
        v["files"] = json!(files);
    }
    if m.headers_only {
        v["text"] = json!("(Not downloaded yet: Moss can't reach the server.)");
    }
    v
}

fn full(t: &Thread) -> Value {
    let mut v = summary(&t.summary);
    v["messages"] = json!(t.messages.iter().map(message).collect::<Vec<_>>());
    v
}

fn draft_view(d: &Draft, signatures: &[Signature]) -> Value {
    let signature = d
        .signature_id
        .as_ref()
        .and_then(|id| signatures.iter().find(|s| &s.id == id))
        .map(|s| s.name.as_str());
    json!({
        "draftId": d.id,
        "revision": d.revision,
        "mode": name(d.mode),
        "threadId": d.thread_id,
        "account": d.from_account_id,
        "from": d.from_address,
        "to": whom(&d.to),
        "cc": whom(&d.cc),
        "bcc": whom(&d.bcc),
        "subject": d.subject,
        "body": cut(&d.body),
        "signature": signature,
        "files": d.attachments.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        "updated": when(d.updated_at),
    })
}

/// Whether `email` is one of the account's own addresses.
fn owns(a: &AccountInfo, email: &str) -> bool {
    a.account.address.eq_ignore_ascii_case(email)
        || a.addresses
            .iter()
            .any(|x| x.email.eq_ignore_ascii_case(email))
}

/// The From address for `from` in this account: `None` for its own, the address for one it
/// sends from, and an error for any other.
fn sends_from(a: &AccountInfo, from: &str) -> Result<Option<String>, String> {
    let from = from.trim().to_lowercase();
    if a.account.address.eq_ignore_ascii_case(&from) {
        return Ok(None);
    }
    let sends = a.addresses.iter().any(|x| {
        x.state == moss_core::AddressState::Added
            && !x.blocked
            && x.email.eq_ignore_ascii_case(&from)
    });
    match sends {
        true => Ok(Some(from)),
        false => Err(format!(
            "{} doesn't send from {from}. list_accounts says which addresses it does.",
            a.account.id
        )),
    }
}

/// The files of an email being forwarded, as the window keeps them.
fn forwarded_files(m: &Message) -> Vec<DraftFile> {
    m.attachments
        .iter()
        .map(|a| DraftFile {
            id: format!("{}:{}", m.id, a.index),
            name: a.name.clone(),
            bytes: a.bytes,
            content_type: a.content_type.clone(),
            forwarded: Some(FileRef {
                message_id: m.id.clone(),
                index: a.index,
            }),
        })
        .collect()
}

/// What goes out for `d`: as the window sends it, with its signature's lines.
fn outgoing(d: &Draft, signatures: &[Signature]) -> OutgoingDraft {
    let signature = d
        .signature_id
        .as_ref()
        .and_then(|id| signatures.iter().find(|s| &s.id == id))
        .map(|s| s.lines.clone())
        .unwrap_or_default();
    OutgoingDraft {
        mode: d.mode,
        thread_id: d.thread_id.clone(),
        from_account_id: d.from_account_id.clone(),
        to: d.to.clone(),
        cc: d.cc.clone(),
        bcc: d.bcc.clone(),
        subject: d.subject.clone(),
        body: d.body.clone(),
        body_html: d.body_html.clone(),
        signature,
        from_address: d.from_address.clone(),
        remind_if_no_reply: d.remind_if_no_reply,
        attachments: d.attachments.clone(),
    }
}

/// An RFC 3339 time as milliseconds since the epoch.
fn time(s: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .map(|t| t.timestamp_millis())
        .map_err(|_| {
            format!("{s} isn't a time Moss can read. Use RFC 3339, like 2026-11-09T09:00:00+01:00.")
        })
}

/// A new draft's id: random, like the window's.
fn new_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the Mac gives random bytes");
    uuid(b, 0x40)
}

/// The draft id and the send-later operation an agent's `request_id` stands for: the same on
/// every try, so a retry finds what the first one did. Shaped like the window's ids.
pub(crate) fn request_ids(request: &str) -> (String, String) {
    let id = |kind: &str| {
        let digest = moss_store::delivery_fingerprint(kind, vec![json!(request)]);
        let hex = digest.trim_start_matches("v1:");
        let mut b = [0u8; 16];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("a SHA-256 digest is hex");
        }
        uuid(b, 0x80)
    };
    (id("moss-mcp-request-draft"), id("moss-mcp-request-schedule"))
}

/// `b` as a UUID of `version` (0x40 random, 0x80 made from a name).
fn uuid(mut b: [u8; 16], version: u8) -> String {
    b[6] = (b[6] & 0x0f) | version;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = crate::hex(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_by_name_in_any_case() {
        assert_eq!(split("Important"), Ok(SplitId::Important));
        assert_eq!(split(" other "), Ok(SplitId::Other));
        assert_eq!(split("calendar"), Ok(SplitId::Calendar));
        assert_eq!(split("newsletters"), Ok(SplitId::Newsletters));
        assert_eq!(split("receipts"), Ok(SplitId::Receipts));
        assert!(split("spam").unwrap_err().contains("no split called spam"));
    }

    #[test]
    fn limits_stay_between_one_and_the_most() {
        assert_eq!(limit(None), SOME);
        assert_eq!(limit(Some(0)), 1);
        assert_eq!(limit(Some(5)), 5);
        assert_eq!(limit(Some(5000)), MOST);
    }

    #[test]
    fn replies_lose_every_re() {
        assert_eq!(strip_re("Re: RE:re: Lunch"), "Lunch");
        assert_eq!(strip_re("Lunch"), "Lunch");
        assert_eq!(strip_re("Re"), "Re");
        // A letter of more than one byte across the third byte is no "Re:" and no panic.
        assert_eq!(strip_re("Noël"), "Noël");
        assert_eq!(strip_re("Re: Grüße"), "Grüße");
    }

    #[test]
    fn people_are_written_with_or_without_a_name() {
        assert_eq!(
            person("a@b.co"),
            Ok(Person {
                name: "".into(),
                email: "a@b.co".into()
            })
        );
        assert_eq!(
            person(" \"Dan O\" <dan@north.co> "),
            Ok(Person {
                name: "Dan O".into(),
                email: "dan@north.co".into()
            })
        );
        for bad in ["dan", "@b.co", "a@b", "a b@c.co", "Dan <dan>"] {
            assert!(
                person(bad).unwrap_err().contains("isn't an email address"),
                "{bad}"
            );
        }
        assert_eq!(
            who(&Person {
                name: "".into(),
                email: "a@b.co".into()
            }),
            "a@b.co"
        );
        assert_eq!(
            who(&Person {
                name: "A".into(),
                email: "a@b.co".into()
            }),
            "A <a@b.co>"
        );
    }

    #[test]
    fn long_text_is_cut_on_a_character() {
        let long = "é".repeat(LONGEST_TEXT + 5);
        let out = cut(&long);
        assert!(out.starts_with(&"é".repeat(LONGEST_TEXT)));
        assert!(out.ends_with("(cut, the rest is in Moss)"));
        assert_eq!(cut("short"), "short");
    }

    #[test]
    fn times_are_rfc_3339_in_utc() {
        assert_eq!(when(1_762_000_000_000), "2025-11-01T12:26:40Z");
        assert_eq!(when(i64::MAX), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn draft_ids_look_like_the_windows() {
        let id = new_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(new_id(), id);
    }

    #[test]
    fn a_request_id_stands_for_the_same_draft_and_operation_every_time() {
        let (draft, operation) = request_ids("agent-email-1");
        assert_eq!(request_ids("agent-email-1"), (draft.clone(), operation.clone()));
        for id in [&draft, &operation] {
            assert_eq!(id.len(), 36);
            assert_eq!(&id[14..15], "8");
            assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
        }
        assert_ne!(draft, operation);
        assert_ne!(request_ids("agent-email-2").0, draft);
    }
}
