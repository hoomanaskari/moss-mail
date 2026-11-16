import { protectQuit } from "./quit";
/** Ask Moss in the desktop app: each call is one Tauri command into Rust (`src-tauri/src/ask.rs`). */
import { invoke } from "@tauri-apps/api/core";
import type { AskAnswer, AskBackend, AskEvent, AskFile } from "./ask";
import { toThread, type RustSummary } from "./convert";
import { reopened, type RustDraft } from "./tauriMail";
import { subscribe } from "./tauriEvents";

/** An answer as Rust sends it: the cited emails as list rows, the drafts as the engine keeps them. */
interface RustAnswer {
  text: string;
  sources: RustSummary[];
  messages: string[];
  files: AskFile[];
  drafts: RustDraft[];
  sendAt: Record<string, number>;
  did: string[];
  stopped: boolean;
}

type RustAskEvent = Exclude<AskEvent, { type: "done" }> | { type: "done"; chat: string; answer: RustAnswer };

function toAnswer(a: RustAnswer): AskAnswer {
  const now = Date.now();
  const { text, messages, files, sendAt, did, stopped } = a;
  return { text, sources: a.sources.map((t) => toThread(t, now)), messages, files, drafts: a.drafts.map(reopened), sendAt, did, stopped };
}

export const tauriAsk: AskBackend = {
  askMoss: (chat, question, open) => protectQuit("An AI request can still change drafts or mail.", async () => toAnswer(await invoke<RustAnswer>("ask_moss", { chat, question, open: open ?? null }))),
  stop: (chat) => invoke<boolean>("ask_stop", { chat }),
  forget: (chat) => invoke<void>("ask_forget", { chat }),
  mailCount: () => invoke<number>("ask_mail_count"),
  onEvent: (handler) =>
    subscribe<RustAskEvent>("moss://ask", (e) => handler(e.type === "done" ? { ...e, answer: toAnswer(e.answer) } : e)),
};
