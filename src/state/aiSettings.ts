import { create } from "zustand";
import { ai, type AiPlan, type AiStatus } from "../services/ai";
import { errorText } from "../services/errorText";

interface AiSettingsState {
  /** The Mac, the models and the download, once loaded. */
  status?: AiStatus;
  /** Why the last action didn't work, as a sentence. */
  problem?: string;
  load(): Promise<void>;
  /** The status only: for the top bar and hints. */
  loadStatus(): Promise<void>;
  chooseModel(id: string): Promise<void>;
  download(): Promise<void>;
  /** What to get, from the first-run step; starts getting it. */
  choosePlan(plan: AiPlan): Promise<void>;
  /** Pauses a download; what's in is kept. */
  cancel(): Promise<void>;
  /** Deletes the models: Settings → Storage. */
  remove(): Promise<void>;
  /** Deletes one model: `embedder`, `decider` or a writing model's id. */
  removeModel(id: string): Promise<void>;
  /** Gets models on their own: `embedder`, `decider` or writing models' ids. The chosen model stays. */
  downloadModels(ids: string[]): Promise<void>;
  /** Stops getting one model, keeping what's in of it. */
  stopModel(id: string): Promise<void>;
}

/**
 * The AI's status, from the AI backend: the real Mac in the desktop app, the design's demo
 * elsewhere. Settings → AI, the first-run step, the top bar's download progress and the
 * search hint all read it.
 */
export const useAiSettings = create<AiSettingsState>()((set, get) => {
  /** Runs an action and shows the status it returns, or why it failed (and the status `before` it, given one). */
  const run = async (action: () => Promise<AiStatus | void>, before?: AiStatus) => {
    try {
      const status = await action();
      set({ problem: undefined, ...(status && { status }) });
    } catch (error) {
      set({ problem: errorText(error), ...(before && { status: before }) });
    }
  };
  return {
    load: async () => set({ status: await ai().status() }),
    loadStatus: () => run(() => ai().status()),
    chooseModel: async (id) => {
      const before = get().status;
      // Shown as chosen at once; a model that needs more memory never is.
      if (before?.writers.some((w) => w.id === id && !w.tooBig)) set({ status: { ...before, writer: id } });
      await run(() => ai().choose(id), before);
    },
    download: () => run(() => ai().download()),
    choosePlan: (plan) => run(() => ai().plan(plan)),
    cancel: () => run(() => ai().cancel()),
    remove: () => run(() => ai().remove()),
    removeModel: (id) => run(() => ai().removeModel(id)),
    downloadModels: (ids) => run(() => ai().downloadModels(ids)),
    stopModel: (id) => run(() => ai().stopModel(id)),
  };
});

/** Keeps Settings → AI in step with the AI's own status events; returns a function that stops it. */
export function watchAi() {
  return ai().onStatus((status) => useAiSettings.setState({ status }));
}
