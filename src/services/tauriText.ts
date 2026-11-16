import { protectQuit } from "./quit";
/** Text the writing model streams from Rust (`moss://ai-text`, `{id, text}`), for any job that streams. */
import { subscribe } from "./tauriEvents";

/** Calls `handler` with stream `id`'s whole text so far each time it grows; returns a function that stops it. */
export function onAiText(id: string, handler: (text: string) => void): () => void {
  return subscribe<{ id: string; text: string }>("moss://ai-text", (e) => {
    if (e.id === id) handler(e.text);
  });
}

/** Runs `call` while hearing stream `id` (see {@link onAiText}), and stops listening when it ends, however it ends. */
export async function whileStreaming<T>(id: string, onText: (text: string) => void, call: () => Promise<T>): Promise<T> {
  const stop = onAiText(id, onText);
  return protectQuit("An AI writing request is still running.", async () => {
    try { return await call(); }
    finally { stop(); }
  });
}
