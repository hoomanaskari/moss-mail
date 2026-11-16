import { protectQuit } from "./quitState";
/**
 * The Ask Moss panel: one right-hand panel, one chat. The email you are reading is its context;
 * Moss works out from each question whether it's about that email or any other, and can act on
 * mail as the window does. The chat lives until New chat or until Moss quits. See
 * `docs/research/ask-agent.md`.
 *
 * Moss never sends: an email it writes waits in its card until you press Send.
 */
import { create } from "zustand";
import type { Thread } from "../data/types";
import { replyFrom } from "../lib/addresses";
import { applyEvent, ask as askBackend, startTurn, type AskAnswer, type AskError, type ChatTurn } from "../services/ask";
import { errorText } from "../services/errorText";
import { keepDraft, type SavedDraft } from "./drafts";
import { scheduleDraft } from "./scheduled";
import { accountFor, findThread, signaturePick, useStore } from "./store";

export interface AskPanelState {
  open: boolean;
  /** The chat's id; New chat starts another. */
  chat: string;
  /** The chat's turns, by chat id. */
  chats: Record<string, ChatTurn[]>;
  input: string;
  /** What ↑ ↓ picked: an email card of the last answer. -1: none. */
  pick: number;
  /** Drafts whose cards you closed with Not now, or sent. */
  dismissed: string[];
  /** Emails on this Mac, once known. */
  mailCount?: number;
  /** A message a source chip pointed at, shown open in the thread. */
  reveal?: string;
  show(): void;
  hide(): void;
  /** ⌘J: opens it, or closes it when open. */
  toggle(): void;
  type(text: string): void;
  /** Asks `question`, or what's in the box. Nothing while the last answer is still coming. */
  ask(question?: string): Promise<void>;
  /** Esc: stops the answer being written. */
  stop(): void;
  newChat(): void;
  /** ⌘R: asks Moss to draft a reply from what was said. */
  draftReply(): Promise<void>;
  /** ↑ ↓ among `count` things to pick. */
  move(step: 1 | -1, count: number): void;
  dismiss(draftId: string): void;
  /** A source chip: shows that message in the open email and scrolls to it. */
  showMessage(messageId: string): void;
}

const blank = { open: false, chat: "moss-1", chats: {}, input: "", pick: -1, dismissed: [], mailCount: undefined, reveal: undefined };

/** The email the chat is about when a question is: the one open. */
export function emailThread(): Thread | undefined {
  const s = useStore.getState();
  return s.view === "thread" ? findThread(s, s.openThreadId) : undefined;
}

export const turnsOf = (s: Pick<AskPanelState, "chat" | "chats">): ChatTurn[] => s.chats[s.chat] ?? [];

const busy = (turn?: ChatTurn) => turn?.state === "thinking" || turn?.state === "writing";

/** Changes the last turn of `chat`. */
function updateLast(chat: string, change: (turn: ChatTurn) => ChatTurn) {
  useAskPanel.setState((s) => {
    const turns = s.chats[chat];
    if (!turns?.length) return {};
    return { chats: { ...s.chats, [chat]: [...turns.slice(0, -1), change(turns[turns.length - 1])] } };
  });
}

/**
 * A drafted reply as the composer would start it: from the address the email came to (when you
 * send from it), with that address's signature. Kept in Drafts.
 */
function readied(draft: SavedDraft): SavedDraft {
  const thread = findThread(useStore.getState(), draft.threadId);
  if (!thread) return draft;
  const { address } = replyFrom(thread, accountFor(thread.accountId));
  const signatureId = draft.signatureId ?? signaturePick({ ...draft, ...(address && { fromAddress: address }) }).id;
  const ready = { ...draft, ...(address && { fromAddress: address }), signatureId };
  keepDraft(ready);
  return ready;
}

const failure = (error: unknown): AskError =>
  error && typeof error === "object" && "kind" in error && typeof error.kind === "string"
    ? { kind: error.kind, message: errorText(error) }
    : { kind: "model", message: errorText(error) };

export const useAskPanel = create<AskPanelState>()((set, get) => ({
  ...blank,

  show: () => {
    set({ open: true, pick: -1 });
    askBackend()
      .mailCount()
      .then(
        (mailCount) => set({ mailCount }),
        () => {},
      );
  },
  hide: () => set({ open: false }),
  toggle: () => (get().open ? get().hide() : get().show()),
  type: (input) => set({ input, pick: -1 }),

  ask: (question) => protectQuit("An AI answer is still being applied to drafts and mail.", async () => {
    const q = (question ?? get().input).trim();
    const s = get();
    const { chat } = s;
    const turns = turnsOf(s);
    if (!q || busy(turns.at(-1))) return;
    set({ chats: { ...s.chats, [chat]: [...turns, startTurn(q)] }, input: "", pick: -1 });
    let answer: AskAnswer;
    try {
      answer = await askBackend().askMoss(chat, q, emailThread()?.id);
    } catch (error) {
      updateLast(chat, (t) => applyEvent(t, { type: "failed", chat, error: failure(error) }));
      return;
    }
    const ready = { ...answer, drafts: answer.drafts.map(readied) };
    updateLast(chat, (t) => applyEvent(t, { type: "done", chat, answer: ready }));
    // The first email card is picked, so ↵ opens it.
    if (ready.sources.length) set({ pick: 0 });
  }),

  stop: () => void askBackend().stop(get().chat),

  newChat: () => {
    const { chat } = get();
    void askBackend().forget(chat);
    set({ chats: {}, chat: `moss-${Number(chat.slice(5)) + 1}`, pick: -1 });
  },

  draftReply: () => get().ask("Draft a reply based on this chat."),

  move: (step, count) => set((s) => ({ pick: count ? Math.min(count - 1, Math.max(0, s.pick + step)) : -1 })),

  dismiss: (id) => set((s) => ({ dismissed: [...s.dismissed, id] })),

  showMessage: (id) => {
    set({ reveal: id });
    // Once the thread has drawn it.
    setTimeout(() => document.querySelector(`[data-message=${JSON.stringify(id)}]`)?.scrollIntoView({ block: "start", behavior: "smooth" }));
  },
}));

/** Back to how Moss starts: for tests. */
export function resetAskPanel() {
  useAskPanel.setState(blank);
}

/**
 * Streams each answer into its turn as it's written. The finished answer, or the failure, comes
 * from the call itself, so a late event never changes a turn that is over.
 */
export function watchAsk(): () => void {
  return askBackend().onEvent((event) => {
    if (event.type === "done" || event.type === "failed") return;
    updateLast(event.chat, (turn) => (busy(turn) ? applyEvent(turn, event) : turn));
  });
}

/** ⌘E: the draft in the full composer. */
export function editDraft(draft: SavedDraft) {
  useAskPanel.getState().hide();
  useStore.getState().openDraft(draft.id);
}

/**
 * ⌘↵: sends it, with Undo, or at `at` when you asked Moss to send it later; and closes its card.
 * Sending never marks the email Done.
 */
export function sendDraft(draft: SavedDraft, at?: number) {
  useAskPanel.getState().dismiss(draft.id);
  if (at === undefined) useStore.getState().sendDraft(draft);
  else scheduleDraft(draft, at);
}

/** Opens an email a card points at; one that isn't in the mailbox on screen is fetched first. */
export async function openEmail(id: string) {
  await useStore.getState().openFound(id);
}
