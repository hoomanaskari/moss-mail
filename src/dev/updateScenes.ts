import { mossDev, useAppIdentity } from "../services/appIdentity";
import { demoUpdates, setUpdates } from "../services/updates";
import { useStore } from "../state/store";
import { useUpdates, type UpdateState } from "../state/updates";
import type { Scene } from "./scenes";

const notes = "**The update menu.** Choose Moss, Check for Updates… to look for a new version.\n\n- A new version shows at the top of every screen until you install it.";

/** The inbox with the update at `update`, and the update window open on it if `open`. */
const at = (update: UpdateState, open = false) => () => {
  setUpdates(demoUpdates({ next: "0.1.1", notes }));
  useUpdates.setState({ version: "0.1.0", update, open, checkedAt: Date.now() - 3 * 60_000 });
};

export const updateScenes: Record<string, Scene> = {
  // The top bar, on every screen, until you act on it.
  "update-available": { shows: /Update to Moss 0\.1\.1/, apply: at({ state: "available", version: "0.1.1", notes }) },
  "update-downloading": { shows: /Updating to Moss 0\.1\.1 · 40%/, apply: at({ state: "downloading", version: "0.1.1", done: 4, total: 10 }) },
  "inbox-update-ready": { shows: /Moss 0\.1\.1 is ready/, apply: at({ state: "ready", version: "0.1.1" }) },
  "update-download-failed": { shows: /Couldn’t download Moss 0\.1\.1/, apply: at({ state: "failed", during: "download", message: "The network connection was lost.", version: "0.1.1", notes }) },
  "update-available-in-a-thread": {
    shows: /Update to Moss 0\.1\.1/,
    apply: () => {
      at({ state: "available", version: "0.1.1", notes })();
      useStore.getState().open("q4-budget");
    },
  },
  // The update window, after Check for Updates… in the app menu or the palette.
  "update-window-checking": { shows: /Checking for updates…/, apply: at({ state: "checking" }, true) },
  "update-window-current": { shows: /You have Moss 0\.1\.0, the newest version\./, apply: at({ state: "current" }, true) },
  "update-window-available": { shows: /What’s new/, apply: at({ state: "available", version: "0.1.1", notes }, true) },
  "update-window-downloading": { shows: /Downloaded 40%\. Moss restarts when it’s done\./, apply: at({ state: "downloading", version: "0.1.1", done: 4, total: 10 }, true) },
  "update-window-ready": { shows: /Your drafts are saved first\./, apply: at({ state: "ready", version: "0.1.1" }, true) },
  "update-window-failed": { shows: /Couldn’t check for updates/, apply: at({ state: "failed", during: "check", message: "The network connection was lost." }, true) },
  // Settings, where the version is.
  "settings-updates-current": {
    shows: /Last checked 3 min ago/,
    apply: () => {
      at({ state: "current" })();
      useStore.getState().openSettings("appearance");
    },
  },
  "settings-updates-available": {
    shows: /Moss 0\.1\.1 is available\./,
    apply: () => {
      at({ state: "available", version: "0.1.1", notes })();
      useStore.getState().openSettings("appearance");
    },
  },
  // Moss Dev: its mark in the top bar, and no updates.
  "dev-app": { shows: /^Dev$/, apply: () => useAppIdentity.setState(mossDev) },
  "dev-app-settings": {
    shows: /Updates are off in Moss Dev\./,
    apply: () => {
      useAppIdentity.setState(mossDev);
      useStore.getState().openSettings("appearance");
    },
  },
};
