//! Background AI work waits while the Mac runs on its battery: indexing mail by meaning and
//! the summaries nobody has asked for yet. It picks up again once the Mac is back on power.
//! What you ask for (Ask Moss, Quick reply, a summary you open, a search) always runs.

use std::sync::Arc;
use std::time::Duration;

use moss_ai::power::PowerSource;
use moss_core::{SummaryContent, SummaryRequest};
use moss_store::{ToEmbed, ToSummarize};

/// How often Moss checks whether the Mac is back on power.
pub const RECHECK: Duration = Duration::from_secs(60);

/// An embedder or summarizer whose background work runs only while the Mac is on power.
pub struct OnPower<T> {
    pub inner: T,
    pub power: Arc<dyn PowerSource>,
}

#[async_trait::async_trait]
impl<T: moss_engine::Embedder> moss_engine::Embedder for OnPower<T> {
    fn model(&self) -> String {
        self.inner.model()
    }

    /// Indexing in the background: `None` (the engine stops, and goes on when woken) on battery.
    async fn documents(&self, emails: &[ToEmbed]) -> Option<Vec<Vec<f32>>> {
        if self.power.on_battery() {
            return None;
        }
        self.inner.documents(emails).await
    }

    /// Someone searching: always.
    async fn query(&self, text: &str) -> Option<Vec<f32>> {
        self.inner.query(text).await
    }
}

#[async_trait::async_trait]
impl<T: moss_engine::Summarizer> moss_engine::Summarizer for OnPower<T> {
    fn model(&self) -> String {
        self.inner.model()
    }

    async fn summarize(&self, conversation: &ToSummarize) -> Option<String> {
        if self.power.on_battery() {
            return None;
        }
        self.inner.summarize(conversation).await
    }

    async fn summarize_as(&self, conversation: &ToSummarize, request: &SummaryRequest) -> Option<SummaryContent> {
        if self.power.on_battery() {
            return None;
        }
        self.inner.summarize_as(conversation, request).await
    }
}

/// Every `every`, asks where the power comes from, and calls `resume` each time the Mac goes
/// from its battery back to power. Runs until dropped.
pub async fn resume_on_power(power: Arc<dyn PowerSource>, every: Duration, resume: impl Fn() + Send) {
    let mut every = tokio::time::interval(every);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    every.tick().await;
    let mut was = power.on_battery();
    loop {
        every.tick().await;
        let now = power.on_battery();
        if was && !now {
            resume();
        }
        was = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moss_engine::{Embedder, Summarizer};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Battery(AtomicBool);

    impl PowerSource for Battery {
        fn on_battery(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    /// Answers everything, counting what it was asked.
    #[derive(Default)]
    struct Model(AtomicUsize);

    #[async_trait::async_trait]
    impl Embedder for Model {
        fn model(&self) -> String {
            "m".into()
        }
        async fn documents(&self, emails: &[ToEmbed]) -> Option<Vec<Vec<f32>>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(emails.iter().map(|_| vec![1.0]).collect())
        }
        async fn query(&self, _: &str) -> Option<Vec<f32>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(vec![1.0])
        }
    }

    #[async_trait::async_trait]
    impl Summarizer for Model {
        fn model(&self) -> String {
            "m".into()
        }
        async fn summarize(&self, _: &ToSummarize) -> Option<String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some("s".into())
        }
        async fn summarize_as(&self, _: &ToSummarize, _: &SummaryRequest) -> Option<SummaryContent> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(SummaryContent::Text { text: "s".into() })
        }
    }

    fn conversation() -> ToSummarize {
        ToSummarize {
            thread_id: 1,
            account_id: "w".into(),
            subject: "Deck".into(),
            through: 2,
            messages: vec![],
            you: "maya@northwind.co".into(),
            why: moss_core::SummaryWhy::Important,
        }
    }

    #[tokio::test]
    async fn background_work_waits_on_battery_but_searching_never_does() {
        let battery = Arc::new(Battery::default());
        let gated = OnPower { inner: Model::default(), power: battery.clone() };
        let email = ToEmbed { message_id: 1, subject: "Flight".into(), text: "Lisbon".into() };
        let facts = SummaryRequest::of(moss_core::SummaryKind::Facts);
        assert_eq!(Embedder::model(&gated), "m");
        assert_eq!(Summarizer::model(&gated), "m");
        assert!(gated.documents(std::slice::from_ref(&email)).await.is_some());
        assert!(gated.summarize(&conversation()).await.is_some());
        assert!(gated.summarize_as(&conversation(), &facts).await.is_some());
        battery.0.store(true, Ordering::SeqCst);
        assert_eq!(gated.documents(&[email]).await, None);
        assert_eq!(gated.summarize(&conversation()).await, None);
        assert_eq!(gated.summarize_as(&conversation(), &facts).await, None);
        assert_eq!(gated.inner.0.load(Ordering::SeqCst), 3, "nothing reached the model on battery");
        assert!(gated.query("trips").await.is_some(), "a search you typed runs anyway");
    }

    #[tokio::test(start_paused = true)]
    async fn background_work_resumes_when_the_mac_is_back_on_power() {
        let battery = Arc::new(Battery(AtomicBool::new(true)));
        let resumed = Arc::new(AtomicUsize::new(0));
        let count = resumed.clone();
        let watch = tokio::spawn(resume_on_power(battery.clone(), RECHECK, move || {
            count.fetch_add(1, Ordering::SeqCst);
        }));
        tokio::time::sleep(RECHECK * 3).await;
        assert_eq!(resumed.load(Ordering::SeqCst), 0, "still on battery");
        battery.0.store(false, Ordering::SeqCst);
        tokio::time::sleep(RECHECK + Duration::from_secs(1)).await;
        assert_eq!(resumed.load(Ordering::SeqCst), 1);
        tokio::time::sleep(RECHECK * 3).await;
        assert_eq!(resumed.load(Ordering::SeqCst), 1, "once per return to power");
        battery.0.store(true, Ordering::SeqCst);
        tokio::time::sleep(RECHECK).await;
        battery.0.store(false, Ordering::SeqCst);
        tokio::time::sleep(RECHECK).await;
        assert_eq!(resumed.load(Ordering::SeqCst), 2);
        watch.abort();
        assert_eq!(RECHECK, Duration::from_secs(60));
    }
}
