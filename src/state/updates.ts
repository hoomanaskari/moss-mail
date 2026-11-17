import { create } from "zustand";
import { useAppIdentity } from "../services/appIdentity";
import { errorText } from "../services/errorText";
import { updates } from "../services/updates";
import { currentDraftQuitBlockers } from "./drafts";
import { useStore } from "./store";

/**
 * Where an update stands. Moss looks for one by itself, a little after launch and every few
 * hours, and says nothing until it finds one: then the top bar offers it (`available`) until
 * you install it. A check you asked for (the app menu's Check for Updates…, the palette,
 * Settings) always answers: `checking`, then `current`, `available` or `failed`.
 */
export type UpdateState =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "current" }
  | { state: "available"; version: string; notes?: string }
  | { state: "downloading"; version: string; notes?: string; done: number; total: number }
  | { state: "ready"; version: string }
  | { state: "failed"; during: "check" | "download"; message: string; version?: string; notes?: string };

interface Updates {
  /** This Moss's version, once known. */
  version?: string;
  update: UpdateState;
  /** When Moss last heard back from the update server, in ms since the epoch. */
  checkedAt?: number;
  /** The update window is open: you asked from the menu or the palette. */
  open: boolean;
  load(): Promise<void>;
  /** Looks for a newer Moss. `asked`: you asked, so say how it went; `open`: in the update window. */
  check(how?: { asked?: boolean; open?: boolean }): Promise<void>;
  /** Downloads and installs the newer Moss found, then restarts into it, unless you're writing. */
  install(): Promise<void>;
  restart(): Promise<void>;
  close(): void;
}

export const pristineUpdates = { version: undefined, update: { state: "idle" } as UpdateState, checkedAt: undefined, open: false };

/** You're writing: a restart would take the composer away mid-sentence, so Restart now waits. */
function writing() {
  const s = useStore.getState();
  return s.view === "compose" || s.replyDraft !== undefined || currentDraftQuitBlockers().length > 0;
}

export const useUpdates = create<Updates>()((set, get) => ({
  ...pristineUpdates,
  load: async () => set({ version: await updates().version() }),
  check: async ({ asked = false, open = false } = {}) => {
    if (!useAppIdentity.getState().updates) return;
    if (open) set({ open: true });
    const now = get().update.state;
    if (now === "checking" || now === "downloading" || now === "ready") return;
    if (asked) set({ update: { state: "checking" } });
    let found;
    try {
      found = await updates().check();
    } catch (error) {
      // Only a check you asked for says it failed; a quiet one tries again later.
      if (asked) set({ update: { state: "failed", during: "check", message: errorText(error) } });
      return;
    }
    const update: UpdateState = found ? { state: "available", version: found.version, notes: found.notes } : { state: asked ? "current" : "idle" };
    set({ update, checkedAt: Date.now() });
  },
  install: async () => {
    const u = get().update;
    if (u.state !== "available" && !(u.state === "failed" && u.version)) return;
    const version = u.version!;
    const notes = u.notes;
    set({ update: { state: "downloading", version, notes, done: 0, total: 0 } });
    try {
      await updates().install((done, total) => set({ update: { state: "downloading", version, notes, done, total } }));
    } catch (error) {
      set({ update: { state: "failed", during: "download", message: errorText(error), version, notes } });
      return;
    }
    set({ update: { state: "ready", version } });
    if (!writing()) await get().restart();
  },
  restart: () => updates().restart(),
  close: () => set({ open: false }),
}));

/** How long after launch the first check waits, so it doesn't slow the start. */
const LAUNCH_DELAY = 10_000;
const SIX_HOURS = 6 * 60 * 60 * 1000;

/**
 * Loads the version, checks `delay` ms after launch, then every `every` ms, and when the window
 * comes back after more than `every` (timers stop while the Mac sleeps). Moss Dev never checks.
 * Returns a stop.
 */
export function startUpdates({ delay = LAUNCH_DELAY, every = SIX_HOURS } = {}) {
  void useUpdates.getState().load();
  if (!useAppIdentity.getState().updates) return () => {};
  let last = Date.now();
  const check = () => {
    last = Date.now();
    void useUpdates.getState().check();
  };
  const first = setTimeout(check, delay);
  const timer = setInterval(check, every);
  const onFocus = () => Date.now() - last > every && check();
  window.addEventListener("focus", onFocus);
  return () => {
    clearTimeout(first);
    clearInterval(timer);
    window.removeEventListener("focus", onFocus);
  };
}

const two = (n: number) => String(n).padStart(2, "0");
const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** When Moss last checked, in a few words: "Last checked 5 min ago". */
export function checkedText(at: number | undefined, now: number): string {
  if (at === undefined) return "Not checked yet";
  const minutes = Math.floor((now - at) / 60_000);
  if (minutes < 1) return "Last checked just now";
  if (minutes < 60) return `Last checked ${minutes} min ago`;
  const when = new Date(at);
  const time = `${two(when.getHours())}:${two(when.getMinutes())}`;
  return when.toDateString() === new Date(now).toDateString()
    ? `Last checked today at ${time}`
    : `Last checked on ${when.getDate()} ${months[when.getMonth()]} at ${time}`;
}
