import { useUpdates } from "../state/updates";
import { subscribe } from "./tauriEvents";

/** What the app menu's "Check for Updates…" sends (src-tauri/src/menu.rs). */
export const checkForUpdates = "check-for-updates";

/** Acts on items chosen in the macOS menu bar: Check for Updates… checks and opens the update window. Returns a stop. */
export function watchMenu(): () => void {
  return subscribe<string>("moss://menu", (item) => {
    if (item === checkForUpdates) void useUpdates.getState().check({ asked: true, open: true });
  });
}
