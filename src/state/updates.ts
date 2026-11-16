import { create } from "zustand";
import { updates } from "../services/updates";

/**
 * Where an update stands. Moss checks by itself and downloads quietly; it only speaks up when
 * one is ready, or when you asked it to check (`checking`, `current`, `failed`).
 */
export type UpdateState =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "current" }
  | { state: "downloading"; version: string; done: number; total: number }
  | { state: "ready"; version: string }
  | { state: "failed"; message: string };

interface Updates {
  /** This Moss's version, once known. */
  version?: string;
  update: UpdateState;
  load(): Promise<void>;
  /** Looks for a newer Moss and downloads it. `asked`: you asked, so say how it went. */
  check(asked?: boolean): Promise<void>;
  restart(): Promise<void>;
}

/** How long "Up to date" or "Couldn't check" stays before going quiet. */
const SAYS_FOR = 4000;

const SIX_HOURS = 6 * 60 * 60 * 1000;

export const pristineUpdates = { version: undefined, update: { state: "idle" } as UpdateState };

export const useUpdates = create<Updates>()((set, get) => {
  /** Shows a short answer, then goes quiet unless something newer replaced it. */
  const say = (update: UpdateState) => {
    set({ update });
    setTimeout(() => get().update === update && set({ update: { state: "idle" } }), SAYS_FOR);
  };

  return {
    ...pristineUpdates,
    load: async () => set({ version: await updates().version() }),
    check: async (asked = false) => {
      const now = get().update.state;
      if (now === "downloading" || now === "ready") return;
      if (asked) set({ update: { state: "checking" } });
      let found;
      try {
        found = await updates().check();
      } catch {
        return asked ? say({ state: "failed", message: "Couldn’t check for updates. Try again later." }) : set({ update: { state: "idle" } });
      }
      if (!found) return asked ? say({ state: "current" }) : set({ update: { state: "idle" } });
      const { version } = found;
      set({ update: { state: "downloading", version, done: 0, total: 0 } });
      try {
        await updates().install((done, total) => set({ update: { state: "downloading", version, done, total } }));
        set({ update: { state: "ready", version } });
      } catch {
        // The next check tries again.
        set({ update: { state: "idle" } });
      }
    },
    restart: () => updates().restart(),
  };
});

/** Loads the version and checks now, then every `every` ms. Returns a stop. */
export function startUpdates(every = SIX_HOURS) {
  const s = useUpdates.getState();
  void s.load();
  void s.check();
  const timer = setInterval(() => void useUpdates.getState().check(), every);
  return () => clearInterval(timer);
}
