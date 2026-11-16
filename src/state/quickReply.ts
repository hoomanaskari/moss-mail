import { create } from "zustand";
import type { ReplyOption, Thread } from "../data/types";
import { replyFrom } from "../lib/addresses";
import { replies } from "../services/replies";
import { useSignatures } from "./signatures";
import { accountFor, openThread, signaturePick, useStore } from "./store";

/** The signature Moss picks for a reply to `thread`, the same one the composer would. */
export function replyPick(thread: Thread) {
  const { address } = replyFrom(thread, accountFor(thread.accountId));
  return signaturePick({ fromAccountId: thread.accountId, fromAddress: address, mode: "reply", threadId: thread.id });
}

/** The signature a reply to `thread` gets, the one the composer adds when it's sent; none when it gets none. */
export function replySignature(thread: Thread) {
  const { id } = replyPick(thread);
  return useSignatures.getState().signatures.find((x) => x.id === id);
}

/** Quick reply (Q) in the reading view: Moss writing replies to the open email. */
export interface QuickReplyState {
  /** The email Quick reply is open on. */
  threadId?: string;
  /** Still writing, the writing model isn't downloaded, or it failed. None once the replies are in. */
  status?: "writing" | "missing" | "failed";
  /** The stream being written now, so Stop can stop it and older ones' replies are dropped. */
  asking?: string;
  /** The signature you picked for one email's Quick reply (null: none); others get their usual one. */
  signature?: { threadId: string; id: string | null };
  /** The signature picker is open on the Quick reply (⌥S). */
  signatureOpen: boolean;
  toggleSignature(): void;
  /** Picks the open email's Quick reply signature, or none, and closes the picker. */
  chooseSignature(id: string | null): void;
  /** Asks Moss for replies to the open email (or shows the ones it has). Also Try again. */
  start(): Promise<void>;
  close(): void;
  /** Takes the open email's replies away, with your changes to them; Q writes new ones. */
  discard(): void;
}

/**
 * The signature a Quick reply to `thread` goes with: the one you picked on it (`chosen`, null for
 * none), or else Moss's `pick` for a reply (`chosen` undefined).
 */
export function quickSignature(thread: Thread) {
  const picked = useQuickReply.getState().signature;
  if (picked?.threadId !== thread.id) return { signature: replySignature(thread), chosen: undefined, pick: replyPick(thread) };
  return { signature: useSignatures.getState().signatures.find((x) => x.id === picked.id), chosen: picked.id, pick: undefined };
}

/** Puts `options` on email `threadId`, or takes its replies away. */
function putReplies(threadId: string, options: ReplyOption[] | undefined) {
  const put = <T extends { id: string; replyOptions?: ReplyOption[] }>(list: T[]) =>
    list.map((t) => (t.id === threadId ? { ...t, replyOptions: options } : t));
  useStore.setState((s) => ({ threads: put(s.threads), doneThreads: put(s.doneThreads) }));
}

/** How many times replies were asked for: each streams under its own name. */
let asks = 0;

export const useQuickReply = create<QuickReplyState>()((set, get) => ({
  signatureOpen: false,
  toggleSignature: () => set({ signatureOpen: !get().signatureOpen }),
  chooseSignature: (id) => {
    const threadId = useStore.getState().openThreadId;
    if (!threadId) return;
    if (id) useSignatures.getState().markUsed(id);
    set({ signature: { threadId, id }, signatureOpen: false });
  },
  start: async () => {
    const thread = openThread(useStore.getState());
    if (!thread) return;
    // Already writing them: they're on their way.
    if (get().asking && get().threadId === thread.id) return;
    if (thread.replyOptions?.length) return set({ threadId: thread.id, status: undefined });
    const id = `quick-${++asks}`;
    set({ threadId: thread.id, status: "writing", asking: id });
    useStore.setState({ replyChoice: 0, replyDraft: undefined });
    const current = () => get().asking === id;
    const request = { id, threadId: thread.id, accountId: thread.accountId, signature: quickSignature(thread).signature?.lines ?? [] };
    try {
      // Each reply shows as it's written.
      const options = await replies().options(request, (so_far) => current() && putReplies(thread.id, so_far));
      if (!current()) return;
      putReplies(thread.id, options);
      set({ status: undefined, asking: undefined });
    } catch (error) {
      if (!current()) return;
      putReplies(thread.id, undefined);
      set({ status: (error as { kind?: string }).kind === "notReady" ? "missing" : "failed", asking: undefined });
    }
  },
  close: () => {
    const { asking, threadId } = get();
    set({ threadId: undefined, status: undefined, asking: undefined });
    // Stopped while writing: what it wrote so far goes too.
    if (!asking || !threadId) return;
    void replies().stop(asking);
    putReplies(threadId, undefined);
  },
  discard: () => {
    get().close();
    set({ signature: undefined, signatureOpen: false });
    const id = useStore.getState().openThreadId;
    if (!id) return;
    putReplies(id, undefined);
    useStore.setState({ replyChoice: 0, replyDraft: undefined });
  },
}));
