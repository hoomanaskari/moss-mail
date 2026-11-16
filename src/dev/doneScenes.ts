/** Dev scenes for the Done list and Undo. */
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

export const doneScenes: Record<string, Scene> = {
  "done-list": { shows: /archived, not deleted/, apply: () => useStore.getState().openDone() },
  "done-empty": {
    shows: /Nothing done yet/,
    apply: () => {
      useStore.setState({ doneThreads: [] });
      useStore.getState().openDone();
    },
  },
  "done-email": {
    shows: /Not done/,
    apply: () => {
      useStore.getState().openDone();
      useStore.getState().open("standup-notes");
    },
  },
  "undo-toast": {
    shows: /Moved to Done/,
    // Dev mode applies a scene twice; a second E would bring the email back.
    apply: () => useStore.getState().threads.some((t) => t.id === "q4-budget") && useStore.getState().markDone("q4-budget"),
  },
};
