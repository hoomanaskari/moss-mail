//! Drafts in each account's Drafts folder, so they show on your phone and in webmail.
//!
//! - Up: a draft goes up a few seconds after you stop typing, and on every sync while its
//!   copy is behind (saved offline, or before a restart). After an accepted append, its
//!   receipt and old-copy cleanup commit together. Failed cleanup keeps both copies until retry.
//! - Down: each sync lists the Drafts folder. Drafts started elsewhere come in; ones sent or
//!   deleted elsewhere go here too, unless they have changes here not yet up, which go up
//!   again.
//! - Conflicting edits are kept as separate drafts. A timestamp alone never grants
//!   permission to delete an unknown remote copy.
//! - Sent or discarded: removed from the server through the outbox, like done and read, so
//!   it happens even after going offline.
//!
//! Pushes and pulls run on the account's server queue, one after another, so they never
//! see each other half done. Providers without folders (Resend) keep drafts on this Mac.

use std::collections::HashSet;
use std::time::Duration;

use moss_accounts::MailService;
use moss_core::{Account, AddressState, ComposeMode, Draft, EmailKind, SignatureEmail};
use moss_providers::{OutgoingMessage, Provider};
use moss_store::{ServerCopy, DRAFT_HEADER, MODE_HEADER, SIGNATURE_HEADER};

use crate::{compose, Error, Result, ServerChange, Shared};

/// How long after the last change a draft goes up: long enough to not save every pause.
pub(crate) const DRAFT_PUSH_DELAY: Duration = Duration::from_secs(3);

/// Post-APPEND receipt recovery still retains legacy cleanup atomically with its receipt.
/// Do not add fallible encoding here before unrepresentable cleanup can be kept durably.
/// The claimed sender retains failures; a pre-upload ledger and exact origins remain required.
pub(crate) fn cleanup_change(copy: &ServerCopy) -> String {
    serde_json::to_string(&ServerChange::DeleteDrafts { message_ids: vec![copy.message_id.clone()] })
        .expect("server changes serialise")
}

/// Required intent for local actions that have not crossed a network mutation boundary.
pub(crate) fn cleanup_changes(copy: &ServerCopy) -> moss_store::Result<Vec<String>> {
    crate::change_batches::encode(vec![ServerChange::DeleteDrafts { message_ids: vec![copy.message_id.clone()] }])
}

/// Called by preparation/scheduling builders inside their owning transaction.
pub(crate) fn saved_cleanup(store: &moss_store::Store, id: &str) -> moss_store::Result<Option<(String, Vec<String>)>> {
    let Some(saved) = store.synced_draft(id)? else { return Ok(None); };
    let Some(copy) = saved.copy else { return Ok(None); };
    Ok(Some((saved.draft.from_account_id, cleanup_changes(&copy)?)))
}

/// Can't reach the server now: stop, and try again on the next sync.
fn unreachable(e: &Error) -> bool {
    matches!(e, Error::Offline(_) | Error::SignedOut(_) | Error::Keychain(_))
}

impl Shared {
    /// The Drafts folder, on servers that keep drafts.
    pub(crate) async fn drafts_folder(&self, id: &str, provider: &dyn Provider) -> Result<Option<String>> {
        if !provider.capabilities().server_flags {
            return Ok(None);
        }
        Ok(self.folders(id, provider).await?.drafts)
    }

    /// Pushes the account's drafts after a pause, restarted by every save. Outside the
    /// async runtime there's no pause to wait: they go up on the next sync.
    pub(crate) fn push_soon(self: &std::sync::Arc<Self>, account_id: &str) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let shared = self.clone();
        let id = account_id.to_string();
        let timer = runtime.spawn(async move {
            tokio::time::sleep(DRAFT_PUSH_DELAY).await;
            shared.send_waiting(&id);
        });
        if let Some(before) = crate::lock(&self.draft_timers).insert(account_id.into(), timer) {
            before.abort();
        }
    }

    /// The draft's copy for the Drafts folder: what you wrote and its files, with its
    /// threading headers, from the address it goes from (the account's own, if that's not
    /// allowed). Files too big for `service` to send keep it here: it couldn't go anyway.
    async fn draft_message(&self, account: &Account, draft: &Draft, service: MailService) -> Result<OutgoingMessage> {
        let mut message = self.draft_text(account, draft)?;
        message.attachments = self.files_to_send(&draft.id, &draft.attachments, service).await?;
        Ok(message)
    }

    fn draft_text(&self, account: &Account, draft: &Draft) -> Result<OutgoingMessage> {
        let from = self
            .sender(account, draft.from_address.as_deref())
            .unwrap_or_else(|_| self.sender(account, None).expect("the account's own address"));
        let headers = match &draft.thread_id {
            Some(thread) => self.store().reply_headers(thread)?,
            None => None,
        };
        Ok(compose::draft_copy(draft, &from, headers))
    }

    /// Pushes every draft whose copy on the server is behind. One the server refuses is
    /// skipped until the next sync; losing the connection stops them all.
    #[cfg(test)]
    pub(crate) async fn push_drafts(&self, id: &str) -> Result<()> {
        let _guard = self.mutation_guard(id).await;
        self.push_drafts_unlocked(id).await
    }

    pub(crate) async fn push_drafts_unlocked(&self, id: &str) -> Result<()> {
        self.require_empty_queue(id)?;
        let behind: Vec<_> =
            self.store().synced_drafts(id)?.into_iter().filter(|d| !d.pushed).collect();
        if behind.is_empty() {
            return Ok(());
        }
        let provider = self.provider(id).await?;
        let Some(mailbox) = self.drafts_folder(id, provider.as_ref()).await? else {
            return Ok(());
        };
        let account = self.account(id)?;
        let service = self.connection(id)?.service;
        for d in behind {
            let message = match self.draft_message(&account, &d.draft, service).await {
                Ok(message) => message,
                Err(e) if unreachable(&e) => return Err(e),
                Err(e) => {
                    eprintln!("Moss: draft {} can't go up yet: {e}", d.draft.id);
                    continue;
                }
            };
            self.require_empty_queue(id)?;
            let saved = match provider.save_draft(&mailbox, &message).await {
                Ok(Some(saved)) => saved,
                Ok(None) => return Ok(()),
                Err(e) => match Error::from(e) {
                    e if unreachable(&e) => return Err(e),
                    e => {
                        eprintln!("Moss: draft {} didn't go up: {e}", d.draft.id);
                        continue;
                    }
                },
            };
            let copy = ServerCopy { message_id: saved.message_id, provider_id: saved.provider_id };
            let receipt = self.store().finish_draft_upload(&d, &copy, self.now(), cleanup_change);
            let cleanup = match receipt {
                Ok(cleanup) => cleanup,
                Err(error @ moss_store::Error::UploadReceiptConflict) => {
                    self.report(id, &error.into());
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if cleanup.is_some() {
                self.drain_changes(id).await?;
            }
        }
        Ok(())
    }

    /// Brings in what changed in the Drafts folder since the last sync (see the module
    /// docs), then pushes what this left behind.
    pub(crate) async fn pull_drafts(&self, id: &str) -> Result<()> {
        let _guard = self.mutation_guard(id).await;
        self.require_empty_queue(id)?;
        let provider = self.provider(id).await?;
        let Some(mailbox) = self.drafts_folder(id, provider.as_ref()).await? else {
            return Ok(());
        };
        let Some(on_server) = provider.all_ids(&mailbox).await? else {
            return Ok(());
        };
        let account = self.account(id)?;
        let mut local = self.store().synced_drafts(id)?;
        // IMAP APPEND often has no UID receipt. Resolve those copies against the
        // complete listing before deciding whether an incoming edit replaced one.
        // Otherwise two devices can recover each other's uploads indefinitely.
        let mut absent: HashSet<String> = local.iter().filter_map(|draft| {
            let copy=draft.copy.as_ref()?;
            copy.provider_id.is_none().then(||copy.message_id.clone())
        }).collect();
        if !absent.is_empty() {
            let wanted=absent.clone();
            let mut complete=true;
            for ids in on_server.chunks(32) {
                let headers=provider.fetch_headers(&mailbox,ids).await?;
                if ids.iter().any(|id|!headers.iter().any(|header|&header.provider_id==id)) {complete=false;}
                for header in headers {
                    let store=self.store();
                    crate::server_queue::require_empty(&store,id)?;
                    let Some(read)=store.draft_from_server(id,&header.raw,self.now())? else {
                        complete=false;
                        continue;
                    };
                    if !wanted.contains(&read.message_id) {continue;}
                    absent.remove(&read.message_id);
                    for draft in &mut local {
                        let Some(copy)=draft.copy.as_mut().filter(|copy|copy.message_id==read.message_id) else {continue;};
                        let found=ServerCopy{message_id:copy.message_id.clone(),provider_id:Some(header.provider_id.clone())};
                        if store.confirm_server_copy(&draft.draft.id,id,&found)? {*copy=found;}
                    }
                }
                if absent.is_empty() {break;}
            }
            // Partial FETCH and malformed items grant no absence authority. Keep
            // confirming readable copies and syncing unrelated drafts nonetheless.
            if !complete {absent.clear();}
        }
        let known: HashSet<&str> =
            local.iter().filter_map(|d| d.copy.as_ref()?.provider_id.as_deref()).collect();
        let new: Vec<String> = on_server.iter().filter(|p| !known.contains(p.as_str())).cloned().collect();
        let mut changed = false;
        let mut stale = Vec::new();
        if !new.is_empty() {
            for pid in &new {
                // Whole, files and all: a draft's files are part of it.
                let m = match provider.fetch_message(&mailbox, pid).await {
                    Ok(m) => m,
                    Err(moss_providers::Error::NotFound(_)) => continue,
                    Err(e) => return Err(e.into()),
                };
                let undated = m.received_at.map_or_else(|| self.now(), |t| t.timestamp_millis());
                let Some(mut read) = self.store().draft_from_server(id, &m.raw, undated)? else {
                    continue;
                };
                let observed=local.iter().find(|draft|read.moss_id.as_deref()==Some(draft.draft.id.as_str()));
                if observed.is_none() {
                    if read.draft.from_address.is_some() && self.sender(&account,read.draft.from_address.as_deref()).is_err() {
                        read.draft.from_address=None;
                    }
                    if read.draft.body_html.is_empty() {read.draft.body_html=compose::text_to_html(&read.draft.body);}
                    self.sign_server_draft(&mut read.draft)?;
                }
                let copy = ServerCopy { message_id: read.message_id.clone(), provider_id: Some(m.provider_id) };
                let mut store = self.store();
                crate::server_queue::require_empty(&store, id)?;
                if !store.server_draft_account_matches(&read.draft)? {
                    self.report(id,&crate::Error::Invalid("A server draft refers to another account. Its remote copy was kept; other drafts can still sync.".into()));
                    continue;
                }
                // The copy this Mac pushed, now with its id in the folder.
                if let Some(d) = local.iter().find(|d| d.copy.as_ref().is_some_and(|c| c.message_id == read.message_id)) {
                    store.confirm_server_copy(&d.draft.id, id, &copy)?;
                    continue;
                }
                // A newer remote edit can replace an unchanged pushed snapshot.
                // Local edits, ambiguous dates and concurrent changes keep both versions.
                if let Some(d) = observed {
                    let mut theirs = Draft {
                        signature_id: d.draft.signature_id.clone(),
                        signature_chosen: d.draft.signature_chosen,
                        remind_if_no_reply: d.draft.remind_if_no_reply,
                        ..read.draft
                    };
                    let replaced_copy=d.copy.as_ref().is_some_and(|copy| match &copy.provider_id {
                        Some(provider_id)=>!on_server.contains(provider_id),
                        None=>absent.contains(&copy.message_id),
                    });
                    if d.pushed && replaced_copy && read.saved_at>d.draft.updated_at
                        && store.apply_server_draft(&theirs,&copy,&read.files,self.now(),Some(d))? {
                        stale.extend(d.copy.iter().map(|copy|copy.message_id.clone()));
                        changed=true;
                    } else {
                        theirs.id=format!("recovered:{}",uuid::Uuid::new_v4().simple());
                        if store.keep_conflicting_server_draft(&d.draft.id,&theirs,&copy,&read.files,self.now())? {
                            changed=true;
                            self.report(id,&crate::Error::Invalid("A conflicting remote edit was kept as a separate draft. Review Drafts before sending.".into()));
                        }
                    }
                    continue;
                }
                // Started elsewhere. Normalize before taking the store lock, then
                // retain that lock through the account check and insert-only save.
                if store.apply_server_draft(&read.draft, &copy, &read.files, self.now(), None)? {
                    changed=true;
                } else {
                    // Another copy in this pull, or a local save during fetch, already
                    // established this identity. Preserve the additional remote text.
                    let original_id=read.draft.id.clone();
                    read.draft.id=format!("recovered:{}",uuid::Uuid::new_v4().simple());
                    if store.keep_conflicting_server_draft(&original_id,&read.draft,&copy,&read.files,self.now())? {
                        changed=true;
                        self.report(id,&crate::Error::Invalid("A conflicting remote edit was kept as a separate draft. Review Drafts before sending.".into()));
                    }
                }
            }
        }
        // Gone from the server: sent or deleted elsewhere. Changes made here go up again.
        let now_here = self.store().synced_drafts(id)?;
        for d in now_here {
            let Some(_copy) = d.copy.as_ref().filter(|copy| match &copy.provider_id {
                Some(provider_id)=>!on_server.contains(provider_id),
                None=>absent.contains(&copy.message_id),
            }) else {
                continue;
            };
            let mut store = self.store();
            crate::server_queue::require_empty(&store, id)?;
            if d.pushed {
                changed |= store.delete_server_draft(&d)?;
            } else {
                // Its Message-ID stays, so the push removes it if it turns up again.
                store.forget_server_provider_id(&d)?;
            }
        }
        if !stale.is_empty() {
            self.require_empty_queue(id)?;
            provider.delete_drafts(&mailbox, &stale).await?;
        }
        if changed {
            self.events.emit(crate::Event::DraftsChanged { account_id: id.into() });
        }
        self.push_drafts_unlocked(id).await
    }

    /// A draft from another app (or another Mac) gets the signature a new one would get here,
    /// unless you chose one in Moss. When its text already ends with one of your saved
    /// signatures, that one comes out of the text and becomes its signature, so it isn't
    /// added twice and Remove takes it off.
    fn sign_server_draft(&self, draft: &mut Draft) -> Result<()> {
        let settings = self.store().signatures()?;
        let saved = |id: &String| settings.signatures.iter().any(|s| &s.id == id);
        if draft.signature_chosen && draft.signature_id.as_ref().is_none_or(saved) {
            return Ok(());
        }
        draft.signature_chosen = false;
        if let Some((id, body)) = compose::ending_signature(&settings.signatures, &draft.body) {
            draft.body_html = compose::text_to_html(&body);
            draft.body = body;
            draft.signature_id = Some(id);
            return Ok(());
        }
        let kind = match draft.mode {
            ComposeMode::New => EmailKind::New,
            ComposeMode::Forward => EmailKind::Forward,
            ComposeMode::Reply | ComposeMode::ReplyAll => EmailKind::Reply,
        };
        let signed_before = match &draft.thread_id {
            Some(thread) => self.signed_before(thread)?,
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

    /// How many emails in the thread you sent, from any of your accounts' added addresses.
    /// Same as `signedBefore` in src/state/signatures.ts.
    fn signed_before(&self, thread_id: &str) -> Result<u32> {
        let store = self.store();
        let messages = store.thread(thread_id, self.now())?.map(|t| t.messages).unwrap_or_default();
        let mut mine = Vec::new();
        for account in store.accounts()? {
            mine.extend(store.addresses(&account.id)?.into_iter().filter(|a| a.state == AddressState::Added).map(|a| a.email));
            mine.push(account.address);
        }
        let yours = |email: &str| mine.iter().any(|m| m.eq_ignore_ascii_case(email));
        Ok(messages.iter().filter(|m| yours(&m.from.email)).count() as u32)
    }

    /// Pulls the account's drafts in the background, after the changes waiting for it.
    pub(crate) fn pull_in_background(self: &std::sync::Arc<Self>, id: &str) {
        let shared = self.clone();
        let account = id.to_string();
        self.queue(id, async move {
            if let Err(e) = shared.pull_drafts(&account).await {
                shared.report(&account, &e);
            }
        });
    }
}

/// The headers a draft's server copy carries, so Moss knows it again.
pub(crate) fn moss_headers(draft: &Draft) -> Vec<(String, String)> {
    let mode = serde_json::to_value(draft.mode).ok().and_then(|v| v.as_str().map(String::from));
    let mut headers = vec![(DRAFT_HEADER.into(), draft.id.clone()), (MODE_HEADER.into(), mode.unwrap_or_default())];
    if draft.signature_chosen {
        headers.push((SIGNATURE_HEADER.into(), draft.signature_id.clone().unwrap_or_else(|| "none".into())));
    }
    headers
}
