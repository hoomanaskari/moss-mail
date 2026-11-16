import { listen } from "@tauri-apps/api/event";

/** Calls `handler` with each `name` event from Rust; returns a function that stops it, even before listening began. */
export function subscribe<T>(name: string, handler: (payload: T) => void): () => void {
  let stop: (() => void) | undefined;
  let stopped = false;
  void listen<T>(name, (e) => handler(e.payload)).then((unlisten) => {
    if (stopped) unlisten();
    else stop = unlisten;
  });
  return () => {
    stopped = true;
    stop?.();
  };
}
