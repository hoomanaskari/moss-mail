//! The background worker that keeps one account in step with its server.
//!
//! Each pass: learn the folders (once), fetch the Inbox (new mail first, then older mail,
//! up to a budget), then Sent (so replies thread), then compare the Inbox with the server
//! to catch mail archived in other apps. Checkpoints are saved after every batch. Between
//! passes the worker waits on IMAP IDLE, or for "sync now"; after a failure it backs off.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use moss_core::{Mailbox, MailboxRole, MessageSource, SyncCheckpoint};
use moss_providers::{Change, Checkpoint, ConversationMessage, Provider, RawMessage};
use moss_store::{Store, BLOCKED_FOLDER};
use tokio::sync::Notify;

use crate::events::{Event, SyncProgress};
use crate::{naming, Error, Folders, Result, Shared};

pub(crate) const BATCH: usize = 200;
/// Inbox messages per pass, so new mail never waits long behind a big backfill.
pub(crate) const INBOX_PER_PASS: usize = 1000;
pub(crate) const SENT_PER_PASS: usize = 200;
/// Sent mail is only needed for threading and for knowing you replied. The newest this many
/// come whole; older ones come as headers only, which is all those need (a few hundred
/// bytes each instead of the whole message), and their bodies are fetched when opened.
pub(crate) const SENT_KEEP: usize = 1000;
/// Blocked mail fetched per pass: Spark users may have thousands, none of them urgent.
pub(crate) const BLOCKED_PER_PASS: usize = 200;
/// Headers-only sent messages per pass.
pub(crate) const SENT_HEADERS_PER_PASS: usize = 1000;
/// Gmail: Inbox and Sent messages per mailbox per pass whose conversation is looked up.
/// Cheap: ids only, no message content.
pub(crate) const LINK_PER_PASS: usize = 200;
/// Gmail: conversations per pass whose archived messages are fetched from All Mail.
pub(crate) const FILL_PER_PASS: usize = 20;
/// How long IDLE waits before syncing anyway. Gmail sometimes never says new mail came,
/// so this is the longest new mail can take to show up.
pub(crate) const IDLE: Duration = Duration::from_secs(5 * 60);
const MONTH_MS: i64 = 30 * 86_400_000;

/// Gmail lets IMAP download about 2.5 GB a day before it locks the account out for a
/// while. Sync stops a little short of that, and carries on the next day.
pub(crate) const DAILY_DOWNLOAD: u64 = 2_400_000_000;
const DAY_MS: i64 = 86_400_000;

/// The longest wait before retrying after a failure.
pub(crate) const MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);

pub(crate) fn backoff(failures: u32) -> Duration {
    let secs = 5u64.saturating_mul(1 << failures.saturating_sub(1).min(16));
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

/// Crashes in a row of an account's worker, counting one that just happened after it ran for
/// `ran`: one that ran longer than the longest pause starts the count again.
pub(crate) fn crashes_in_a_row(before: u32, ran: Duration) -> u32 {
    if ran > MAX_BACKOFF { 1 } else { before.saturating_add(1) }
}

/// What a sync pass left for the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Waiting {
    /// Older Inbox mail, or Sent mail needed for threading.
    pub mail: bool,
    /// Gmail conversations still to fill in from All Mail.
    pub history: bool,
}

/// What sync has seen of an account's Inbox so far.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tally {
    done: u32,
    oldest: Option<i64>,
    more: bool,
}

impl Tally {
    pub(crate) fn progress(&self, account_id: &str, folders: u32, now: i64) -> SyncProgress {
        let recent = match (self.more, self.oldest) {
            (false, _) => 1.0,
            (true, Some(oldest)) => ((now - oldest) as f64 / MONTH_MS as f64).clamp(0.0, 1.0),
            (true, None) => 0.0,
        };
        let year = self.oldest.and_then(chrono::DateTime::from_timestamp_millis).map(|d| {
            use chrono::Datelike;
            d.year()
        });
        SyncProgress {
            account_id: account_id.into(),
            done: self.done,
            folders,
            recent,
            oldest_year: year,
            more: self.more,
        }
    }
}

/// Stores one fetched message. Something that isn't an email (a broken message) is skipped,
/// not fatal.
fn store_message(store: &mut Store, account_id: &str, mailbox: &str, m: &RawMessage) -> Result<()> {
    let source = MessageSource {
        account_id: account_id.into(),
        mailbox: mailbox.into(),
        provider_id: m.provider_id.clone(),
        seen: m.flags.seen,
        flagged: m.flags.flagged,
        forwarded: m.flags.forwarded(),
        // Gmail's system labels (\\Inbox, \\Important…) aren't labels people chose.
        labels: m.labels.iter().filter(|l| !l.starts_with('\\')).cloned().collect(),
        received_at: m.received_at.map(|t| t.timestamp_millis()),
        headers_only: m.headers_only,
    };
    match store.ingest(&m.raw, &source) {
        Ok(_) | Err(moss_store::Error::NotEmail) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// A conversation member that sync fetches through its own folder, or not at all: Inbox
/// mail (the Inbox decides whether it's done), sent mail, and drafts.
fn synced_elsewhere(m: &ConversationMessage) -> bool {
    m.flags.draft || m.labels.iter().any(|l| matches!(l.as_str(), "\\Inbox" | "\\Sent" | "\\Draft"))
}

fn core_role(role: moss_providers::MailboxRole) -> Option<MailboxRole> {
    use moss_providers::MailboxRole as R;
    match role {
        R::Inbox => Some(MailboxRole::Inbox),
        R::Sent => Some(MailboxRole::Sent),
        R::Drafts => Some(MailboxRole::Drafts),
        R::Trash => Some(MailboxRole::Trash),
        R::Junk => Some(MailboxRole::Junk),
        R::Archive => Some(MailboxRole::Archive),
        R::All => Some(MailboxRole::All),
        R::Flagged | R::Important => None,
    }
}

impl Shared {
    /// The Inbox, Sent and All Mail folders, asked of the server once per run.
    pub(crate) async fn folders(&self, id: &str, provider: &dyn Provider) -> Result<Folders> {
        if let Some(f) = crate::lock(&self.folders).get(id) {
            return Ok(f.clone());
        }
        let boxes = provider.list_mailboxes().await?;
        let mut folders = Folders {
            inbox: "INBOX".into(),
            sent: None,
            all: None,
            blocked: None,
            drafts: None,
            count: boxes.len() as u32,
        };
        {
            let mut store = self.store();
            for b in &boxes {
                let role = b.role.and_then(core_role);
                match role {
                    Some(MailboxRole::Inbox) => folders.inbox = b.id.clone(),
                    Some(MailboxRole::Sent) => folders.sent = Some(b.id.clone()),
                    Some(MailboxRole::All) => folders.all = Some(b.id.clone()),
                    Some(MailboxRole::Drafts) => folders.drafts = Some(b.id.clone()),
                    _ if b.id.eq_ignore_ascii_case(BLOCKED_FOLDER) => {
                        folders.blocked = Some(b.id.clone())
                    }
                    _ => {}
                }
                if role.is_some() {
                    store.upsert_mailbox(&Mailbox {
                        account_id: id.into(),
                        name: b.id.clone(),
                        role,
                    })?;
                }
            }
            // Servers that list no roles still have an INBOX.
            store.upsert_mailbox(&Mailbox {
                account_id: id.into(),
                name: folders.inbox.clone(),
                role: Some(MailboxRole::Inbox),
            })?;
        }
        crate::lock(&self.folders).insert(id.into(), folders.clone());
        Ok(folders)
    }

    /// The Blocked folder, made on the server if it isn't there yet.
    async fn blocked_folder(&self, id: &str, provider: &dyn Provider) -> Result<String> {
        if let Some(folder) = self.folders(id, provider).await?.blocked {
            return Ok(folder);
        }
        self.require_empty_queue(id)?;
        provider.create_mailbox(BLOCKED_FOLDER).await?;
        if let Some(f) = crate::lock(&self.folders).get_mut(id) {
            f.blocked = Some(BLOCKED_FOLDER.into());
        }
        Ok(BLOCKED_FOLDER.into())
    }

    /// Moves blocked mail that's still in the Inbox on the server to the Blocked folder.
    pub(crate) async fn move_blocked(&self, id: &str, provider: &dyn Provider) -> Result<()> {
        let _guard = self.mutation_guard(id).await;
        if self.store().earliest_change_metadata(id)?.is_some() { return Ok(()); }
        if !provider.capabilities().server_flags {
            return Ok(());
        }
        let inbox = self.folders(id, provider).await?.inbox;
        let ids = self.store().blocked_in(id, &inbox)?;
        if ids.is_empty() {
            return Ok(());
        }
        let to = self.blocked_folder(id, provider).await?;
        self.require_empty_queue(id)?;
        provider.apply(&inbox, &ids, &Change::Move { to }).await?;
        self.store().forget_locations(id, &inbox, &ids)?;
        Ok(())
    }

    /// Moves mail no longer blocked from the Blocked folder back to the Inbox on the server.
    pub(crate) async fn move_unblocked(&self, id: &str, provider: &dyn Provider) -> Result<()> {
        let _guard = self.mutation_guard(id).await;
        if self.store().earliest_change_metadata(id)?.is_some() { return Ok(()); }
        let folders = self.folders(id, provider).await?;
        let Some(blocked) = folders.blocked else {
            return Ok(());
        };
        let ids = self.store().unblocked_in(id, &blocked)?;
        if ids.is_empty() {
            return Ok(());
        }
        self.require_empty_queue(id)?;
        provider.apply(&blocked, &ids, &Change::Move { to: folders.inbox }).await?;
        self.store().forget_locations(id, &blocked, &ids)?;
        Ok(())
    }

    fn saved_checkpoint(&self, id: &str, mailbox: &str) -> Result<Option<Checkpoint>> {
        let saved = self.store().checkpoint(id, Some(mailbox))?;
        // A checkpoint this Moss can't read just means starting that mailbox again.
        Ok(saved.and_then(|c| c.cursor).and_then(|json| serde_json::from_str(&json).ok()))
    }

    fn downloaded_key(id: &str) -> String {
        format!("sync.downloaded:{id}")
    }

    /// Bytes sync has downloaded for an account today (UTC).
    pub(crate) fn downloaded_today(&self, id: &str) -> Result<u64> {
        let today = self.now() / DAY_MS;
        let saved = self.store().setting(&Self::downloaded_key(id))?.unwrap_or_default();
        let (day, bytes) = saved.split_once(':').unwrap_or_default();
        Ok(if day.parse() == Ok(today) { bytes.parse().unwrap_or(0) } else { 0 })
    }

    /// Counts what a batch downloaded towards today's limit.
    fn count_download(&self, id: &str, messages: &[RawMessage]) -> Result<()> {
        let bytes: u64 = messages.iter().map(|m| m.raw.len() as u64).sum();
        let total = self.downloaded_today(id)? + bytes;
        let value = format!("{}:{total}", self.now() / DAY_MS);
        self.store().set_setting(&Self::downloaded_key(id), Some(&value))?;
        Ok(())
    }

    /// Whether sync has downloaded all it may today. Mail someone opens still comes.
    fn at_daily_limit(&self, id: &str) -> Result<bool> {
        Ok(self.downloaded_today(id)? >= DAILY_DOWNLOAD)
    }

    /// Fetches up to `budget` messages of one mailbox. Returns whether more are waiting.
    async fn sync_mailbox(
        &self,
        id: &str,
        provider: &dyn Provider,
        mailbox: &str,
        budget: usize,
        inbox: bool,
        shallow: bool,
    ) -> Result<bool> {
        let mut checkpoint = self.saved_checkpoint(id, mailbox)?;
        let mut fetched = 0;
        loop {
            if self.at_daily_limit(id)? {
                return Ok(false);
            }
            let batch = if shallow {
                provider.fetch_since_shallow(mailbox, checkpoint.as_ref(), BATCH).await?
            } else {
                provider.fetch_since(mailbox, checkpoint.as_ref(), BATCH).await?
            };
            let touched = !batch.messages.is_empty()
                || !batch.flag_updates.is_empty()
                || !batch.vanished.is_empty();
            let mut oldest = None::<i64>;
            // The server started the mailbox again: what it holds now goes in at once, so
            // nothing acts on it half known.
            let mut reset = batch.reset.then(|| self.store());
            if let Some(store) = &mut reset {
                store.forget_mailbox_ids(id, mailbox)?;
            }
            // Otherwise one message at a time: reading a big email takes a while, and the app
            // needn't wait for the whole batch.
            for m in &batch.messages {
                let received = m.received_at.map(|t| t.timestamp_millis());
                oldest = match (oldest, received) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                match &mut reset {
                    Some(store) => store_message(store, id, mailbox, m)?,
                    None => store_message(&mut self.store(), id, mailbox, m)?,
                }
            }
            drop(reset);
            {
                let mut store = self.store();
                for update in &batch.flag_updates {
                    store.set_seen(id, mailbox, &update.provider_id, update.flags.seen)?;
                    store.set_forwarded(id, mailbox, &update.provider_id, update.flags.forwarded())?;
                    // A star from another app moves it to Important; taking it away, out.
                    store.set_flagged(id, mailbox, &update.provider_id, update.flags.flagged)?;
                }
                store.remove_from_mailbox(id, mailbox, &batch.vanished, self.now())?;
                let cursor =
                    serde_json::to_string(&batch.checkpoint).expect("checkpoints serialise");
                store.save_checkpoint(
                    id,
                    Some(mailbox),
                    &SyncCheckpoint { cursor: Some(cursor), ..Default::default() },
                )?;
            }
            fetched += batch.messages.len();
            self.count_download(id, &batch.messages)?;
            if inbox {
                let done = self.store().mailbox_count(id, mailbox)?;
                let folders = crate::lock(&self.folders).get(id).map_or(0, |f| f.count);
                let progress = {
                    let mut all = crate::lock(&self.progress);
                    let tally = all.entry(id.into()).or_default();
                    tally.done = done;
                    tally.more = batch.more;
                    tally.oldest = match (tally.oldest, oldest) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    tally.progress(id, folders, self.now())
                };
                self.events.emit(Event::SyncProgress(progress));
            }
            if touched {
                self.changed(id);
            }
            checkpoint = Some(batch.checkpoint);
            if !batch.more {
                return Ok(false);
            }
            if fetched >= budget {
                return Ok(true);
            }
        }
    }

    /// Mail that left the Inbox in another app, on servers that don't report it.
    async fn reconcile(&self, id: &str, provider: &dyn Provider, inbox: &str) -> Result<()> {
        let Some(on_server) = provider.all_ids(inbox).await? else {
            return Ok(());
        };
        let on_server: HashSet<&str> = on_server.iter().map(String::as_str).collect();
        let stored = self.store().provider_ids(id, inbox)?;
        let gone: Vec<String> = stored
            .into_iter()
            .filter(|p| !p.starts_with("local:") && !on_server.contains(p.as_str()))
            .collect();
        if !gone.is_empty() && self.store().remove_from_mailbox(id, inbox, &gone, self.now())? > 0 {
            self.changed(id);
        }
        Ok(())
    }

    /// Gmail keeps mail you archived only in All Mail, so the Inbox alone leaves gaps in a
    /// conversation. This asks which conversation Inbox mail is in, then fetches the rest of
    /// a few conversations from All Mail, newest first, as headers only: their bodies come
    /// when someone opens them. Conversations only in Sent aren't filled in. It never
    /// downloads mail that's here already, and leaves mail still in the Inbox or Sent to
    /// those folders, which decide its state; the rest arrives done. Returns whether more
    /// is waiting.
    async fn fill_conversations(
        &self,
        id: &str,
        provider: &dyn Provider,
        folders: &Folders,
        all: &str,
    ) -> Result<bool> {
        let mut more = false;
        let mailbox = &folders.inbox;
        let asked = self.store().unlinked(id, mailbox, LINK_PER_PASS)?;
        if !asked.is_empty() {
            let Some(found) = provider.conversations_of(mailbox, &asked).await? else {
                // Not Gmail: an All Mail folder without conversations.
                return Ok(false);
            };
            self.store().link_conversations(id, mailbox, &asked, &found)?;
            more |= asked.len() == LINK_PER_PASS;
        }

        let conversations = self.store().conversations_to_fill(id, FILL_PER_PASS)?;
        if conversations.is_empty() {
            return Ok(more);
        }
        let members = provider.conversation_messages(all, &conversations).await?;
        let mut wanted = Vec::new();
        {
            let store = self.store();
            for m in members {
                if !synced_elsewhere(&m) && !store.knows(id, &m.headers)? {
                    wanted.push(m.provider_id);
                }
            }
        }
        for chunk in wanted.chunks(BATCH) {
            if self.at_daily_limit(id)? {
                return Ok(false);
            }
            // Earlier mail comes as headers only: enough to thread and show who said what.
            let messages = provider.fetch_headers(all, chunk).await?;
            self.count_download(id, &messages)?;
            for m in &messages {
                store_message(&mut self.store(), id, all, m)?;
            }
            if !messages.is_empty() {
                self.changed(id);
            }
        }
        self.store().mark_filled(id, &conversations)?;
        Ok(more || conversations.len() == FILL_PER_PASS)
    }

    /// Your name as it appears on mail you sent, instead of the guess from the address.
    fn learn_sender_name(&self, id: &str) -> Result<()> {
        let mut account = self.account(id)?;
        if account.sender_name != naming::name_from_address(&account.address) {
            return Ok(());
        }
        let Some(name) = self.store().sender_name_seen(id)? else {
            return Ok(());
        };
        if name != account.sender_name {
            account.sender_name = name;
            self.store().upsert_account(&account)?;
            let account = self.info(account)?;
            self.events.emit(Event::AccountUpdated { account });
        }
        Ok(())
    }

    /// One sync pass. `history_only` when it only continues filling in conversations: then
    /// Sent waits, once plenty is in, so older sent mail still comes one batch per sync.
    pub(crate) async fn pass(&self, id: &str, history_only: bool) -> Result<Waiting> {
        let provider = self.provider(id).await?;
        let folders = self.folders(id, provider.as_ref()).await?;
        let inbox_more = self
            .sync_mailbox(id, provider.as_ref(), &folders.inbox, INBOX_PER_PASS, true, false)
            .await?;
        let mut waiting = Waiting { mail: inbox_more, history: false };
        if let Some(sent) = &folders.sent {
            let shallow = self.store().mailbox_count(id, sent)? as usize >= SENT_KEEP;
            if !(history_only && shallow) {
                let budget = if shallow { SENT_HEADERS_PER_PASS } else { SENT_PER_PASS };
                waiting.mail |=
                    self.sync_mailbox(id, provider.as_ref(), sent, budget, false, shallow).await?;
                self.learn_sender_name(id)?;
            }
        }
        self.reconcile(id, provider.as_ref(), &folders.inbox).await?;
        if let Some(blocked) = &folders.blocked {
            self.sync_mailbox(id, provider.as_ref(), blocked, BLOCKED_PER_PASS, false, false).await?;
        }
        self.move_blocked(id, provider.as_ref()).await?;
        // Archived history once the Inbox is all here: new mail always comes first.
        if let (false, Some(all)) = (inbox_more, &folders.all) {
            waiting.history = self.fill_conversations(id, provider.as_ref(), &folders, all).await?;
        }
        Ok(waiting)
    }

    /// Waits out a backoff, or until someone asks for a sync.
    async fn pause(&self, failures: u32, wake: &Notify) {
        tokio::select! {
            _ = tokio::time::sleep(backoff(failures)) => {}
            _ = wake.notified() => {}
        }
    }

    /// The account's worker: runs until the account is removed.
    pub(crate) async fn run(self: Arc<Self>, id: String, wake: Arc<Notify>) {
        let mut failures = 0u32;
        let mut history_only = false;
        loop {
            // Changes made offline, or before a restart, go before the server is read.
            self.send_waiting(&id);
            let pass = self.pass(&id, history_only).await;
            history_only = false;
            match pass {
                Ok(waiting) => {
                    self.recovered(&id);
                    // After the Inbox, so a reply started on your phone finds its conversation.
                    self.pull_in_background(&id);
                    failures = 0;
                    // Once the inbox is in, every few hours: the addresses its mail came to.
                    if !waiting.mail && self.discovery_due(&id) {
                        self.discover_in_background(&id);
                    }
                    if waiting.mail || waiting.history {
                        history_only = !waiting.mail;
                        tokio::task::yield_now().await;
                        continue;
                    }
                }
                Err(Error::UnknownAccount(_)) => return,
                Err(e) => {
                    failures += 1;
                    self.report(&id, &e);
                    if matches!(e, Error::SignedOut(_) | Error::Keychain(_)) {
                        // Retrying a refused password can lock the account: wait to be asked.
                        self.forget_provider(&id);
                        wake.notified().await;
                    } else {
                        self.pause(failures, &wake).await;
                    }
                    continue;
                }
            }
            let Ok(provider) = self.provider(&id).await else { continue };
            let inbox = self.inbox_name(&id);
            tokio::select! {
                result = provider.wait_for_change(&inbox, IDLE) => {
                    if let Err(e) = result {
                        failures += 1;
                        self.report(&id, &Error::from(e));
                        self.pause(failures, &wake).await;
                    }
                }
                _ = wake.notified() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_five_minutes() {
        assert_eq!(backoff(0), Duration::from_secs(5));
        assert_eq!(backoff(1), Duration::from_secs(5));
        assert_eq!(backoff(2), Duration::from_secs(10));
        assert_eq!(backoff(4), Duration::from_secs(40));
        assert_eq!(backoff(9), MAX_BACKOFF);
        assert_eq!(backoff(u32::MAX), MAX_BACKOFF);
    }

    #[test]
    fn crashes_count_in_a_row_until_the_worker_ran_a_good_while() {
        assert_eq!(crashes_in_a_row(0, Duration::from_secs(1)), 1);
        assert_eq!(crashes_in_a_row(3, MAX_BACKOFF), 4);
        assert_eq!(crashes_in_a_row(3, MAX_BACKOFF + Duration::from_secs(1)), 1);
        assert_eq!(crashes_in_a_row(u32::MAX, Duration::ZERO), u32::MAX);
    }

    #[test]
    fn progress_counts_the_last_month_and_the_oldest_year() {
        let now = 1_793_524_320_000;
        let tally = Tally { done: 10, oldest: Some(now - MONTH_MS / 2), more: true };
        let p = tally.progress("gmail", 4, now);
        assert_eq!((p.done, p.folders, p.more), (10, 4, true));
        assert!((p.recent - 0.5).abs() < 1e-9);
        assert_eq!(p.oldest_year, Some(2026));
        let old = Tally { oldest: Some(now - 4 * MONTH_MS), ..tally.clone() };
        assert_eq!(old.progress("gmail", 4, now).recent, 1.0);
        let empty = Tally { more: true, ..Default::default() };
        assert_eq!(empty.progress("gmail", 0, now).recent, 0.0);
        assert_eq!(empty.progress("gmail", 0, now).oldest_year, None);
        assert_eq!(Tally::default().progress("gmail", 0, now).recent, 1.0);
    }

    #[test]
    fn provider_roles_map_onto_store_roles() {
        use moss_providers::MailboxRole as R;
        assert_eq!(core_role(R::Inbox), Some(MailboxRole::Inbox));
        assert_eq!(core_role(R::Sent), Some(MailboxRole::Sent));
        assert_eq!(core_role(R::Drafts), Some(MailboxRole::Drafts));
        assert_eq!(core_role(R::Trash), Some(MailboxRole::Trash));
        assert_eq!(core_role(R::Junk), Some(MailboxRole::Junk));
        assert_eq!(core_role(R::Archive), Some(MailboxRole::Archive));
        assert_eq!(core_role(R::All), Some(MailboxRole::All));
        assert_eq!(core_role(R::Flagged), None);
        assert_eq!(core_role(R::Important), None);
    }
}
