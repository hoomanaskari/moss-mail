//! Moss desktop shell. The UI lives in `src/` (React); mail, search and AI live in the `crates/` workspace.
//!
//! Each command is one call into [`moss_engine::Engine`] or [`moss_ai::Ai`], which do the work
//! and are tested there. Engine events go to the window as `moss://event`, AI status as `moss://ai`,
//! and answers to Ask Moss as `moss://ask` (see `ask.rs`).

mod app_icon;
mod backups;
mod crash_log;
mod ask;
mod frame;
pub mod identity;
mod leftovers;
mod links;
pub mod mcp;
pub mod menu;
mod outbox;
pub mod power;
mod private_directory;
mod quit_gate;
mod replies;
mod startup;
mod summaries;
mod window;
mod writing;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use moss_accounts::{Accounts, MailService, PasswordSignIn, SecretStore, ServerSettings};
use moss_ai::{Ai, AiError, Plan, Status};
use moss_engine::{
    AccountInfo, AccountPatch, BrowserSignIn, BrowserStart, Engine, Error, Event, Events, NetConnector,
    OutgoingDraft, Parts, SyncIssue, SyncProgress,
};
use moss_providers::TlsRoots;
use serde::Deserialize;
use outbox::Outbox;
/// The mail Ask Moss reads, for the real-model tests in `tests/`.
pub use ask::{actions as ask_actions, EngineActions, EngineMail};
use tauri::{Emitter, Manager, Runtime, State};

/// The window event every engine event is sent as.
pub const EVENT: &str = "moss://event";
/// The window event the AI's status is sent as whenever it changes.
pub const AI_EVENT: &str = "moss://ai";

/// Version info for the about screen and bug reports.
#[tauri::command]
fn app_info() -> moss_core::AppInfo {
    moss_core::AppInfo::current()
}

type Result<T> = std::result::Result<T, Error>;

#[tauri::command]
async fn accounts(engine: State<'_, Engine>) -> Result<Vec<AccountInfo>> {
    engine.accounts()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordRequest {
    service: MailService,
    email: String,
    password: String,
    servers: Option<ServerSettings>,
    /// The @icloud.com address behind an iCloud custom domain.
    #[serde(default)]
    login: Option<String>,
}

#[tauri::command]
async fn add_password_account(
    engine: State<'_, Engine>,
    request: PasswordRequest,
) -> Result<AccountInfo> {
    let PasswordRequest { service, email, password, servers, login } = request;
    engine.add_password_account(PasswordSignIn { service, email, password, servers, login }).await
}

#[tauri::command]
async fn check_resend_key(
    engine: State<'_, Engine>,
    key: String,
) -> Result<Vec<moss_accounts::ResendDomain>> {
    engine.check_resend_key(&key).await
}

/// Outlook, or Gmail with your own client: the page to open in the browser.
#[tauri::command]
async fn start_browser_sign_in(engine: State<'_, Engine>, request: BrowserSignIn) -> Result<BrowserStart> {
    engine.start_browser_sign_in(request).await
}

/// Waits for the browser to come back, then saves the account that signed in.
#[tauri::command]
async fn finish_browser_sign_in(engine: State<'_, Engine>) -> Result<AccountInfo> {
    engine.finish_browser_sign_in().await
}

#[tauri::command]
async fn add_resend_account(
    engine: State<'_, Engine>,
    key: String,
    email: String,
) -> Result<AccountInfo> {
    engine.add_resend_account(&key, &email).await
}

#[tauri::command]
async fn discover(engine: State<'_, Engine>, email: String) -> Result<moss_accounts::Discovered> {
    engine.discover(&email).await
}

#[tauri::command]
async fn update_account(
    engine: State<'_, Engine>,
    id: String,
    patch: AccountPatch,
) -> Result<AccountInfo> {
    engine.update_account(&id, patch)
}

#[tauri::command]
async fn reorder_accounts(engine: State<'_, Engine>, ids: Vec<String>) -> Result<()> {
    engine.reorder_accounts(&ids)
}

/// With `discard_server_changes`, the account goes although that many server changes haven't
/// reached the server: the number the refusal said, after you agreed to lose them.
#[tauri::command]
async fn remove_account(engine: State<'_, Engine>, id: String, discard_server_changes: Option<usize>) -> Result<moss_engine::CredentialCleanupStatus> {
    match discard_server_changes {
        None => engine.remove_account(&id),
        Some(changes) => engine.remove_account_discarding(&id, changes),
    }
}

#[tauri::command]
async fn credential_cleanup_status(engine: State<'_, Engine>) -> Result<moss_engine::CredentialCleanupStatus> {
    engine.credential_cleanup_status()
}

#[tauri::command]
async fn retry_credential_cleanup(engine: State<'_, Engine>) -> Result<moss_engine::CredentialCleanupStatus> {
    engine.retry_credential_cleanup()
}

/// Another address the account sends from; `name` for mail from it, `null` for the
/// account's sender name.
#[tauri::command]
async fn add_address(
    engine: State<'_, Engine>,
    account: String,
    email: String,
    name: Option<String>,
) -> Result<AccountInfo> {
    engine.add_address(&account, &email, name.as_deref())
}

#[tauri::command]
async fn update_address(
    engine: State<'_, Engine>,
    account: String,
    email: String,
    name: Option<String>,
) -> Result<AccountInfo> {
    engine.update_address(&account, &email, name.as_deref())
}

/// Removes an address, or dismisses one Moss suggested.
#[tauri::command]
async fn remove_address(engine: State<'_, Engine>, account: String, email: String) -> Result<AccountInfo> {
    engine.remove_address(&account, &email)
}

/// Mail sent to the address goes to Blocked (`blocked: true`), or comes back.
#[tauri::command]
async fn block_address(
    engine: State<'_, Engine>,
    account: String,
    email: String,
    blocked: bool,
) -> Result<AccountInfo> {
    engine.block_address(&account, &email, blocked)
}

#[tauri::command]
async fn discover_addresses(engine: State<'_, Engine>, account: String) -> Result<AccountInfo> {
    engine.discover_addresses(&account).await
}

/// The signatures mail from an address adds; `choice: null` uses the account's again.
#[tauri::command]
async fn set_address_signatures(
    engine: State<'_, Engine>,
    account: String,
    email: String,
    choice: Option<moss_core::AccountSignatures>,
) -> Result<()> {
    engine.set_address_signatures(&account, &email, choice)
}

#[tauri::command]
async fn threads(
    engine: State<'_, Engine>,
    account: Option<String>,
) -> Result<Vec<moss_core::ThreadSummary>> {
    engine.threads(account.as_deref())
}

#[tauri::command]
async fn split_counts(
    engine: State<'_, Engine>,
    account: Option<String>,
) -> Result<moss_core::SplitCounts> {
    engine.split_counts(account.as_deref())
}

#[tauri::command]
async fn thread(engine: State<'_, Engine>, id: String) -> Result<moss_core::Thread> {
    engine.thread(&id)
}

#[tauri::command]
async fn mark_done(engine: State<'_, Engine>, id: String) -> Result<()> {
    engine.mark_done(&id)
}

#[tauri::command]
async fn mark_not_done(engine: State<'_, Engine>, id: String) -> Result<()> {
    engine.mark_not_done(&id)
}

#[tauri::command]
async fn delete_thread(engine: State<'_, Engine>, id: String) -> Result<()> {
    engine.delete_thread(&id)
}

#[tauri::command]
async fn restore_thread(engine: State<'_, Engine>, id: String) -> Result<()> {
    engine.restore_thread(&id)
}

#[tauri::command]
async fn done_threads(
    engine: State<'_, Engine>,
    account: Option<String>,
    limit: u32,
) -> Result<Vec<moss_core::ThreadSummary>> {
    engine.done_threads(account.as_deref(), limit as usize)
}

#[tauri::command]
async fn block_sender(engine: State<'_, Engine>, id: String, whole_domain: bool) -> Result<()> {
    engine.block_sender(&id, whole_domain)
}

#[tauri::command]
async fn accept_sender(engine: State<'_, Engine>, id: String, whole_domain: bool) -> Result<()> {
    engine.accept_sender(&id, whole_domain)
}

#[tauri::command]
async fn unblock_sender(engine: State<'_, Engine>, id: String) -> Result<()> {
    engine.unblock_sender(&id)
}

#[tauri::command]
async fn sender_rules(engine: State<'_, Engine>) -> Result<Vec<moss_core::SenderRule>> {
    engine.sender_rules()
}

#[tauri::command]
async fn forget_sender_rule(engine: State<'_, Engine>, account: String, sender: String) -> Result<()> {
    engine.forget_sender_rule(&account, &sender)
}

#[tauri::command]
async fn blocked_threads(
    engine: State<'_, Engine>,
    account: Option<String>,
    limit: u32,
) -> Result<Vec<moss_core::ThreadSummary>> {
    engine.blocked_threads(account.as_deref(), limit as usize)
}

#[tauri::command]
async fn mark_read(engine: State<'_, Engine>, id: String, read: bool) -> Result<()> {
    engine.mark_read(&id, read)
}

#[tauri::command]
async fn snooze(engine: State<'_, Engine>, id: String, until: Option<i64>) -> Result<()> {
    engine.snooze(&id, until)
}

/// Moves a thread to another split tab for good; `to: null` gives it back to Moss's sorting.
#[tauri::command]
async fn move_thread(engine: State<'_, Engine>, id: String, to: Option<moss_core::SplitId>) -> Result<()> {
    engine.move_thread(&id, to)
}

/// Out of sight until `until`, then back unless someone answered. False when the latest
/// email isn't yours.
#[tauri::command]
async fn wait_for_reply(engine: State<'_, Engine>, id: String, until: i64) -> Result<bool> {
    engine.wait_for_reply(&id, until)
}

/// A correction to what Moss decided, as the UI sends it.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Correction {
    Section { section: moss_core::Section },
    Split { split: moss_core::SplitId },
    NoDeadline,
    RemindIfNoReply { on: bool },
}

/// Your corrections to what Moss decided: they win, now and later. False only when Moss
/// can't wait on an answer because the latest email in the thread isn't yours.
#[tauri::command]
async fn correct(engine: State<'_, Engine>, id: String, correction: Correction) -> Result<bool> {
    match correction {
        Correction::Section { section } => engine.set_section(&id, section).map(|_| true),
        Correction::Split { split } => engine.set_split(&id, split).map(|_| true),
        Correction::NoDeadline => engine.clear_deadline(&id).map(|_| true),
        Correction::RemindIfNoReply { on } => engine.remind_if_no_reply(&id, on),
    }
}

#[tauri::command]
async fn allow_images(engine: State<'_, Engine>, account: String, email: String) -> Result<()> {
    engine.allow_images(&account, &email)
}

#[tauri::command]
async fn signatures(engine: State<'_, Engine>) -> Result<moss_core::SignatureSettings> {
    engine.signatures()
}

#[tauri::command]
async fn save_signature(engine: State<'_, Engine>, signature: moss_core::Signature) -> Result<()> {
    engine.save_signature(&signature)
}

#[tauri::command]
async fn delete_signature(engine: State<'_, Engine>, id: String) -> Result<()> {
    engine.delete_signature(&id)
}

/// The signature an account adds to new emails or replies; `signature: null` for none.
#[tauri::command]
async fn set_signature_choice(
    engine: State<'_, Engine>,
    account: String,
    kind: moss_core::SignatureUse,
    signature: Option<String>,
) -> Result<()> {
    engine.set_signature_choice(&account, kind, signature.as_deref())
}

#[tauri::command]
async fn set_let_moss_choose(engine: State<'_, Engine>, on: bool) -> Result<()> {
    engine.set_let_moss_choose(on)
}

#[tauri::command]
async fn mark_signature_used(engine: State<'_, Engine>, id: String) -> Result<()> {
    engine.mark_signature_used(&id)
}

#[tauri::command]
async fn contacts(engine: State<'_, Engine>) -> Result<Vec<moss_core::Person>> {
    engine.contacts()
}

#[tauri::command]
async fn pick_recipient(engine: State<'_, Engine>, typed: String, email: String) -> Result<()> {
    engine.pick_recipient(&typed, &email)
}

#[tauri::command]
async fn recipient_picks(engine: State<'_, Engine>) -> Result<Vec<moss_core::RecipientPick>> {
    engine.recipient_picks()
}

#[tauri::command]
async fn drafts(engine: State<'_, Engine>) -> Result<Vec<moss_core::Draft>> {
    engine.drafts()
}

#[tauri::command]
async fn save_draft(engine: State<'_, Engine>, draft: moss_core::Draft) -> Result<moss_core::Draft> {
    engine.save_draft(&draft)
}

#[tauri::command]
async fn draft_save_receipt(engine: State<'_, Engine>, draft: moss_core::Draft) -> Result<Option<moss_core::Draft>> {
    engine.draft_save_receipt(&draft)
}

#[tauri::command]
async fn keep_local_draft_recovery(engine:State<'_,Engine>,id:String,previous:Option<String>,draft:moss_core::Draft)->Result<moss_engine::LocalDraftRecoveryWrite> {
    engine.keep_local_draft_recovery(&id,previous.as_deref(),&draft)
}

#[tauri::command]
async fn delivery_attention(engine:State<'_,Engine>,after:Option<moss_store::DeliveryAttentionCursor>)->Result<moss_store::DeliveryAttentionPage> {
    engine.delivery_attention(after.as_ref())
}

#[tauri::command]
async fn delivery_attention_draft(engine:State<'_,Engine>,id:String)->Result<Option<moss_core::Draft>> {
    engine.delivery_attention_draft(&id)
}

#[tauri::command]
async fn resolve_delivery(engine:State<'_,Engine>,id:String,action:moss_store::DeliveryResolution)->Result<moss_store::DeliveryResolutionOutcome> {
    engine.resolve_delivery(&id,action)
}

#[tauri::command]
async fn local_draft_recovery_index(engine:State<'_,Engine>)->Result<Vec<moss_store::LocalDraftRecoveryEntry>> {
    engine.local_draft_recovery_index()
}

#[tauri::command]
async fn local_draft_recovery(engine:State<'_,Engine>,id:String)->Result<Option<moss_store::LocalDraftRecovery>> {
    engine.local_draft_recovery(&id)
}

#[tauri::command]
async fn discard_local_draft_recovery(engine:State<'_,Engine>,id:String,version:String)->Result<()> {
    engine.discard_local_draft_recovery(&id,&version)
}

#[tauri::command]
async fn adopt_local_draft_recovery(engine:State<'_,Engine>,id:String,version:String,new_id:String,account:String,missing:Vec<String>)->Result<Option<moss_core::Draft>> {
    engine.adopt_local_draft_recovery(&id,&version,&new_id,&account,&missing)
}

/// Deletes a draft and its files; with `keep_files`, it is being sent and they stay until it has gone.
#[tauri::command]
async fn delete_draft(engine: State<'_, Engine>, id: String, revision: Option<String>, keep_files: Option<bool>) -> Result<bool> {
    engine.delete_editable_draft(&id, revision.as_deref(), keep_files.unwrap_or(false))
}

/// Keeps copies of the files at `paths` (chosen or dropped in the composer) for draft `draft_id`.
#[tauri::command]
async fn attach_files(
    engine: State<'_, Engine>,
    draft_id: String,
    paths: Vec<std::path::PathBuf>,
) -> Result<Vec<moss_core::DraftFile>> {
    engine.attach_files(&draft_id, &paths)
}

/// Releases an unreferenced staged file without touching saved or pending mail.
#[tauri::command]
async fn discard_draft_file(engine: State<'_, Engine>, draft_id: String, id: String) -> Result<bool> {
    engine.discard_draft_file(&draft_id, &id)
}

/// A thumbnail of a file you attached, as a data URL; none if it can't have one.
#[tauri::command]
async fn draft_preview<R: Runtime>(app: tauri::AppHandle<R>, engine: State<'_, Engine>, draft_id: String, id: String) -> Result<Option<String>> {
    let thumbnail = engine.draft_preview(&draft_id, &id, &files_dir(&app)).await?;
    Ok(thumbnail.map(|jpeg| jpeg_data_url(&jpeg)))
}

#[tauri::command]
async fn send(engine: State<'_, Engine>, draft: OutgoingDraft, recovery: Option<moss_core::Draft>) -> Result<String> {
    match recovery {
        Some(recovery) => engine.send_saved(&recovery, &draft).await,
        None => engine.send(draft).await,
    }
}

/// What became of a held email: `false` when Undo took it back, or the id of the conversation
/// it went into, so the screen can open it.
#[derive(Debug, PartialEq)]
enum Sent {
    TakenBack,
    Into(String),
}

impl serde::Serialize for Sent {
    fn serialize<S: serde::Serializer>(&self, to: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Sent::TakenBack => to.serialize_bool(false),
            Sent::Into(thread) => to.serialize_str(thread),
        }
    }
}

/// Holds the email for `delay_ms` so Undo can take it back, then sends it. False when
/// [`cancel_send`] took it back first, else the conversation it went into. Quitting Moss
/// sends it at once.
#[tauri::command]
async fn send_after<R: Runtime>(
    app: tauri::AppHandle<R>,
    engine: State<'_, Engine>,
    outbox: State<'_, Outbox>,
    hold: String,
    draft: OutgoingDraft,
    recovery: moss_core::Draft,
    delay_ms: u64,
) -> Result<Sent> {
    use moss_store::DeliveryState;
    if !engine.prepare_send(&hold, &recovery, &draft).await? {
        if engine.send_cancelled(&hold)? { return Ok(Sent::TakenBack); }
        return match engine.delivery_state(&hold)? {
            Some(DeliveryState::Held) => Err(moss_engine::Error::Invalid("This email is already waiting to send.".into())),
            _ => submit_held(&engine, &hold).await,
        };
    }
    let Some(_going) = outbox.hold_with_registered(&hold, std::time::Duration::from_millis(delay_ms),|| { let _=app.emit("moss://send-prepared",&hold); }).await else { return Ok(Sent::TakenBack); };
    if engine.send_cancelled(&hold)? { return Ok(Sent::TakenBack); }
    submit_held(&engine, &hold).await
}

/// Desktop rejections restore the draft immediately. Scheduled retries have a separate owner.
async fn submit_held(engine: &Engine, hold: &str) -> Result<Sent> {
    match engine.submit_send(hold).await {
        Ok(thread) => Ok(Sent::Into(thread)),
        Err(moss_engine::Error::DeliveryBusy) if engine.send_cancelled(hold)? => Ok(Sent::TakenBack),
        Err(error @ (moss_engine::Error::DeliveryUnknown(_) | moss_engine::Error::DeliveryAccepted(_) | moss_engine::Error::DeliveryBusy)) => Err(error),
        Err(error) => {
            if engine.send_cancelled(hold)? { return Ok(Sent::TakenBack); }
            engine.cancel_send(hold)?;
            Err(error)
        }
    }
}

/// Send later: `email` goes at `send_at` (ms), from this Mac; `draft` opens again to edit.
#[tauri::command]
async fn schedule_send(
    engine: State<'_, Engine>,
    draft: moss_core::Draft,
    email: OutgoingDraft,
    send_at: i64,
    operation: String,
) -> Result<moss_store::ScheduleOutcome> {
    engine.schedule_with_operation(&operation, &draft, &email, send_at)
}

/// Emails set to send later, the soonest first.
#[tauri::command]
async fn scheduled(engine: State<'_, Engine>) -> Result<Vec<moss_core::ScheduledEmail>> {
    engine.scheduled()
}

/// A new time for a scheduled email; now or earlier sends it now.
#[tauri::command]
async fn reschedule_send(engine: State<'_, Engine>, operation: String, id: String, send_at: i64, expected_revision: i64) -> Result<moss_store::ScheduleOutcome> {
    engine.reschedule_with_revision(&operation, &id, send_at, expected_revision)
}

/// Won't send it: it goes back to Drafts, and comes back here to open.
#[tauri::command]
async fn unschedule_send(engine: State<'_, Engine>, operation: String, id: String) -> Result<moss_store::ScheduleOutcome> {
    engine.cancel_schedule_with_operation(&operation, &id)
}

/// Undo send: true when the email was still held and now won't go.
#[tauri::command]
fn cancel_send(engine: State<'_, Engine>, outbox: State<'_, Outbox>, hold: String) -> Result<moss_engine::CancelSendOutcome> {
    // Stop a registered timer even when storage cannot record the Undo yet.
    // The durable cancellation intent also covers release and registration races.
    outbox.cancel(&hold);
    let outcome = engine.cancel_send_restoring(&hold)?;
    Ok(outcome)
}

#[tauri::command]
async fn search(
    engine: State<'_, Engine>,
    query: String,
    account: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<moss_core::SearchHit>> {
    engine.find(&query, account.as_deref(), limit.unwrap_or(50)).await
}

#[tauri::command]
async fn sync_now(engine: State<'_, Engine>, account: Option<String>) -> Result<()> {
    engine.sync_now(account.as_deref());
    Ok(())
}

#[tauri::command]
async fn sync_progress(engine: State<'_, Engine>, account: String) -> Result<Option<SyncProgress>> {
    Ok(engine.sync_progress(&account))
}

#[tauri::command]
async fn server_change_status(engine: State<'_, Engine>, account: String) -> Result<Option<moss_engine::ServerChangeStatus>> {
    engine.server_change_status(&account)
}

#[tauri::command]
async fn interrupted_sends(engine: State<'_, Engine>) -> Result<Vec<moss_store::InterruptedSend>> {
    engine.interrupted_sends()
}

#[tauri::command]
async fn dismiss_interrupted_sends(engine: State<'_, Engine>, ids: Vec<String>) -> Result<()> {
    engine.dismiss_interrupted_sends(&ids)
}

#[tauri::command]
async fn retry_server_change(engine: State<'_, Engine>, account: String, change: String, revision: i64) -> Result<()> {
    engine.retry_server_change(&account, &change, revision)
}

#[tauri::command]
async fn discard_server_change(engine: State<'_, Engine>, account: String, change: String, revision: i64) -> Result<()> {
    engine.discard_server_change(&account, &change, revision)
}

#[tauri::command]
async fn sync_problems(engine: State<'_, Engine>) -> Result<Vec<SyncIssue>> {
    Ok(engine.sync_problems())
}

#[tauri::command]
async fn ai_status(ai: State<'_, Ai>) -> std::result::Result<Status, AiError> {
    Ok(ai.status())
}

#[tauri::command]
async fn ai_choose(ai: State<'_, Ai>, id: String) -> std::result::Result<Status, AiError> {
    ai.choose_writer(&id).await
}

/// What to get, from the first-run step: starts it at once. Progress comes as `moss://ai` events.
#[tauri::command]
async fn ai_plan(ai: State<'_, Ai>, plan: Plan) -> std::result::Result<Status, AiError> {
    ai.choose_plan(plan)
}

/// Deletes the models and turns the AI off, from Settings → Storage.
#[tauri::command]
async fn ai_remove(ai: State<'_, Ai>) -> std::result::Result<Status, AiError> {
    ai.remove_models().await
}

/// Deletes one model, from Settings → AI: `embedder`, `decider` or a writing model's id.
#[tauri::command]
async fn ai_remove_model(ai: State<'_, Ai>, id: String) -> std::result::Result<Status, AiError> {
    ai.remove_model(&id).await
}

/// Starts getting the models. Progress comes as `moss://ai` events.
#[tauri::command]
async fn ai_download(ai: State<'_, Ai>) -> std::result::Result<Status, AiError> {
    ai.download();
    Ok(ai.status())
}

/// Gets models on their own from Settings → AI (`embedder`, `decider` or writing models' ids),
/// without changing who writes. Progress comes as `moss://ai` events.
#[tauri::command]
async fn ai_download_models(ai: State<'_, Ai>, ids: Vec<String>) -> std::result::Result<Status, AiError> {
    ai.download_models(&ids)?;
    Ok(ai.status())
}

/// Stops getting one model, keeping what's in of it.
#[tauri::command]
async fn ai_stop_model(ai: State<'_, Ai>, id: String) -> std::result::Result<Status, AiError> {
    ai.stop_model(&id)
}

#[tauri::command]
async fn ai_cancel(ai: State<'_, Ai>) -> std::result::Result<(), AiError> {
    ai.cancel();
    Ok(())
}

/// "The ask" for a thread, written by the writing model. Errors keep their engine or AI shape.
#[tauri::command]
async fn ai_ask(
    engine: State<'_, Engine>,
    ai: State<'_, Ai>,
    id: String,
) -> std::result::Result<Option<String>, serde_json::Value> {
    let thread = engine.thread(&id).map_err(json)?;
    ai.ask_for(&thread).await.map_err(json)
}

/// An error as the JSON the UI reads.
fn json(e: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(e).unwrap_or_default()
}

/// The decision model, for the engine's sorting.
pub struct AiSorter(pub Ai);

/// The decision model's split labels are the splits' own names; anything else is Important,
/// where the rules put mail they can't place.
fn split_named(label: &str) -> moss_core::SplitId {
    serde_json::from_value(serde_json::Value::String(label.into())).unwrap_or(moss_core::SplitId::Important)
}

fn email_text(email: &moss_store::Undecided) -> String {
    let sender = moss_ai::prompt::sender(&email.from);
    moss_ai::prompt::email(&sender, &email.subject, &email.attachments, &email.body)
}

#[async_trait::async_trait]
impl moss_engine::Sorter for AiSorter {
    async fn needs_reply(&self, email: &moss_store::Undecided) -> Option<f64> {
        self.0.needs_reply(&email_text(email)).await.ok()
    }

    /// Mail still in Newsletters is there only for its list headers: the question for mail
    /// from a mailing list says whether it is a newsletter at all.
    async fn split(&self, email: &moss_store::Undecided) -> Option<(moss_core::SplitId, f64)> {
        let listed = email.split == moss_core::SplitId::Newsletters;
        self.0.split_for(&email_text(email), listed).await.ok().map(|(label, likely)| (split_named(&label), likely))
    }

    async fn pitch(&self, email: &moss_store::Undecided) -> Option<moss_engine::Pitch> {
        self.0.pitch(&email_text(email)).await.ok().map(moss_engine::Pitch::from)
    }

    async fn deadline(&self, email: &moss_store::Undecided) -> Option<f64> {
        self.0.deadline(&email_text(email)).await.ok()
    }

    async fn expects_reply(&self, email: &moss_store::Undecided) -> Option<f64> {
        self.0.expects_reply(&email_text(email)).await.ok()
    }
}

/// The embedding model, for search by meaning.
pub struct AiEmbedder(pub Ai);

#[async_trait::async_trait]
impl moss_engine::Embedder for AiEmbedder {
    fn model(&self) -> String {
        self.0.embedder_model().into()
    }

    async fn documents(&self, emails: &[moss_store::ToEmbed]) -> Option<Vec<Vec<f32>>> {
        let texts: Vec<String> = emails.iter().map(|e| moss_ai::prompt::document(&e.subject, &e.text)).collect();
        self.0.embed(&texts).await.ok()
    }

    async fn query(&self, text: &str) -> Option<Vec<f32>> {
        self.0.embed(&[moss_ai::prompt::query(text)]).await.ok()?.pop()
    }
}

/// What reads the text inside attachments, on this Mac: PDFKit, Vision and the Office readers
/// of `moss-extract`. In the background only while `conditions` (the AI's power check) says
/// nothing should wait.
pub struct MacReader {
    pub native: Arc<dyn moss_extract::Native>,
    pub conditions: fn() -> Option<moss_ai::power::Pause>,
}

impl moss_engine::Reader for MacReader {
    fn may_run(&self) -> bool {
        (self.conditions)().is_none()
    }

    fn reads(&self, name: &str, content_type: &str) -> bool {
        moss_extract::readable(name, content_type)
    }

    fn read(&self, bytes: &[u8], name: &str, content_type: &str) -> Option<String> {
        moss_extract::extract(bytes, name, content_type, self.native.as_ref())
    }
}

/// The writing model, for summaries of conversations.
pub struct AiSummarizer(pub Ai);

#[async_trait::async_trait]
impl moss_engine::Summarizer for AiSummarizer {
    fn model(&self) -> String {
        moss_ai::summaries::model_key(&self.0.writer_key())
    }

    async fn summarize(&self, c: &moss_store::ToSummarize) -> Option<String> {
        self.0.summary(&summaries::text(c)).await.ok().flatten()
    }

    async fn summarize_as(&self, c: &moss_store::ToSummarize, request: &moss_core::SummaryRequest) -> Option<moss_core::SummaryContent> {
        summaries::write(&self.0, c, request).await
    }
}

/// Apple's helper: beside Moss's own program (`Contents/MacOS/moss-apple` in the app), or
/// `MOSS_APPLE` in debug builds, to try one built from `apple/`.
pub fn apple_helper(exe: Option<PathBuf>, env: Option<std::ffi::OsString>) -> Option<PathBuf> {
    if let Some(path) = env.filter(|_| cfg!(debug_assertions)) {
        return Some(path.into());
    }
    exe.map(|e| e.with_file_name("moss-apple")).filter(|p| p.is_file())
}

/// The AI over `<data_dir>/ai`. `on_status` hears every change; `runtime` is a `llama-server`
/// to use instead of downloading one, honoured in debug builds only. Apple Intelligence writes
/// too where this Mac has it.
pub fn ai(
    data_dir: &Path,
    on_status: Arc<dyn Fn(Status) + Send + Sync>,
    runtime: Option<&Path>,
    free_space: fn(&Path) -> Option<u64>,
) -> Ai {
    let helper = apple_helper(std::env::current_exe().ok(), std::env::var_os("MOSS_APPLE"));
    let ai = Ai::new(moss_ai::Parts {
        apple: helper.and_then(|h| moss_ai::apple::detect(&h)),
        dir: data_dir.join("ai"),
        machine: moss_ai::Machine::detect(),
        catalog: moss_ai::manager::Catalog::pinned(),
        launcher: Arc::new(moss_ai::manager::Llama),
        events: on_status,
        free_space,
        runtime: runtime.filter(|_| cfg!(debug_assertions)).map(Path::to_path_buf),
        conditions: moss_ai::power::conditions,
        recheck: std::time::Duration::from_secs(30),
    });
    // A run that crashed or was force-quit can leave models running and holding memory.
    ai.stop_leftovers();
    ai
}

/// Sends AI status to the window, and starts each AI job as its model arrives: sorting,
/// indexing older mail by meaning, summaries.
pub fn ai_events<R: Runtime>(app: tauri::AppHandle<R>, engine: Engine) -> Arc<dyn Fn(Status) + Send + Sync> {
    Arc::new(move |status: Status| {
        if status.features.sorting {
            engine.wake_sorter();
        }
        if status.features.search {
            engine.wake_embedder();
        }
        if status.features.writing {
            engine.wake_summarizer();
        }
        let _ = app.emit(AI_EVENT, status);
    })
}

/// Where opened attachments are saved as files, in the app's cache folder.
fn files_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> std::path::PathBuf {
    app.path().app_cache_dir().unwrap_or_else(|_| std::env::temp_dir()).join("attachments")
}

/// Settings → Storage: the engine's figures, and the AI models on disk.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageView {
    #[serde(flatten)]
    info: moss_engine::StorageInfo,
    models: u64,
}

#[tauri::command]
async fn storage<R: Runtime>(
    app: tauri::AppHandle<R>,
    engine: State<'_, Engine>,
    ai: State<'_, Ai>,
) -> Result<StorageView> {
    Ok(StorageView { info: engine.storage(&files_dir(&app))?, models: ai.status().on_disk })
}

#[tauri::command]
async fn set_offline(engine: State<'_, Engine>, offline: moss_store::Offline) -> Result<()> {
    engine.set_offline(offline)
}

/// Settings → Storage → Search inside attachments.
#[tauri::command]
async fn set_attachment_text(engine: State<'_, Engine>, on: bool) -> Result<()> {
    engine.set_reading_attachments(on)
}

#[tauri::command]
async fn set_cache_limit(engine: State<'_, Engine>, bytes: u64) -> Result<()> {
    engine.set_cache_limit(bytes)
}

/// Clears the cache; returns the bytes freed.
#[tauri::command]
async fn clear_cache<R: Runtime>(app: tauri::AppHandle<R>, engine: State<'_, Engine>) -> Result<u64> {
    engine.clear_cache(&files_dir(&app))
}

#[tauri::command]
async fn rebuild_index(engine: State<'_, Engine>) -> Result<()> {
    engine.rebuild_index()
}

/// Fetches the bodies of these threads that aren't here yet, so they open at once.
#[tauri::command]
async fn preload(engine: State<'_, Engine>, ids: Vec<String>) -> Result<()> {
    engine.preload(&ids).await
}

/// An attachment saved as a file in the app's cache folder (fetched from the server if
/// need be), for the window to open with the default app. Returns its path.
#[tauri::command]
async fn attachment_file<R: Runtime>(
    app: tauri::AppHandle<R>,
    engine: State<'_, Engine>,
    message_id: String,
    index: u32,
) -> Result<String> {
    let file = engine.attachment_file(&message_id, index, &files_dir(&app)).await;
    file.map(|path| path.display().to_string())
}

/// A thumbnail of an attachment for its card, as a data URL; none if it can't have one.
#[tauri::command]
async fn attachment_preview<R: Runtime>(
    app: tauri::AppHandle<R>,
    engine: State<'_, Engine>,
    message_id: String,
    index: u32,
) -> Result<Option<String>> {
    let thumbnail = engine.attachment_preview(&message_id, index, &files_dir(&app)).await?;
    Ok(thumbnail.map(|jpeg| jpeg_data_url(&jpeg)))
}

fn jpeg_data_url(jpeg: &[u8]) -> String {
    use base64::Engine as _;
    format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg))
}

/// The body of a message stored with headers only (older sent mail), when it's opened.
#[tauri::command]
async fn load_message(engine: State<'_, Engine>, message_id: String) -> Result<bool> {
    engine.load_message(&message_id).await
}

/// Sends engine events to the window.
pub struct WindowEvents<R: Runtime>(pub tauri::AppHandle<R>);

impl<R: Runtime> Events for WindowEvents<R> {
    fn emit(&self, event: Event) {
        // Nothing to do if the window is gone; the next load reads the current state.
        let _ = self.0.emit(EVENT, event);
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Which certificates to trust: the OS store, or in a debug build only, the DER file in
/// `MOSS_TLS_CA` (for the end-to-end test against a local mail server).
pub fn tls_roots(ca_file: Option<&Path>) -> TlsRoots {
    match ca_file.filter(|_| cfg!(debug_assertions)).and_then(|p| std::fs::read(p).ok()) {
        Some(der) => TlsRoots::Custom(vec![der]),
        None => TlsRoots::Platform,
    }
}

/// Where this Moss keeps its database: its app-data folder, or for Moss Dev and debug builds,
/// the folder in `MOSS_DATA_DIR` (so a test or a try never touches your real mail). See
/// [`identity::data_dir`].
pub fn data_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<PathBuf> {
    let flavor = identity::Flavor::of(&app.config().identifier);
    let custom = std::env::var_os("MOSS_DATA_DIR").map(PathBuf::from);
    Ok(identity::data_dir(flavor, app.path().app_data_dir()?, custom.as_deref(), cfg!(debug_assertions)))
}

/// The file Moss saves its window's size and place in, beside its database.
fn window_file<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<std::path::PathBuf> {
    Some(data_dir(app).ok()?.join("window.json"))
}

/// The mail database's file name in the data folder.
pub const DATABASE: &str = "moss.db";

/// The engine over the database in `data_dir`, with secrets in `secrets`.
pub fn engine(
    data_dir: &Path,
    secrets: Arc<dyn SecretStore>,
    events: Arc<dyn Events>,
    roots: TlsRoots,
) -> std::result::Result<Engine, Box<dyn std::error::Error>> {
    private_directory::prepare(data_dir)?;
    let mut store = moss_store::Store::open(data_dir.join(DATABASE))?;
    store.claim_delivery_session()?;
    Ok(Engine::new(Parts {
        store,
        accounts: Accounts::new(secrets.clone()),
        secrets,
        connector: Arc::new(NetConnector { roots }),
        events,
        clock: now_ms,
    }))
}

/// Adds the commands and the engine-backed state to a builder.
pub fn with_commands<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(links::plugin())
        .plugin(window::plugin(window::CLOSING_HIDES))
        .manage(writing::Writing::default())
        .manage(quit_gate::QuitGate::default())
        .manage(backups::Backups::default())
        .invoke_handler(tauri::generate_handler![
        app_info,
        identity::app_identity,
        quit_gate::quit_ready,
        quit_gate::quit_blocked,
        quit_gate::quit_cancel,
        quit_gate::request_restart,
        backups::backup_status,
        backups::backup_start,
        backups::backup_cancel,
        app_icon::set_app_icon,
        accounts,
        send_after,
        cancel_send,
        schedule_send,
        scheduled,
        reschedule_send,
        unschedule_send,
        add_password_account,
        check_resend_key,
        add_resend_account,
        start_browser_sign_in,
        finish_browser_sign_in,
        discover,
        update_account,
        reorder_accounts,
        remove_account,
        credential_cleanup_status,
        retry_credential_cleanup,
        add_address,
        update_address,
        remove_address,
        block_address,
        discover_addresses,
        set_address_signatures,
        threads,
        split_counts,
        thread,
        mark_done,
        mark_not_done,
        delete_thread,
        restore_thread,
        done_threads,
        block_sender,
        accept_sender,
        unblock_sender,
        sender_rules,
        forget_sender_rule,
        blocked_threads,
        mark_read,
        snooze,
        move_thread,
        wait_for_reply,
        correct,
        allow_images,
        signatures,
        save_signature,
        delete_signature,
        set_signature_choice,
        set_let_moss_choose,
        mark_signature_used,
        contacts,
        pick_recipient,
        recipient_picks,
        drafts,
        save_draft,
        draft_save_receipt,
        keep_local_draft_recovery,
        local_draft_recovery_index,
        delivery_attention,
        delivery_attention_draft,
        resolve_delivery,
        local_draft_recovery,
        discard_local_draft_recovery,
        adopt_local_draft_recovery,
        delete_draft,
        attach_files,
        discard_draft_file,
        draft_preview,
        send,
        search,
        sync_now,
        sync_progress,
        sync_problems,
        server_change_status,
        retry_server_change,
        discard_server_change,
        interrupted_sends,
        dismiss_interrupted_sends,
        ai_status,
        ai_choose,
        ai_download,
        ai_download_models,
        ai_stop_model,
        ai_cancel,
        ai_ask,
        summaries::ai_summarize,
        summaries::ai_summaries,
        summaries::discard_summary,
        summaries::summary_rules,
        summaries::save_summary_rule,
        summaries::remove_summary_rule,
        summaries::automatic_summaries,
        summaries::set_automatic_summaries,
        ai_plan,
        ai_remove,
        ai_remove_model,
        replies::ai_reply_options,
        replies::ai_reply,
        writing::ai_stop,
        ask::ask_moss,
        ask::ask_stop,
        ask::ask_forget,
        ask::ask_mail_count,
        attachment_file,
        attachment_preview,
        load_message,
        storage,
        set_offline,
        set_attachment_text,
        set_cache_limit,
        clear_cache,
        rebuild_index,
        preload,
        mcp::mcp_status,
        mcp::mcp_set_on,
        mcp::mcp_new_token,
    ])
}

/// `builder` as the app for `flavor`: its plugins, commands and menu bar. `run` adds the start.
/// (`tests/menu.rs` builds it, as AppKit makes menus on the main thread only.)
pub fn app_builder<R: Runtime>(builder: tauri::Builder<R>, flavor: identity::Flavor) -> tauri::Builder<R> {
    let builder = with_commands(builder)
        .plugin(tauri_plugin_opener::init())
        // The composer's Attach opens the system file picker.
        .plugin(tauri_plugin_dialog::init())
        .plugin(frame::plugin(window_file))
        .menu(move |app| menu::build(app, flavor))
        .on_menu_event(|app, event| menu::on_event(app, event.id()));
    // New versions from GitHub Releases, checked against Moss's update key; then a restart.
    // Moss Dev has no updater at all.
    if flavor.updates() { builder.plugin(tauri_plugin_updater::Builder::new().build()) } else { builder }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut context = tauri::generate_context!();
    let flavor = identity::settle(context.config_mut(), cfg!(debug_assertions));
    app_builder(tauri::Builder::default(), flavor)
        .setup(move |app| {
            use tauri_plugin_dialog::DialogExt;
            let handle = app.handle().clone();
            app.manage(backups::BackupDialog(Box::new(move |selected| {
                handle.dialog().file().set_title("Save recovery archive")
                    .set_file_name(flavor.backup_name())
                    .add_filter("Moss recovery archive", &["moss-backup"])
                    .save_file(move |file| selected(file.map(|file| file.into_path()
                        .map_err(|_| "The archive destination could not be selected.".to_string())).transpose()));
            })));
            let dir = data_dir(app.handle())?;
            crash_log::install(dir.join(crash_log::FILE));
            let ca = std::env::var_os("MOSS_TLS_CA").map(std::path::PathBuf::from);
            let events = Arc::new(WindowEvents(app.handle().clone()));
            let secrets = |path: &Path| private_directory::secrets(path, flavor.keychain_service());
            let (secrets, engine) = match startup::open(&dir, secrets, events.clone(), tls_roots(ca.as_deref())) {
                Ok(opened) => opened,
                Err(failure) => {
                    // Say why, then quit with an error instead of not appearing at all. The
                    // dialog needs the event loop, which runs once setup returns.
                    let (title, text) = failure.message();
                    if let Some(window) = app.get_webview_window("main") { let _ = window.hide(); }
                    app.manage(quit_gate::Actions(Arc::new(|_| std::process::exit(1))));
                    app.dialog().message(text).title(title)
                        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCustom("Quit Moss".into()))
                        .show(|_| std::process::exit(1));
                    return Ok(());
                }
            };
            // Your own agents: off until you turn it on in Settings → AI.
            let agents = mcp::start(mcp::mcp(&engine, events.clone(), secrets, &dir, flavor.mcp_port()));
            let actions = ask::actions(&engine, events, &dir);
            let runtime = std::env::var_os("MOSS_LLAMA_SERVER").map(std::path::PathBuf::from);
            let on_status = ai_events(app.handle().clone(), engine.clone());
            let ai = ai(&dir, on_status, runtime.as_deref(), moss_ai::manager::free_space);
            let started = engine.clone();
            let sorter = Arc::new(AiSorter(ai.clone()));
            // Background indexing and summaries wait while the Mac is on its battery.
            let on_power: Arc<dyn moss_ai::power::PowerSource> = Arc::new(moss_ai::power::MacPower);
            let embedder = Arc::new(power::OnPower { inner: AiEmbedder(ai.clone()), power: on_power.clone() });
            let summarizer = Arc::new(power::OnPower { inner: AiSummarizer(ai.clone()), power: on_power.clone() });
            let resumes = engine.clone();
            tauri::async_runtime::spawn(power::resume_on_power(on_power, power::RECHECK, move || {
                resumes.wake_embedder();
                resumes.wake_summarizer();
                resumes.wake_reader();
            }));
            let reader = Arc::new(MacReader { native: Arc::new(moss_extract::mac::Mac), conditions: moss_ai::power::conditions });
            tauri::async_runtime::spawn(async move {
                started.set_sorter(sorter);
                started.set_embedder(embedder);
                started.set_summarizer(summarizer);
                started.set_reader(reader);
                started.start()
            });
            let quitting=app.handle().clone();
            app.manage(quit_gate::Actions(Arc::new(move |effect| {
                use quit_gate::Effect;
                match effect {
                    Effect::Ask(token) => { let _=quitting.emit("moss://quit-requested",token); }
                    Effect::Focus => window::reopen(quitting.get_webview_window("main").as_ref()),
                    Effect::Finish { restart } => {
                        if restart { quitting.request_restart(); } else { quitting.exit(0); }
                    }
                    Effect::Confirm(token) => {
                        let app=quitting.clone();
                        quitting.dialog().message("Moss could not confirm that your latest drafts are saved. Quit anyway and risk losing those edits?")
                            .title("Drafts may not be saved")
                            .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom("Quit anyway".into(),"Keep working".into()))
                            .show(move |confirmed| {
                                let gate=app.state::<quit_gate::QuitGate>();let actions=app.state::<quit_gate::Actions>();
                                if confirmed {gate.ready(&token,&actions);} else {gate.cancel(&token,&actions);}
                            });
                    }
                }
            })));
            app.manage(engine);
            app.manage(ai);
            app.manage(moss_ai::ask::Chats::default());
            app.manage(actions);
            app.manage(Outbox::default());
            app.manage(agents);
            Ok(())
        })
        .build(context)
        .expect("error while running Moss")
        .run(on_run_event);
}

/// What Moss does as it runs: a Dock click brings back the window the red button hid (the test
/// runtime can't send one; `window::reopen` is tested), and quitting finishes up.
fn on_run_event<R: Runtime>(app: &tauri::AppHandle<R>, event: tauri::RunEvent) {
    match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => window::reopen(app.get_webview_window("main").as_ref()),
        tauri::RunEvent::ExitRequested { api, .. } => {
            if app.state::<quit_gate::QuitGate>().request(false,&app.state::<quit_gate::Actions>()) { api.prevent_exit(); }
        }
        tauri::RunEvent::Exit => quit(app),
        _ => {}
    }
}

/// Quitting restores the Dock icon, cancels backup work, flushes email waiting for Undo,
/// then stops the models, which would keep running.
fn quit<R: Runtime>(app: &tauri::AppHandle<R>) {
    app_icon::reset();
    tauri::async_runtime::block_on(async {
        let backups = app.state::<backups::Backups>();
        let backup_shutdown = backups.shutdown(std::time::Duration::from_secs(30));
        let outbox = app.state::<Outbox>();
        let (_, ()) = tokio::join!(backup_shutdown, outbox.flush(std::time::Duration::from_secs(30)));
        app.state::<Ai>().stop_all().await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use moss_ai::Download;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
    use tauri::Listener;

    #[test]
    fn the_bundle_requires_the_macos_version_used_by_native_ocr() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let version = conf["bundle"]["macOS"]["minimumSystemVersion"].as_str().unwrap_or("10.13");
        let major: u32 = version.split('.').next().unwrap().parse().unwrap();
        assert!(major >= 13, "Vision automatic language detection requires macOS 13 or newer");
    }

    #[test]
    fn app_info_reports_this_build() {
        let info = app_info();
        assert_eq!(info.name, "Moss");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    fn csp_directive(name: &str) -> Vec<String> {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let csp = conf["app"]["security"]["csp"].as_str().unwrap().to_string();
        csp.split(';')
            .map(str::trim)
            .find_map(|d| {
                d.strip_prefix(name).map(|v| v.split_whitespace().map(String::from).collect())
            })
            .unwrap_or_default()
    }

    #[test]
    fn the_app_csp_lets_emails_show_https_images_once_allowed() {
        // Each email's own iframe CSP blocks remote images until "Show images"; the app's
        // CSP is the ceiling, so it has to allow https images or that button does nothing.
        let img = csp_directive("img-src");
        assert!(img.contains(&"https:".to_string()), "{img:?}");
        assert!(!img.iter().any(|s| s == "*" || s == "http:"), "{img:?}");
        assert_eq!(csp_directive("default-src"), ["'self'"]);
        assert!(!csp_directive("script-src").iter().any(|s| s.contains("unsafe")));
    }

    #[test]
    fn the_app_csp_keeps_web_pages_out_of_frames() {
        // Links in an email reach the browser through the email frame's click handler, so no
        // frame ever needs to load a web page: one an email slips past that handler is blocked.
        assert_eq!(csp_directive("frame-src"), ["'self'"]);
    }

    #[test]
    fn the_window_may_open_links_in_the_browser() {
        let caps: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let permissions = caps["permissions"].as_array().unwrap();
        assert!(permissions.contains(&json!("opener:default")), "{permissions:?}");
    }

    #[test]
    fn a_test_certificate_is_trusted_only_when_given() {
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("ca.der");
        std::fs::write(&ca, b"der bytes").unwrap();
        assert_eq!(tls_roots(Some(&ca)), TlsRoots::Custom(vec![b"der bytes".to_vec()]));
        assert_eq!(tls_roots(Some(&dir.path().join("missing"))), TlsRoots::Platform);
        assert_eq!(tls_roots(None), TlsRoots::Platform);
        assert!(now_ms() > 1_700_000_000_000);
    }

    #[test]
    fn each_moss_keeps_its_data_in_a_folder_named_after_it() {
        for (identifier, folder) in [("app.moss.mail", "app.moss.mail"), ("app.moss.mail.dev", "app.moss.mail.dev")] {
            let mut context = mock_context(noop_assets());
            context.config_mut().identifier = identifier.into();
            let app = mock_builder().build(context).unwrap();
            let dir = data_dir(app.handle()).unwrap();
            // MOSS_DATA_DIR is for tests run by hand; none is set here.
            assert_eq!(dir.file_name().unwrap(), folder);
            assert_eq!(window_file(app.handle()).unwrap(), dir.join("window.json"));
        }
    }

    struct App {
        app: tauri::App<MockRuntime>,
        webview: tauri::WebviewWindow<MockRuntime>,
        secrets: Arc<moss_accounts::MemorySecretStore>,
        _dir: tempfile::TempDir,
    }

    fn app() -> App {
        app_in(tempfile::tempdir().unwrap())
    }

    fn app_in(dir: tempfile::TempDir) -> App {
        app_in_with_dialog(dir, None)
    }

    fn app_in_with_dialog(dir: tempfile::TempDir, dialog: Option<backups::BackupDialog>) -> App {
        let app = with_commands(mock_builder()).build(mock_context(noop_assets())).unwrap();
        let output = dir.path().join("mail.backup");
        app.manage(dialog.unwrap_or_else(|| backups::BackupDialog(Box::new(move |selected| selected(Ok(Some(output.clone())))))));
        let events = Arc::new(WindowEvents(app.handle().clone()));
        let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
        let engine = engine(&dir.path().join("data"), secrets.clone(), events.clone(), TlsRoots::Platform).unwrap();
        app.manage(ask::actions(&engine, events, &dir.path().join("data")));
        // No room on the disk, so a download stops before it reaches the network.
        let on_status = ai_events(app.handle().clone(), engine.clone());
        app.manage(ai(&dir.path().join("data"), on_status, None, |_| Some(0)));
        app.manage(engine);
        app.manage(moss_ai::ask::Chats::default());
        app.manage(Outbox::default());
        let webview =
            tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        App { app, webview, secrets, _dir: dir }
    }

    impl App {
        fn call(&self, cmd: &str, args: Value) -> std::result::Result<Value, Value> {
            invoke(&self.webview, cmd, args)
        }

        fn kind(&self, cmd: &str, args: Value) -> String {
            let err = self.call(cmd, args).unwrap_err();
            err["kind"].as_str().unwrap_or_default().to_string()
        }
    }

    /// Calls a command as the window would, from any thread.
    fn invoke(webview: &tauri::WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> std::result::Result<Value, Value> {
        let request = tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // Where the app's pages come from: Windows serves them over http.
            url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }
                .parse()
                .unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(webview, request).map(|body| body.deserialize::<Value>().unwrap())
    }

    #[test]
    fn commands_refuse_requests_before_the_mail_engine_is_initialized() {
        let app = with_commands(mock_builder()).build(mock_context(noop_assets())).unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        for command in [
            "accounts", "add_password_account", "check_resend_key", "start_browser_sign_in",
            "finish_browser_sign_in", "add_resend_account", "discover", "update_account",
            "reorder_accounts", "remove_account", "credential_cleanup_status", "retry_credential_cleanup",
            "retry_server_change", "discard_server_change", "interrupted_sends", "dismiss_interrupted_sends",
            "add_address", "update_address", "remove_address", "block_address", "discover_addresses",
            "drafts", "save_draft", "draft_save_receipt", "keep_local_draft_recovery", "local_draft_recovery_index", "local_draft_recovery", "discard_local_draft_recovery", "adopt_local_draft_recovery", "delete_draft", "attach_files", "discard_draft_file", "draft_preview",
            "send_after", "schedule_send", "scheduled", "reschedule_send", "unschedule_send", "cancel_send",
            "backup_start", "storage", "clear_cache", "rebuild_index", "preload", "attachment_file",
            "summary_rules", "save_summary_rule", "remove_summary_rule", "automatic_summaries",
            "set_automatic_summaries", "discard_summary", "ai_summaries", "ai_summarize", "ask_moss",
            "pick_recipient", "recipient_picks",
        ] {
            let error = invoke(&webview, command, json!({})).unwrap_err();
            assert!(error.as_str().is_some_and(|error| error.contains("state") && error.contains("engine")), "{command}: {error}");
        }
    }

    #[test]
    fn malformed_renderer_arguments_never_reach_account_or_mail_mutations() {
        let a=app();
        let accounts=a.call("accounts",json!({})).unwrap();
        for command in ["add_password_account","check_resend_key","start_browser_sign_in","add_resend_account","discover","update_account","reorder_accounts","remove_account","add_address","update_address","remove_address","block_address","discover_addresses","set_address_signatures","thread","mark_done","mark_not_done","delete_thread","restore_thread","done_threads","block_sender","accept_sender","unblock_sender","forget_sender_rule","blocked_threads","save_draft","delete_draft","attach_files","discard_draft_file","draft_preview","send_after","schedule_send","reschedule_send","unschedule_send","cancel_send","mark_read","snooze","move_thread","wait_for_reply","correct","allow_images","save_signature","delete_signature","set_signature_choice","search","sync_progress","server_change_status","retry_server_change","discard_server_change","dismiss_interrupted_sends","ai_choose","ai_plan","ai_remove_model","ai_download_models","ai_stop_model","ai_ask"] {
            let error=a.call(command,json!({})).unwrap_err();
            assert!(error.as_str().is_some_and(|message|message.contains("invalid args")),"{command}: {error}");
        }
        assert_eq!(a.call("accounts",json!({})).unwrap(),accounts);
        assert_eq!(a.call("drafts",json!({})).unwrap(),json!([]));
        assert_eq!(a.call("scheduled",json!({})).unwrap(),json!([]));
    }

    #[test]
    fn mcp_commands_refuse_requests_before_the_server_is_initialized() {
        let app = with_commands(mock_builder()).build(mock_context(noop_assets())).unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        for command in ["mcp_status", "mcp_set_on", "mcp_new_token"] {
            let error = invoke(&webview, command, json!({"on": true})).unwrap_err();
            assert!(error.as_str().is_some_and(|error| error.contains("state") && error.contains("mcp")), "{command}: {error}");
        }
    }

    #[test]
    fn backup_commands_keep_status_and_publish_a_verified_archive() {
        let a = app();
        assert_eq!(a.call("backup_status", json!({})).unwrap(), json!({"state":"idle"}));
        assert_eq!(a.call("backup_status", json!({})).unwrap(), json!({"state":"idle"}));
        assert_eq!(a.call("backup_cancel", json!({"id":1})).unwrap(), false);
        let output = a._dir.path().join("mail.backup");
        let forged = a._dir.path().join("renderer-chosen.backup");
        let id = a.call("backup_start", json!({"destination":forged})).unwrap();
        assert_eq!(id, 1);
        let status = tauri::async_runtime::block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let status = a.call("backup_status", json!({})).unwrap();
                    if status["state"] != "running" { break status; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap()
        });
        assert_eq!(status["state"], "complete");
        assert_eq!(status["id"], id);
        assert!(!forged.exists(), "renderer must not choose the backup destination");
        let verified = moss_store::verify_backup(&output, status["sha256"].as_str(), moss_store::BackupVerification::default()).unwrap();
        assert_eq!(status["bytes"], verified.info().bytes);
        assert_eq!(a.call("backup_status", json!({})).unwrap(), status);
        assert_eq!(a.call("backup_start", json!({})).unwrap_err(), "That file already exists. Choose a new name for the archive.");
        assert_eq!(a.call("backup_status", json!({})).unwrap(), status);
        assert_eq!(a.call("backup_cancel", json!({"id":1})).unwrap(), false);
        on_run_event(a.app.handle(), tauri::RunEvent::Exit);
        assert_eq!(a.call("backup_start", json!({"destination":a._dir.path().join("after-quit")})).unwrap_err(), "Moss is quitting. A backup cannot start.");
        assert!(!a._dir.path().join("after-quit").exists());
    }

    #[test]
    fn backup_picker_rejects_dangling_links_and_uncheckable_paths_before_starting() {
        for link in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let destination = if link {
                let path = dir.path().join("dangling");
                std::os::unix::fs::symlink(dir.path().join("missing"), &path).unwrap();
                path
            } else {
                let parent = dir.path().join("regular-file");
                std::fs::write(&parent, b"preserve").unwrap();
                parent.join("archive")
            };
            let a = app_in_with_dialog(dir, Some(backups::BackupDialog(Box::new(move |selected| selected(Ok(Some(destination.clone())))))));
            let expected = if link { "That file already exists. Choose a new name for the archive." } else { "The archive destination could not be checked." };
            assert_eq!(a.call("backup_start", json!({})).unwrap_err(), expected);
            assert_eq!(a.call("backup_status", json!({})).unwrap()["state"], "idle");
        }
    }

    #[test]
    fn backup_picker_cancellation_and_failure_never_start_a_job() {
        for selection in [Ok(None), Err("Picker unavailable".to_string())] {
            let supplied = selection.clone();
            let a = app_in_with_dialog(tempfile::tempdir().unwrap(), Some(backups::BackupDialog(Box::new(move |selected| selected(supplied.clone())))));
            let result = a.call("backup_start", json!({}));
            assert_eq!(result, selection.map(|_| Value::Null).map_err(Value::String));
            assert_eq!(a.call("backup_status", json!({})).unwrap()["state"], "idle");
            assert!(!a._dir.path().join("mail.backup").exists());
        }
        let a = app_in_with_dialog(tempfile::tempdir().unwrap(), Some(backups::BackupDialog(Box::new(drop))));
        assert_eq!(a.call("backup_start", json!({})).unwrap_err(), "The archive destination could not be selected.");
        assert_eq!(a.call("backup_status", json!({})).unwrap()["state"], "idle");
        assert!(!a._dir.path().join("mail.backup").exists());
    }

    #[test]
    fn recovery_status_command_reads_persistent_metadata_without_running_changes() {
        let a = app();
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        store.upsert_account(&work_account()).unwrap();
        let id = store.queue_change("work", "private payload must not enter status", 1).unwrap();
        store.block_change("work", id, 0, moss_store::ChangeFailure::Unreadable).unwrap();
        assert_eq!(a.call("server_change_status", json!({"account":"work"})).unwrap(), json!({
            "accountId":"work", "change":id.to_hex(), "revision":1, "state":"blocked", "failure":"unreadable", "kind":"legacy",
            "completed":0, "pending":1, "discarded":0, "dropped":0
        }));
        assert_eq!(store.pending_changes("work").unwrap().len(), 1);
        assert_eq!(a.kind("server_change_status", json!({"account":"removed"})), "unknownAccount");
    }

    #[test]
    fn a_refused_server_change_is_tried_again_discarded_or_lost_with_its_account_only_when_asked() {
        let a = app();
        cleanup_fixture(&a);
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        let refused = store.queue_change("work", "refused", 1).unwrap();
        let revision = store.block_change("work", refused, 0, moss_store::ChangeFailure::Refused).unwrap();
        let later = store.queue_change("work", "later", 2).unwrap();
        let args = |revision: i64| json!({"account":"work", "change":refused.to_hex(), "revision":revision});
        // Retry and Discard each start the account's sender in the background. It reports
        // how its attempt ended with a sync error (these changes can't be read, so each is
        // blocked in turn); waiting for that keeps the test from racing it for the queue.
        let (tx, attempts) = std::sync::mpsc::channel();
        a.app.listen(EVENT, move |e| {
            let event: Value = serde_json::from_str(e.payload()).unwrap();
            if event["type"] == "syncError" && event["accountId"] == "work" { let _ = tx.send(()); }
        });
        let attempted = || attempts.recv_timeout(std::time::Duration::from_secs(10)).expect("the sender's attempt ends");
        assert_eq!(a.kind("retry_server_change", args(revision + 1)), "invalid");
        assert_eq!(a.kind("discard_server_change", json!({"account":"work", "change":"not a change", "revision":revision})), "invalid");
        assert_eq!(a.call("retry_server_change", args(revision)).unwrap(), Value::Null);
        // Tried again at once, as asked: still unreadable, so blocked again, at a newer revision.
        attempted();
        let head = store.earliest_change("work").unwrap().unwrap();
        assert_eq!((head.id, head.failure), (refused, Some(moss_store::ChangeFailure::Unreadable)));
        assert!(head.revision > revision);
        assert_eq!(a.kind("discard_server_change", args(revision)), "invalid", "only as the window last saw it");
        assert_eq!(a.call("discard_server_change", args(head.revision)).unwrap(), Value::Null);
        // The later change gets its turn.
        attempted();
        assert_eq!(store.pending_changes("work").unwrap(), [(later, "later".into())]);

        let refused = a.call("remove_account", json!({"id":"work"})).unwrap_err();
        assert_eq!(refused, json!({"kind":"unsentServerChanges", "count":1, "message":"1 change hasn't reached the server and will be lost."}));
        assert_eq!(a.kind("remove_account", json!({"id":"work", "discardServerChanges":2})), "invalid");
        assert_eq!(a.call("accounts", json!({})).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(a.call("remove_account", json!({"id":"work", "discardServerChanges":1})).unwrap(), json!({"pending":false}));
        assert_eq!(a.call("accounts", json!({})).unwrap(), json!([]));
        assert!(store.pending_changes("work").unwrap().is_empty());
    }

    #[test]
    fn a_send_moss_stopped_is_listed_until_the_window_says_it_was_shown() {
        let a = app();
        cleanup_fixture(&a);
        assert_eq!(a.call("interrupted_sends", json!({})).unwrap(), json!([]));
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        let mut draft: moss_core::Draft = serde_json::from_value(json!({
            "id":"held", "mode":"new", "threadId":null, "fromAccountId":"work", "to":[], "cc":[], "bcc":[],
            "subject":"Contract", "body":"", "bodyHtml":"", "remindIfNoReply":false, "updatedAt":1,
            "signatureId":null, "signatureChosen":true, "fromAddress":null, "attachments":[],
        })).unwrap();
        draft.revision = None;
        store.prepare_delivery("held-op", &draft, "{}", "held@moss.local", 1).unwrap();
        assert!(store.cancel_interrupted_delivery("held-op", 7).unwrap());
        assert_eq!(a.call("interrupted_sends", json!({})).unwrap(), json!([{"id":"held-op", "subject":"Contract", "at":7}]));
        assert_eq!(a.call("dismiss_interrupted_sends", json!({"ids":["held-op"]})).unwrap(), Value::Null);
        assert_eq!(a.call("interrupted_sends", json!({})).unwrap(), json!([]));
    }

    fn cleanup_fixture(a: &App) -> moss_accounts::SecretRef {
        use moss_accounts::{AccountConnection, AuthMethod, MailService, SecretRef, SecretStore};
        let account = work_account();
        let key = SecretRef::for_account(&account.address, "imap");
        let connection = AccountConnection {
            email: account.address.clone(),
            kind: account.provider,
            service: MailService::Imap,
            servers: None,
            auth: AuthMethod::Password,
            secret: key.clone(),
        };
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        store.upsert_account(&account).unwrap();
        store.set_setting("connection:work", Some(&serde_json::to_string(&connection).unwrap())).unwrap();
        a.secrets.save(&key, &moss_accounts::Secret::Password { password: "cleanup-fixture".into() }).unwrap();
        key
    }

    #[test]
    fn cleanup_pending_and_retry_cross_the_real_command_boundary() {
        use moss_accounts::SecretStore;
        let a = app();
        let key = cleanup_fixture(&a);
        let db = rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER ignore_cleanup_ack BEFORE DELETE ON credential_cleanup BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert_eq!(a.call("remove_account", json!({"id":"work"})).unwrap(), json!({"pending":true}));
        assert_eq!(a.call("accounts", json!({})).unwrap(), json!([]));
        assert_eq!(a.call("credential_cleanup_status", json!({})).unwrap(), json!({"pending":true}));
        assert_eq!(a.call("retry_credential_cleanup", json!({})).unwrap(), json!({"pending":true}));
        assert_eq!(a.secrets.load(&key).unwrap(), None);
        db.execute_batch("DROP TRIGGER ignore_cleanup_ack").unwrap();
        assert_eq!(a.call("retry_credential_cleanup", json!({})).unwrap(), json!({"pending":false}));
        assert_eq!(a.call("credential_cleanup_status", json!({})).unwrap(), json!({"pending":false}));
        assert_eq!(a.call("accounts", json!({})).unwrap(), json!([]));
    }

    #[test]
    fn a_missing_cleanup_intent_prevents_native_removal_and_keeps_the_sign_in() {
        use moss_accounts::SecretStore;
        let a = app();
        let key = cleanup_fixture(&a);
        let db = rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER ignore_cleanup_intent BEFORE INSERT ON credential_cleanup BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert_eq!(a.kind("remove_account", json!({"id":"work"})), "invalid");
        assert_eq!(a.call("accounts", json!({})).unwrap().as_array().unwrap().len(), 1);
        assert!(a.secrets.load(&key).unwrap().is_some());
        assert_eq!(a.call("credential_cleanup_status", json!({})).unwrap(), json!({"pending":false}));
        db.execute_batch("DROP TRIGGER ignore_cleanup_intent").unwrap();
        assert_eq!(a.call("remove_account", json!({"id":"work"})).unwrap(), json!({"pending":false}));
        assert!(a.secrets.load(&key).unwrap().is_none());
    }

    #[test]
    fn native_removal_refuses_a_credential_reference_from_another_application() {
        use moss_accounts::SecretStore;
        let a = app();
        let key = cleanup_fixture(&a);
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        let original = store.setting("connection:work").unwrap().unwrap();
        let mut connection: moss_accounts::AccountConnection = serde_json::from_str(&original).unwrap();
        connection.secret.service = "another.application".into();
        store.set_setting("connection:work", Some(&serde_json::to_string(&connection).unwrap())).unwrap();
        assert_eq!(a.kind("remove_account", json!({"id":"work"})), "invalid");
        assert!(a.secrets.load(&key).unwrap().is_some());
        assert_eq!(a.call("accounts", json!({})).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(a.call("credential_cleanup_status", json!({})).unwrap(), json!({"pending":false}));
        store.set_setting("connection:work", Some(&original)).unwrap();
        assert_eq!(a.call("remove_account", json!({"id":"work"})).unwrap(), json!({"pending":false}));
        assert!(a.secrets.load(&key).unwrap().is_none());
    }

    #[test]
    fn unreadable_cleanup_metadata_cannot_report_native_cleanup_complete() {
        use moss_accounts::SecretStore;
        let a = app();
        let key = cleanup_fixture(&a);
        let db = rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
        db.execute_batch("ALTER TABLE credential_cleanup RENAME TO retained_cleanup").unwrap();
        assert_eq!(a.kind("credential_cleanup_status", json!({})), "store");
        assert_eq!(a.call("retry_credential_cleanup", json!({})).unwrap(), json!({"pending":true}));
        assert_eq!(a.call("accounts", json!({})).unwrap().as_array().unwrap().len(), 1);
        assert!(a.secrets.load(&key).unwrap().is_some());
        assert_eq!(a.kind("remove_account", json!({"id":"work"})), "store");
        assert!(a.secrets.load(&key).unwrap().is_some());
        db.execute_batch("ALTER TABLE retained_cleanup RENAME TO credential_cleanup").unwrap();
        assert_eq!(a.call("credential_cleanup_status", json!({})).unwrap(), json!({"pending":false}));
        assert_eq!(a.call("remove_account", json!({"id":"work"})).unwrap(), json!({"pending":false}));
        assert!(a.secrets.load(&key).unwrap().is_none());
    }

    #[test]
    fn cleanup_commands_refuse_requests_before_the_engine_is_available() {
        let app = with_commands(mock_builder()).build(mock_context(noop_assets())).unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        for command in ["credential_cleanup_status", "retry_credential_cleanup"] {
            let error = invoke(&webview, command, json!({})).unwrap_err();
            assert!(error.as_str().unwrap().contains("state"));
            assert!(!error.to_string().contains("pending"));
        }
    }

    fn held_args(a: &App, hold: &str, delay: u64) -> Value {
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        let account = work_account();
        store.upsert_account(&account).unwrap();
        let connection = moss_accounts::AccountConnection {
            email: account.address.clone(), kind: account.provider,
            service: moss_accounts::MailService::Resend, servers: None,
            auth: moss_accounts::AuthMethod::ApiKey,
            secret: moss_accounts::SecretRef::for_account(&account.address, "fixture"),
        };
        store.set_setting("connection:work", Some(&serde_json::to_string(&connection).unwrap())).unwrap();
        let draft = json!({"mode":"new", "fromAccountId":"work", "to":[{"name":"Dana", "email":"dana@example.test"}], "cc":[], "bcc":[], "subject":"Held", "body":"Hello", "bodyHtml":"Hello", "remindIfNoReply":false});
        let mut recovery = draft.clone();
        recovery["id"] = json!(format!("draft-{hold}"));
        recovery["updatedAt"] = json!(1);
        json!({"hold":hold, "draft":draft, "recovery":recovery, "delayMs":delay})
    }

    #[test]
    fn sending_a_saved_draft_cannot_overtake_its_existing_hold() {
        let a = app();
        let args = held_args(&a, "saved-hold", 0);
        a.app.state::<Engine>().recover_deliveries().unwrap();
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        let recovery: moss_core::Draft = serde_json::from_value(args["recovery"].clone()).unwrap();
        assert!(store.prepare_delivery("saved-hold", &recovery, "{}", "held@example.test", 1).unwrap());
        assert_eq!(a.kind("send", json!({"draft":args["draft"], "recovery":args["recovery"]})), "deliveryBusy");
        assert_eq!(store.delivery_state("saved-hold").unwrap(), Some(moss_store::DeliveryState::Held));
        assert_eq!(store.pending_deliveries().unwrap().len(), 1);
    }

    #[test]
    fn a_second_send_of_a_waiting_or_uncertain_email_never_submits_it_again() {
        let a = app();
        let args = held_args(&a, "repeat-hold", 0);
        let recovery: moss_core::Draft = serde_json::from_value(args["recovery"].clone()).unwrap();
        let email: OutgoingDraft = serde_json::from_value(args["draft"].clone()).unwrap();
        let engine = a.app.state::<Engine>();
        assert!(tauri::async_runtime::block_on(engine.prepare_send("repeat-hold", &recovery, &email)).unwrap());
        assert_eq!(a.kind("send_after", args.clone()), "invalid");
        let mut store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        assert!(store.claim_delivery("repeat-hold").unwrap());
        assert!(store.uncertain_delivery("repeat-hold", "no receipt").unwrap());
        assert_eq!(a.kind("send_after", args), "deliveryUnknown");
        assert_eq!(store.delivery_state("repeat-hold").unwrap(), Some(moss_store::DeliveryState::Uncertain));
    }

    #[test]
    fn a_busy_submission_keeps_the_desktop_hold_owned() {
        let a = app();
        let args = held_args(&a, "busy-hold", 0);
        let database = a._dir.path().join("data/moss.db");
        let store = moss_store::Store::open(&database).unwrap();
        let connection: moss_accounts::AccountConnection = serde_json::from_str(&store.setting("connection:work").unwrap().unwrap()).unwrap();
        a.secrets.save(&connection.secret, &moss_accounts::Secret::ApiKey { key: "fixture-no-network".into() }).unwrap();
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute_batch("CREATE TRIGGER lose_claim BEFORE UPDATE OF state ON deliveries WHEN OLD.state = 'held' AND NEW.state = 'claimed' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert_eq!(a.kind("send_after", args), "deliveryBusy");
        assert_eq!(store.delivery_state("busy-hold").unwrap(), Some(moss_store::DeliveryState::Held));
        assert!(store.drafts(None).unwrap().is_empty(), "busy is not permission to restore another owner's draft");
    }

    #[test]
    fn a_second_desktop_engine_cannot_open_an_active_data_folder() {
        let a = app();
        let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
        let events = Arc::new(WindowEvents(a.app.handle().clone()));
        let second = engine(&a._dir.path().join("data"), secrets, events, TlsRoots::Platform);
        assert!(second.is_err(), "startup must acquire ownership before any recovery or commands");
    }

    #[test]
    fn stopped_send_replays_and_released_timers_report_cancelled_without_restoring() {
        let a=app();
        let args=held_args(&a,"stopped-replay",0);
        let recovery:moss_core::Draft=serde_json::from_value(args["recovery"].clone()).unwrap();
        let email:OutgoingDraft=serde_json::from_value(args["draft"].clone()).unwrap();
        let engine=a.app.state::<Engine>();
        assert!(tauri::async_runtime::block_on(engine.prepare_send("stopped-replay",&recovery,&email)).unwrap());
        let db=rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert_eq!(a.call("cancel_send",json!({"hold":"stopped-replay"})).unwrap()["restoreFailed"],true);
        assert_eq!(a.call("send_after",args).unwrap(),json!(false));
        assert_eq!(tauri::async_runtime::block_on(submit_held(&engine,"stopped-replay")).unwrap(),Sent::TakenBack);
        let store=moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        let connection:moss_accounts::AccountConnection=serde_json::from_str(&store.setting("connection:work").unwrap().unwrap()).unwrap();
        a.secrets.save(&connection.secret,&moss_accounts::Secret::ApiKey { key:"fixture-no-network".into() }).unwrap();
        assert_eq!(tauri::async_runtime::block_on(submit_held(&engine,"stopped-replay")).unwrap(),Sent::TakenBack,"a refused submission claim is still cancelled, not busy");
        assert_eq!(engine.delivery_state("stopped-replay").unwrap(),Some(moss_store::DeliveryState::Held));
        assert_eq!(db.query_row("SELECT count(*) FROM drafts WHERE id=?1",[&recovery.id],|r|r.get::<_,i64>(0)).unwrap(),0);
    }

    #[test]
    fn failed_early_undo_write_still_prevents_later_timer_registration() {
        let a=app();
        let args=held_args(&a,"before-registration",60_000);
        let recovery:moss_core::Draft=serde_json::from_value(args["recovery"].clone()).unwrap();
        let email:OutgoingDraft=serde_json::from_value(args["draft"].clone()).unwrap();
        let engine=a.app.state::<Engine>();
        assert!(tauri::async_runtime::block_on(engine.prepare_send("before-registration",&recovery,&email)).unwrap());
        let db=rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_early_intent BEFORE INSERT ON delivery_cancellations BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert!(a.call("cancel_send",json!({"hold":"before-registration"})).is_err());
        let outbox=a.app.state::<Outbox>();
        assert!(tauri::async_runtime::block_on(outbox.hold("before-registration",std::time::Duration::ZERO)).is_none());
        assert_eq!(outbox.waiting(),0);
        assert_eq!(engine.delivery_state("before-registration").unwrap(),Some(moss_store::DeliveryState::Held));
        assert_eq!(db.query_row("SELECT count(*) FROM delivery_cancellations",[],|r|r.get::<_,i64>(0)).unwrap(),0,"never pretend the failed intent was durable");
    }

    #[test]
    fn failed_undo_restoration_stops_the_desktop_timer() {
        for intent_failure in [false, true] {
        let a = app();
        let args = held_args(&a, "failed-undo", 60_000);
        let webview = a.webview.clone();
        std::thread::scope(|scope| {
            let held = scope.spawn(move || invoke(&webview, "send_after", args));
            let give_up = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while a.app.state::<Outbox>().waiting() == 0 {
                assert!(std::time::Instant::now() < give_up, "never held");
                std::thread::yield_now();
            }
            let db = rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
            db.execute_batch(if intent_failure { "CREATE TRIGGER refuse_intent BEFORE INSERT ON delivery_cancellations BEGIN SELECT RAISE(ABORT, 'fixture'); END;" } else { "CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'fixture'); END;" }).unwrap();
            let outcome = a.call("cancel_send", json!({"hold":"failed-undo"}));
            if intent_failure { assert!(outcome.is_err(), "failed intent must not confirm cancellation"); }
            else { let outcome=outcome.unwrap(); assert_eq!(outcome["restoreFailed"], true); assert_eq!(outcome["cancelled"], true); }
            // Always release the fixture before asserting, even on the regression path.
            let timer_was_still_live = a.app.state::<Outbox>().cancel("failed-undo");
            assert_eq!(held.join().unwrap().unwrap(), json!(false));
            assert!(!timer_was_still_live, "failed restoration must not leave a timer free to send");
        });
        }
    }

    #[test]
    fn sent_email_waits_for_undo_then_goes() {
        let a = app();
        let mut now = held_args(&a, "h1", 0);
        now["draft"]["to"] = json!([]);
        assert_eq!(a.kind("send_after", now), "noRecipients");
        assert_eq!(a.call("cancel_send", json!({"hold": "h1"})).unwrap()["cancelled"], json!(true));
        let webview = a.webview.clone();
        let args = held_args(&a, "h2", 60_000);
        std::thread::scope(|scope| {
            let held = scope.spawn(move || invoke(&webview, "send_after", args));
            let give_up = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while a.app.state::<Outbox>().waiting() == 0 {
                assert!(std::time::Instant::now() < give_up, "never held");
                std::thread::yield_now();
            }
            assert_eq!(a.call("cancel_send", json!({"hold": "h2"})).unwrap()["cancelled"], json!(true));
            assert_eq!(held.join().unwrap().unwrap(), json!(false));
            let store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
            assert_eq!(store.delivery("h2").unwrap().unwrap().state, moss_store::DeliveryState::Cancelled);
            assert_eq!(store.drafts(None).unwrap()[0].body, "Hello");
        });
    }

    #[test]
    fn quit_commands_require_current_tokens_and_do_not_exit_before_confirmation() {
        let a=app();let seen=Arc::new(std::sync::Mutex::new(Vec::new()));let sink=seen.clone();
        a.app.manage(quit_gate::Actions(Arc::new(move |effect|sink.lock().unwrap().push(effect))));
        assert_eq!(a.call("quit_ready",json!({"token":"stale"})).unwrap(),json!(false));
        a.call("request_restart",json!({})).unwrap();
        assert_eq!(*seen.lock().unwrap(),vec![quit_gate::Effect::Ask("1".into())]);
        assert_eq!(a.call("quit_blocked",json!({"token":"1"})).unwrap(),json!(true));
        assert_eq!(a.call("quit_cancel",json!({"token":"1"})).unwrap(),json!(true));
        assert_eq!(a.call("quit_ready",json!({"token":"1"})).unwrap(),json!(false));
        assert_eq!(seen.lock().unwrap().len(),3);
        a.call("request_restart",json!({})).unwrap();
        a.call("quit_ready",json!({"token":"2"})).unwrap();
        assert_eq!(seen.lock().unwrap().last(),Some(&quit_gate::Effect::Finish { restart:true }));
    }

    #[test]
    fn delivery_review_commands_preserve_unsent_copies_and_validate_actions() {
        let a=app();let args=held_args(&a,"native-review",0);
        let recovery:moss_core::Draft=serde_json::from_value(args["recovery"].clone()).unwrap();
        let email:OutgoingDraft=serde_json::from_value(args["draft"].clone()).unwrap();
        let engine=a.app.state::<Engine>();
        tauri::async_runtime::block_on(engine.prepare_send("native-review",&recovery,&email)).unwrap();
        let db=rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert_eq!(a.call("cancel_send",json!({"hold":"native-review"})).unwrap()["restoreFailed"],true);
        assert_eq!(a.call("delivery_attention",json!({"after":null})).unwrap()["entries"][0]["id"],"native-review");
        assert_eq!(a.call("delivery_attention_draft",json!({"id":"native-review"})).unwrap()["body"],recovery.body);
        assert!(a.call("resolve_delivery",json!({"id":"native-review","action":"invalid"})).is_err());
        let outcome=a.call("resolve_delivery",json!({"id":"native-review","action":"copied"})).unwrap();
        assert_eq!(outcome["status"],"resolved");
        assert_eq!(a.call("local_draft_recovery",json!({"id":outcome["recoveryId"]})).unwrap()["draft"]["body"],recovery.body);
        assert_eq!(a.call("delivery_attention",json!({"after":null})).unwrap()["entries"],json!([]));
        assert_eq!(a.call("delivery_attention_draft",json!({"id":"native-review"})).unwrap(),Value::Null);
        assert_eq!(a.call("resolve_delivery",json!({"id":"native-review","action":"copied"})).unwrap(),outcome);
    }

    #[test]
    fn local_draft_recovery_commands_preserve_conflicts_until_explicit_adoption() {
        let a=app();let original=held_args(&a,"journal-native",0)["recovery"].clone();
        let winner=a.call("save_draft",json!({"draft":original})).unwrap();
        let mut local=winner.clone();local["body"]=json!("Local conflict text");
        assert_eq!(a.call("local_draft_recovery_index",json!({})).unwrap(),json!([]));
        assert_eq!(a.kind("keep_local_draft_recovery",json!({"id":"","previous":null,"draft":local})),"localRecoveryRejected");
        let kept=a.call("keep_local_draft_recovery",json!({"id":"native-recovery","previous":null,"draft":local})).unwrap();
        assert_eq!(kept["status"],"saved");let recovery=&kept["recovery"];
        assert_eq!(a.call("local_draft_recovery",json!({"id":"native-recovery"})).unwrap(),*recovery);
        assert_eq!(a.call("drafts",json!({})).unwrap(),json!([winner]));
        assert_eq!(a.call("local_draft_recovery_index",json!({})).unwrap()[0]["version"],recovery["version"]);
        assert_eq!(a.kind("adopt_local_draft_recovery",json!({"id":"native-recovery","version":recovery["version"],"newId":"","account":"work","missing":[]})),"localRecoveryRejected");
        assert_eq!(a.kind("adopt_local_draft_recovery",json!({"id":"native-recovery","version":recovery["version"],"newId":winner["id"],"account":"work","missing":[]})),"localRecoveryRejected");
        let occupied=held_args(&a,"occupied-adoption",0)["recovery"].clone();
        a.call("save_draft",json!({"draft":occupied})).unwrap();
        assert_eq!(a.kind("adopt_local_draft_recovery",json!({"id":"native-recovery","version":recovery["version"],"newId":occupied["id"],"account":"work","missing":[]})),"localRecoveryRejected");
        a.call("delete_draft",json!({"id":occupied["id"],"revision":a.call("drafts",json!({})).unwrap().as_array().unwrap().iter().find(|row|row["id"]==occupied["id"]).unwrap()["revision"]})).unwrap();
        assert_eq!(a.kind("adopt_local_draft_recovery",json!({"id":"native-recovery","version":"stale","newId":"native-adopted","account":"work","missing":[]})),"localRecoveryRejected");
        assert_eq!(a.kind("adopt_local_draft_recovery",json!({"id":"native-recovery","version":recovery["version"],"newId":"native-adopted","account":"missing-account","missing":[]})),"localRecoveryRejected");
        assert_eq!(a.kind("keep_local_draft_recovery",json!({"id":"native-recovery","previous":"stale","draft":winner})),"store");
        let db=rusqlite::Connection::open(a._dir.path().join("data/moss.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_adoption BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        assert_eq!(a.kind("adopt_local_draft_recovery",json!({"id":"native-recovery","version":recovery["version"],"newId":"native-adopted","account":"work","missing":[]})),"store");
        assert_eq!(a.call("local_draft_recovery",json!({"id":"native-recovery"})).unwrap(),*recovery,"failed adoption must keep the complete recoverable snapshot");
        db.execute_batch("DROP TRIGGER refuse_adoption").unwrap();
        let adopted=a.call("adopt_local_draft_recovery",json!({"id":"native-recovery","version":recovery["version"],"newId":"native-adopted","account":"work","missing":[]})).unwrap();
        assert_eq!(adopted["body"],"Local conflict text");assert_eq!(adopted["mode"],"new");
        assert_eq!(a.call("keep_local_draft_recovery",json!({"id":"native-recovery","previous":null,"draft":local})).unwrap(),json!({"status":"resolved","adoptedId":"native-adopted"}));
        let disposable=a.call("keep_local_draft_recovery",json!({"id":"native-discard","previous":null,"draft":local})).unwrap();
        a.call("discard_local_draft_recovery",json!({"id":"native-discard","version":disposable["recovery"]["version"]})).unwrap();
        assert_eq!(a.call("keep_local_draft_recovery",json!({"id":"native-discard","previous":null,"draft":local})).unwrap(),json!({"status":"resolved","adoptedId":null}));
        assert_eq!(a.call("local_draft_recovery_index",json!({})).unwrap(),json!([]));
    }

    #[test]
    fn recovery_commands_reject_malformed_ipc_without_mutating_storage() {
        let a=app();
        for (command,args) in [
            ("keep_local_draft_recovery",json!({"id":5,"previous":null,"draft":null})),
            ("local_draft_recovery",json!({"id":5})),
            ("discard_local_draft_recovery",json!({"id":5,"version":null})),
            ("adopt_local_draft_recovery",json!({"id":5,"version":null,"newId":null,"account":null,"missing":null})),
        ] {
            let error=a.call(command,args).unwrap_err();
            assert!(error.as_str().is_some_and(|message|message.contains("invalid args")),"{command}: {error}");
        }
        assert_eq!(a.call("local_draft_recovery_index",json!({})).unwrap(),json!([]));
        assert_eq!(a.call("drafts",json!({})).unwrap(),json!([]));
    }

    #[test]
    fn lost_draft_save_acknowledgement_can_be_read_without_overwriting() {
        let a = app();
        let attempted = held_args(&a, "receipt-native", 0)["recovery"].clone();
        let ack = a.call("save_draft", json!({"draft": attempted})).unwrap();
        assert_eq!(a.call("draft_save_receipt", json!({"draft": attempted})).unwrap(), ack);
        assert_eq!(a.kind("save_draft", json!({"draft": attempted})), "draftConflict");
        let mut changed = ack.clone(); changed["body"] = json!("Newer edit");
        let winner = a.call("save_draft", json!({"draft": changed})).unwrap();
        assert_eq!(a.call("draft_save_receipt", json!({"draft": attempted})).unwrap(), Value::Null);
        assert_eq!(a.call("drafts", json!({})).unwrap(), json!([winner]));
    }

    #[test]
    fn a_held_email_answers_false_when_taken_back_and_its_conversation_once_gone() {
        // The screen opens that conversation; false keeps the answer Undo always had.
        assert_eq!(serde_json::to_value(Sent::TakenBack).unwrap(), json!(false));
        assert_eq!(serde_json::to_value(Sent::Into("t-42".into())).unwrap(), json!("t-42"));
    }

    #[test]
    fn undo_arriving_before_the_send_command_prevents_the_later_handoff() {
        let a = app();
        let args = held_args(&a, "early-undo", 0);
        a.call("save_draft", json!({"draft": args["recovery"]})).unwrap();
        assert_eq!(a.call("cancel_send", json!({"hold": "early-undo"})).unwrap()["cancelled"], json!(true));
        for _ in 0..2 {
            assert_eq!(a.call("send_after", args.clone()).unwrap(), json!(false));
        }
        let store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
        assert!(store.delivery("early-undo").unwrap().is_none());
        assert_eq!(store.delivery_state("early-undo").unwrap(), Some(moss_store::DeliveryState::Cancelled));
        assert_eq!(store.drafts(None).unwrap()[0].body, "Hello");
    }

    #[test]
    fn quitting_sends_email_still_waiting_for_undo() {
        let a = app();
        let webview = a.webview.clone();
        let args = held_args(&a, "h1", 20_000);
        let (prepared,receipt)=std::sync::mpsc::channel();
        a.app.listen("moss://send-prepared",move |event|prepared.send(event.payload().to_string()).unwrap());
        std::thread::scope(|scope| {
            let held = scope.spawn(move || invoke(&webview, "send_after", args));
            let give_up = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while a.app.state::<Outbox>().waiting() == 0 {
                assert!(std::time::Instant::now() < give_up, "never held");
                std::thread::yield_now();
            }
            assert_eq!(receipt.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),"\"h1\"");
            let store=moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
            assert_eq!(store.delivery("h1").unwrap().unwrap().state,moss_store::DeliveryState::Held);
            drop(store);
            // Only quitting lets it go early.
            on_run_event(a.app.handle(), tauri::RunEvent::Ready);
            assert_eq!(a.app.state::<Outbox>().waiting(), 1);
            let start = std::time::Instant::now();
            on_run_event(a.app.handle(), tauri::RunEvent::Exit);
            // Quitting attempts submission immediately; this fixture has no saved credential.
            assert_eq!(held.join().unwrap().unwrap_err()["kind"], "signedOut");
            let store = moss_store::Store::open(a._dir.path().join("data/moss.db")).unwrap();
            assert_eq!(store.delivery("h1").unwrap().unwrap().state, moss_store::DeliveryState::Cancelled);
            assert_eq!(store.drafts(None).unwrap()[0].body, "Hello");
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
        });
    }

    #[test]
    fn the_ui_sets_the_dock_icon() {
        let a = app();
        assert_eq!(a.call("set_app_icon", json!({"icon": "dark"})).unwrap(), Value::Null);
        assert_eq!(a.call("set_app_icon", json!({"icon": "system"})).unwrap(), Value::Null);
        assert!(a.call("set_app_icon", json!({"icon": "blue"})).is_err());
    }

    #[test]
    fn the_window_asks_which_moss_it_is() {
        // The test app's identifier isn't production's, so it is Moss Dev.
        let a = app();
        let identity = a.call("app_identity", json!({})).unwrap();
        assert_eq!(identity, json!({"name": "Moss Dev", "dev": true, "updates": false, "agentName": "moss-dev"}));
    }

    #[test]
    fn signatures_are_kept_through_commands() {
        let a = app();
        let sig = json!({"id": "s1", "name": "Work", "lines": ["Maya Ross", "northwind.co"]});
        assert_eq!(a.call("save_signature", json!({"signature": sig})).unwrap(), Value::Null);
        assert_eq!(a.call("set_let_moss_choose", json!({"on": false})).unwrap(), Value::Null);
        assert_eq!(a.call("mark_signature_used", json!({"id": "s1"})).unwrap(), Value::Null);
        assert_eq!(
            a.call("signatures", json!({})).unwrap(),
            json!({"signatures": [sig], "byAccount": {}, "byAddress": {}, "letMossChoose": false, "recent": ["s1"]})
        );
        let choice = json!({"account": "gmail", "kind": "replies", "signature": "s1"});
        assert_eq!(a.kind("set_signature_choice", choice), "store");
        assert_eq!(a.call("delete_signature", json!({"id": "s1"})).unwrap(), Value::Null);
        assert_eq!(a.call("signatures", json!({})).unwrap()["signatures"], json!([]));
    }

    #[test]
    fn addresses_are_kept_through_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let mut store = moss_store::Store::open(dir.path().join("data/moss.db")).unwrap();
        store
            .upsert_account(&moss_core::Account {
                id: "icloud".into(),
                name: "iCloud".into(),
                letter: "I".into(),
                address: "maya@icloud.com".into(),
                sender_name: "Maya".into(),
                color: moss_core::AccountColor::Personal,
                provider: moss_core::ProviderKind::Imap,
                org: None,
                org_domain: None,
            })
            .unwrap();
        drop(store);
        let a = app_in(dir);
        let hi = "hi@rossstudio.art";
        let added = a.call("add_address", json!({"account": "icloud", "email": hi, "name": "Ross Studio"})).unwrap();
        assert_eq!(
            added["addresses"],
            json!([{"email": hi, "name": "Ross Studio", "state": "added", "source": "you", "seen": 0, "blocked": false}])
        );
        assert_eq!(a.call("accounts", json!({})).unwrap()[0]["addresses"], added["addresses"]);
        let renamed = a.call("update_address", json!({"account": "icloud", "email": hi, "name": null})).unwrap();
        assert!(renamed["addresses"][0].get("name").is_none());
        let sig = json!({"id": "s1", "name": "Studio", "lines": []});
        a.call("save_signature", json!({"signature": sig})).unwrap();
        let choice = json!({"newEmails": "s1", "replies": null});
        let args = json!({"account": "icloud", "email": hi, "choice": choice});
        assert_eq!(a.call("set_address_signatures", args).unwrap(), Value::Null);
        assert_eq!(a.call("signatures", json!({})).unwrap()["byAddress"][hi], choice);
        a.call("set_address_signatures", json!({"account": "icloud", "email": hi, "choice": null})).unwrap();
        assert_eq!(a.call("signatures", json!({})).unwrap()["byAddress"], json!({}));
        let blocked = a.call("block_address", json!({"account": "icloud", "email": "bookshop@rossstudio.art", "blocked": true})).unwrap();
        assert_eq!(blocked["addresses"][1]["blocked"], true);
        let found = a.call("discover_addresses", json!({"account": "icloud"})).unwrap();
        assert_eq!(found["addresses"].as_array().unwrap().len(), 2);
        let removed = a.call("remove_address", json!({"account": "icloud", "email": hi})).unwrap();
        assert_eq!(removed["addresses"].as_array().unwrap().len(), 1);
        assert_eq!(a.kind("add_address", json!({"account": "icloud", "email": "nope"})), "invalidAddress");
        assert_eq!(a.kind("add_address", json!({"account": "icloud", "email": "maya@icloud.com"})), "primaryAddress");
        assert_eq!(a.kind("remove_address", json!({"account": "icloud", "email": "x@y.co"})), "notYourAddress");
        assert_eq!(a.kind("discover_addresses", json!({"account": "nobody"})), "unknownAccount");
    }

    #[test]
    fn people_you_write_to_come_through_a_command() {
        assert_eq!(app().call("contacts", json!({})).unwrap(), json!([]));
    }

    #[test]
    fn who_you_picked_from_the_suggestions_comes_through_commands() {
        let a = app();
        assert_eq!(a.call("pick_recipient", json!({"typed": "ask", "email": "kindle@x.io"})).unwrap(), Value::Null);
        assert_eq!(a.call("recipient_picks", json!({})).unwrap(), json!([{"typed": "ask", "email": "kindle@x.io"}]));
    }

    #[test]
    fn drafts_are_kept_through_commands() {
        let a = app();
        assert_eq!(a.call("drafts", json!({})).unwrap(), json!([]));
        let draft = json!({
            "id": "d1", "mode": "new", "threadId": null, "fromAccountId": "gmail", "to": [], "cc": [],
            "bcc": [], "subject": "Lunch?", "body": "", "bodyHtml": "", "remindIfNoReply": false,
            "updatedAt": 1, "signatureId": null, "signatureChosen": false, "fromAddress": null
        });
        // No such account here, so the store refuses it.
        assert_eq!(a.kind("save_draft", json!({"draft": draft})), "store");
        assert_eq!(a.call("delete_draft", json!({"id": "d1"})).unwrap(), false);
        assert_eq!(a.call("delete_draft", json!({"id": "d1", "keepFiles": true})).unwrap(), false);
    }

    #[test]
    fn files_are_attached_and_drawn_through_commands() {
        let a = app();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"hi").unwrap();
        let files = a.call("attach_files", json!({"draftId": "d1", "paths": [path]})).unwrap();
        assert_eq!(files[0]["name"], "notes.txt");
        assert_eq!(files[0]["bytes"], 2);
        assert_eq!(files[0]["contentType"], "text/plain");
        assert_eq!(a.kind("draft_preview", json!({"draftId":"other", "id":files[0]["id"]})), "unavailable");
        // Text gets its icon, not a thumbnail.
        assert_eq!(a.call("draft_preview", json!({"draftId":"d1", "id": files[0]["id"]})).unwrap(), Value::Null);
        assert_eq!(a.kind("draft_preview", json!({"draftId":"d1", "id": "999"})), "unavailable");
        let missing = dir.path().join("gone.pdf");
        assert_eq!(a.kind("attach_files", json!({"draftId": "d1", "paths": [missing]})), "unavailable");
        assert_eq!(a.call("discard_draft_file", json!({"draftId": "other", "id": files[0]["id"]})).unwrap(), false);
        assert_eq!(a.call("discard_draft_file", json!({"draftId": "d1", "id": files[0]["id"]})).unwrap(), true);
        assert_eq!(a.kind("draft_preview", json!({"draftId":"d1", "id": files[0]["id"]})), "unavailable");
        assert_eq!(a.call("discard_draft_file", json!({"draftId": "d1", "id": files[0]["id"]})).unwrap(), false);
    }

    #[test]
    fn send_later_goes_through_commands() {
        let a = app();
        assert_eq!(a.call("scheduled", json!({})).unwrap(), json!([]));
        let draft = json!({
            "id": "d1", "mode": "new", "threadId": null, "fromAccountId": "gmail", "to": [], "cc": [],
            "bcc": [], "subject": "Lunch?", "body": "", "bodyHtml": "", "remindIfNoReply": false,
            "updatedAt": 1, "signatureId": null, "signatureChosen": false, "fromAddress": null
        });
        let email = json!({"mode": "new", "fromAccountId": "gmail", "to": [{"name": "Dana", "email": "dana@x.co"}], "subject": "Lunch?", "body": "", "bodyHtml": ""});
        // No such account here.
        assert_eq!(a.kind("schedule_send", json!({"operation": "op", "draft": draft, "email": email, "sendAt": 1})), "unknownAccount");
        assert_eq!(a.kind("reschedule_send", json!({"operation": "op", "id": "d1", "sendAt": 1, "expectedRevision": 1})), "invalid");
        assert_eq!(a.call("unschedule_send", json!({"operation": "op", "id": "d1"})).unwrap(), json!({"status": "cancelled", "early": true, "restored": null}));
    }

    #[test]
    fn schedule_cancellation_before_creation_is_retained_through_ipc() {
        let a = app();
        let cancelled = json!({"status": "cancelled", "early": true, "restored": null});
        assert_eq!(a.call("unschedule_send", json!({"operation": "early", "id": "d1"})).unwrap(), cancelled);
        assert_eq!(a.call("reschedule_send", json!({"operation": "early", "id": "d1", "sendAt": 10, "expectedRevision": 1})).unwrap(), cancelled);
        assert_eq!(a.call("scheduled", json!({})).unwrap(), json!([]));
    }

    #[test]
    fn desktop_reschedules_require_the_revision_the_caller_observed() {
        let a = app();
        let args = held_args(&a, "revision", 6000);
        a.call("schedule_send", json!({"operation": "revision", "draft": args["recovery"], "email": args["draft"], "sendAt": 4000000000000_i64})).unwrap();
        let id = "draft-revision";
        assert!(a.call("reschedule_send", json!({"operation": "revision", "id": id, "sendAt": 4000000001000_i64})).is_err());
        assert_eq!(a.call("reschedule_send", json!({"operation": "revision", "id": id, "sendAt": 4000000002000_i64, "expectedRevision": 1})).unwrap(), json!({"status": "rescheduled", "revision": 2}));
        let stale = a.call("reschedule_send", json!({"operation": "revision", "id": id, "sendAt": 4000000001000_i64, "expectedRevision": 1})).unwrap();
        assert_eq!(stale["status"], "changed");
        assert_eq!(stale["current"]["revision"], 2);
        assert_eq!(stale["current"]["sendAt"], 4000000002000_i64);
        assert_eq!(a.call("scheduled", json!({})).unwrap()[0]["sendAt"], 4000000002000_i64);
    }

    #[test]
    fn every_command_reaches_the_engine() {
        let a = app();
        assert_eq!(a.call("app_info", json!({})).unwrap()["name"], "Moss");
        assert_eq!(a.call("accounts", json!({})).unwrap(), json!([]));
        assert_eq!(a.call("threads", json!({})).unwrap(), json!([]));
        assert_eq!(a.call("threads", json!({"account": "gmail"})).unwrap(), json!([]));
        assert_eq!(
            a.call("done_threads", json!({"account": null, "limit": 200})).unwrap(),
            json!([])
        );
        assert_eq!(
            a.call("done_threads", json!({"account": "gmail", "limit": 5})).unwrap(),
            json!([])
        );
        assert_eq!(
            a.call("split_counts", json!({})).unwrap(),
            json!({"important": 0, "other": 0, "calendar": 0, "newsletters": 0, "receipts": 0})
        );
        assert_eq!(a.call("search", json!({"query": "budget"})).unwrap(), json!([]));
        assert_eq!(
            a.call("search", json!({"query": "budget", "account": "x", "limit": 5})).unwrap(),
            json!([])
        );
        assert_eq!(a.call("sync_now", json!({})).unwrap(), Value::Null);
        assert_eq!(a.call("sync_progress", json!({"account": "gmail"})).unwrap(), Value::Null);
        assert_eq!(a.call("sync_problems", json!({})).unwrap(), json!([]));
        assert_eq!(a.call("reorder_accounts", json!({"ids": []})).unwrap(), Value::Null);
        assert_eq!(a.call("remove_account", json!({"id": "gmail"})).unwrap(), json!({"pending":false}));
        assert_eq!(a.call("credential_cleanup_status", json!({})).unwrap(), json!({"pending":false}));
        assert_eq!(a.call("retry_credential_cleanup", json!({})).unwrap(), json!({"pending":false}));

        assert_eq!(a.kind("thread", json!({"id": "1"})), "unknownThread");
        assert_eq!(a.kind("mark_done", json!({"id": "1"})), "unknownThread");
        assert_eq!(a.kind("mark_read", json!({"id": "1", "read": true})), "unknownThread");
        assert_eq!(a.kind("mark_not_done", json!({"id": "1"})), "unknownThread");
        assert_eq!(a.kind("delete_thread", json!({"id": "1"})), "unknownThread");
        assert_eq!(a.kind("restore_thread", json!({"id": "1"})), "unknownThread");
        assert!(a.call("done_threads", json!({"account": null})).is_err(), "limit is required");
        assert_eq!(a.kind("snooze", json!({"id": "1", "until": 5})), "unknownThread");
        assert_eq!(a.kind("move_thread", json!({"id": "1", "to": "important"})), "unknownThread");
        assert_eq!(a.kind("move_thread", json!({"id": "1", "to": null})), "unknownThread");
        assert_eq!(a.kind("wait_for_reply", json!({"id": "1", "until": 5})), "unknownThread");
        for correction in [
            json!({"kind": "section", "section": "info"}),
            json!({"kind": "split", "split": "other"}),
            json!({"kind": "noDeadline"}),
            json!({"kind": "remindIfNoReply", "on": true}),
        ] {
            assert_eq!(a.kind("correct", json!({"id": "1", "correction": correction})), "unknownThread");
        }
        assert_eq!(
            a.kind("block_sender", json!({"id": "1", "wholeDomain": true})),
            "unknownThread"
        );
        assert_eq!(
            a.kind("accept_sender", json!({"id": "1", "wholeDomain": true})),
            "unknownThread"
        );
        assert_eq!(a.kind("unblock_sender", json!({"id": "1"})), "unknownThread");
        assert_eq!(
            a.call("blocked_threads", json!({"account": null, "limit": 200})).unwrap(),
            json!([])
        );
        assert_eq!(a.call("sender_rules", json!({})).unwrap(), json!([]));
        assert_eq!(
            a.call("forget_sender_rule", json!({"account": "gmail", "sender": "x@y.z"})).unwrap(),
            Value::Null
        );
        assert_eq!(
            a.call("allow_images", json!({"account": "gmail", "email": "alex@studio-build.example.test"})).unwrap(),
            Value::Null
        );
        assert_eq!(
            a.kind("update_account", json!({"id": "x", "patch": {"name": "Home"}})),
            "unknownAccount"
        );
        let draft = json!({"mode": "new", "fromAccountId": "gmail", "to": [], "subject": "", "body": "", "bodyHtml": ""});
        assert_eq!(a.kind("send", json!({"draft": draft})), "noRecipients");
        let request = json!({"service": "gmail", "email": "nope", "password": "x"});
        assert_eq!(a.kind("add_password_account", json!({"request": request})), "invalidEmail");
        assert_eq!(a.kind("check_resend_key", json!({"key": " "})), "invalidResendKey");
        assert_eq!(
            a.kind("add_resend_account", json!({"key": "", "email": "nope"})),
            "invalidEmail"
        );
        assert_eq!(a.kind("discover", json!({"email": "nope"})), "invalidEmail");
        let outlook = json!({"request": {"provider": "outlook", "email": "nope"}});
        assert_eq!(a.kind("start_browser_sign_in", outlook), "invalidEmail");
        assert_eq!(a.kind("finish_browser_sign_in", json!({})), "invalid");
        let started = a.call("start_browser_sign_in", json!({"request": {"provider": "outlook"}})).unwrap();
        assert_eq!(started["clientId"], moss_accounts::MOSS_MICROSOFT_CLIENT_ID);
        assert!(started["url"]
            .as_str()
            .unwrap()
            .starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?"));
        assert_eq!(a.kind("attachment_file", json!({"messageId": "1", "index": 0})), "unavailable");
        assert_eq!(a.kind("attachment_preview", json!({"messageId": "1", "index": 0})), "unavailable");
        assert_eq!(a.kind("load_message", json!({"messageId": "1"})), "unavailable");
    }

    #[test]
    fn a_thumbnail_reaches_the_window_as_a_data_url() {
        assert_eq!(jpeg_data_url(b"JPEG"), "data:image/jpeg;base64,SlBFRw==");
    }

    #[test]
    fn storage_commands_reach_the_engine() {
        let a = app();
        let storage = a.call("storage", json!({})).unwrap();
        assert_eq!(storage["offline"], "text");
        assert_eq!(storage["cacheLimit"], 500_000_000);
        assert_eq!(storage["models"], 0);
        assert!(storage["usage"]["index"].as_u64().unwrap() > 0);
        assert_eq!(a.call("set_offline", json!({"offline": "recent"})).unwrap(), Value::Null);
        assert_eq!(a.call("set_cache_limit", json!({"bytes": 250_000_000})).unwrap(), Value::Null);
        let storage = a.call("storage", json!({})).unwrap();
        assert_eq!((storage["offline"].as_str(), storage["cacheLimit"].as_u64()), (Some("recent"), Some(250_000_000)));
        assert_eq!(a.call("clear_cache", json!({})).unwrap(), 0);
        assert_eq!(a.call("rebuild_index", json!({})).unwrap(), Value::Null);
        assert_eq!(a.call("preload", json!({"ids": ["1"]})).unwrap(), Value::Null);
        assert!(a.call("set_offline", json!({"offline": "floppy"})).is_err());
        assert_eq!(storage["attachmentText"], true);
        assert_eq!(a.call("set_attachment_text", json!({"on": false})).unwrap(), Value::Null);
        assert_eq!(a.call("storage", json!({})).unwrap()["attachmentText"], false);
    }

    #[test]
    fn commands_refuse_arguments_of_the_wrong_type() {
        // What the window sends is checked before it reaches the engine: an error, never a panic.
        let a = app();
        let first = [
            ("add_resend_account", "key"), ("update_account", "id"), ("add_address", "account"),
            ("update_address", "account"), ("remove_address", "account"), ("block_address", "account"),
            ("set_address_signatures", "account"), ("threads", "account"), ("split_counts", "account"),
            ("block_sender", "id"), ("accept_sender", "id"), ("forget_sender_rule", "account"),
            ("mark_read", "id"), ("snooze", "id"), ("move_thread", "id"), ("wait_for_reply", "id"),
            ("correct", "id"), ("allow_images", "account"), ("set_signature_choice", "account"),
            ("set_let_moss_choose", "on"), ("mark_signature_used", "id"), ("pick_recipient", "typed"), ("draft_save_receipt", "draft"),
            ("keep_local_draft_recovery", "id"), ("delivery_attention", "after"),
            ("delivery_attention_draft", "id"), ("discard_local_draft_recovery", "id"),
            ("adopt_local_draft_recovery", "id"), ("delete_draft", "id"), ("attach_files", "draftId"),
            ("discard_draft_file", "draftId"), ("draft_preview", "draftId"), ("send", "draft"),
            ("send_after", "hold"), ("reschedule_send", "operation"), ("schedule_send", "draft"),
            ("unschedule_send", "operation"), ("search", "query"), ("sync_now", "account"),
            ("set_attachment_text", "on"), ("set_cache_limit", "bytes"), ("preload", "ids"),
            ("attachment_file", "messageId"), ("attachment_preview", "messageId"), ("load_message", "messageId"), ("ask_moss", "chat"),
        ];
        for (command, arg) in first {
            let refused = a.call(command, json!({ arg: [[]] })).unwrap_err();
            let text = refused.as_str().unwrap_or_default();
            assert!(text.contains(&format!("invalid args `{arg}` for command `{command}`")), "{command}: {refused}");
        }
        // With the first one right, the next is checked too.
        let second = [
            ("add_resend_account", "key", "email"), ("remove_address", "account", "email"),
            ("allow_images", "account", "email"), ("forget_sender_rule", "account", "sender"),
            ("pick_recipient", "typed", "email"),
        ];
        for (command, given, arg) in second {
            let refused = a.call(command, json!({ given: "fixture", arg: [[]] })).unwrap_err();
            let text = refused.as_str().unwrap_or_default();
            assert!(text.contains(&format!("invalid args `{arg}` for command `{command}`")), "{command}: {refused}");
        }
    }

    /// Reads every PDF as "pdf", and pictures as nothing.
    struct Plain;
    impl moss_extract::Native for Plain {
        fn pdf_text(&self, _: &[u8]) -> Option<String> {
            Some("Resident parking permit on 12 December".into())
        }
        fn pdf_ocr(&self, _: &[u8]) -> Option<String> {
            None
        }
        fn image_ocr(&self, _: &[u8]) -> Option<String> {
            None
        }
    }

    #[test]
    fn the_mac_reader_reads_what_moss_extract_reads_and_waits_on_a_low_battery() {
        use moss_engine::Reader;
        let reader = MacReader { native: Arc::new(Plain), conditions: || None };
        assert!(reader.may_run());
        assert!(reader.reads("P2026-0457.pdf", "application/pdf") && !reader.reads("clip.mov", "video/quicktime"));
        assert_eq!(reader.read(b"%PDF", "P2026-0457.pdf", "").as_deref(), Some("Resident parking permit on 12 December"));
        assert_eq!(reader.read(b"x", "photo.jpg", ""), None);
        let low = MacReader { native: Arc::new(Plain), conditions: || Some(moss_ai::power::Pause::Battery) };
        assert!(!low.may_run());
    }

    #[test]
    fn the_first_run_choice_and_removing_models_reach_the_ai() {
        let a = app();
        let status = a.call("ai_plan", json!({"plan": "skip"})).unwrap();
        assert_eq!(status["plan"], "skip");
        assert_eq!(status["plans"].as_array().unwrap().len(), 3);
        assert_eq!(status["plans"][1]["size"], "About 1.4 GB");
        assert_eq!(a.call("ai_remove", json!({})).unwrap()["onDisk"], 0);
        assert_eq!(a.call("ai_remove_model", json!({"id": "e2b"})).unwrap()["downloaded"], json!([]));
        assert_eq!(a.kind("ai_remove_model", json!({"id": "gpt"})), "choice");
        assert!(a.call("ai_plan", json!({"plan": "lots"})).is_err());
        let stopped = a.call("ai_stop_model", json!({"id": "e2b"})).unwrap();
        assert_eq!(stopped["writers"][0]["download"], json!({"state": "missing", "done": 0}));
        assert_eq!(a.kind("ai_stop_model", json!({"id": "gpt"})), "choice");
        assert_eq!(a.kind("ai_download_models", json!({"ids": ["gpt"]})), "choice");
        assert_eq!(a.call("ai_download_models", json!({"ids": []})).unwrap()["writer"], stopped["writer"]);
    }

    #[test]
    fn summary_rules_and_the_auto_switch_go_through_their_commands() {
        let a = app();
        assert_eq!(a.call("summary_rules", json!({})).unwrap(), json!([]));
        let rule = a.call("save_summary_rule", json!({"sender": "acme.co", "kind": "short"})).unwrap();
        assert_eq!(rule["sender"], "acme.co");
        assert_eq!(a.call("summary_rules", json!({})).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(a.call("remove_summary_rule", json!({"id": rule["id"]})).unwrap(), json!(true));
        a.call("set_automatic_summaries", json!({"on": false})).unwrap();
        assert_eq!(a.call("automatic_summaries", json!({})).unwrap(), json!(false));
    }

    #[test]
    fn ai_commands_reach_the_ai() {
        let a = app();
        let status = a.call("ai_status", json!({})).unwrap();
        assert!(status["machine"].as_str().unwrap().contains("GB memory"), "{status}");
        assert_eq!(status["writers"].as_array().unwrap().len(), 3);
        assert_eq!(status["download"]["state"], "missing");
        assert_eq!(a.kind("ai_choose", json!({"id": "gpt"})), "choice");
        assert_eq!(a.call("ai_choose", json!({"id": "e2b"})).unwrap()["writer"], "e2b");
        assert_eq!(a.call("ai_cancel", json!({})).unwrap(), Value::Null);
        // Moss writes with its own models only: no commands for other AIs.
        for gone in ["ai_cloud", "ai_other_connect", "ai_other_choose", "ai_other_sign_in"] {
            assert!(a.call(gone, json!({})).is_err(), "{gone}");
        }
        assert_eq!(a.kind("ai_ask", json!({"id": "1"})), "unknownThread");

        let (tx, rx) = std::sync::mpsc::channel();
        a.app.listen(AI_EVENT, move |e| tx.send(e.payload().to_string()).unwrap());
        a.call("ai_download", json!({})).unwrap();
        // The disk is full, so it fails at once and says why.
        let failed = loop {
            let status: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
            if status["download"]["state"] == "failed" {
                break status;
            }
        };
        assert!(failed["download"]["message"].as_str().unwrap().starts_with("Not enough free space"));
    }

    #[test]
    fn the_ask_needs_the_writing_model() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let mut store = moss_store::Store::open(dir.path().join("data/moss.db")).unwrap();
        store
            .upsert_account(&moss_core::Account {
                id: "work".into(),
                name: "Work".into(),
                letter: "W".into(),
                address: "maya@northwind.co".into(),
                sender_name: "Maya".into(),
                color: moss_core::AccountColor::Work,
                provider: moss_core::ProviderKind::Imap,
                org: None,
                org_domain: None,
            })
            .unwrap();
        let raw = b"From: Dana <dana@acme.co>\r\nTo: maya@northwind.co\r\nSubject: Deck\r\n\
            Message-ID: <d@acme.co>\r\n\r\nCan you send it?\r\n";
        let source = moss_core::MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: "1".into(),
            ..Default::default()
        };
        let thread = store.ingest(raw, &source).unwrap().thread_id;
        drop(store);
        let a = app_in(dir);
        assert_eq!(a.kind("ai_ask", json!({"id": thread})), "notReady");
    }

    #[test]
    fn ai_status_reaches_the_window() {
        let a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.app.listen(AI_EVENT, move |e| tx.send(e.payload().to_string()).unwrap());
        let ready = Status {
            machine: "M3, 16 GB memory".into(),
            writers: vec![],
            writer: "e4b".into(),
            decider_size: "About 1.1 GB".into(),
            embedder_size: "About 334 MB".into(),
            plan: Some(Plan::Recommended),
            plans: vec![],
            features: moss_ai::Features { search: true, sorting: true, writing: true },
            on_disk: 6_400_000_000,
            downloaded: vec![],
            embedder_download: moss_ai::catalog::ModelDownload::Ready,
            decider_download: moss_ai::catalog::ModelDownload::Ready,
            paused: None,
            took_over: None,
            download: Download::Ready,
        };
        let engine = a.app.state::<Engine>().inner().clone();
        ai_events(a.app.handle().clone(), engine)(ready);
        let payload: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
        assert_eq!(payload["download"], json!({"state": "ready"}));
        assert_eq!(payload["deciderSize"], "About 1.1 GB");
    }

    #[test]
    fn the_decision_model_s_split_labels_are_the_splits_names() {
        assert_eq!(split_named("receipts"), moss_core::SplitId::Receipts);
        assert_eq!(split_named("newsletters"), moss_core::SplitId::Newsletters);
        assert_eq!(split_named("spam"), moss_core::SplitId::Important);
        for (label, _) in moss_ai::prompt::SPLITS {
            assert_eq!(serde_json::to_value(split_named(label)).unwrap(), label);
        }
    }

    #[tokio::test]
    async fn sorting_waits_for_the_decision_model() {
        use moss_engine::Sorter;
        let dir = tempfile::tempdir().unwrap();
        let sorter = AiSorter(ai(dir.path(), Arc::new(|_| {}), None, |_| None));
        let email = moss_store::Undecided {
            message_id: 1,
            account_id: "gmail".into(),
            from: moss_core::Person { name: "Dana".into(), email: "dana@acme.co".into() },
            to: vec![],
            subject: "Deck".into(),
            attachments: vec![],
            body: "Can you send it?".into(),
            sent_at: 0,
            split: moss_core::SplitId::Important,
            known: false,
        };
        assert_eq!(sorter.needs_reply(&email).await, None);
        assert_eq!(sorter.split(&email).await, None);
        assert_eq!(sorter.pitch(&email).await, None);
        let listed = moss_store::Undecided { split: moss_core::SplitId::Newsletters, ..email.clone() };
        assert_eq!(sorter.split(&listed).await, None);
        assert_eq!(sorter.deadline(&email).await, None);
        assert_eq!(sorter.expects_reply(&email).await, None);
    }

    #[tokio::test]
    async fn search_by_meaning_and_summaries_wait_for_their_models() {
        use moss_engine::{Embedder, Summarizer};
        let dir = tempfile::tempdir().unwrap();
        let ai = ai(dir.path(), Arc::new(|_| {}), None, |_| None);
        let embedder = AiEmbedder(ai.clone());
        assert_eq!(embedder.model(), "embeddinggemma-300m");
        let email = moss_store::ToEmbed { message_id: 1, subject: "Flight".into(), text: "Lisbon".into() };
        assert_eq!(embedder.documents(&[email]).await, None);
        assert_eq!(embedder.query("trips").await, None);
        let summarizer = AiSummarizer(ai);
        assert_eq!(summarizer.model(), format!("e4b+prompts-{}", moss_ai::summaries::PROMPTS));
        let c = moss_store::ToSummarize {
            thread_id: 1,
            account_id: "w".into(),
            subject: "Deck".into(),
            through: 2,
            messages: vec![],
            you: "maya@northwind.co".into(),
            why: moss_core::SummaryWhy::Important,
        };
        assert_eq!(summarizer.summarize(&c).await, None);
        let facts = moss_core::SummaryRequest::of(moss_core::SummaryKind::Facts);
        assert_eq!(summarizer.summarize_as(&c, &facts).await, None);
    }

    #[test]
    fn apples_helper_is_the_one_beside_moss() {
        let dir = tempfile::tempdir().unwrap();
        let moss = dir.path().join("Moss");
        assert_eq!(apple_helper(Some(moss.clone()), None), None, "none built in");
        std::fs::write(dir.path().join("moss-apple"), b"").unwrap();
        assert_eq!(apple_helper(Some(moss), None), Some(dir.path().join("moss-apple")));
        assert_eq!(apple_helper(None, None), None);
        // A debug build tries one built from apple/.
        assert_eq!(apple_helper(None, Some("/src/apple/moss-apple".into())), Some(PathBuf::from("/src/apple/moss-apple")));
    }

    #[test]
    fn a_debug_build_can_run_a_llama_server_from_this_mac() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("llama-server");
        std::fs::write(&binary, b"").unwrap();
        let ai = ai(dir.path(), Arc::new(|_| {}), Some(&binary), |_| None);
        // The runtime isn't part of the download any more.
        let total = |ai: &Ai| match ai.status().download {
            Download::Missing { total, .. } => total,
            other => panic!("{other:?}"),
        };
        let without = total(&ai);
        let with = total(&super::ai(dir.path(), Arc::new(|_| {}), None, |_| None));
        assert_eq!(with - without, moss_ai::catalog::RUNTIME.bytes);
    }

    #[test]
    fn engine_events_reach_the_window() {
        let a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.app.listen(EVENT, move |e| tx.send(e.payload().to_string()).unwrap());
        WindowEvents(a.app.handle().clone())
            .emit(Event::MailChanged { account_id: "gmail".into() });
        let payload: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
        assert_eq!(payload, json!({"type": "mailChanged", "accountId": "gmail"}));
    }

    #[cfg(unix)]
    #[test]
    fn desktop_startup_keeps_its_data_directory_private() {
        use std::os::unix::fs::PermissionsExt;
        let a = app();
        for existing in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("data");
            if existing {
                std::fs::create_dir(&data).unwrap();
                std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
                std::fs::write(data.join("kept"), b"fixture").unwrap();
            }
            let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
            let owner = engine(&data, secrets, Arc::new(WindowEvents(a.app.handle().clone())), TlsRoots::Platform).unwrap();
            assert_eq!(std::fs::metadata(&data).unwrap().permissions().mode() & 0o777, 0o700);
            assert!(data.join("moss.db").exists());
            if existing { assert_eq!(std::fs::read(data.join("kept")).unwrap(), b"fixture"); }
            drop(owner);
        }
    }

    #[test]
    fn the_engine_needs_a_writable_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
        struct Quiet;
        impl Events for Quiet {
            fn emit(&self, _: Event) {}
        }
        Quiet.emit(Event::MailChanged { account_id: "x".into() });
        assert!(engine(&file.join("data"), secrets, Arc::new(Quiet), TlsRoots::Platform).is_err());
    }

    mod summaries;
    /// The account the Ask tests file mail under.
    pub(crate) fn work_account() -> moss_core::Account {
        moss_core::Account {
            id: "work".into(),
            name: "Work".into(),
            letter: "W".into(),
            address: "maya@northwind.co".into(),
            sender_name: "Maya".into(),
            color: moss_core::AccountColor::Work,
            provider: moss_core::ProviderKind::Imap,
            org: None,
            org_domain: None,
        }
    }

    #[test]
    fn asking_needs_the_writing_model_and_the_window_hears_why() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let mut store = moss_store::Store::open(dir.path().join("data/moss.db")).unwrap();
        store.upsert_account(&work_account()).unwrap();
        let raw = b"From: Dana <dana@acme.co>\r\nTo: maya@northwind.co\r\nSubject: Deck\r\n\
            Message-ID: <d@acme.co>\r\n\r\nCan you send it?\r\n";
        let source = moss_core::MessageSource { account_id: "work".into(), mailbox: "INBOX".into(), provider_id: "1".into(), ..Default::default() };
        let thread = store.ingest(raw, &source).unwrap().thread_id;
        drop(store);
        let a = app_in(dir);
        let (tx, rx) = std::sync::mpsc::channel();
        a.app.listen(ask::ASK_EVENT, move |e| tx.send(e.payload().to_string()).unwrap());

        let open = |id: &str| json!({"chat": "m1", "question": "When?", "open": id});
        assert_eq!(a.kind("ask_moss", open("999")), "unknownThread");
        assert_eq!(a.kind("ask_moss", open(&thread)), "notReady");
        let heard: Vec<Value> = rx.try_iter().map(|p| serde_json::from_str(&p).unwrap()).collect();
        assert_eq!(heard[0], json!({"type": "step", "chat": "m1", "step": 1}));
        assert_eq!(heard.last().unwrap()["type"], "failed");
        assert_eq!(heard.last().unwrap()["error"]["kind"], "notReady");

        assert_eq!(a.kind("ask_moss", json!({"chat": "m1", "question": "Who is Dana?"})), "notReady");
        assert_eq!(a.call("ask_stop", json!({"chat": "m1"})).unwrap(), json!(false));
        assert_eq!(a.call("ask_forget", json!({"chat": "m1"})).unwrap(), Value::Null);
        assert_eq!(a.call("ask_mail_count", json!({})).unwrap(), json!(1));
    }
}
