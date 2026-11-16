import { beforeEach, describe, expect, it, vi } from "vitest";
import { demoContent, demoSummaries, demoView, setSummaries, type SummaryBackend, type SummaryContent, type SummaryRule } from "../services/summaries";
import { lastChosen, useSummary, useSummarySettings } from "./summary";

const u = () => useSummary.getState();
const rule = (kind: SummaryRule["kind"], extra: Partial<SummaryRule> = {}): SummaryRule => ({ id: 1, sender: "northwind.co", kind, createdAt: 0, ...extra });

function backend(start: Parameters<typeof demoSummaries>[0] = {}): SummaryBackend {
  const demo = demoSummaries(start);
  setSummaries(demo);
  return demo;
}

beforeEach(() => {
  backend();
  localStorage.clear();
  useSummary.setState({ threadId: undefined, run: 0, last: undefined });
});

describe("opening an email", () => {
  it("shows nothing until something is kept", async () => {
    await u().load("q4-budget");
    expect(u()).toMatchObject({ threadId: "q4-budget", phase: "none", shown: undefined });
  });

  it("shows the summary kept, and its rule", async () => {
    const facts = demoView({ kind: "facts" }, { why: "rule" });
    backend({ kept: { "q4-budget": facts }, rules: [rule("facts")] });
    await u().load("q4-budget");
    expect(u()).toMatchObject({ phase: "shown", shown: facts, rule: { kind: "facts" } });
  });

  it("a custom summary brings its instruction back", async () => {
    backend({ kept: { "q4-budget": demoView({ kind: "custom", instruction: "In French" }) } });
    await u().load("q4-budget");
    expect(u().instruction).toBe("In French");
  });

  it("forgets what another email was showing, even if its answer comes late", async () => {
    const slow = demoSummaries({ kept: { a: demoView({ kind: "short" }) } });
    let release: () => void = () => {};
    setSummaries({ ...slow, kept: (id) => new Promise((r) => (release = () => r(slow.kept(id)))) });
    const first = u().load("a");
    backend();
    await u().load("b");
    release();
    await first;
    expect(u()).toMatchObject({ threadId: "b", shown: undefined });
  });
});

describe("asking for a summary", () => {
  beforeEach(() => u().load("q4-budget"));

  it("writes the kind picked and offers to make it the rule", async () => {
    u().setMenu(true);
    u().pick("actions");
    expect(u()).toMatchObject({ menuOpen: false, phase: "loading", asked: { kind: "actions" } });
    await expect.poll(() => u().phase).toBe("shown");
    expect(u()).toMatchObject({ shown: { kind: "actions", why: "asked" }, offer: true, asked: undefined });
    u().notNow();
    expect(u().offer).toBe(false);
  });

  it("shows the summary as it's written, until it's done", async () => {
    let hear: (c: SummaryContent) => void = () => {};
    let answer: (v: unknown) => void = () => {};
    setSummaries({
      ...demoSummaries(),
      summarize: (_id, _request, _again, onContent) => new Promise((r) => ((hear = onContent!), (answer = r as never))),
    });
    const writing = u().summarize({ kind: "short" });
    expect(u().writing).toBeUndefined();
    hear({ format: "text", text: "Dana" });
    expect(u()).toMatchObject({ phase: "loading", writing: { format: "text", text: "Dana" } });
    answer(demoView({ kind: "short" }));
    await writing;
    expect(u()).toMatchObject({ phase: "shown", writing: undefined, shown: { content: demoContent.short } });
  });

  it("Stop drops what's written so far, and what an older ask writes after", async () => {
    let hear: (c: SummaryContent) => void = () => {};
    setSummaries({ ...demoSummaries(), summarize: (_id, _request, _again, onContent) => ((hear = onContent!), new Promise(() => {})) });
    void u().summarize({ kind: "short" });
    hear({ format: "text", text: "Dana" });
    u().stop();
    expect(u()).toMatchObject({ phase: "none", writing: undefined });
    hear({ format: "text", text: "Dana needs" });
    expect(u().writing).toBeUndefined();
    // A failure takes it away too.
    setSummaries({ ...demoSummaries(), summarize: async (_id, _request, _again, onContent) => (onContent!({ format: "text", text: "Da" }), Promise.reject({ kind: "model" })) });
    await u().summarize({ kind: "short" });
    expect(u()).toMatchObject({ phase: "failed", writing: undefined });
  });

  it("Custom asks for the instruction first, and an empty one does nothing", async () => {
    u().pick("custom");
    expect(u()).toMatchObject({ phase: "custom", instruction: "" });
    await u().summarize({ kind: "custom", instruction: "  " });
    expect(u().phase).toBe("custom");
    u().setInstruction("Only the amount");
    await u().summarize({ kind: "custom", instruction: u().instruction });
    expect(u()).toMatchObject({ phase: "shown", shown: { kind: "custom", instruction: "Only the amount" }, instruction: "Only the amount" });
    // Custom again starts from the instruction shown.
    u().pick("custom");
    expect(u().instruction).toBe("Only the amount");
  });

  it("says when the writing model isn't here, or failed, and tries again", async () => {
    backend({ state: "notReady" });
    await u().summarize({ kind: "facts" });
    expect(u().phase).toBe("notReady");
    backend({ state: "failed" });
    await u().summarize({ kind: "facts" });
    expect(u()).toMatchObject({ phase: "failed", asked: { kind: "facts" }, problem: "the AI model failed: it wrote no summary" });
    backend();
    await u().retry();
    expect(u()).toMatchObject({ phase: "shown", shown: { kind: "facts" } });
    await u().retry();
    expect(u().phase).toBe("shown");
  });

  it("can be stopped: what was shown before comes back, and the late answer is dropped", async () => {
    await u().summarize({ kind: "short" });
    const short = u().shown;
    let answer: (v: unknown) => void = () => {};
    setSummaries({ ...demoSummaries(), summarize: () => new Promise((r) => (answer = r as never)) });
    const writing = u().summarize({ kind: "facts" });
    expect(u().phase).toBe("loading");
    u().stop();
    expect(u()).toMatchObject({ phase: "shown", shown: short });
    answer(demoView({ kind: "facts" }));
    await writing;
    expect(u().shown).toBe(short);
    // A failure that comes after Stop is dropped too.
    setSummaries({ ...demoSummaries(), summarize: () => Promise.reject({ kind: "model", message: "x" }) });
    const failing = u().summarize({ kind: "facts" });
    u().stop();
    await failing;
    expect(u().phase).toBe("shown");
  });

  it("stopping with nothing shown goes back to nothing", async () => {
    setSummaries({ ...demoSummaries(), summarize: () => new Promise(() => {}) });
    void u().summarize({ kind: "short" });
    u().stop();
    expect(u().phase).toBe("none");
  });

  it("remembers the kind you chose last, on this device, and S writes it straight away", async () => {
    // Nothing chosen yet: S opens the menu.
    u().summariseOrChoose();
    expect(u()).toMatchObject({ menuOpen: true, phase: "none" });
    u().pick("detailed");
    await expect.poll(() => u().phase).toBe("shown");
    expect(u().last).toEqual({ kind: "detailed" });
    expect(JSON.parse(localStorage.getItem("moss.summaryKind")!)).toEqual({ kind: "detailed" });
    // With a summary shown, S changes its kind.
    u().summariseOrChoose();
    expect(u().menuOpen).toBe(true);
    u().setMenu(false);
    // Another email: S writes it in the kind remembered.
    await u().load("other");
    u().summariseOrChoose();
    expect(u()).toMatchObject({ menuOpen: false, phase: "loading", asked: { kind: "detailed" } });
    await expect.poll(() => u().phase).toBe("shown");
    // A custom one brings its instruction.
    await u().summarize({ kind: "custom", instruction: "In French" });
    await u().load("third");
    u().summariseOrChoose();
    expect(u().asked).toEqual({ kind: "custom", instruction: "In French" });
  });

  it("reads the kind remembered as it starts, and copes without it", async () => {
    localStorage.setItem("moss.summaryKind", JSON.stringify({ kind: "facts" }));
    expect(lastChosen()).toEqual({ kind: "facts" });
    localStorage.setItem("moss.summaryKind", "{not json");
    expect(lastChosen()).toBeUndefined();
    localStorage.setItem("moss.summaryKind", JSON.stringify({ kind: "nonsense" }));
    expect(lastChosen()).toBeUndefined();
    localStorage.setItem("moss.summaryKind", JSON.stringify({ kind: "custom" }));
    expect(lastChosen()).toBeUndefined();
    localStorage.removeItem("moss.summaryKind");
    expect(lastChosen()).toBeUndefined();
    // Storage that can't be written still remembers it until Moss quits.
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("full");
    });
    await u().summarize({ kind: "actions" });
    expect(u().last).toEqual({ kind: "actions" });
    setItem.mockRestore();
  });

  it("needs an open email", async () => {
    useSummary.setState({ threadId: undefined });
    await u().summarize({ kind: "short" });
    expect(u().phase).toBe("none");
  });
});

describe("discarding a summary", () => {
  beforeEach(() => u().load("q4-budget"));

  it("takes it off the card and out of what's kept", async () => {
    const demo = backend({ kept: { "q4-budget": demoView({ kind: "facts" }) } });
    await u().load("q4-budget");
    await u().discard();
    expect(u()).toMatchObject({ phase: "none", shown: undefined, offer: false });
    expect(await demo.kept("q4-budget")).toEqual({});
    // With nothing shown there's nothing to discard.
    await u().discard();
    expect(u().phase).toBe("none");
  });

  it("brings it back when it couldn't be discarded", async () => {
    const view = demoView({ kind: "facts" });
    setSummaries({ ...demoSummaries({ kept: { "q4-budget": view } }), discard: () => Promise.reject({ kind: "store", message: "disk full" }) });
    await u().load("q4-budget");
    await u().discard();
    expect(u()).toMatchObject({ phase: "shown", shown: view });
    // Not over an email opened meanwhile.
    let fail: (e: unknown) => void = () => {};
    setSummaries({ ...demoSummaries({ kept: { "q4-budget": view } }), discard: () => new Promise((_, reject) => (fail = reject)) });
    await u().load("q4-budget");
    const discarding = u().discard();
    await u().load("other");
    fail({ kind: "store" });
    await discarding;
    expect(u()).toMatchObject({ threadId: "other", phase: "none" });
  });
});

describe("rules from the card", () => {
  beforeEach(() => u().load("q4-budget"));

  it("Always makes the kind shown the rule, with a custom instruction", async () => {
    await u().always("x.co");
    expect(u().rule).toBeUndefined();
    await u().summarize({ kind: "custom", instruction: "In French" });
    await u().always("daniel@northwind.co");
    expect(u()).toMatchObject({
      offer: false,
      rule: { sender: "daniel@northwind.co", kind: "custom", instruction: "In French" },
      shown: { why: "rule" },
    });
    // With a rule, the next summary asks nothing.
    await u().summarize({ kind: "short" });
    expect(u().offer).toBe(false);
  });

  it("says why a rule didn't save", async () => {
    await u().summarize({ kind: "short" });
    await u().always("gmail.com");
    expect(u()).toMatchObject({ offer: true, problem: "gmail.com is a public mail service: make the rule for one address there" });
  });

  it("Stop rule removes it at once, and ⌘Z brings it back", async () => {
    const demo = backend({ rules: [rule("facts")] });
    await u().load("q4-budget");
    await u().stopRule();
    expect(u()).toMatchObject({ rule: undefined, stopped: { kind: "facts" } });
    expect(await demo.rules()).toEqual([]);
    await u().undoStop();
    expect(u()).toMatchObject({ stopped: undefined, rule: { sender: "northwind.co", kind: "facts" } });
    expect((await demo.rules()).map((r) => r.sender)).toEqual(["northwind.co"]);
    await u().undoStop();
    await u().stopRule();
    await u().stopRule();
    expect(await demo.rules()).toEqual([]);
  });

  it("a summary a rule wrote in the background shows up", async () => {
    const demo = backend({ rules: [rule("facts")] });
    await u().load("q4-budget");
    await demo.summarize("q4-budget", { kind: "facts" });
    await u().written("other");
    expect(u().shown).toBeUndefined();
    await u().written("q4-budget");
    expect(u().shown).toMatchObject({ kind: "facts", why: "rule" });
    // Not while you're writing a custom instruction.
    u().pick("custom");
    await u().written("q4-budget");
    expect(u().phase).toBe("custom");
  });
});

describe("Settings → AI → Summaries", () => {
  const r = () => useSummarySettings.getState();

  it("loads and changes the automatic setting, and holds no rules", async () => {
    const demo = backend({ rules: [rule("facts")], automatic: true });
    await r().load();
    expect(r().automatic).toBe(true);
    expect(r()).not.toHaveProperty("rules");
    await r().setAutomatic(false);
    expect(r().automatic).toBe(false);
    expect(await demo.automatic()).toBe(false);
  });

  it("says why a change didn't save, and clears it once one does", async () => {
    backend();
    setSummaries({ ...demoSummaries(), setAutomatic: () => Promise.reject({ kind: "store", message: "disk full" }) });
    await r().setAutomatic(false);
    expect(r().problem).toBe("disk full");
    backend();
    await r().setAutomatic(true);
    expect(r()).toMatchObject({ problem: undefined, automatic: true });
  });
});
