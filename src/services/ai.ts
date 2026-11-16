/**
 * The AI on this Mac. In the desktop app that is the Rust side (`tauriAi`): it reads the real
 * Mac, downloads the models and runs them. In the browser, dev scenes and tests it is the demo
 * below, drawn from design.pen.
 */

/** Where the model download stands. `done` and `total` are bytes. */
export type AiDownload =
  | { state: "missing"; done: number; total: number }
  | { state: "downloading"; done: number; total: number }
  | { state: "failed"; done: number; total: number; message: string }
  | { state: "ready" };

/** Where one model's own file stands; `done` is the bytes of it on this Mac. A missing one with `done` above 0 is partly downloaded. */
export type AiModelDownload =
  | { state: "missing"; done: number }
  | { state: "queued"; done: number }
  | { state: "downloading"; done: number }
  | { state: "failed"; done: number; message: string }
  | { state: "ready" };

/** A writing model you can choose. */
export interface AiWriter {
  id: string;
  name: string;
  sub: string;
  /** e.g. "About 5 GB". */
  size: string;
  bytes: number;
  recommended: boolean;
  /** Needs more memory than this Mac has, so it can't be chosen. */
  tooBig: boolean;
  /** Its own file: none of the files every model shares counts. */
  download: AiModelDownload;
}

/** What to get: everything this Mac runs well, the small models only, or nothing for now. */
export type AiPlan = "recommended" | "essentials" | "skip";

/** A plan as the first-run step offers it, with its real size on this Mac. */
export interface AiPlanInfo {
  plan: AiPlan;
  bytes: number;
  /** e.g. "About 6.4 GB". */
  size: string;
}

/** What the AI can do now: each part works as soon as its model is in. */
export interface AiFeatures {
  /** Search by meaning. */
  search: boolean;
  /** Sorting and reading mail. */
  sorting: boolean;
  /** Summaries, the ask and drafts. */
  writing: boolean;
}

/** Why a download is waiting. */
export type AiPause = "battery" | "hotspot";

export interface AiStatus {
  /** e.g. "M4 Max, 36 GB memory". */
  machine: string;
  writers: AiWriter[];
  /** The chosen writing model's id. */
  writer: string;
  /** The decision model's size, e.g. "About 1.1 GB". */
  deciderSize: string;
  /** The embedding model's size, e.g. "About 334 MB". */
  embedderSize: string;
  /** What was picked; null until someone has been asked. */
  plan: AiPlan | null;
  plans: AiPlanInfo[];
  features: AiFeatures;
  /** Bytes of models on this Mac, for Settings → Storage. */
  onDisk: number;
  /** The models with any of their file on this Mac, so each can be deleted: `embedder`, `decider` and writing models' ids. */
  downloaded: string[];
  /** The embedding and decision models' own files. */
  embedderDownload: AiModelDownload;
  deciderDownload: AiModelDownload;
  paused?: AiPause;
  /** The writing model that started writing by itself when its download finished, until your next choice. */
  tookOver: string | null;
  download: AiDownload;
}

/**
 * Whether a writing model whose download just finished should write from now on: when you asked
 * for it and haven't chosen anything else since, or when nothing on this Mac can write (the model
 * that did was deleted). As in Rust's `manager::takes_over`.
 */
export function takesOver(finished: string, askedFor: string | null, chosenReady: boolean) {
  return askedFor === finished || !chosenReady;
}

export interface AiBackend {
  status(): Promise<AiStatus>;
  choose(id: string): Promise<AiStatus>;
  /** Starts getting the models; progress comes through `onStatus`. */
  download(): Promise<AiStatus>;
  /** Picks what to get on the first-run step, and starts getting it. */
  plan(plan: AiPlan): Promise<AiStatus>;
  /** Stops a download, keeping what is in. */
  cancel(): Promise<void>;
  /** Deletes the models and turns the AI off. */
  remove(): Promise<AiStatus>;
  /** Deletes one model (`embedder`, `decider` or a writing model's id); its job stops until it's downloaded again. */
  removeModel(id: string): Promise<AiStatus>;
  /** Gets models on their own (`embedder`, `decider` or writing models' ids), without changing who writes. */
  downloadModels(ids: string[]): Promise<AiStatus>;
  /** Stops getting one model, keeping what's in of it. */
  stopModel(id: string): Promise<AiStatus>;
  /** "The ask" for a thread: one line on what it wants of you, or null. */
  ask(threadId: string): Promise<string | null>;
  /** Calls `handler` with every new status; returns a function that stops it. */
  onStatus(handler: (status: AiStatus) => void): () => void;
}

/** The demo's models: the real catalog, on the Mac drawn in design.pen. */
export const demoWriters: AiWriter[] = [
  { id: "e2b", name: "Gemma 4 E2B", sub: "Fastest. Good for summaries and quick replies.", size: "About 3.1 GB", bytes: 3_106_738_272, recommended: false, tooBig: false, download: { state: "missing", done: 0 } },
  { id: "e4b", name: "Gemma 4 E4B", sub: "Smarter replies, still quick. Needs 16 GB of memory or more.", size: "About 5 GB", bytes: 4_977_171_584, recommended: true, tooBig: false, download: { state: "missing", done: 0 } },
  { id: "26b", name: "Gemma 4 26B", sub: "Smartest. Needs 32 GB of memory or more.", size: "About 17 GB", bytes: 16_947_541_728, recommended: false, tooBig: true, download: { state: "missing", done: 0 } },
];

const runtimeBytes = 16_465_074;
const embedderBytes = 333_590_944;
const deciderBytes = 1_082_017_344;
const essentials = runtimeBytes + embedderBytes + deciderBytes;

/** "About 6.4 GB", like the Rust side's size labels. */
/** Apple Intelligence, on a Mac that has it: built in, so always ready, with nothing to download. */
export const demoApple: AiWriter = {
  id: "apple",
  name: "Apple Intelligence",
  sub: "Built into macOS: quick, with nothing to download.",
  size: "Built in",
  bytes: 0,
  recommended: true,
  tooBig: false,
  download: { state: "ready" },
};

export function sizeLabel(bytes: number) {
  const gb = bytes / 1e9;
  if (gb >= 10) return `About ${Math.round(gb)} GB`;
  if (gb >= 1) return `About ${gb.toFixed(1).replace(/\.0$/, "")} GB`;
  return `About ${Math.round(bytes / 1e6)} MB`;
}

/** Bytes to download for a writing model: the runtime, the embedding and decision models and the writer. */
export const totalFor = (writer: AiWriter) => essentials + writer.bytes;

const none: AiFeatures = { search: false, sorting: false, writing: false };
const all: AiFeatures = { search: true, sorting: true, writing: true };

/** Where the demo starts: `download`, and what was picked and can run. */
export interface DemoAiStart {
  download?: AiDownload;
  plan?: AiPlan | null;
  features?: AiFeatures;
  paused?: AiPause;
  /** Models' own downloads that aren't simply in or not, by id: a download partly in, running or failed. */
  models?: Record<string, AiModelDownload>;
  /** Writing models on this Mac besides what `features` says. */
  have?: string[];
  /** The writing model chosen, when it isn't the recommended one. */
  writer?: string;
  /** A writing model that started writing by itself when its download finished. */
  tookOver?: string;
  /** This Mac has Apple Intelligence: it's listed first, recommended, and writes unless `writer` says otherwise. */
  apple?: boolean;
}

/** Every model, by the id `removeModel` takes, in the order the Rust side lists them. */
const models = ["embedder", "decider", ...demoWriters.map((w) => w.id)];

const bytesOf = (id: string) =>
  id === "embedder" ? embedderBytes : id === "decider" ? deciderBytes : demoWriters.find((w) => w.id === id)!.bytes;

const sum = (ids: string[]) => ids.reduce((n, id) => n + bytesOf(id), 0);

/**
 * The demo AI. `start` is where it starts; a demo download is ready at once, and the demo
 * writes no asks. Like the Rust side, it keeps a writing model you switched away from until you
 * delete it.
 */
export function demoAi(start: AiDownload | DemoAiStart = {}): AiBackend {
  const from: DemoAiStart = "state" in start ? { download: start } : start;
  const handlers = new Set<(status: AiStatus) => void>();
  const writers = from.apple ? [demoApple, ...demoWriters.map((w) => ({ ...w, recommended: false }))] : demoWriters;
  /** Built in: never downloaded, never deleted. */
  const builtIn = (id: string) => id === demoApple.id;
  let writer = writers.find((w) => w.id === from.writer) ?? (from.apple ? demoApple : demoWriters[1]);
  let state: AiDownload = from.download ?? { state: "missing", done: 0, total: totalFor(writer) };
  let plan: AiPlan | null = from.plan ?? (from.download ? "recommended" : null);
  const features = from.features ?? (state.state === "ready" ? all : none);
  const downloaded = new Set<string>([
    ...["embedder", "decider", writer.id].filter((id, i) => [features.search, features.sorting, features.writing][i] && !builtIn(id)),
    ...(from.have ?? []),
  ]);
  const own = new Map(Object.entries(from.models ?? {}));
  let tookOver: string | null = from.tookOver ?? null;
  /** Where a model's own file stands. */
  const modelDownload = (id: string): AiModelDownload => {
    if (downloaded.has(id) || builtIn(id)) return { state: "ready" };
    return own.get(id) ?? { state: "missing", done: 0 };
  };
  /** What a download gets: the small models, and the writing model unless the plan is essentials. */
  const wanted = () => ["embedder", "decider", ...(plan === "essentials" || builtIn(writer.id) ? [] : [writer.id])];
  /** Where the download stands, from what's on disk. */
  const settle = (): AiDownload => {
    const want = wanted();
    const have = want.filter((id) => downloaded.has(id));
    if (have.length === want.length) return { state: "ready" };
    return { state: "missing", done: have.length ? runtimeBytes + sum(have) : 0, total: runtimeBytes + sum(want) };
  };
  const plans = (): AiPlanInfo[] =>
    (
      [
        ["recommended", totalFor(writer)],
        ["essentials", essentials],
        ["skip", 0],
      ] as const
    ).map(([plan, bytes]) => ({ plan, bytes, size: sizeLabel(bytes) }));
  const status = (): AiStatus => {
    const ids = models.filter((id) => downloaded.has(id));
    return {
      machine: "M3, 16 GB memory",
      writers: writers.map((w) => ({ ...w, download: modelDownload(w.id) })),
      writer: writer.id,
      deciderSize: "About 1.1 GB",
      embedderSize: "About 334 MB",
      plan,
      plans: plans(),
      features: { search: downloaded.has("embedder"), sorting: downloaded.has("decider"), writing: modelDownload(writer.id).state === "ready" },
      onDisk: ids.length ? runtimeBytes + sum(ids) : 0,
      downloaded: ids,
      tookOver,
      embedderDownload: modelDownload("embedder"),
      deciderDownload: modelDownload("decider"),
      ...(from.paused && { paused: from.paused }),
      download: state,
    };
  };
  const change = (next: AiDownload) => {
    state = next;
    handlers.forEach((h) => h(status()));
    return status();
  };
  const get = () => {
    wanted().forEach((id) => downloaded.add(id));
    return change(settle());
  };
  return {
    status: async () => status(),
    choose: async (id) => {
      const chosen = writers.find((w) => w.id === id && !w.tooBig);
      if (!chosen) throw { kind: "choice", message: `That model needs more memory than this Mac has (${id}).` };
      writer = chosen;
      tookOver = null;
      return change(settle());
    },
    download: async () => {
      plan = plan === "essentials" ? plan : "recommended";
      return get();
    },
    plan: async (picked) => {
      plan = picked;
      return picked === "skip" ? change(state) : get();
    },
    cancel: async () => {
      if (state.state !== "ready") change({ state: "missing", done: state.done, total: state.total });
    },
    remove: async () => {
      plan = "skip";
      downloaded.clear();
      return change(settle());
    },
    removeModel: async (id) => {
      if (!models.includes(id)) throw { kind: "choice", message: `That model needs more memory than this Mac has (${id}).` };
      downloaded.delete(id);
      own.delete(id);
      return change(settle());
    },
    downloadModels: async (ids) => {
      if (ids.some((id) => !models.includes(id))) throw { kind: "choice", message: `Moss doesn’t know that model (${ids.join(", ")}).` };
      // A demo download is in at once: the writing model asked for writes, as the Rust side decides.
      const asked = [...ids].reverse().find((id) => demoWriters.some((w) => w.id === id)) ?? null;
      ids.forEach((id) => {
        const w = demoWriters.find((x) => x.id === id);
        if (w && w !== writer && !w.tooBig && takesOver(id, asked, modelDownload(writer.id).state === "ready")) {
          writer = w;
          tookOver = id;
        }
        downloaded.add(id);
        own.delete(id);
      });
      return change(settle());
    },
    stopModel: async (id) => {
      const now = own.get(id);
      if (now && (now.state === "downloading" || now.state === "queued")) own.set(id, { state: "missing", done: now.done });
      return change(state);
    },
    ask: async () => null,
    onStatus: (handler) => {
      handlers.add(handler);
      return () => handlers.delete(handler);
    },
  };
}

let current: AiBackend = demoAi();

export const ai = () => current;

export function setAi(backend: AiBackend) {
  current = backend;
}
