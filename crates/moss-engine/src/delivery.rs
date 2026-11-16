//! Durable preparation and recovery of outgoing mail.
use moss_core::{Draft, ComposeMode};
use moss_providers::OutgoingMessage;
use serde::{Serialize, Deserialize};
use crate::{Engine, Error, OutgoingDraft, Result, compose};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PreparedSend {
    pub email: OutgoingDraft,
    pub message: OutgoingMessage,
    pub forwarded_message: Option<String>,
}

pub(crate) fn logical_fingerprint(draft: &Draft, email: &OutgoingDraft) -> String {
    moss_store::delivery_fingerprint("logical",vec![
        serde_json::to_value(Draft {revision:None,updated_at:0,..draft.clone()}).expect("draft serializes"),
        serde_json::to_value(email).expect("outgoing draft serializes")])
}

pub(crate) fn legacy_logical_fingerprint(draft: &Draft, outgoing: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct EmailOnly { email: OutgoingDraft }
    serde_json::from_str::<EmailOnly>(outgoing).ok().map(|prepared|logical_fingerprint(draft,&prepared.email))
}

impl crate::Shared {
    pub(crate) fn compact_deliveries(&self) {
        loop {
            let step=self.store().compact_delivery_step(self.now(),legacy_logical_fingerprint);
            match step {
                Ok(true)=>std::thread::yield_now(),
                Ok(false)=>break,
                Err(_)=>{ eprintln!("Moss: completed delivery cleanup will retry later."); break; }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelSendOutcome {
    #[serde(rename = "restoreFailed", default)]
    pub restore_failed: bool,
    pub cancelled: bool,
    pub early: bool,
    pub restored: Option<Draft>,
}

impl Engine {
    /// Freeze the exact content before entering Undo or attempting network submission.
    /// Repeating an operation returns its existing durable state, never a new message.
    pub async fn prepare_send(&self, id: &str, draft: &Draft, email: &OutgoingDraft) -> Result<bool> {
        self.prepare_send_authorized(id, draft, email, false).await
    }

    pub(crate) async fn prepare_scheduled_send(&self, id: &str, draft: &Draft, email: &OutgoingDraft) -> Result<bool> {
        self.prepare_send_authorized(id, draft, email, true).await
    }

    async fn prepare_send_authorized(&self, id: &str, draft: &Draft, email: &OutgoingDraft, scheduled: bool) -> Result<bool> {
        self.recover_deliveries()?;
        let fingerprint=logical_fingerprint(draft,email);
        {
        let store=self.shared.store();
        if let Some((hash,compacted))=store.delivery_replay_hash(id)? {
            if hash.as_deref()==Some(fingerprint.as_str()) { return Ok(false); }
            if compacted { return Err(Error::Invalid("This delivery's payload was retired. This request cannot be replayed. Review its delivery status in Moss; do not resend it automatically.".into())); }
        }
        if let Some(existing) = store.delivery(id)? {
            let prepared: PreparedSend = serde_json::from_str(&existing.outgoing)
                .map_err(|_| Error::Invalid("The saved delivery cannot be read.".into()))?;
            let compared = Draft { revision: None, updated_at: existing.draft.updated_at, ..draft.clone() };
            let previous = Draft { revision: None, ..existing.draft.clone() };
            if previous != compared || prepared.email != *email {
                return Err(Error::Invalid("This delivery operation already belongs to different mail.".into()));
            }
            return Ok(false);
        }
        }
        if email.to.is_empty() && email.cc.is_empty() && email.bcc.is_empty() {
            return Err(Error::NoRecipients);
        }
        let account = self.shared.account(&email.from_account_id)?;
        let (headers, original) = match &email.thread_id {
            Some(id) => {
                let store = self.shared.store();
                let thread = store.thread(id, self.shared.now())?;
                (store.reply_headers(id)?, thread.and_then(|t| t.messages.last().cloned()))
            }
            None => (None, None),
        };
        let from = self.shared.sender(&account, email.from_address.as_deref())?;
        let mut message = compose::outgoing(email, &from, headers, original.as_ref(), *chrono::Local::now().offset());
        let service = self.shared.connection(&account.id)?.service;
        message.attachments = self.shared.files_to_send(&draft.id, &email.attachments, service).await?;
        let message_id = format!("<{}@moss.local>", uuid::Uuid::new_v4().simple());
        message.message_id = Some(message_id.clone());
        message.date = Some(chrono::DateTime::from_timestamp_millis(self.shared.now())
            .ok_or_else(|| Error::Invalid("The send time is out of range.".into()))?);
        let prepared = PreparedSend {
            email: email.clone(), message,
            forwarded_message: original.filter(|_| email.mode == ComposeMode::Forward).map(|m| m.id),
        };
        let json = serde_json::to_string(&prepared).expect("prepared mail serializes");
        let (added, cleanup_account) = {
            let mut store = self.shared.store();
            let mut cleanup_account = None;
            let added = store.prepare_delivery_fingerprinted(id, draft, &json, &message_id, self.shared.now(), Some(&fingerprint), |store| {
                if scheduled {
                    let (owner, outgoing) = store.due_email(&draft.id, self.shared.now())?.ok_or(moss_store::Error::DraftConflict)?;
                    let snapshot = Draft { revision: None, ..draft.clone() };
                    let recorded = Draft { revision: None, ..owner.draft };
                    if owner.operation_id != id || recorded != snapshot || serde_json::from_str::<OutgoingDraft>(&outgoing).ok().as_ref() != Some(email) {
                        return Err(moss_store::Error::DraftConflict);
                    }
                } else {
                    store.require_draft_revision(&draft.id, draft.revision.as_deref())?;
                }
                let cleanup = crate::drafts::saved_cleanup(store, &draft.id)?;
                cleanup_account = cleanup.as_ref().map(|(account, _)| account.clone());
                Ok(cleanup)
            })?;
            (added, cleanup_account)
        };
        if let Some(account) = cleanup_account { self.shared.send_waiting(&account); }
        Ok(added)
    }
}

impl Engine {
    /// A durable claim is the only permission to cross the network submission boundary.
    pub async fn submit_send(&self, id: &str) -> Result<String> {
        self.recover_deliveries()?;
        use moss_store::DeliveryState;
        let state = self.delivery_state(id)?
            .ok_or_else(|| Error::Unavailable("The delivery operation is missing.".into()))?;
        if matches!(state,DeliveryState::Held | DeliveryState::Uncertain) && self.send_cancelled(id)? {
            return Err(Error::Invalid("This send was cancelled.".into()));
        }
        match state {
            DeliveryState::Complete | DeliveryState::Accepted => return self.repair_known_accepted(id),
            DeliveryState::Claimed | DeliveryState::Uncertain => return Err(Error::DeliveryUnknown("No definite server receipt is available.".into())),
            DeliveryState::Cancelled => return Err(Error::Invalid("This send was cancelled.".into())),
            DeliveryState::Held => {}
        }
        let kept = self.shared.store().delivery(id)?
            .ok_or_else(|| Error::Unavailable("The delivery operation is missing.".into()))?;
        let prepared: PreparedSend = serde_json::from_str(&kept.outgoing)
            .map_err(|_| Error::Invalid("The saved delivery cannot be read.".into()))?;
        // Connecting cannot submit mail. Keep this before the claim so offline sign-in
        // failures don't turn a definitely unsent message into an uncertain delivery.
        let provider = self.shared.provider(&prepared.email.from_account_id).await?;
        if !self.shared.store().claim_delivery(id)? {
            // Another owner may already have submitted. Losing the claim is not proof
            // of rejection, and never permits this caller to cross the send boundary.
            return match self.delivery_state(id)
                .map_err(|_| Error::DeliveryUnknown("The delivery outcome cannot be read.".into()))? {
                Some(DeliveryState::Accepted | DeliveryState::Complete) => self.repair_known_accepted(id),
                Some(DeliveryState::Cancelled) => Err(Error::Invalid("This send was cancelled.".into())),
                Some(DeliveryState::Held) => Err(Error::DeliveryBusy),
                _ => Err(Error::DeliveryUnknown("No definite server receipt is available.".into())),
            };
        }
        let sent = match provider.send(&prepared.message).await {
            Ok(sent) => sent,
            Err(error) => {
                let uncertain = matches!(error, moss_providers::Error::DeliveryUnknown(_));
                let mut store = self.shared.store();
                let recorded = if uncertain { store.uncertain_delivery(id, "The server did not return a definite delivery receipt.") }
                    else { store.release_delivery(id, "The server did not accept the email.") };
                if recorded.is_err() {
                    return Err(Error::DeliveryUnknown("The outcome could not be saved.".into()));
                }
                return Err(error.into());
            }
        };
        let source = moss_core::MessageSource {
            account_id: prepared.email.from_account_id.clone(),
            mailbox: crate::lock(&self.shared.folders).get(&prepared.email.from_account_id)
                .and_then(|folders| folders.sent.clone()).unwrap_or_else(|| "Sent".into()),
            provider_id: sent.provider_id.unwrap_or_else(|| format!("local:{}", sent.message_id)),
            seen: true, received_at: Some(self.shared.now()), ..Default::default()
        };
        let source_json = serde_json::to_string(&source).expect("message sources serialize");
        if !self.shared.store().accept_delivery(id, &sent.raw, &source_json)
            .map_err(|_| Error::DeliveryUnknown("The server accepted the email, but its receipt could not be saved.".into()))? {
            return Err(Error::DeliveryUnknown("The acceptance state could not be recorded.".into()));
        }
        self.repair_known_accepted(id)
    }

    /// Replaying local repair never invokes the provider's send operation.
    pub fn repair_send(&self, id: &str) -> Result<String> {
        use moss_store::DeliveryState;
        let state = self.delivery_state(id)?
            .ok_or_else(|| Error::Unavailable("The delivery operation is missing.".into()))?;
        if matches!(state,DeliveryState::Held | DeliveryState::Uncertain) && self.send_cancelled(id)? {
            return Err(Error::Invalid("This send was cancelled.".into()));
        }
        match state {
            DeliveryState::Complete | DeliveryState::Accepted => self.repair_known_accepted(id),
            DeliveryState::Claimed | DeliveryState::Uncertain => Err(Error::DeliveryUnknown("No definite server receipt is available.".into())),
            DeliveryState::Held | DeliveryState::Cancelled => Err(Error::Invalid("There is no accepted delivery to recover.".into())),
        }
    }

    /// The caller has already observed durable acceptance. Later read failures cannot
    /// turn that knowledge into a claim that the message was not sent.
    fn repair_known_accepted(&self, id: &str) -> Result<String> {
        let mut queued = None;
        let mut changed = None;
        let result = {
            let mut store = self.shared.store();
            (|| {
                if store.delivery_state(id)? == Some(moss_store::DeliveryState::Complete) {
                    return store.delivery_receipt_thread(id)?.ok_or_else(|| Error::Invalid("This email was sent; its conversation is no longer on this Mac.".into()));
                }
                if store.delivery_resolved(id)? {
                    return Err(Error::Invalid("This delivery was reviewed. Its local repair is stopped.".into()));
                }
                let kept = store.delivery_repair(id)?
                    .ok_or_else(|| Error::Unavailable("The delivery operation is missing.".into()))?;
                self.repair_send_inner(&mut store, id, kept, &mut queued, &mut changed)
            })()
        };
        // Event callbacks and worker wakeups may reenter storage. Run them only
        // after releasing the guard, including effects committed before a failure.
        if let Some(account) = queued { self.shared.send_waiting(&account); }
        if let Some(account) = changed { self.shared.changed(&account); }
        result.map_err(|error| Error::DeliveryAccepted(error.to_string()))
    }

    fn repair_send_inner(&self, store: &mut moss_store::Store, id: &str, kept: moss_store::DeliveryRepair,
        queued: &mut Option<String>, changed: &mut Option<String>) -> Result<String> {
        let prepared: PreparedSend = serde_json::from_str(&kept.outgoing)
            .map_err(|_| Error::Invalid("The saved delivery cannot be read.".into()))?;
        let source: moss_core::MessageSource = serde_json::from_str(kept.source.as_deref().unwrap_or_default())
            .map_err(|_| Error::Invalid("The delivery source cannot be read.".into()))?;
        let raw = kept.raw.as_deref().ok_or_else(|| Error::Invalid("The accepted email copy is missing.".into()))?;
        let ingested = store.ingest(raw, &source)?;
        *changed = Some(source.account_id.clone());
        if let Some(original) = prepared.forwarded_message {
            *queued = self.record_forwarded(store, &original, &ingested.message_id)?;
        }
        if prepared.email.remind_if_no_reply {
            store.apply_delivery_reminder(id, &ingested.thread_id, &ingested.message_id, crate::sort::WAIT, self.shared.now())?;
        }
        let files: Vec<_> = prepared.email.attachments.iter().filter(|f| f.forwarded.is_none()).map(|f| f.id.clone()).collect();
        store.delete_draft_files(&files)?;
        if !store.complete_delivery_with_receipt(id, Some(&ingested.thread_id))? {
            return Err(Error::Invalid("The local delivery receipt could not be completed.".into()));
        }
        Ok(ingested.thread_id)
    }
}

impl Engine {
    pub fn delivery_state(&self, id: &str) -> Result<Option<moss_store::DeliveryState>> {
        Ok(self.shared.store().delivery_state(id)?)
    }

    /// Sending is stopped; this does not imply the draft was successfully restored.
    pub fn send_cancelled(&self, id: &str) -> Result<bool> {
        let store = self.shared.store();
        Ok(store.delivery_stopped(id)?)
    }

    /// Undo reads the restored version under the same store lock as cancellation.
    pub fn cancel_send_restoring(&self, id: &str) -> Result<CancelSendOutcome> {
        let outcome = {
            let mut store = self.shared.store();
            let restored: Result<CancelSendOutcome> = (|| {
                if store.delivery_stopped(id)? && store.delivery_resolved(id)? {
                    return Ok(CancelSendOutcome { cancelled: true, early: false, restored: None, restore_failed: false });
                }
                if store.delivery_state(id)? == Some(moss_store::DeliveryState::Uncertain) && store.delivery_restoration_pending(id)? {
                    return Ok(CancelSendOutcome { cancelled: true, early: false, restored: None, restore_failed: true });
                }
                let cancelled = store.delivery_state(id)? == Some(moss_store::DeliveryState::Cancelled) || store.cancel_delivery(id)?;
                if !cancelled { return Ok(CancelSendOutcome { cancelled: false, early: false, restored: None, restore_failed: false }); }
                let draft_id = store.delivery_draft_id(id)?;
                let early = draft_id.is_none();
                let restored = match draft_id { Some(id) => store.editable_draft(&id)?, None => None };
                Ok(CancelSendOutcome { cancelled, early, restored, restore_failed: false })
            })();
            match restored {
                Ok(outcome) => outcome,
                Err(error) => {
                    // This is an explicit partial outcome, not successful restoration.
                    // The original held payload remains durable and cannot be claimed.
                    if matches!(&error, Error::Store(moss_store::Error::UnreadableDeliveryDraft)) {
                        store.retain_interrupted_delivery(id, "This unsent draft cannot be read. Its saved data is retained for review.")?;
                    }
                    if !store.delivery_restoration_pending(id)? { return Err(error); }
                    CancelSendOutcome { cancelled: true, early: false, restored: None, restore_failed: true }
                }
            }
        };
        if let Some(restored) = &outcome.restored { self.shared.push_soon(&restored.from_account_id); }
        Ok(outcome)
    }

    /// Undo succeeds only while the database still says no submission has claimed it.
    pub fn cancel_send(&self, id: &str) -> Result<bool> {
        Ok(self.cancel_send_restoring(id)?.cancelled)
    }

}

impl Engine {
    /// Run once per engine lifetime, before a new hold or submission is allowed.
    /// A prior process's claim is evidence of uncertainty, never permission to retry.
    pub fn recover_deliveries(&self) -> Result<()> {
        use moss_store::DeliveryState;
        let mut recovered = crate::lock(&self.shared.delivery_recovered);
        if *recovered { return Ok(()); }
        self.shared.store().claim_delivery_session()?;
        let pending = self.shared.store().interrupted_deliveries()?;
        let mut failure=None;
        for (id,state,account_id) in pending {
            let result=(|| -> Result<()> {
                if state == DeliveryState::Held {
                    // Scheduled mail remains owned by its schedule; undo holds become drafts.
                    if self.shared.store().is_scheduled_delivery(&id)? { return Ok(()); }
                    self.restore_interrupted_hold(&id,account_id)?;
                } else {
                    // The lightweight sweep reads no possibly damaged draft payload.
                    self.shared.store().retain_interrupted_delivery(&id,"Moss stopped before a definite delivery outcome was saved. Check Sent before sending again.")?;
                }
                Ok(())
            })();
            if let Err(error)=result { failure.get_or_insert(error); }
        }
        // Keep sending gated until every prior owner is recovered, but do not let
        // one bad draft hide later uncertain deliveries from the review list.
        if let Some(error)=failure { return Err(error); }
        *recovered = true;
        drop(recovered);
        // Local receipt repair is independent of recovering prior process ownership.
        // A failure must not rerun the sweep over holds this process creates later.
        if self.repair_due_deliveries().is_err() {
            eprintln!("Moss: local delivery repair is waiting for its retry record.");
        }
        Ok(())
    }

    pub(crate) fn restore_interrupted_hold(&self,id:&str,account_id:String)->Result<()> {
        let restored=self.shared.store().cancel_interrupted_delivery(id,self.shared.now());
        match restored {
            Ok(true)=>{
                self.shared.events.emit(crate::Event::DraftsChanged {account_id});
                self.shared.events.emit(crate::Event::SendsInterrupted);
            }
            Ok(false)=>{}, // Another client may already have cancelled this snapshot.
            Err(moss_store::Error::UnreadableDeliveryDraft)=>self.shared.store().retain_interrupted_delivery(id,"This unsent draft cannot be read. Its saved data is retained for review.")?,
            Err(error)=>return Err(error.into()),
        }
        Ok(())
    }

    /// Only local repair is retried. The durable batch reservation limits work and
    /// prevents restart or repeated wakes from bypassing a failed repair's backoff.
    pub(crate) fn repair_due_deliveries(&self) -> Result<()> {
        let ids=self.shared.store().claim_delivery_repairs(self.shared.now())?;
        for id in ids {
            if self.repair_send(&id).is_err() {
                eprintln!("Moss: an accepted email still needs local recovery.");
            }
        }
        Ok(())
    }
}

impl Engine {
    /// Direct sends retain the caller's recoverable draft and reuse its active receipt.
    pub async fn send_saved(&self, draft: &Draft, email: &OutgoingDraft) -> Result<String> {
        self.recover_deliveries()?;
        let existing = self.shared.store().active_delivery_id(&draft.id)?;
        if let Some(id) = &existing {
            if matches!(self.delivery_state(id)?, Some(moss_store::DeliveryState::Held | moss_store::DeliveryState::Claimed)) {
                return Err(Error::DeliveryBusy);
            }
        }
        let operation = existing.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let created = match self.prepare_send(&operation, draft, email).await {
            Ok(created) => created,
            Err(error) => {
                // Another preparation may have won while content was being assembled.
                if self.shared.store().active_delivery_id(&draft.id)?.is_some_and(|owner| owner != operation) {
                    return Err(Error::DeliveryBusy);
                }
                return Err(error);
            }
        };
        match self.submit_send(&operation).await {
            Ok(thread) => Ok(thread),
            Err(error @ (Error::DeliveryUnknown(_) | Error::DeliveryAccepted(_) | Error::DeliveryBusy)) => Err(error),
            Err(error) => {
                if created && self.cancel_send(&operation).is_err() {
                    eprintln!("Moss: an unsent email needs draft recovery.");
                }
                Err(error)
            }
        }
    }

    /// A repeated send by draft ID can inspect its receipt without recreating Drafts.
    pub fn delivery_receipt(&self, draft_id: &str) -> Result<Option<moss_store::DeliveryReceipt>> {
        Ok(self.shared.store().delivery_receipt(draft_id)?)
    }
}

impl Engine {
    /// Emails you sent that Moss stopped before they went (it quit during Undo), back in
    /// Drafts, until you say you've seen them.
    pub fn interrupted_sends(&self) -> Result<Vec<moss_store::InterruptedSend>> {
        Ok(self.shared.store().interrupted_sends()?)
    }

    pub fn dismiss_interrupted_sends(&self, ids: &[String]) -> Result<()> {
        Ok(self.shared.store().dismiss_interrupted_sends(ids)?)
    }

    /// Recovery decisions must remain available even when the startup sweep fails.
    pub fn delivery_attention(&self, after: Option<&moss_store::DeliveryAttentionCursor>) -> Result<moss_store::DeliveryAttentionPage> {
        Ok(self.shared.store().delivery_attention(after,50)?)
    }
    pub fn delivery_attention_draft(&self, id: &str) -> Result<Option<Draft>> {
        Ok(self.shared.store().delivery_attention_draft(id)?)
    }
    pub fn resolve_delivery(&self, id: &str, action: moss_store::DeliveryResolution) -> Result<moss_store::DeliveryResolutionOutcome> {
        let outcome={
            let going=crate::lock(&self.shared.scheduled_going);
            let mut store=self.shared.store();
            if store.delivery_draft_id(id)?.is_some_and(|draft|going.contains(&draft)) {
                return Ok(moss_store::DeliveryResolutionOutcome::Sending);
            }
            store.resolve_delivery(id,action,self.shared.now())?
        };
        if matches!(outcome,moss_store::DeliveryResolutionOutcome::Resolved {..}) {
            self.shared.events.emit(crate::Event::ScheduledChanged);
            self.shared.scheduled_wake.notify_one();
        }
        Ok(outcome)
    }
}
