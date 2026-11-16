import { useEffect, useRef } from "react";
import { useQuit } from "./quitState";
import { isTauri } from "../lib/platform";

/**
 * Files dropped on the desktop app's window from Finder. The window takes the drop, so the
 * page never sees the files, only their paths here. `over` says while files are over it.
 */
export function useFileDrop(active: boolean, drop: (paths: string[]) => void, over: (over: boolean) => void) {
  // The latest callbacks, without listening again each render.
  const handlers = useRef({ drop, over });
  handlers.current = { drop, over };
  useEffect(() => {
    if (!active || !isTauri) return;
    let stopped = false;
    let stop = () => {};
    void import("@tauri-apps/api/webview").then(async ({ getCurrentWebview }) => {
      const unlisten = await getCurrentWebview().onDragDropEvent(({ payload }) => {
        if (useQuit.getState().token) { handlers.current.over(false);return; }
        if (payload.type === "drop") {
          handlers.current.over(false);
          if (payload.paths.length) handlers.current.drop(payload.paths);
        } else handlers.current.over(payload.type !== "leave");
      });
      if (stopped) unlisten();
      else stop = unlisten;
    });
    return () => {
      stopped = true;
      stop();
    };
  }, [active]);
}
