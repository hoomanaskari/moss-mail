/** Dev scenes for moving an email to another tab yourself (M), and waiting for an answer. */
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

/** IT's laptop update, filed in Other, that matters to you this week. */
const laptop = "drive-storage";

/** Moves the laptop email to Important, once: dev mode applies a scene twice. */
function moveLaptop() {
  const s = useStore.getState();
  if (s.threads.find((t) => t.id === laptop)?.split !== "important") s.moveTo("important", laptop);
}

export const moveScenes: Record<string, Scene> = {
  // Other, with one email reminded for next week: apart at the bottom, out of the tab's count.
  "other-reminded": {
    shows: /Coming back later/,
    apply: () => {
      const s = useStore.getState();
      if (s.threads.find((t) => t.id === "site-visits")?.section !== "later") s.remindLater("site-visits", Date.now() + 5 * 86_400_000);
      useStore.getState().setSplit("other");
    },
  },
  "move-menu": {
    shows: /Move to/,
    apply: () => {
      const s = useStore.getState();
      s.setSplit("other");
      s.setCursor(laptop);
      s.openMove(laptop);
    },
  },
  "moved-important": {
    shows: /Moved to Important/,
    apply: () => {
      moveLaptop();
      useStore.getState().setSplit("important");
      useStore.getState().setCursor(laptop);
    },
  },
  "moved-email": {
    shows: /You moved this to Important/,
    apply: () => {
      moveLaptop();
      useStore.getState().open(laptop);
    },
  },
  "wait-for-reply": {
    shows: /Remind me if no reply/,
    apply: () => {
      // Your proposal to Jonas, still unanswered.
      useStore.setState((s) => ({ threads: s.threads.map((t) => (t.id === "contract-renewal" ? { ...t, youWroteLast: true } : t)) }));
      useStore.getState().openRemind("contract-renewal", true);
    },
  },
};
