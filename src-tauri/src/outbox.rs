//! Sent email waits here for a few seconds, so Undo can take it back. Quitting sends what is
//! waiting at once instead of losing it.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::{oneshot, watch};

/// Emails held after Send, by the id the window gave each one.
pub struct Outbox {
    held: Mutex<HoldState>,
    /// Emails held or on their way; quitting waits for this to reach 0.
    busy: watch::Sender<usize>,
}

struct HoldState {
    /// `None` once Moss is quitting: uncancelled mail no longer waits.
    pending: Option<HashMap<String, oneshot::Sender<bool>>>,
    cancelled: HashSet<String>,
}

impl Default for Outbox {
    fn default() -> Self {
        Self {
            held: Mutex::new(HoldState { pending: Some(HashMap::new()), cancelled: HashSet::new() }),
            busy: watch::Sender::new(0),
        }
    }
}

/// An email that may go now. Holding it keeps quitting waiting until the send is over.
pub struct Going<'a>(&'a Outbox);

impl Drop for Going<'_> {
    fn drop(&mut self) {
        self.0.busy.send_modify(|n| *n -= 1);
    }
}

impl Outbox {
    /// Waits `delay`, then lets the email go. `None` when [`Outbox::cancel`] took it back.
    #[cfg(test)]
    pub async fn hold(&self, id: &str, delay: Duration) -> Option<Going<'_>> {
        self.hold_with_registered(id,delay,||{}).await
    }

    /// Acknowledge only after shutdown owns this already-persisted delivery.
    pub async fn hold_with_registered(&self,id:&str,delay:Duration,registered:impl FnOnce())->Option<Going<'_>> {
        self.busy.send_modify(|n| *n += 1);
        let going = Going(self);
        let (release, released) = oneshot::channel();
        let waiting = {
            let mut state = self.held.lock().unwrap();
            if state.cancelled.contains(id) { return None; }
            match state.pending.as_mut() {
                Some(held) => { held.insert(id.to_string(), release); true },
                None => false,
            }
        };
        registered();
        if !waiting {return Some(going);}
        let send = tokio::select! {
            _ = tokio::time::sleep(delay) => true,
            answer = released => answer.unwrap_or(true),
        };
        if let Some(held) = self.held.lock().unwrap().pending.as_mut() {
            held.remove(id);
        }
        send.then_some(going)
    }

    /// Holds the email, then sends it unless Undo took it back first. False when it did.
    #[cfg(test)]
    pub async fn send_after<T, E>(
        &self,
        id: &str,
        delay: Duration,
        send: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<bool, E> {
        let Some(_going) = self.hold(id, delay).await else {
            return Ok(false);
        };
        send.await?;
        Ok(true)
    }

    /// Takes back a held email. False when it isn't held: already on its way, or unknown.
    pub fn cancel(&self, id: &str) -> bool {
        let release = {
            let mut state = self.held.lock().unwrap();
            // Keep the intent for this session, including registration after Undo
            // or shutdown. Durable storage may have failed independently.
            state.cancelled.insert(id.to_string());
            state.pending.as_mut().and_then(|held| held.remove(id))
        };
        release.is_some_and(|release| release.send(false).is_ok())
    }

    /// Emails held or on their way.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        *self.busy.borrow()
    }

    /// Quitting: lets every held email go now, and waits up to `limit` for all of them to be sent.
    pub async fn flush(&self, limit: Duration) {
        let held = self.held.lock().unwrap().pending.take().unwrap_or_default();
        for release in held.into_values() {
            let _ = release.send(true);
        }
        let mut busy = self.busy.subscribe();
        let _ = tokio::time::timeout(limit, busy.wait_for(|n| *n == 0)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::time::Instant;

    const SIX: Duration = Duration::from_secs(6);

    #[tokio::test(start_paused = true)]
    async fn early_undo_is_remembered_before_registration_and_after_quit() {
        let outbox = Outbox::default();
        assert!(!outbox.cancel("early"), "there is no registered timer yet");
        assert!(outbox.hold("early", Duration::ZERO).await.is_none());
        assert_eq!(outbox.waiting(), 0);
        let called = std::sync::atomic::AtomicBool::new(false);
        let send = || async {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok::<(), ()>(())
        };
        let result = outbox.send_after("early", SIX, send()).await;
        assert_eq!(result, Ok(false));
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        outbox.flush(Duration::ZERO).await;
        assert!(outbox.hold("early", SIX).await.is_none());
        assert!(!outbox.cancel("after-quit"));
        assert!(outbox.hold("after-quit", SIX).await.is_none());
        assert_eq!(outbox.send_after("different", SIX, send()).await, Ok(true));
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(outbox.waiting(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn preparation_acknowledgement_follows_shutdown_registration() {
        let outbox=Outbox::default();let called=std::sync::atomic::AtomicBool::new(false);
        let going=outbox.hold_with_registered("accepted",Duration::ZERO,|| {
            assert_eq!(outbox.waiting(),1);
            assert!(outbox.held.lock().unwrap().pending.as_ref().unwrap().contains_key("accepted"));
            called.store(true,std::sync::atomic::Ordering::SeqCst);
        }).await;
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
        drop(going);assert_eq!(outbox.waiting(),0);
        outbox.flush(Duration::ZERO).await;
        let going=outbox.hold_with_registered("during-quit",SIX,||assert_eq!(outbox.waiting(),1)).await;
        assert!(going.is_some());drop(going);assert_eq!(outbox.waiting(),0);
    }

    #[tokio::test(start_paused = true)]
    async fn an_email_waits_its_delay_then_goes() {
        let outbox = Outbox::default();
        let start = Instant::now();
        assert!(outbox.hold("a", SIX).await.is_some());
        assert_eq!(start.elapsed(), SIX);
        assert!(!outbox.cancel("a"), "gone, so nothing to take back");
    }

    #[tokio::test(start_paused = true)]
    async fn undo_takes_a_held_email_back() {
        let outbox = Arc::new(Outbox::default());
        let held = tokio::spawn({
            let outbox = outbox.clone();
            async move { outbox.hold("a", SIX).await.is_some() }
        });
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(outbox.cancel("a"));
        assert!(!held.await.unwrap(), "it never goes");
        assert!(!outbox.cancel("a"));
        assert!(!outbox.cancel("unknown"));
    }

    #[tokio::test(start_paused = true)]
    async fn quitting_sends_held_email_at_once_and_waits_for_it() {
        let outbox = Arc::new(Outbox::default());
        let (sent_tx, sent_rx) = oneshot::channel();
        let sender = tokio::spawn({
            let outbox = outbox.clone();
            async move {
                let going = outbox.hold("a", Duration::from_secs(60)).await;
                // Sending takes a moment; quitting waits for it.
                tokio::time::sleep(Duration::from_secs(1)).await;
                sent_tx.send(going.is_some()).unwrap();
            }
        });
        tokio::task::yield_now().await;
        let start = Instant::now();
        outbox.flush(Duration::from_secs(30)).await;
        assert_eq!(start.elapsed(), Duration::from_secs(1));
        assert!(sent_rx.await.unwrap());
        sender.await.unwrap();
        // After that, Send no longer waits at all.
        let start = Instant::now();
        assert!(outbox.hold("b", SIX).await.is_some());
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn quitting_gives_up_on_a_send_that_takes_too_long() {
        let outbox = Arc::new(Outbox::default());
        let outbox2 = outbox.clone();
        // Never finishes sending.
        let stuck = tokio::spawn(async move { let _going = outbox2.hold("a", SIX).await; std::future::pending::<()>().await });
        tokio::task::yield_now().await;
        let start = Instant::now();
        outbox.flush(Duration::from_secs(30)).await;
        assert_eq!(start.elapsed(), Duration::from_secs(30));
        stuck.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn send_after_sends_once_held_and_not_when_taken_back() {
        let outbox = Arc::new(Outbox::default());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let send = |what: &'static str| {
            let sent = sent.clone();
            async move {
                sent.lock().unwrap().push(what);
                Ok::<_, String>(())
            }
        };
        assert_eq!(outbox.send_after("a", SIX, send("a")).await, Ok(true));
        let undone = tokio::spawn({
            let outbox = outbox.clone();
            let send = send("b");
            async move { outbox.send_after("b", SIX, send).await }
        });
        tokio::task::yield_now().await;
        assert!(outbox.cancel("b"));
        assert_eq!(undone.await.unwrap(), Ok(false));
        assert_eq!(*sent.lock().unwrap(), ["a"]);
        // A failed send says why.
        assert_eq!(outbox.send_after("c", SIX, async { Err::<(), _>("offline") }).await, Err("offline"));
    }

    #[tokio::test]
    async fn quitting_with_nothing_held_is_instant() {
        Outbox::default().flush(Duration::from_secs(30)).await;
    }
}
