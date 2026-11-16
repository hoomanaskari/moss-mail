/** Dev scenes for blocking senders: the banner, ⌘K, the Blocked list, the Senders list and a blocked email. */
import { newSenderThread } from "../data/demo";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

/** Jordan's first email takes Priya's place in the inbox, as in the design. Safe to run twice. */
function openNewSender() {
  useStore.setState((s) => ({
    threads: s.threads.map((t) => (t.id === "fridays-shoot" ? newSenderThread : t)),
    blockedThreads: s.blockedThreads.filter((t) => t.id !== newSenderThread.id),
  }));
  useStore.getState().open(newSenderThread.id);
}

export const blockScenes: Record<string, Scene> = {
  "new-sender": { shows: /Block this sender\?/, apply: openNewSender },
  "palette-block": {
    shows: /Block Jordan Blake/,
    apply: () => {
      openNewSender();
      useStore.setState({ paletteOpen: true, paletteQuery: "block" });
    },
  },
  "blocked-list": { shows: /each account’s Blocked folder/, apply: () => useStore.getState().openBlocked() },
  "blocked-empty": {
    shows: /No blocked mail/,
    apply: () => {
      useStore.setState({ blockedThreads: [] });
      useStore.getState().openBlocked();
    },
  },
  senders: { shows: /Who Moss keeps out/, apply: () => useStore.getState().openSenders() },
  "blocked-email": {
    shows: /You blocked this sender/,
    apply: () => {
      useStore.getState().openBlocked();
      useStore.getState().open(newSenderThread.id);
    },
  },
};
