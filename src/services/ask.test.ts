import { describe, expect, it } from "vitest";
import { message, thread } from "../test/fixtures";
import {
  applyEvent,
  ask,
  demoAsk,
  demoTurn,
  modelMissing,
  setAsk,
  startTurn,
  type AskAnswer,
  type AskEvent,
  type ChatTurn,
} from "./ask";

const threads = [
  thread("41", { subject: "Q4 budget", messages: [message({ id: "41-1" }), message({ id: "41-2" })] }),
  thread("57", { subject: "Re: Q4 budget" }),
];
const answer: AskAnswer = { text: "It's 182k.", sources: [threads[0]], messages: ["41-1"], drafts: [], stopped: false };

const run = (events: AskEvent[], from: ChatTurn = startTurn("How much?")) => events.reduce(applyEvent, from);

describe("a chat turn", () => {
  it("starts out thinking", () => {
    expect(startTurn("How much?")).toEqual({
      question: "How much?",
      state: "thinking",
      text: "",
      sources: [],
      messages: [],
      files: [],
      drafts: [],
      sendAt: {},
      did: [],
    });
  });

  it("says what it's looking up, writes, then shows the answer with its emails", () => {
    const looking = run([
      { type: "step", chat: "c", step: 1 },
      { type: "tool", chat: "c", tool: "search_mail", label: "Searching mail for “Q4”" },
    ]);
    expect(looking).toMatchObject({ state: "thinking", activity: "Searching mail for “Q4”" });
    const writing = run([{ type: "text", chat: "c", delta: "It's " }, { type: "text", chat: "c", delta: "182k" }], looking);
    expect(writing).toMatchObject({ state: "writing", text: "It's 182k" });
    expect(writing.activity).toBeUndefined();
    const done = applyEvent(writing, { type: "done", chat: "c", answer });
    expect(done).toEqual({
      question: "How much?",
      state: "done",
      text: "It's 182k.",
      sources: [threads[0]],
      messages: ["41-1"],
      files: [],
      drafts: [],
      sendAt: {},
      did: [],
    });
  });

  it("keeps what it changed, and when an email it readied should go", () => {
    const done = applyEvent(startTurn("Done with these, send Dana's reply tomorrow"), {
      type: "done",
      chat: "c",
      answer: { ...answer, did: ["Marked done: Q4 budget"], sendAt: { d1: 1_800_000_000_000 } },
    });
    expect(done).toMatchObject({ did: ["Marked done: Q4 budget"], sendAt: { d1: 1_800_000_000_000 } });
  });

  it("keeps the files the answer came from", () => {
    const file = { threadId: "41", messageId: "41-1", index: 2, name: "P2026-0457.pdf" };
    const done = applyEvent(startTurn("When?"), { type: "done", chat: "c", answer: { ...answer, files: [file] } });
    expect(done.files).toEqual([file]);
  });

  it("drops text written before it went to look something up", () => {
    const turn = run([
      { type: "text", chat: "c", delta: "Let me check." },
      { type: "step", chat: "c", step: 2 },
    ]);
    expect(turn).toMatchObject({ state: "thinking", text: "" });
  });

  it("can be stopped, or fail, and says when the model isn't downloaded", () => {
    expect(applyEvent(startTurn("q"), { type: "done", chat: "c", answer: { ...answer, stopped: true } }).state).toBe("stopped");
    const missing = applyEvent(startTurn("q"), { type: "failed", chat: "c", error: { kind: "notReady", message: "the AI model isn't downloaded yet" } });
    expect(missing).toMatchObject({ state: "failed", error: { kind: "notReady" } });
    expect(modelMissing(missing)).toBe(true);
    const failed = applyEvent(startTurn("q"), { type: "failed", chat: "c", error: { kind: "model", message: "boom" } });
    expect(modelMissing(failed)).toBe(false);
    expect(modelMissing(startTurn("q"))).toBe(false);
  });
});

describe("the demo", () => {
  const heard = (backend: ReturnType<typeof demoAsk>) => {
    const events: AskEvent[] = [];
    backend.onEvent((e) => events.push(e));
    return events;
  };

  it("answers Ask Moss citing emails, step by step", async () => {
    const demo = demoAsk({ threads });
    const events = heard(demo);
    const got = await demo.askMoss("m1", "What did Dana say about the budget?");
    expect(got.sources.map((t) => t.id)).toEqual(["41", "57"]);
    expect(got.drafts).toEqual([]);
    expect(events.map((e) => e.type)).toEqual(["step", "tool", "step", "tool", "step", ...events.slice(5, -1).map(() => "text"), "done"]);
    expect(events.every((e) => e.chat === "m1")).toBe(true);
    const text = events.flatMap((e) => (e.type === "text" ? [e.delta] : [])).join("");
    expect(text).toBe(got.text);
    expect(events[1]).toMatchObject({ label: "Searching mail for “What did Dana say about the budget?”" });
  });

  it("drafts a reply when asked for one, and never sends it", async () => {
    const demo = demoAsk({ threads });
    const events = heard(demo);
    const got = await demo.askMoss("m1", "Reply to Dana saying yes");
    expect(got.drafts).toHaveLength(1);
    expect(got.drafts[0]).toMatchObject({ mode: "reply", threadId: "41", fromAccountId: "work", subject: "Re: Q4 budget", signatureId: null });
    expect(got.drafts[0].to).toEqual([threads[0].from]);
    expect(got.drafts[0].bodyHtml).toBe(got.drafts[0].body.replace(/\n/g, "<br>"));
    expect(events.some((e) => e.type === "tool" && e.label === "Writing a draft reply")).toBe(true);
    expect(got.sources.map((t) => t.id)).toEqual(["41"]);
  });

  it("with no mail to point at, it answers without cards or drafts", async () => {
    const demo = demoAsk();
    expect((await demo.askMoss("m", "anything")).sources).toEqual([]);
    expect((await demo.askMoss("m", "draft a reply")).drafts).toEqual([]);
  });

  it("with an email open, answers from it", async () => {
    const demo = demoAsk({ threads });
    const events = heard(demo);
    const got = await demo.askMoss("m1", "When is it due?", "41");
    expect(got.text).toMatch(/Friday/);
    expect(got.sources).toEqual([]);
    expect(got.messages).toEqual(["41-1", "41-2"]);
    expect((await demo.askMoss("m1", "When?", "99")).messages).toEqual([]);
    expect(events[0]).toEqual({ type: "step", chat: "m1", step: 1 });
  });

  it("drafts a reply to the open email when asked", async () => {
    const got = await demoAsk({ threads }).askMoss("m1", "Draft a reply that says yes", "57");
    expect(got.drafts).toMatchObject([{ threadId: "57", subject: "Re: Q4 budget" }]);
  });

  it("does what it's told, and says what it did", async () => {
    const demo = demoAsk({ threads });
    const events = heard(demo);
    const got = await demo.askMoss("m1", "Mark the budget emails done");
    expect(got.did).toEqual(["Marked done: Q4 budget", "Marked done: Re: Q4 budget"]);
    expect(events.some((e) => e.type === "tool" && e.label === "Marking an email Done")).toBe(true);
    expect((await demoAsk().askMoss("m", "archive them")).did).toEqual([]);
  });

  it("says so plainly when nothing matches", async () => {
    const got = await demoAsk({ answer: "nothing", threads }).askMoss("m", "Find the PDF from the bank");
    expect(got).toMatchObject({ sources: [], drafts: [] });
    expect(got.text).toMatch(/couldn’t find/);
  });

  it("knows how many emails it looks through", async () => {
    expect(await demoAsk().mailCount()).toBe(2431);
  });

  it("can play a model that's missing or failing", async () => {
    await expect(demoAsk({ answer: "missing" }).askMoss("m", "q")).rejects.toEqual({ kind: "notReady", message: "the AI model isn't downloaded yet" });
    const failing = demoAsk({ answer: "failed" });
    const events = heard(failing);
    await expect(failing.askMoss("m", "q", "41")).rejects.toMatchObject({ kind: "model" });
    expect(events.at(-1)).toMatchObject({ type: "failed", chat: "m" });
  });

  it("keeps thinking until stopped, then returns what it has", async () => {
    const demo = demoAsk({ answer: "thinking", threads });
    const events = heard(demo);
    const pending = demo.askMoss("m", "q");
    await new Promise((r) => setTimeout(r, 5));
    expect(events.map((e) => e.type)).toEqual(["step", "tool"]);
    expect(await demo.stop("other")).toBe(false);
    expect(await demo.stop("m")).toBe(true);
    expect(await pending).toEqual({ text: "", sources: [], messages: [], drafts: [], sendAt: {}, did: [], stopped: true });
    expect(events.at(-1)).toMatchObject({ type: "done", answer: { stopped: true } });
    expect(await demo.stop("m")).toBe(false);
  });

  it("stopping mid-answer keeps the text so far", async () => {
    const demo = demoAsk({ threads, delayMs: 5 });
    const events = heard(demo);
    const pending = demo.askMoss("m", "q");
    while (!events.some((e) => e.type === "text")) await new Promise((r) => setTimeout(r, 1));
    await demo.stop("m");
    const got = await pending;
    expect(got.stopped).toBe(true);
    expect(got.text.length).toBeGreaterThan(0);
    expect(got.sources).toEqual([]);
  });

  it("forgets chats and stops listening when asked", async () => {
    const demo = demoAsk({ threads });
    const events: AskEvent[] = [];
    const stop = demo.onEvent((e) => events.push(e));
    stop();
    await demo.askMoss("m", "q");
    expect(events).toEqual([]);
    await expect(demo.forget("m")).resolves.toBeUndefined();
  });

  it("builds each turn a scene shows", () => {
    expect(demoTurn("cited", "q", threads)).toMatchObject({ state: "done", sources: [{ id: "41" }, { id: "57" }] });
    expect(demoTurn("draft", "q", threads).drafts).toHaveLength(1);
    expect(demoTurn("thread", "q", threads).state).toBe("done");
    expect(demoTurn("thinking", "q", threads)).toMatchObject({ state: "thinking", activity: "Searching mail for “q”" });
    expect(modelMissing(demoTurn("missing", "q", threads))).toBe(true);
    expect(demoTurn("failed", "q", threads)).toMatchObject({ state: "failed", error: { kind: "model" } });
    expect(demoTurn("nothing", "q", threads)).toMatchObject({ state: "done", sources: [] });
    expect(demoTurn("thread", "q", threads, threads[0]).messages).toEqual(["41-1", "41-2"]);
    expect(demoTurn("acted", "q", threads).did).toHaveLength(2);
  });
});

it("the app's backend can be swapped", () => {
  const demo = demoAsk();
  setAsk(demo);
  expect(ask()).toBe(demo);
});
