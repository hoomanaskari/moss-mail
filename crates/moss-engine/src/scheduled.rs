//! Send later. Mail servers here can't send at a time, so an email set to send later waits
//! on this Mac (the store's `scheduled` table, beside the outbox) and the engine sends it at
//! its time while Moss runs. One due while Moss was closed goes soon after the next launch,
//! and says it was late. A failed try stays, says why, and is tried again a little later
//! each time: an email set to send later is never lost.

use std::time::Duration;

use moss_core::{Draft, ScheduledEmail};

use crate::{Engine, Error, Event, OutgoingDraft, Result};

const MINUTE: i64 = 60_000;
/// How long after each failed try the next one waits; then every hour.
const RETRY_MINUTES: [i64; 6] = [1, 2, 5, 15, 30, 60];
/// Later than this after its time, an email went late: Moss was closed or the Mac asleep.
const LATE: i64 = 2 * MINUTE;
/// How long after launch the first look waits, so the window is up to hear what went late.
const FIRST_LOOK: Duration = Duration::from_secs(10);
/// The longest the engine waits before looking again, so a Mac that slept through an
/// email's time still sends it soon after waking.
const LOOK_AGAIN: Duration = Duration::from_secs(30);

/// An email on its way: off the list again when its try is over, however it ends. A panic
/// mid-send must not leave it "sending" for good. The delivery's own claim is what stops it
/// going twice (see [`Engine::submit_send`]): after a panic past the claim, nobody knows
/// whether the server took it, so it is held for you to check, as after a crash.
struct Going<'a>(&'a Engine, String);

impl Drop for Going<'_> {
    fn drop(&mut self) {
        crate::lock(&self.0.shared.scheduled_going).remove(&self.1);
        if std::thread::panicking() {
            let mut store = self.0.shared.store();
            if let Ok(Some(operation)) = store.scheduled_operation(&self.1) {
                let _ = store.uncertain_delivery(&operation, INTERRUPTED);
            }
        }
    }
}

/// Why a scheduled email whose send was cut short by a bug wasn't sent again.
const INTERRUPTED: &str = "Moss stopped before a definite delivery outcome was saved. Check Sent before sending again.";

fn retry_after(tries: u32) -> i64 {
    let i = (tries as usize).clamp(1, RETRY_MINUTES.len()) - 1;
    RETRY_MINUTES[i] * MINUTE
}

impl Engine {
    /// Sets an email to send at `send_at` (ms; a time past means now) and takes its draft out
    /// of Drafts. `draft` opens again if you edit or cancel it; `email` is what goes. Checked
    /// now, so an email that can't go says so before you leave it.
    pub fn schedule(&self, draft: &Draft, email: &OutgoingDraft, send_at: i64) -> Result<()> {
        self.schedule_with_operation(&uuid::Uuid::new_v4().to_string(), draft, email, send_at).map(|_| ())
    }

    pub fn schedule_with_operation(&self, operation: &str, draft: &Draft, email: &OutgoingDraft, send_at: i64) -> Result<moss_store::ScheduleOutcome> {
        self.recover_deliveries()?;
        let going = crate::lock(&self.shared.scheduled_going);
        let json = serde_json::to_string(email).expect("emails serialise");
        {
            let store = self.shared.store();
            if let Some(outcome) = store.existing_schedule_outcome(operation, draft, &json)? {
                let busy = going.contains(&draft.id) && store.scheduled_operation(&draft.id)?.as_deref() == Some(operation);
                return Ok(if busy && matches!(outcome, moss_store::ScheduleOutcome::AlreadyScheduled | moss_store::ScheduleOutcome::InDelivery { .. }) { moss_store::ScheduleOutcome::Sending } else { outcome });
            }
        }
        if email.to.is_empty() && email.cc.is_empty() && email.bcc.is_empty() {
            return Err(Error::NoRecipients);
        }
        let account = self.shared.account(&email.from_account_id)?;
        self.shared.sender(&account, email.from_address.as_deref())?;
        let at = send_at.max(self.shared.now());
        let (outcome, cleanup_account) = {
            let mut store = self.shared.store();
            let mut cleanup_account = None;
            let outcome = store.schedule_operation_with_change(operation, draft, &json, at, self.shared.now(), |store| {
                store.require_draft_revision(&draft.id, draft.revision.as_deref())?;
                let cleanup = crate::drafts::saved_cleanup(store, &draft.id)?;
                cleanup_account = cleanup.as_ref().map(|(account, _)| account.clone());
                Ok(cleanup)
            })?;
            (outcome, cleanup_account)
        };
        drop(going);
        if let Some(account) = cleanup_account { self.shared.send_waiting(&account); }
        if outcome == moss_store::ScheduleOutcome::Scheduled {
            self.shared.scheduled_wake.notify_one();
            self.shared.events.emit(Event::ScheduledChanged);
        }
        Ok(outcome)
    }

    /// Emails set to send later, the soonest first.
    pub fn scheduled(&self) -> Result<Vec<ScheduledEmail>> {
        Ok(self.shared.store().scheduled_emails()?)
    }

    /// A new time for a scheduled email (a time past means now: Send now).
    #[cfg(test)]
    pub fn reschedule(&self, id: &str, send_at: i64) -> Result<()> {
        let going = crate::lock(&self.shared.scheduled_going);
        Self::not_going(&going, id)?;
        let at = send_at.max(self.shared.now());
        if !self.shared.store().reschedule_email(id, at)? {
            return Err(gone(id));
        }
        drop(going);
        self.shared.scheduled_wake.notify_one();
        self.shared.events.emit(Event::ScheduledChanged);
        Ok(())
    }

    /// Won't send it: the email goes back to Drafts as it was written, and is returned.
    #[cfg(test)]
    pub fn unschedule(&self, id: &str) -> Result<Draft> {
        let going = crate::lock(&self.shared.scheduled_going);
        Self::not_going(&going, id)?;
        let draft = {
            let mut store = self.shared.store();
            let restored = store.unschedule_email(id, self.shared.now())?.ok_or_else(|| gone(id))?;
            store.editable_draft(&restored.id)?.ok_or(Error::DraftConflict)?
        };
        drop(going);
        // A draft again: up to the server's Drafts too.
        self.shared.push_soon(&draft.from_account_id);
        self.shared.events.emit(Event::ScheduledChanged);
        Ok(draft)
    }

    pub fn cancel_schedule_with_operation(&self, operation: &str, id: &str) -> Result<moss_store::ScheduleOutcome> {
        self.cancel_schedule(operation, id, true)
    }

    pub fn cancel_known_schedule(&self, operation: &str, id: &str) -> Result<moss_store::ScheduleOutcome> {
        self.cancel_schedule(operation, id, false)
    }

    fn cancel_schedule(&self, operation: &str, id: &str, allow_early: bool) -> Result<moss_store::ScheduleOutcome> {
        let (outcome, changed) = {
            let going = crate::lock(&self.shared.scheduled_going);
            let mut store = self.shared.store();
            if store.scheduled_operation(id)?.as_deref() == Some(operation) && going.contains(id) {
                return Ok(moss_store::ScheduleOutcome::Sending);
            }
            let changed = store.scheduled_operation(id)?.as_deref() == Some(operation);
            let outcome = if allow_early { store.cancel_schedule_operation(operation, id, self.shared.now())? }
                else { store.cancel_known_schedule_operation(operation, id, self.shared.now())? };
            let outcome = match outcome {
                moss_store::ScheduleOutcome::Cancelled { early: false, restored: Some(draft) } => moss_store::ScheduleOutcome::Cancelled {
                    early: false,
                    restored: Some(store.editable_draft(&draft.id)?.ok_or(Error::DraftConflict)?),
                },
                outcome => outcome,
            };
            (outcome, changed)
        };
        if let moss_store::ScheduleOutcome::Cancelled { early: false, restored: Some(draft) } = &outcome {
            self.shared.push_soon(&draft.from_account_id);
            if changed { self.shared.events.emit(Event::ScheduledChanged); }
        }
        Ok(outcome)
    }

    pub fn reschedule_with_revision(&self, operation: &str, id: &str, send_at: i64, expected_revision: i64) -> Result<moss_store::ScheduleOutcome> {
        self.change_schedule_time(operation, id, send_at, Some(expected_revision))
    }

    #[cfg(test)]
    pub fn reschedule_with_operation(&self, operation: &str, id: &str, send_at: i64) -> Result<moss_store::ScheduleOutcome> {
        self.change_schedule_time(operation, id, send_at, None)
    }

    fn change_schedule_time(&self, operation: &str, id: &str, send_at: i64, expected_revision: Option<i64>) -> Result<moss_store::ScheduleOutcome> {
        let outcome = {
            let going = crate::lock(&self.shared.scheduled_going);
            let mut store = self.shared.store();
            if store.scheduled_operation(id)?.as_deref() == Some(operation) && going.contains(id) {
                return Ok(moss_store::ScheduleOutcome::Sending);
            }
            match expected_revision {
                Some(revision) => store.reschedule_at_revision(operation, id, send_at.max(self.shared.now()), revision)?,
                None => store.reschedule_operation(operation, id, send_at.max(self.shared.now()))?,
            }
        };
        if matches!(outcome, moss_store::ScheduleOutcome::Scheduled | moss_store::ScheduleOutcome::Rescheduled { .. }) {
            self.shared.scheduled_wake.notify_one();
            self.shared.events.emit(Event::ScheduledChanged);
        }
        Ok(outcome)
    }

    /// An email on its way can't be changed any more.
    #[cfg(test)]
    fn not_going(going: &std::collections::HashSet<String>, id: &str) -> Result<()> {
        if going.contains(id) {
            return Err(Error::Invalid("It’s sending now.".into()));
        }
        Ok(())
    }

    /// Sends every email that is due, one after another, and says how each went.
    pub(crate) async fn send_due(&self) -> Option<i64> {
        if self.recover_deliveries().is_err() {
            eprintln!("Moss: scheduled mail is waiting for delivery recovery.");
            let reason="Sending is waiting for delivery recovery. No new send attempt has been made.";
            let due={
                let mut store=self.shared.store();
                // Even if storage is read-only, show the reason for readable due rows.
                let _=store.note_due_email_wait(self.shared.now(),reason);
                store.due_emails(self.shared.now())
            };
            if let Ok(due)=due {
                for (mut email,_) in due {
                    email.error=Some(reason.into());
                    let announce={
                        let mut announced=crate::lock(&self.shared.scheduled_recovery_announced);
                        announced.insert(email.operation_id.clone(),email.revision)!=Some(email.revision)
                    };
                    if announce { self.shared.events.emit(Event::ScheduledWaiting {email}); }
                }
            }
            return None;
        }
        crate::lock(&self.shared.scheduled_recovery_announced).clear();
        // Repairing an already accepted local copy is independent of new scheduled
        // submissions. Its failed bookkeeping must not silently delay those emails.
        if self.repair_due_deliveries().is_err() {
            eprintln!("Moss: local delivery repair is waiting for its retry record.");
        }
        let now = self.shared.now();
        let due = match self.shared.store().due_emails(now) {
            Ok(due) => due,
            Err(e) => { eprintln!("Moss: can't read the emails to send later: {e}"); return None; },
        };
        for (snapshot, _) in due {
            let id = snapshot.draft.id.clone();
            // Serialize the final eligibility check and claim with every schedule edit.
            // A snapshot taken before another send awaited the network is not authority.
            let claimed = {
                let mut going = crate::lock(&self.shared.scheduled_going);
                if going.contains(&id) { continue; }
                match self.shared.store().due_email(&id, self.shared.now()) {
                    Ok(current) => current.map(|email| { going.insert(id.clone()); (email, Going(self, id.clone())) }),
                    Err(e) => { eprintln!("Moss: can't claim a scheduled email: {e}"); None }
                }
            };
            let Some(((email, json), _going)) = claimed else { continue; };
            let result = match serde_json::from_str::<OutgoingDraft>(&json) {
                Ok(outgoing) => self.submit_scheduled(&email.draft, &outgoing).await,
                Err(e) => Err(Error::Invalid(format!("Moss can’t read this email: {e}"))),
            };
            self.sent_or_failed(email, result);
        }
        Some(now)
    }

    async fn submit_scheduled(&self, draft: &Draft, outgoing: &OutgoingDraft) -> Result<()> {
        let operation = self.shared.store().scheduled_operation(&draft.id)?
            .ok_or_else(|| gone(&draft.id))?;
        self.prepare_scheduled_send(&operation, draft, outgoing).await?;
        self.submit_send(&operation).await.map(|_| ())
    }

    fn sent_or_failed(&self, email: ScheduledEmail, result: Result<()>) {
        let now = self.shared.now();
        let id = email.draft.id.clone();
        let event = match result {
            Ok(()) => {
                let forgot = self.shared.store().email_sent(&id, self.shared.now());
                if forgot.is_err() {
                    eprintln!("Moss: a sent email still needs schedule cleanup.");
                    // The completed receipt prevents another submission. Back off local
                    // cleanup too, rather than repeatedly waking on an already-due row.
                    let _ = self.shared.store().email_failed(&id, "Sent. Local schedule cleanup is pending.", now + retry_after(email.tries + 1));
                }
                Event::ScheduledSent { late: now - email.send_at > LATE, email }
            }
            Err(e) => {
                let tries = email.tries + 1;
                let next_try = now + retry_after(tries);
                let error = e.to_string();
                if let Err(e) = self.shared.store().email_failed(&id, &error, next_try) {
                    eprintln!("Moss: can't note that {id} failed: {e}");
                }
                Event::ScheduledFailed { email: ScheduledEmail { tries, next_try, error: Some(error), ..email } }
            }
        };
        self.shared.events.emit(event);
    }

    /// Sends emails set to send later at their time, from now on, in the background.
    pub(crate) fn send_later(&self) {
        let engine = self.clone();
        crate::keep_running(move || {
            let engine = engine.clone();
            async move {
                tokio::time::sleep(FIRST_LOOK).await;
                loop {
                    let cutoff=engine.send_due().await;
                    let next=cutoff.and_then(|_|engine.shared.store().next_due().ok().flatten());
                    let wait=scheduler_wait(next,engine.shared.now(),cutoff);
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = engine.shared.scheduled_wake.notified() => {}
                    }
                }
            }
        });
    }
}

/// Back off rows a pass already considered. A deadline crossed during a slow send
/// was not in that snapshot and must get an immediate next pass instead.
fn scheduler_wait(next:Option<i64>,now:i64,cutoff:Option<i64>)->Duration {
    let Some(cutoff)=cutoff else {return LOOK_AGAIN;};
    next.map_or(LOOK_AGAIN,|at| {
        if at<=cutoff {LOOK_AGAIN}
        else {Duration::from_millis(at.saturating_sub(now).max(0) as u64).min(LOOK_AGAIN)}
    })
}

#[cfg(test)]
mod wait_tests {
    use super::*;
    #[test]
    fn recovery_wait_deadlines_never_spin_and_preserve_future_wakes() {
        for next in [None,Some(99),Some(100),Some(30_100),Some(i64::MAX)] {
            assert_eq!(scheduler_wait(next,100,Some(100)),LOOK_AGAIN);
            assert_eq!(scheduler_wait(next,100,None),LOOK_AGAIN);
        }
        assert_eq!(scheduler_wait(Some(101),100,Some(100)),Duration::from_millis(1));
        assert_eq!(scheduler_wait(Some(101),105,Some(100)),Duration::ZERO);
        assert_eq!(scheduler_wait(Some(106),105,Some(100)),Duration::from_millis(1));
        assert_eq!(scheduler_wait(Some(i64::MAX),i64::MIN,Some(i64::MIN)),LOOK_AGAIN);
    }
}

fn gone(id: &str) -> Error {
    Error::Unavailable(format!("scheduled email {id}: sent or cancelled already"))
}
