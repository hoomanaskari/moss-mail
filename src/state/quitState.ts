import { create } from "zustand";
import type { DraftQuitBlocker } from "./drafts";

/** UI state is independent of native transport so menus can yield before it loads. */
export const useQuit = create<{ token?: string; checking: boolean; exiting?: boolean; blockers: DraftQuitBlocker[]; error?: string }>(() => ({ checking: false, blockers: [] }));

const operations = new Map<symbol, string>();

/** Track a user-initiated handoff from before its first await until its native reply. */
export async function protectQuit<T>(reason: string, work: (accepted: () => void) => Promise<T>): Promise<T> {
  const id = Symbol();operations.set(id, reason);
  const accepted = () => { operations.delete(id); };
  try { return await work(accepted); }
  finally { accepted(); }
}

export const quitOperationReasons = (): string[] => [...operations.values()];
