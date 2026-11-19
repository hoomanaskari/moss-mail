/**
 * Opens a link. Only web and mail links; anything else is ignored.
 * A mail link opens a new email in Moss. A web link, in the desktop app, goes to the default
 * browser (Tauri's opener plugin); in a browser tab it opens a new tab that can't reach back into Moss.
 */
import { isTauri } from "./platform";

export function openLink(url: string) {
  if (/^mailto:/i.test(url)) {
    // Loaded when needed: the store itself uses this file.
    void import("../state/store").then(({ useStore }) => useStore.getState().composeFromLink(url));
    return;
  }
  if (!/^https?:/i.test(url)) return;
  if (isTauri) {
    void import("@tauri-apps/plugin-opener").then((opener) => opener.openUrl(url));
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}
