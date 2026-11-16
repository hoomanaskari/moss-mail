import { create } from "zustand";
import { errorText } from "../services/errorText";
import {
  summaries,
  summaryKinds,
  type SummaryError,
  type SummaryKind,
  type SummaryRequest,
  type SummaryContent,
  type SummaryRule,
  type SummaryView,
} from "../services/summaries";

/** What the summary card shows: nothing yet, a summary, Custom waiting for its instruction, or a state. */
export type SummaryPhase = "none" | "shown" | "custom" | "loading" | "notReady" | "failed";

interface SummaryState {
  /** The open email's thread. */
  threadId?: string;
  /** The kind you chose last, on this device: S writes it straight away. */
  last?: SummaryRequest;
  /** The rule for its sender. */
  rule?: SummaryRule;
  /** The summary on the card. */
  shown?: SummaryView;
  phase: SummaryPhase;
  /** What's being written, or failed: Try again asks for it again. */
  asked?: SummaryRequest;
  /** The summary being written, so far: the card shows it as it grows. */
  writing?: SummaryContent;
  menuOpen: boolean;
  /** Custom's instruction box. */
  instruction: string;
  /** Asks "Always summarise mail from…?" under a summary you asked for. */
  offer: boolean;
  /** The rule Stop rule just removed, until ⌘Z brings it back or you leave. */
  stopped?: SummaryRule;
  /** Why the last change didn't save, as a sentence. */
  problem?: string;
  /** Each request's number: the answer to an older one (stopped, or another email) is dropped. */
  run: number;
  /** Shows the summary kept for an email as it opens. */
  load(threadId: string): Promise<void>;
  /** Reloads what's kept after a rule wrote a summary in the background. */
  written(threadId: string): Promise<void>;
  setMenu(open: boolean): void;
  /** S: with nothing shown, writes the kind you chose last; otherwise (or the first time) opens the menu. */
  summariseOrChoose(): void;
  /** A kind from the menu: Custom asks for the instruction first. */
  pick(kind: SummaryKind): void;
  setInstruction(text: string): void;
  summarize(request: SummaryRequest, again?: boolean): Promise<void>;
  /** Stops waiting for a summary; whatever was shown before comes back. */
  stop(): void;
  retry(): Promise<void>;
  /** Takes the summary off the card and out of what's kept. */
  discard(): Promise<void>;
  /** Makes the shown summary's kind the rule for `sender`: an address or a domain. */
  always(sender: string): Promise<void>;
  notNow(): void;
  stopRule(): Promise<void>;
  undoStop(): Promise<void>;
}

const request = (v: SummaryRequest): SummaryRequest => (v.kind === "custom" ? { kind: v.kind, instruction: v.instruction } : { kind: v.kind });

const LAST = "moss.summaryKind";

/** The kind you chose last, as saved on this device; none when it can't be read or used. */
export function lastChosen(): SummaryRequest | undefined {
  try {
    const saved = JSON.parse(localStorage.getItem(LAST) ?? "null") as SummaryRequest | null;
    if (!saved || !summaryKinds.includes(saved.kind) || (saved.kind === "custom" && !saved.instruction)) return undefined;
    return request(saved);
  } catch {
    return undefined;
  }
}

function remember(chosen: SummaryRequest) {
  try {
    localStorage.setItem(LAST, JSON.stringify(chosen));
  } catch {
    // Storage full or unavailable: it's remembered until Moss quits.
  }
}

/** Back to how a newly opened email starts. */
const fresh = { rule: undefined, shown: undefined, phase: "none" as const, asked: undefined, writing: undefined, menuOpen: false, instruction: "", offer: false, stopped: undefined, problem: undefined };

/** The summary card in the reading view. */
export const useSummary = create<SummaryState>()((set, get) => {
  /** Shows what's kept for the open email. */
  const show = async (threadId: string, run: number) => {
    const { summary: shown, rule } = await summaries().kept(threadId);
    if (get().run !== run) return;
    set({ rule, shown, phase: shown ? "shown" : "none", instruction: shown?.instruction ?? "" });
  };
  return {
    ...fresh,
    last: lastChosen(),
    run: 0,
    load: async (threadId) => {
      const run = get().run + 1;
      set({ ...fresh, threadId, run });
      await show(threadId, run);
    },
    written: async (threadId) => {
      const { threadId: open, phase, run } = get();
      // Never under a summary being written or read; one you're waiting for wins.
      if (threadId !== open || phase === "loading" || phase === "custom") return;
      await show(threadId, run);
    },
    setMenu: (menuOpen) => set({ menuOpen }),
    summariseOrChoose: () => {
      const { phase, last } = get();
      if (phase === "none" && last) void get().summarize(last);
      else set({ menuOpen: true });
    },
    pick: (kind) => {
      set({ menuOpen: false });
      if (kind !== "custom") return void get().summarize({ kind });
      // Custom again starts from the instruction shown.
      const { shown, instruction } = get();
      set({ phase: "custom", offer: false, instruction: shown?.kind === "custom" ? instruction : "" });
    },
    setInstruction: (instruction) => set({ instruction }),
    summarize: async (asked, again = false) => {
      const { threadId } = get();
      if (!threadId || (asked.kind === "custom" && !asked.instruction?.trim())) return;
      const run = get().run + 1;
      const last = request(asked);
      remember(last);
      set({ run, phase: "loading", asked, last, writing: undefined, offer: false, menuOpen: false, problem: undefined });
      try {
        const view = await summaries().summarize(threadId, asked, again, (writing) => get().run === run && set({ writing }));
        if (get().run !== run) return;
        const { rule } = get();
        set({
          shown: view,
          phase: "shown",
          asked: undefined,
          writing: undefined,
          instruction: view.instruction ?? "",
          offer: !rule && view.why === "asked",
        });
      } catch (error) {
        if (get().run !== run) return;
        set({ phase: (error as SummaryError)?.kind === "notReady" ? "notReady" : "failed", problem: errorText(error), writing: undefined });
      }
    },
    stop: () => {
      const { run, shown } = get();
      set({ run: run + 1, phase: shown ? "shown" : "none", asked: undefined, writing: undefined });
    },
    retry: async () => {
      const { asked } = get();
      if (asked) await get().summarize(asked, true);
    },
    discard: async () => {
      const { threadId, shown, run } = get();
      if (!threadId || !shown) return;
      set({ run: run + 1, shown: undefined, phase: "none", offer: false });
      try {
        await summaries().discard(threadId);
      } catch {
        // It's still kept: show it again, unless you've moved on.
        if (get().run === run + 1) set({ shown, phase: "shown" });
      }
    },
    always: async (sender) => {
      const { shown } = get();
      if (!shown) return;
      try {
        const rule = await summaries().saveRule({ sender, ...request(shown) });
        set({ rule, offer: false, problem: undefined, shown: { ...shown, why: "rule" } });
      } catch (error) {
        set({ problem: errorText(error) });
      }
    },
    notNow: () => set({ offer: false }),
    stopRule: async () => {
      const { rule } = get();
      if (!rule) return;
      // Gone at once; the card stays until you leave.
      set({ rule: undefined, stopped: rule });
      await summaries().removeRule(rule.id);
    },
    undoStop: async () => {
      const { stopped } = get();
      if (!stopped) return;
      set({ stopped: undefined });
      const rule = await summaries().saveRule({ sender: stopped.sender, ...request(stopped) });
      set({ rule });
    },
  };
});

interface SummarySettingsState {
  /** Whether Moss writes the one-liners by itself. */
  automatic: boolean;
  problem?: string;
  load(): Promise<void>;
  setAutomatic(on: boolean): Promise<void>;
}

/** Settings → AI → Summaries. Rules aren't here: you set and stop them from the email. */
export const useSummarySettings = create<SummarySettingsState>()((set) => ({
  automatic: true,
  load: async () => set({ automatic: await summaries().automatic() }),
  setAutomatic: async (on) => {
    set({ automatic: on });
    try {
      await summaries().setAutomatic(on);
      set({ problem: undefined });
    } catch (error) {
      set({ problem: errorText(error) });
    }
  },
}));
