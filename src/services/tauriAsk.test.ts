import { beforeEach, expect, it, vi } from "vitest";
import type { RustSummary } from "./convert";
import type { RustDraft } from "./tauriMail";

const invoke = vi.fn();
let listener: ((e: { payload: unknown }) => void) | undefined;
let heard: string | undefined;

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    heard = name;
    listener = cb;
    return () => {};
  },
}));

const { tauriAsk } = await import("./tauriAsk");

const person = { name: "Dana Lee", email: "dana@northwind.co", initials: "DL", short: "Dana" };
const summary: RustSummary = {
  id: "41",
  accountId: "work",
  split: "important",
  section: "reply",
  from: person,
  subject: "Q4 budget",
  summary: "The final budget is 182k.",
  lastMessageAt: Date.now(),
  snoozedUntil: null,
  unread: false,
  done: false,
  labels: [],
  messageCount: 2,
  hasAttachments: true,
  sender: "known",
};
const draft: RustDraft = {
  id: "d1",
  mode: "reply",
  threadId: "41",
  fromAccountId: "work",
  to: [{ name: "Dana Lee", email: "dana@northwind.co" }],
  cc: [],
  bcc: [],
  subject: "Re: Q4 budget",
  body: "Yes.",
  bodyHtml: "Yes.",
  remindIfNoReply: false,
  updatedAt: 5,
  signatureId: null,
  signatureChosen: false,
};
const file = { threadId: "41", messageId: "m1", index: 0, name: "budget.xlsx" };
const answer = { text: "It's 182k.", sources: [summary], messages: ["m1"], files: [file], drafts: [draft], sendAt: { d1: 9 }, did: ["Marked done: Lunch"], stopped: false };

beforeEach(() => invoke.mockReset().mockResolvedValue(answer));

it("each call is one Rust command, and answers come back in the UI's shapes", async () => {
  const got = await tauriAsk.askMoss("m1", "How much?", "41");
  await tauriAsk.askMoss("m1", "Who is Dana?");
  invoke.mockResolvedValue(true);
  expect(await tauriAsk.stop("m1")).toBe(true);
  await tauriAsk.forget("m1");
  invoke.mockResolvedValue(2431);
  expect(await tauriAsk.mailCount()).toBe(2431);
  expect(invoke.mock.calls).toEqual([
    ["ask_moss", { chat: "m1", question: "How much?", open: "41" }],
    ["ask_moss", { chat: "m1", question: "Who is Dana?", open: null }],
    ["ask_stop", { chat: "m1" }],
    ["ask_forget", { chat: "m1" }],
    ["ask_mail_count"],
  ]);
  expect(got.messages).toEqual(["m1"]);
  expect(got.text).toBe("It's 182k.");
  expect(got.sources[0]).toMatchObject({ id: "41", subject: "Q4 budget", partial: true, from: { name: "Dana Lee" } });
  expect(got.drafts[0]).toMatchObject({ id: "d1", threadId: "41", to: [{ name: "Dana Lee", email: "dana@northwind.co", initials: "DL" }], fieldsOpen: true });
  expect(got.stopped).toBe(false);
  expect(got.files).toEqual([file]);
  expect(got.sendAt).toEqual({ d1: 9 });
  expect(got.did).toEqual(["Marked done: Lunch"]);
});

it("errors keep their kind", async () => {
  const notReady = { kind: "notReady", message: "the AI model isn’t downloaded yet" };
  invoke.mockRejectedValueOnce(notReady);
  await expect(tauriAsk.askMoss("m", "q")).rejects.toBe(notReady);
});

it("events come from moss://ask, a finished answer in the UI's shapes", async () => {
  const seen: unknown[] = [];
  tauriAsk.onEvent((e) => seen.push(e));
  await Promise.resolve();
  expect(heard).toBe("moss://ask");
  listener!({ payload: { type: "text", chat: "m", delta: "Hi" } });
  listener!({ payload: { type: "done", chat: "m", answer } });
  expect(seen[0]).toEqual({ type: "text", chat: "m", delta: "Hi" });
  expect(seen[1]).toMatchObject({ type: "done", chat: "m", answer: { sources: [{ id: "41", partial: true }], drafts: [{ fieldsOpen: true }] } });
});
