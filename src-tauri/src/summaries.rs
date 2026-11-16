//! Summary commands: a conversation summarised in the kind you pick, the one kept for it
//! (or discarded), "always summarise mail from…" rules, and the setting for the automatic one-liners.
//! The work is in `moss_engine::summarize` and `moss_ai::summaries`; errors reach the window
//! as `{kind, message}` (`notReady` when the writing model isn't downloaded, `model` when it
//! failed, `invalid` for a request it can't use).

use moss_ai::Ai;
use moss_core::{SummaryContent, SummaryKind, SummaryRequest, SummaryRule, SummaryView, ThreadSummaries};
use moss_engine::{Engine, Error};
use moss_store::ToSummarize;
use serde_json::Value;
use tauri::{AppHandle, Runtime, State};

use crate::json;
use crate::writing::Writing;

/// A conversation as the writing model reads it for a summary, today.
pub fn text(c: &ToSummarize) -> String {
    moss_ai::summaries::input(&c.you, crate::now_ms(), &c.subject, &c.messages)
}

fn request(kind: SummaryKind, instruction: Option<String>) -> Result<SummaryRequest, Value> {
    SummaryRequest::new(kind, instruction.as_deref().unwrap_or_default()).map_err(|e| json(Error::Invalid(e.to_string())))
}

/// A conversation summarised as `kind` (with `instruction` for Custom): the one kept when
/// it's fresh, unless `again`; otherwise written now and kept. While it's written it streams
/// to the window as stream `stream`: each text is the summary so far, as `SummaryContent` JSON.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_summarize<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, Engine>,
    ai: State<'_, Ai>,
    writing: State<'_, Writing>,
    id: String,
    kind: SummaryKind,
    instruction: Option<String>,
    again: Option<bool>,
    stream: String,
) -> Result<SummaryView, Value> {
    let request = request(kind, instruction)?;
    let model = moss_ai::summaries::model_key(&ai.status().writer);
    let writer = ai.inner().clone();
    let asked = request.clone();
    let live = writing.begin(&app, &stream);
    let live = &live;
    let write = move |c: ToSummarize| async move {
        let mut on_content = |content: &SummaryContent| live.send(&serde_json::to_string(content).unwrap_or_default());
        writer.summarize_streaming(&text(&c), &asked, &mut on_content).await
    };
    engine.summarize_now(&id, &request, &model, again.unwrap_or(false), write).await.map_err(json)
}

/// The summary kept for a conversation (by whichever model), and its rule: no model needed,
/// so it shows at once.
#[tauri::command]
pub async fn ai_summaries(engine: State<'_, Engine>, ai: State<'_, Ai>, id: String) -> Result<ThreadSummaries, Value> {
    engine.thread_summaries(&id, &moss_ai::summaries::model_key(&ai.status().writer)).map_err(json)
}

/// Discards a conversation's summary, and its one-liner.
#[tauri::command]
pub async fn discard_summary(engine: State<'_, Engine>, id: String) -> Result<(), Value> {
    engine.discard_summary(&id).map_err(json)
}

#[tauri::command]
pub async fn summary_rules(engine: State<'_, Engine>) -> Result<Vec<SummaryRule>, Value> {
    engine.summary_rules().map_err(json)
}

/// Makes a rule (no `id`) or changes one. `sender` is an address or a domain.
#[tauri::command]
pub async fn save_summary_rule(
    engine: State<'_, Engine>,
    id: Option<i64>,
    sender: String,
    kind: SummaryKind,
    instruction: Option<String>,
) -> Result<SummaryRule, Value> {
    let request = request(kind, instruction)?;
    engine.save_summary_rule(id, &sender, &request).map_err(json)
}

/// Deletes a rule; false when there was none.
#[tauri::command]
pub async fn remove_summary_rule(engine: State<'_, Engine>, id: i64) -> Result<bool, Value> {
    engine.remove_summary_rule(id).map_err(json)
}

/// Whether Moss writes the one-liners of conversations that matter by itself.
#[tauri::command]
pub async fn automatic_summaries(engine: State<'_, Engine>) -> Result<bool, Value> {
    engine.automatic_summaries().map_err(json)
}

#[tauri::command]
pub async fn set_automatic_summaries(engine: State<'_, Engine>, on: bool) -> Result<(), Value> {
    engine.set_automatic_summaries(on).map_err(json)
}

/// The writing model's summary in the kind a rule asks for, for the background job.
pub async fn write(ai: &Ai, c: &ToSummarize, request: &SummaryRequest) -> Option<SummaryContent> {
    ai.summarize_as(&text(c), request).await.ok()
}
