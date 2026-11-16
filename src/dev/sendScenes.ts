/** Dev scenes for Send: the pressed button, the countdown while Undo can take it back, sending, and a send that failed. */
import { textToHtml } from "../lib/textHtml";
import { undoSendMs, useStore, type Toast } from "../state/store";
import type { Scene } from "./scenes";

/** The inbox with a note about a sent email. */
const note = (shows: RegExp, toast: () => Omit<Toast, "id">): Scene => ({
  shows,
  apply: () => useStore.setState({ view: "inbox", toast: { id: 1, ...toast() } }),
});

export const sendScenes: Record<string, Scene> = {
  "composer-sending": {
    shows: /Sending…/,
    apply: () => {
      useStore.getState().compose("reply-all", "q4-budget");
      useStore.setState({ sending: true });
    },
  },
  // Counts down in the browser as it would after Send.
  "undo-send": note(/Sending in \ds/, () => ({ text: "Sending", until: Date.now() + undoSendMs, hold: "scene" })),
  "send-going": note(/Sending…/, () => ({ text: "Sending", until: Date.now() - 1, hold: "scene" })),
  // Your reply to Daniel at the end of his email while Undo can still take it back.
  "thread-reply-sending": {
    shows: /^Sending…$/,
    apply: () => {
      const store = useStore.getState();
      store.open("q4-budget");
      store.compose("reply", "q4-budget");
      const body = "Thanks Daniel, approved.\n\nCould you share the testing plan by Thursday?";
      // As the composer leaves it: the text, its HTML, and the reply signature added on sending.
      const draft = { ...useStore.getState().draft!, body, bodyHtml: textToHtml(body) };
      const until = Date.now() + undoSendMs;
      useStore.setState({ view: "thread", draft: undefined, undoSteps: [{ kind: "send", hold: "scene", draft, until }], toast: { id: 1, text: "Sending", until, hold: "scene" } });
    },
  },
  "send-failed": note(/Couldn’t send your email/, () => ({ text: "Couldn’t send your email", problem: true, undoLabel: "Open", stays: true })),
};
