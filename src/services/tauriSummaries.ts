/** Summaries in the desktop app: each call is one Tauri command into Rust (`src-tauri/src/summaries.rs`). */
import { invoke } from "@tauri-apps/api/core";
import type { SummaryBackend, SummaryContent, SummaryRule, SummaryView, ThreadSummaries } from "./summaries";
import { subscribe } from "./tauriEvents";
import { whileStreaming } from "./tauriText";

/** How many summaries were asked for: each streams under its own name. */
let streams = 0;

export const tauriSummaries: SummaryBackend = {
  kept: (id) => invoke<ThreadSummaries>("ai_summaries", { id }),
  discard: (id) => invoke<void>("discard_summary", { id }),
  // It streams as JSON: the summary so far.
  summarize: (id, { kind, instruction }, again = false, onContent) => {
    const stream = `summary-${++streams}`;
    const heard = (text: string) => onContent?.(JSON.parse(text) as SummaryContent);
    return whileStreaming(stream, heard, () => invoke<SummaryView>("ai_summarize", { id, kind, instruction, again, stream }));
  },
  rules: () => invoke<SummaryRule[]>("summary_rules"),
  saveRule: ({ id, sender, kind, instruction }) => invoke<SummaryRule>("save_summary_rule", { id, sender, kind, instruction }),
  removeRule: (id) => invoke<boolean>("remove_summary_rule", { id }),
  automatic: () => invoke<boolean>("automatic_summaries"),
  setAutomatic: (on) => invoke<void>("set_automatic_summaries", { on }),
  onWritten: (handler) =>
    subscribe<{ type: string; threadId?: string }>("moss://event", (e) => {
      if (e.type === "summaryWritten" && e.threadId) handler(e.threadId);
    }),
};
