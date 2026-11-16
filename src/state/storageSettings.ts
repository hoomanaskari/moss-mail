import { create } from "zustand";
import { mail, type Offline, type StorageInfo } from "../services/mail";
import { errorText } from "../services/errorText";

interface StorageSettingsState {
  /** Space used and what's kept, once loaded. */
  info?: StorageInfo;
  /** Why the last action didn't work, as a sentence. */
  problem?: string;
  load(): Promise<void>;
  setOffline(offline: Offline): Promise<void>;
  setCacheLimit(bytes: number): Promise<void>;
  setAttachmentText(on: boolean): Promise<void>;
  clearCache(): Promise<void>;
  rebuildIndex(): Promise<void>;
}

/** Settings → Storage, from the mail engine. */
export const useStorageSettings = create<StorageSettingsState>()((set, get) => {
  /** Runs an action, then shows the new figures, or why it failed and the figures `before` it. */
  const run = async (action: () => Promise<unknown>, before = get().info) => {
    try {
      await action();
      set({ problem: undefined, info: await mail().storage() });
    } catch (error) {
      set({ problem: errorText(error), info: before });
    }
  };
  /** Shows a new setting at once, then saves it. */
  const change = (patch: Partial<StorageInfo>, action: () => Promise<unknown>) => {
    const before = get().info;
    if (before) set({ info: { ...before, ...patch } });
    return run(action, before);
  };
  return {
    load: () => run(async () => {}),
    setOffline: (offline) => change({ offline }, () => mail().setOffline(offline)),
    setCacheLimit: (cacheLimit) => change({ cacheLimit }, () => mail().setCacheLimit(cacheLimit)),
    setAttachmentText: (attachmentText) => change({ attachmentText }, () => mail().setAttachmentText(attachmentText)),
    clearCache: () => run(() => mail().clearCache()),
    rebuildIndex: () => run(() => mail().rebuildIndex()),
  };
});
