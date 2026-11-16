import { useMossWriting } from "./mossWriting";
import { quickSignature, useQuickReply } from "./quickReply";
import { openThread, useStore } from "./store";

function isTyping(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
}

/**
 * Moss's keys: Q (Quick reply), ⌘J (Write with Moss), and Esc / ⌘Z while Moss is writing or
 * has written. Returns whether it took the key; the app's own shortcuts get the rest.
 */
export function mossKeys(e: KeyboardEvent, s: ReturnType<typeof useStore.getState>): boolean {
  const mod = e.metaKey || e.ctrlKey;
  const key = e.key.toLowerCase();
  const moss = useMossWriting.getState();
  const quick = useQuickReply.getState();
  const quickOpen = s.view === "thread" && quick.threadId === s.openThreadId;
  const take = (run: () => unknown) => {
    e.preventDefault();
    void run();
    return true;
  };

  if (s.view === "compose") {
    const here = moss.draftId === s.draft?.id;
    if (mod && key === "j") return take(moss.openMenu);
    if (!here) return false;
    if (e.key === "Escape" && moss.menuOpen) return take(moss.closeMenu);
    if (e.key === "Escape" && moss.status === "writing") return take(moss.stop);
    if (mod && key === "z" && !e.shiftKey && (moss.status === "wrote" || moss.status === "failed")) return take(moss.undo);
    return false;
  }
  if (s.view !== "thread") return false;
  // Still writing the replies: nothing to send, edit or sign yet (⌘J is left for Ask Moss).
  const writing = quickOpen && quick.status === "writing";
  if (writing && ((mod && (key === "e" || e.key === "Enter")) || (e.altKey && e.code === "KeyS"))) return take(() => {});
  // Only on a Quick reply you opened (Q): otherwise ⌘J in the reading view is Ask Moss's.
  const thread = openThread(s);
  if (mod && key === "j" && quickOpen && !writing && thread?.replyOptions?.length)
    return take(() => {
      s.editInstantReply(quickSignature(thread).chosen);
      moss.openMenu();
    });
  if (e.key === "Escape" && quickOpen) return take(quick.status ? quick.close : quick.discard);
  // ⌥ changes e.key on macOS, so use the physical key.
  if (e.altKey && e.code === "KeyS" && thread?.replyOptions?.length) return take(quick.toggleSignature);
  if (key === "q" && !mod && !e.altKey && !isTyping(e.target)) return take(quick.start);
  return false;
}
