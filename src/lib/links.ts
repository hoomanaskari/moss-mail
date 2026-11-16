/**
 * Opens a link outside Moss. Only web and mail links; anything else is ignored.
 * In the desktop app it goes to the default browser (Tauri's opener plugin); in a browser
 * tab it opens a new tab that can't reach back into Moss.
 */
import { isTauri } from "./platform";

export function openLink(url: string) {
  if (!/^(https?:|mailto:)/i.test(url)) return;
  if (isTauri) {
    void import("@tauri-apps/plugin-opener").then((opener) => opener.openUrl(url));
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}
