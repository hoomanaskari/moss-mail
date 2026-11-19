//! Serialized server mutations and crash-surviving queued attempts.
//!
//! What happens when an attempt doesn't simply work, by what the server said:
//!
//! | Answer | Every kind of change |
//! |---|---|
//! | No answer: the connection failed, dropped or stalled, a rate limit, a sign-in to renew | Back in the queue, in its place, tried again on the next sync |
//! | The mailbox was reset, or the server said no and none of the messages are there any more | Dropped: there is nothing left to change, and later changes go |
//! | Any other "no" (IMAP NO or BAD, no Archive or Trash folder) | Blocked: you choose Try again or Discard |
//! | An answer that may mean it worked (never from these, kept to be safe) | Blocked as uncertain |
//!
//! Trying again after no answer is safe for every kind, because each attempt looks at the
//! server as it is now: flags (`Seen`, `Flagged`, `Forwarded`) are set, not toggled, so setting
//! one twice changes nothing; `Archive` moves by UID only after checking the mailbox's
//! UIDVALIDITY, and a UID that already moved is no longer there to move; `Unarchive`, `Trash`,
//! `Untrash` and `DeleteDrafts` find the messages by Message-ID in the folder they are leaving
//! and do nothing when they are gone. The one repeat a server can show: one without IMAP MOVE
//! copies then deletes, so a copy made just before the connection dropped is made once more.
//! A change Moss stopped in the middle of (a crash or force quit) is tried again at the next
//! start for the same reason.
use std::sync::Arc;
use moss_providers::{Change, Provider};
use moss_store::{ChangeFailure, ChangeId, ChangeState};
use crate::{Engine, Error, Result, ServerChange, Shared};

fn recovery_pending() -> Error {
    Error::Invalid("Server changes need recovery. Later changes are waiting; incoming mail can still be read.".into())
}

pub(crate) fn require_empty(store: &moss_store::Store, account: &str) -> Result<()> {
    if store.earliest_change_metadata(account)?.is_some() { return Err(recovery_pending()); }
    Ok(())
}

struct ChangeNotice<'a> { shared: &'a Shared, account: &'a str }
impl Drop for ChangeNotice<'_> {
    fn drop(&mut self) {
        self.shared.events.emit(crate::Event::ServerChangesChanged { account_id: self.account.into() });
    }
}

/// What one attempt at a server change came to. See the table at the top.
enum Attempt {
    Applied,
    Moot,
    Retry(Error),
    Refused(Error),
    Uncertain(Error),
}

fn attempt(error: moss_providers::Error) -> Attempt {
    use moss_providers::Error as P;
    match error {
        P::NotFound(_) => Attempt::Moot,
        P::Network(_) | P::RateLimited { .. } | P::Auth(_) => Attempt::Retry(error.into()),
        P::DeliveryUnknown(_) => Attempt::Uncertain(error.into()),
        _ => Attempt::Refused(error.into()),
    }
}

/// Whether none of `ids` is in `mailbox` on the server any more, as far as it can say.
async fn gone_from_server(provider: &dyn Provider, target: Option<(&String, &Vec<String>)>) -> bool {
    let Some((mailbox, ids)) = target else { return false };
    matches!(provider.all_ids(mailbox).await, Ok(Some(there)) if !ids.iter().any(|id| there.contains(id)))
}

impl Shared {
    fn mutation_lock(&self, account: &str) -> Arc<tokio::sync::Mutex<()>> {
        crate::lock(&self.server_mutations).entry(account.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
    }

    pub(crate) async fn mutation_guard(&self, account: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.mutation_lock(account).lock_owned().await
    }

    /// Direct mutations cannot overtake any retained queue intent.
    pub(crate) fn require_empty_queue(&self, account: &str) -> Result<()> {
        require_empty(&self.store(), account)
    }

    pub(crate) async fn send_all(&self, account: &str) -> Result<()> {
        let _guard = self.mutation_guard(account).await;
        self.drain_changes(account).await?;
        self.push_drafts_unlocked(account).await
    }

    /// At startup, before the account's worker: a change a crash or force quit left on its way
    /// goes back to the queue (see the table at the top). An account whose changes are being
    /// sent already is left alone: nothing of an earlier run can be on its way there.
    pub(crate) fn recover_server_changes(&self, account: &str) -> Result<()> {
        let lock = self.mutation_lock(account);
        let Ok(_guard) = lock.try_lock() else { return Ok(()) };
        if self.store().release_interrupted_change(account)? {
            self.events.emit(crate::Event::ServerChangesChanged { account_id: account.into() });
        }
        Ok(())
    }

    /// Caller holds the account mutation guard. A committed claim precedes the attempt.
    pub(crate) async fn drain_changes(&self, account: &str) -> Result<()> {
        loop {
            let Some(head) = self.store().earliest_change(account)? else { return Ok(()); };
            let _notice = ChangeNotice { shared: self, account };
            if head.state != ChangeState::Queued { return Err(recovery_pending()); }
            let change: ServerChange = match head.payload.as_deref().map(serde_json::from_str) {
                Some(Ok(change)) => change,
                invalid => {
                    let failure = if invalid.is_none() { ChangeFailure::TooLarge } else { ChangeFailure::Unreadable };
                    self.store().block_change(account, head.id, head.revision, failure)?;
                    return Err(recovery_pending());
                }
            };
            if !matches!(crate::change_batches::encode(vec![change.clone()]), Ok(parts) if parts.len() == 1) {
                self.store().block_change(account, head.id, head.revision, ChangeFailure::Unreadable)?;
                return Err(recovery_pending());
            }
            // Connection failures before a claim have not attempted a server mutation.
            let provider = self.provider(account).await?;
            let revision = self.store().claim_change(account, head.id, head.revision)?;
            self.events.emit(crate::Event::ServerChangesChanged { account_id: account.into() });
            let (failure, error) = match self.send_one(account, provider.as_ref(), change).await {
                Ok(Attempt::Applied) => { self.store().complete_change(account, head.id, revision)?; continue; }
                Ok(Attempt::Moot) => { self.store().drop_claimed_change(account, head.id, revision)?; continue; }
                Ok(Attempt::Retry(error)) => {
                    self.store().release_change(account, head.id, revision)?;
                    return Err(error);
                }
                Ok(Attempt::Refused(error)) => (ChangeFailure::Refused, error),
                Ok(Attempt::Uncertain(error)) => (ChangeFailure::Uncertain, error),
                // The server did it; only keeping track of it here failed.
                Err(error) => {
                    self.store().complete_change(account, head.id, revision)?;
                    return Err(error);
                }
            };
            self.store().block_change(account, head.id, revision, failure)?;
            return Err(error);
        }
    }

    /// One attempt at `change`. An error means the server did it but a step here failed.
    async fn send_one(&self, account_id: &str, provider: &dyn Provider, change: ServerChange) -> Result<Attempt> {
        let (result, target) = match &change {
            ServerChange::Archive { mailbox, ids } => (provider.apply(mailbox, ids, &Change::Archive).await, Some((mailbox, ids))),
            ServerChange::Seen { mailbox, ids, seen } => (provider.apply(mailbox, ids, &Change::Seen(*seen)).await, Some((mailbox, ids))),
            ServerChange::Flagged { mailbox, ids, flagged } => (provider.apply(mailbox, ids, &Change::Flagged(*flagged)).await, Some((mailbox, ids))),
            ServerChange::Forwarded { mailbox, ids } => (provider.apply(mailbox, ids, &Change::Forwarded(true)).await, Some((mailbox, ids))),
            ServerChange::Unarchive { message_ids } => (provider.unarchive(message_ids).await, None),
            ServerChange::Trash { message_ids } => (provider.trash(message_ids).await, None),
            ServerChange::Untrash { message_ids, to_inbox } => (provider.untrash(message_ids, *to_inbox).await, None),
            ServerChange::DeleteDrafts { message_ids } => {
                let mailbox = match self.drafts_folder(account_id, provider).await {
                    Ok(Some(mailbox)) => mailbox,
                    Ok(None) => return Ok(Attempt::Refused(Error::Invalid("The server Drafts folder is unavailable. Cleanup has been retained.".into()))),
                    Err(error) => return Ok(Attempt::Retry(error)),
                };
                (provider.delete_drafts(&mailbox, message_ids).await, None)
            }
        };
        match result {
            Ok(()) => {
                if let ServerChange::Archive { mailbox, ids } = &change {
                    // Only where it was: the thread is done here already, or undone since.
                    self.store().forget_locations(account_id, mailbox, ids)?;
                }
                // Gone from the Inbox now. If sync saw it go after Undo had put it back here,
                // it would count it as done in another app.
                if let ServerChange::Trash { message_ids } = &change {
                    let inbox = self.inbox_name(account_id);
                    let mut store = self.store();
                    let ids = store.provider_ids_of(account_id, &inbox, message_ids)?;
                    store.forget_locations(account_id, &inbox, &ids)?;
                }
                Ok(Attempt::Applied)
            }
            Err(error) => Ok(match attempt(error) {
                Attempt::Refused(_) if gone_from_server(provider, target).await => Attempt::Moot,
                other => other,
            }),
        }
    }
}


/// Bounded recovery information. It contains no message targets or payload text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerChangeStatus {
    pub account_id: String,
    /// Which change, and the revision you saw, for Try again and Discard.
    pub change: String,
    pub revision: i64,
    pub state: ChangeState,
    pub failure: Option<ChangeFailure>,
    pub kind: moss_store::BatchKind,
    pub completed: usize,
    pub pending: usize,
    pub discarded: usize,
    pub dropped: usize,
}

impl Engine {
    pub fn server_change_status(&self, account: &str) -> Result<Option<ServerChangeStatus>> {
        self.shared.account(account)?;
        let store = self.shared.store();
        let Some((head, batch)) = store.change_recovery_snapshot(account)? else { return Ok(None); };
        let mut status = ServerChangeStatus { account_id: account.into(), change: head.id.to_hex(), revision: head.revision, state: head.state,
            failure: head.failure, kind: batch.kind, completed: 0, pending: 0, discarded: 0, dropped: 0 };
        for part in batch.parts {
            match part.outcome {
                moss_store::BatchOutcome::Completed => status.completed += 1,
                moss_store::BatchOutcome::Pending => status.pending += 1,
                moss_store::BatchOutcome::Discarded => status.discarded += 1,
                moss_store::BatchOutcome::Dropped => status.dropped += 1,
            }
        }
        Ok(Some(status))
    }

    /// Tries a blocked server change again now. `change` and `revision` are what
    /// [`Engine::server_change_status`] showed you: anything newer is refused.
    pub fn retry_server_change(&self, account: &str, change: &str, revision: i64) -> Result<()> {
        let id = self.blocked_change(account, change)?;
        self.shared.store().retry_blocked_change(account, id, revision).map_err(changed_since)?;
        self.shared.events.emit(crate::Event::ServerChangesChanged { account_id: account.into() });
        self.shared.send_waiting(account);
        Ok(())
    }

    /// Stops trying a blocked server change, with the rest of its action, and lets later
    /// changes go. It stays on this Mac only: the server keeps the mail as it was.
    pub fn discard_server_change(&self, account: &str, change: &str, revision: i64) -> Result<()> {
        let id = self.blocked_change(account, change)?;
        self.shared.store().discard_blocked_batch(account, id, revision).map_err(changed_since)?;
        self.shared.events.emit(crate::Event::ServerChangesChanged { account_id: account.into() });
        self.shared.send_waiting(account);
        Ok(())
    }

    fn blocked_change(&self, account: &str, change: &str) -> Result<ChangeId> {
        self.shared.account(account)?;
        ChangeId::from_hex(change).ok_or_else(changed)
    }
}

/// The change isn't the one you saw any more: it went, or something else changed it.
fn changed() -> Error {
    Error::Invalid("This server change has changed since you saw it. Check its status again.".into())
}

fn changed_since(error: moss_store::Error) -> Error {
    match error {
        moss_store::Error::Invalid(_) => changed(),
        error => error.into(),
    }
}
