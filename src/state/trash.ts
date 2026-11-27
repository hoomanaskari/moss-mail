/**
 * Trash: mail you deleted here, newest first, until you put it back or your provider empties
 * its Trash folder. Reached from ⌘K. E puts an email back in the inbox, wherever it was
 * deleted from, here and on the server.
 */
import { create } from "zustand";
import { trashThreads as demoTrash } from "../data/demo";
import type { Thread } from "../data/types";
import { isTauri } from "../lib/platform";
import { mail, quietly } from "../services/mail";
import { goTo, reloadThreads, toast, useStore } from "./store";

interface TrashState {
  threads: Thread[];
  cursorId?: string;
}

/** The desktop app reads the real list each time Trash opens; the browser shows the demo's. */
export const useTrash = create<TrashState>()(() => ({ threads: isTauri ? [] : demoTrash }));

/** Trash for the account you are looking at. */
export function visibleTrash(threads: Thread[], accountFilter: string) {
  return threads.filter((t) => accountFilter === "all" || t.accountId === accountFilter);
}

const visible = () => visibleTrash(useTrash.getState().threads, useStore.getState().accountFilter);

/** Whether this email is one in Trash. */
export const inTrash = (id: string | undefined) => useTrash.getState().threads.some((t) => t.id === id);

/** The email Trash acts on: the one you're reading, or the one under the cursor. */
export function trashCursor() {
  const s = useStore.getState();
  return s.view === "thread" ? s.openThreadId : useTrash.getState().cursorId;
}

/** From ⌘K: what you deleted. */
export function openTrash() {
  useStore.setState(goTo(useStore.getState(), "trash"));
  useTrash.setState({ cursorId: visible()[0]?.id });
  if (mail().live) quietly(loadTrash());
}

/** Real mail: reads Trash from the engine, keeping the cursor where it can. */
export async function loadTrash() {
  const threads = await mail().trashThreads();
  useTrash.setState((t) => {
    const list = visibleTrash(threads, useStore.getState().accountFilter);
    return { threads, cursorId: list.some((x) => x.id === t.cursorId) ? t.cursorId : list[0]?.id };
  });
}

export function moveTrashCursor(step: 1 | -1) {
  const list = visible();
  const i = Math.max(0, list.findIndex((t) => t.id === useTrash.getState().cursorId));
  useTrash.setState({ cursorId: list[Math.min(list.length - 1, Math.max(0, i + step))]?.id });
}

/** Opens a deleted email to read it; ↑/↓ then go through Trash. */
export function openFromTrash(id: string) {
  const thread = useTrash.getState().threads.find((t) => t.id === id);
  if (!thread) return;
  useStore.setState((s) => ({ foundThreads: s.foundThreads.some((t) => t.id === id) ? s.foundThreads : [...s.foundThreads, thread] }));
  useTrash.setState({ cursorId: id });
  useStore.getState().open(id);
}

/** Reading deleted mail, ↑/↓ open the next or previous email in Trash. */
export function moveTrashReading(step: 1 | -1) {
  const list = visible();
  const i = list.findIndex((t) => t.id === useStore.getState().openThreadId);
  const next = list[Math.min(list.length - 1, Math.max(0, i + step))];
  if (next) openFromTrash(next.id);
}

/** Back in the inbox, open again, wherever it was deleted from. Trash moves on to the next. */
export function restoreToInbox(id = trashCursor()) {
  const thread = useTrash.getState().threads.find((t) => t.id === id);
  if (!thread) return;
  const list = visible();
  const i = list.findIndex((t) => t.id === id);
  const after = list.filter((t) => t.id !== id);
  useTrash.setState((t) => ({ threads: t.threads.filter((x) => x.id !== id), cursorId: after[Math.min(i, after.length - 1)]?.id }));
  const { doneAt: _, ...open } = thread;
  useStore.setState((s) => ({
    threads: [open, ...s.threads.filter((t) => t.id !== id)],
    foundThreads: s.foundThreads.filter((t) => t.id !== id),
    toast: { ...toast("Back in your inbox"), undo: false },
  }));
  if (!mail().live) return;
  mail()
    .restoreToInbox(thread.id)
    .then(
      () => quietly(reloadThreads()),
      (error: unknown) => {
        console.warn("Moss:", error);
        useStore.setState({ toast: { ...toast("Couldn’t put that back, so it is still in Trash"), problem: true } });
        quietly(Promise.all([reloadThreads(), loadTrash()]));
      },
    );
}
