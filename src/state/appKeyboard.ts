import { useQuit } from "./quitState";

/** Capture listeners must also yield to the native modal's keyboard handling. */
export function listenAppKeys(listener: (event: KeyboardEvent) => void, capture = false): () => void {
  const onKey = (event: KeyboardEvent) => {
    if (!useQuit.getState().token) listener(event);
  };
  window.addEventListener("keydown", onKey, capture);
  return () => window.removeEventListener("keydown", onKey, capture);
}
