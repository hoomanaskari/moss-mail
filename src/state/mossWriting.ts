import { protectQuit } from "./quitState";
import { create } from "zustand";
import type { MossPick } from "../design-system/organisms";
import { textToHtml } from "../lib/textHtml";
import { replies } from "../services/replies";
import { useAiSettings } from "./aiSettings";
import { useSignatures } from "./signatures";
import { useStore } from "./store";

/** "Write with Moss" in the composer: the ⌘J menu, and Moss writing into the draft. */
export interface MossWritingState {
  /** The draft all this is about; another draft in the composer ignores it. */
  draftId?: string;
  menuOpen: boolean;
  /** Writing; done (the strip says what it did); the writing model isn't downloaded; or it failed. */
  status?: "writing" | "wrote" | "missing" | "failed";
  /** What was asked last, for the strip and Try again. */
  pick?: MossPick;
  /** The draft before Moss wrote, for Undo. */
  before?: string;
  /** The text so far while Moss writes. */
  streamed: string;
  /** Goes up each time Moss changes the draft, so the editor shows the new text. */
  version: number;
  /** The write running now; a newer one, Undo or leaving drops its result. */
  id?: string;
  openMenu(): void;
  closeMenu(): void;
  write(pick: MossPick): Promise<void>;
  stop(): void;
  undo(): void;
  retry(): Promise<void>;
  /** You typed in the text: the "Moss wrote this" strip goes. */
  dismiss(): void;
  download(): void;
  settings(): void;
  /** Forgets it all. */
  reset(): void;
}

let writes = 0;

const setBody = (body: string) => useStore.getState().updateDraft({ body, bodyHtml: textToHtml(body) });

export const useMossWriting = create<MossWritingState>()((set, get) => ({
  menuOpen: false,
  streamed: "",
  version: 0,
  openMenu: () => {
    const draftId = useStore.getState().draft?.id;
    // Another draft's strip doesn't follow you into this one.
    if (draftId !== get().draftId) get().reset();
    set({ draftId, menuOpen: true });
  },
  closeMenu: () => set({ menuOpen: false }),
  write: (pick) => protectQuit("An AI result is still being applied to a draft.", async () => {
    const draft = useStore.getState().draft;
    if (!draft) return;
    const id = `moss-${++writes}`;
    // Undo, another write, or another draft in the composer drops this one's result.
    const current = () => get().id === id && useStore.getState().draft?.id === draft.id;
    set({ id, draftId: draft.id, menuOpen: false, status: "writing", pick, before: draft.body, streamed: "" });
    try {
      const signature = useSignatures.getState().signatures.find((x) => x.id === draft.signatureId)?.lines ?? [];
      const request = { id, threadId: draft.threadId, accountId: draft.fromAccountId, job: pick.job, text: draft.body, signature };
      const text = await replies().write(request, (streamed) => current() && set({ streamed }));
      if (!current()) return;
      setBody(text);
      set((s) => ({ status: "wrote", version: s.version + 1 }));
    } catch (error) {
      if (current()) set({ status: (error as { kind?: string }).kind === "notReady" ? "missing" : "failed" });
    }
  }),
  stop: () => {
    const { id, status } = get();
    if (id && status === "writing") void replies().stop(id);
  },
  undo: () => {
    const { before, status, draftId } = get();
    if (before === undefined || (status !== "wrote" && status !== "failed") || draftId !== useStore.getState().draft?.id) return;
    setBody(before);
    set((s) => ({ id: undefined, status: undefined, before: undefined, version: s.version + 1 }));
  },
  retry: async () => {
    const pick = get().pick;
    if (pick) await get().write(pick);
  },
  dismiss: () => {
    if (get().status === "wrote") set({ status: undefined, before: undefined });
  },
  download: () => {
    void useAiSettings.getState().download();
    set({ status: undefined });
  },
  settings: () => useStore.getState().openSettings("ai"),
  reset: () => set({ id: undefined, draftId: undefined, menuOpen: false, status: undefined, pick: undefined, before: undefined, streamed: "" }),
}));
