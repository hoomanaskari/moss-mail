import { expect, it } from "vitest";
import type { ReplyOption } from "../data/types";
import { demoReplies, firstHalf, replies, rewritesText, setReplies, type QuickRepliesRequest, type ReplyJob, type ReplyRequest } from "./replies";

const request = (job: ReplyJob, text = "", id = "r1"): ReplyRequest => ({ id, threadId: "t1", accountId: "work", job, text });
const ask = (id = "q1"): QuickRepliesRequest => ({ id, threadId: "t1", accountId: "work" });
const quiet = () => {};

/** Runs a demo write and keeps every text it showed on the way. */
async function written(backend = demoReplies(), job: ReplyJob, text = "") {
  const seen: string[] = [];
  const final = await backend.write(request(job, text), (t) => seen.push(t));
  return { final, seen };
}

it("rewrites need text in the composer; answers and instructions can start from nothing (as in Rust)", () => {
  const rewrites: ReplyJob[] = [
    { kind: "friendlier" },
    { kind: "formal" },
    { kind: "shorter" },
    { kind: "longer" },
    { kind: "fix" },
    { kind: "translate", language: "German" },
  ];
  const answers: ReplyJob[] = [{ kind: "accept" }, { kind: "decline" }, { kind: "moreTime" }, { kind: "instructions", instructions: "no" }];
  rewrites.forEach((job) => expect(rewritesText(job)).toBe(true));
  answers.forEach((job) => expect(rewritesText(job)).toBe(false));
});

it("the demo offers three quick replies, or the ones it's given", async () => {
  const options = await demoReplies().options(ask(), quiet);
  expect(options.map((o) => o.label)).toEqual(["Yes", "No", "Need more info"]);
  expect(options.every((o) => o.draft.length > 0)).toBe(true);
  const own = [{ label: "Sure", draft: "Sure thing." }];
  expect(await demoReplies({ options: own }).options(ask(), quiet)).toEqual(own);
});

it("half a text ends where a word ends, or mid-word when there's one word", () => {
  expect(firstHalf("Sure thing.")).toBe("Sure");
  expect(firstHalf("Thanks!")).toBe("Than");
  expect(firstHalf("")).toBe("");
});

it("the demo streams quick replies in one by one, each after the ones before", async () => {
  const heard: ReplyOption[][] = [];
  const own = [
    { label: "Sure", draft: "Sure thing." },
    { label: "No", draft: "No, sorry." },
  ];
  expect(await demoReplies({ options: own }).options(ask(), (o) => heard.push(o))).toEqual(own);
  expect(heard).toEqual([[{ label: "Sure", draft: "Sure" }], [own[0]], [own[0], { label: "No", draft: "No," }], own]);
});

it("the demo streams each answer in, then gives the whole text", async () => {
  for (const kind of ["accept", "decline", "moreTime"] as const) {
    const { final, seen } = await written(undefined, { kind });
    expect(final.length).toBeGreaterThan(10);
    expect(seen.at(-1)).toBe(final);
    expect(seen.length).toBeGreaterThan(1);
    expect(final.startsWith(seen[0])).toBe(true);
  }
  const { final } = await written(undefined, { kind: "instructions", instructions: "  say Friday works  " });
  expect(final).toBe("Hi,\n\nThanks for this. Say Friday works.");
  expect((await written(undefined, { kind: "instructions", instructions: "Friday works!" })).final).toBe("Hi,\n\nThanks for this. Friday works!");
});

it("the demo rewrites what's in the composer, and an empty composer is nothing to rewrite", async () => {
  const text = "Hi Dana,  i can do Friday.\n\nSee you then.";
  const rewrite = async (job: ReplyJob) => (await written(undefined, job, text)).final;
  expect(await rewrite({ kind: "shorter" })).toBe("Hi Dana,  i can do Friday.");
  expect(await rewrite({ kind: "longer" })).toBe(`${text}\n\nLet me know if you need anything else.`);
  expect(await rewrite({ kind: "friendlier" })).toBe(`${text}\n\nThanks so much!`);
  expect(await rewrite({ kind: "formal" })).toBe("Dear Dana,  i can do Friday.\n\nSee you then.");
  expect(await rewrite({ kind: "fix" })).toBe("Hi Dana, I can do Friday.\n\nSee you then.");
  expect(await rewrite({ kind: "translate", language: "French" })).toBe(`(French) ${text}`);
  expect(await written(undefined, { kind: "shorter" }, "  ")).toEqual({ final: "", seen: [] });
});

it("the demo can be missing its model or fail, like the Rust side", async () => {
  const missing = demoReplies({ state: "missing" });
  await expect(missing.options(ask(), quiet)).rejects.toEqual({ kind: "notReady", message: "the AI model isn't downloaded yet" });
  await expect(missing.write(request({ kind: "accept" }), () => {})).rejects.toMatchObject({ kind: "notReady" });
  const failed = demoReplies({ state: "failed" });
  await expect(failed.options(ask(), quiet)).rejects.toMatchObject({ kind: "model" });
  await expect(failed.write(request({ kind: "accept" }), () => {})).rejects.toMatchObject({ kind: "model" });
  // Nothing to rewrite never reaches the model.
  expect(await missing.write(request({ kind: "fix" }), () => {})).toBe("");
});

it("the demo can stay mid-way through writing until it's stopped", async () => {
  const demo = demoReplies({ state: "writing" });
  const seen: string[] = [];
  const pending = demo.write(request({ kind: "accept" }), (t) => seen.push(t));
  await Promise.resolve();
  expect(seen).toHaveLength(1);
  expect(await demo.stop("other")).toBe(false);
  expect(await demo.stop("r1")).toBe(true);
  expect(await pending).toBe(seen[0]);
  expect(await demo.stop("r1")).toBe(false);
  // Quick replies stop after the first one's start, and keep it.
  const heard: ReplyOption[][] = [];
  const options = demo.options(ask("q1"), (o) => heard.push(o));
  await Promise.resolve();
  expect(heard).toEqual([[{ label: "Yes", draft: "Hi,\n\nYes, that works for me." }]]);
  expect(await demo.stop("q1")).toBe(true);
  expect(await options).toEqual(heard[0]);
});

it("a ready demo has nothing to stop", async () => {
  expect(await demoReplies().stop("r1")).toBe(false);
});

it("the app's replies can be swapped", () => {
  const demo = demoReplies({ state: "missing" });
  setReplies(demo);
  expect(replies()).toBe(demo);
});
