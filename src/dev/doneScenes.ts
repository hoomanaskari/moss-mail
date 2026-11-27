/** Dev scenes for the Done list, Trash and Undo. */
import { useStore } from "../state/store";
import { openFromTrash, openTrash, useTrash } from "../state/trash";
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
  "trash-list": { shows: /E puts one back in your inbox/, apply: () => openTrash() },
  "trash-empty": {
    shows: /Trash is empty/,
    apply: () => {
      useTrash.setState({ threads: [] });
      openTrash();
    },
  },
  "trash-email": {
    shows: /Back to inbox/,
    apply: () => {
      openTrash();
      openFromTrash("offsite-poll");
    },
  },
  "undo-toast": {
    shows: /Moved to Done/,
    // Dev mode applies a scene twice; a second E would bring the email back.
    apply: () => useStore.getState().threads.some((t) => t.id === "q4-budget") && useStore.getState().markDone("q4-budget"),
  },
};
