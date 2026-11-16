//! Summaries of conversations, written by the writing model.
//!
//! In the background, once it's downloaded, and again when a new message arrives: first what
//! your "always summarise mail from…" rules ask for, then the one-liners of the conversations
//! that matter (unless you turned those off). And now, in the kind you pick, when you ask:
//! see [`Engine::summary_for`].

use std::sync::Arc;

use async_trait::async_trait;
use moss_core::{SummaryContent, SummaryRequest, SummaryRule, SummaryView, ThreadSummaries};
use moss_store::ToSummarize;

use crate::events::Event;
use crate::{Engine, Result, Shared};

/// The writing model, as the engine sees it for summaries. The app passes its AI; tests
/// pass a fake.
#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Which model writes them; saved with each, so a new model writes them again.
    fn model(&self) -> String;
    /// One or two lines on the conversation, or `None` while the model can't run.
    async fn summarize(&self, conversation: &ToSummarize) -> Option<String>;
    /// A summary in the kind asked for, or `None` while the model can't run.
    async fn summarize_as(&self, conversation: &ToSummarize, request: &SummaryRequest) -> Option<SummaryContent>;
}

const PER_ROUND: usize = 10;

/// Why [`Engine::summarize_now`] has no summary: the engine failed, or the writing model did.
/// Either reaches the UI as its own `{kind, message}`.
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum SummaryFailure<E> {
    Engine(crate::Error),
    Write(E),
}

/// What to do for a summary you asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryNeed {
    /// A fresh one is kept: show it.
    Kept(SummaryView),
    /// Have the writing model write this conversation, then [`Engine::keep_summary`] it.
    Write(ToSummarize),
}

impl Shared {
    /// Writes every summary waiting: rules' first, then the one-liners, newest first. Stops
    /// when none are left, or the model can't run.
    pub(crate) async fn summarize_waiting(&self, summarizer: &dyn Summarizer) {
        let model = summarizer.model();
        if self.summarize_by_rule(summarizer, &model).await {
            self.summarize_automatic(summarizer, &model).await;
        }
    }

    /// What rules ask for. False when it had to stop.
    async fn summarize_by_rule(&self, summarizer: &dyn Summarizer, model: &str) -> bool {
        loop {
            let Ok(batch) = self.store().to_summarize_by_rule(model, PER_ROUND) else { return false };
            if batch.is_empty() {
                return true;
            }
            for (c, rule) in &batch {
                if !self.write_for_rule(summarizer, model, c, rule).await {
                    return false;
                }
            }
        }
    }

    async fn write_for_rule(&self, summarizer: &dyn Summarizer, model: &str, c: &ToSummarize, rule: &SummaryRule) -> bool {
        let Some(content) = summarizer.summarize_as(c, &rule.request).await else { return false };
        let now = self.now();
        if self.store().keep_summary(c, &rule.request, model, &content, now).is_err() {
            return false;
        }
        self.events.emit(Event::SummaryWritten { thread_id: c.thread_id.to_string() });
        true
    }

    async fn summarize_automatic(&self, summarizer: &dyn Summarizer, model: &str) {
        loop {
            let batch = self.store().to_summarize(model, PER_ROUND).unwrap_or_default();
            if batch.is_empty() {
                return;
            }
            for c in &batch {
                let Some(text) = summarizer.summarize(c).await else { return };
                let now = self.now();
                if self.store().set_summary(c, &text, model, now).is_err() {
                    return;
                }
                self.events.emit(Event::MailChanged { account_id: c.account_id.clone() });
            }
        }
    }
}

impl Engine {
    /// Lets the writing model summarise conversations, from now on, in the background.
    pub fn set_summarizer(&self, summarizer: Arc<dyn Summarizer>) {
        let shared = self.shared.clone();
        crate::keep_running(move || {
            let (shared, summarizer) = (shared.clone(), summarizer.clone());
            async move {
                loop {
                    shared.summarize_waiting(summarizer.as_ref()).await;
                    shared.summary_wake.notified().await;
                }
            }
        });
    }

    /// Summarises again now, e.g. once the writing model has downloaded.
    pub fn wake_summarizer(&self) {
        self.shared.summary_wake.notify_one();
    }

    /// The summary kept for a conversation by `model`, and its rule: shown at once on opening.
    pub fn thread_summaries(&self, thread_id: &str, model: &str) -> Result<ThreadSummaries> {
        Ok(self.shared.store().thread_summaries(thread_id, model)?)
    }

    /// A summary you asked for: the one kept, if it's fresh and you didn't ask `again`;
    /// otherwise what the writing model should read.
    pub fn summary_for(&self, thread_id: &str, request: &SummaryRequest, model: &str, again: bool) -> Result<SummaryNeed> {
        let request = request.clone().checked().map_err(|e| moss_store::Error::Invalid(e.to_string()))?;
        let store = self.shared.store();
        match store.kept_summary(thread_id, &request, model)? {
            Some(kept) if !kept.stale && !again => Ok(SummaryNeed::Kept(kept)),
            _ => Ok(SummaryNeed::Write(store.to_summarize_now(thread_id)?)),
        }
    }

    /// A summary you asked for: the one kept when it's fresh (unless `again`), otherwise
    /// written now by `write` and kept.
    pub async fn summarize_now<E, F, Fut>(
        &self,
        thread_id: &str,
        request: &SummaryRequest,
        model: &str,
        again: bool,
        write: F,
    ) -> std::result::Result<SummaryView, SummaryFailure<E>>
    where
        F: FnOnce(ToSummarize) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<SummaryContent, E>>,
    {
        let checked = request.clone().checked().map_err(|e| SummaryFailure::Engine(moss_store::Error::Invalid(e.to_string()).into()))?;
        let request = &checked;
        let c = match self.summary_for(thread_id, request, model, again).map_err(SummaryFailure::Engine)? {
            SummaryNeed::Kept(kept) => return Ok(kept),
            SummaryNeed::Write(c) => c,
        };
        let content = write(c.clone()).await.map_err(SummaryFailure::Write)?;
        self.keep_summary(&c, request, model, &content).map_err(SummaryFailure::Engine)
    }

    /// Keeps what the writing model wrote for [`Engine::summary_for`].
    pub fn keep_summary(&self, c: &ToSummarize, request: &SummaryRequest, model: &str, content: &SummaryContent) -> Result<SummaryView> {
        let now = self.shared.now();
        Ok(self.shared.store().keep_summary(c, request, model, content, now)?)
    }

    /// Discards a conversation's summary, and its one-liner in the list. Nothing writes
    /// another until a new message comes, or you ask.
    pub fn discard_summary(&self, thread_id: &str) -> Result<()> {
        let account_id = self.shared.store().discard_summary(thread_id)?;
        self.shared.events.emit(Event::MailChanged { account_id });
        Ok(())
    }

    pub fn summary_rules(&self) -> Result<Vec<SummaryRule>> {
        Ok(self.shared.store().summary_rules()?)
    }

    /// Makes a rule (`id` none) or changes one. New mail from its sender is summarised from now on.
    pub fn save_summary_rule(&self, id: Option<i64>, sender: &str, request: &SummaryRequest) -> Result<SummaryRule> {
        let now = self.shared.now();
        let rule = self.shared.store().save_summary_rule(id, sender, request, now)?;
        self.wake_summarizer();
        Ok(rule)
    }

    /// Deletes a rule; false when there was none.
    pub fn remove_summary_rule(&self, id: i64) -> Result<bool> {
        Ok(self.shared.store().remove_summary_rule(id)?)
    }

    pub fn automatic_summaries(&self) -> Result<bool> {
        Ok(self.shared.store().automatic_summaries()?)
    }

    /// Turns the automatic one-liners on (written in the background again) or off (gone
    /// from the list).
    pub fn set_automatic_summaries(&self, on: bool) -> Result<()> {
        self.shared.store().set_automatic_summaries(on)?;
        for account in self.shared.store().accounts()? {
            self.shared.events.emit(Event::MailChanged { account_id: account.id });
        }
        self.wake_summarizer();
        Ok(())
    }
}
