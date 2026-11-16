/** The AI in the desktop app: each call is one Tauri command into Rust (`moss_ai`). */
import { invoke } from "@tauri-apps/api/core";
import type { AiBackend, AiStatus } from "./ai";
import { subscribe } from "./tauriEvents";

export const tauriAi: AiBackend = {
  status: () => invoke<AiStatus>("ai_status"),
  choose: (id) => invoke<AiStatus>("ai_choose", { id }),
  download: () => invoke<AiStatus>("ai_download"),
  plan: (plan) => invoke<AiStatus>("ai_plan", { plan }),
  cancel: () => invoke<void>("ai_cancel"),
  remove: () => invoke<AiStatus>("ai_remove"),
  removeModel: (id) => invoke<AiStatus>("ai_remove_model", { id }),
  downloadModels: (ids) => invoke<AiStatus>("ai_download_models", { ids }),
  stopModel: (id) => invoke<AiStatus>("ai_stop_model", { id }),
  ask: (id) => invoke<string | null>("ai_ask", { id }),
  onStatus: (handler) => subscribe<AiStatus>("moss://ai", handler),
};
