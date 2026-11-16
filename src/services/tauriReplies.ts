/** AI replies in the desktop app: each call is one Tauri command into Rust (`moss_ai::reply`). */
import { invoke } from "@tauri-apps/api/core";
import type { ReplyOption } from "../data/types";
import type { ReplyBackend } from "./replies";
import { whileStreaming } from "./tauriText";

export const tauriReplies: ReplyBackend = {
  // Quick replies stream as JSON: the replies so far.
  options: (request, onOptions) =>
    whileStreaming(request.id, (text) => onOptions(JSON.parse(text) as ReplyOption[]), () => invoke<ReplyOption[]>("ai_reply_options", { ...request })),
  write: (request, onText) => whileStreaming(request.id, onText, () => invoke<string>("ai_reply", { ...request })),
  stop: (id) => invoke<boolean>("ai_stop", { id }),
};
