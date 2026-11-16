import { beforeEach, expect, it, vi } from "vitest";
import { demoReplies, setReplies, type ReplyBackend } from "../services/replies";
import { maya, message, resetStore, thread } from "../test/fixtures";
import { quickSignature, replySignature, useQuickReply } from "./quickReply";
import { useSignatures } from "./signatures";
import { useStore } from "./store";

const q = () => useQuickReply.getState();
const s = () => useStore.getState();
/** What Quick reply asks for on email "a", above `signature`. */
const asked = (signature: string[]) => [expect.objectContaining({ id: expect.any(String), threadId: "a", accountId: "work", signature }), expect.any(Function)];

beforeEach(() => {
  resetStore([thread("a"), thread("b")]);
  useQuickReply.getState().close();
  setReplies(demoReplies());
});

it("Q asks Moss for replies to the open email and puts them on it", async () => {
  s().open("a");
  useStore.setState({ replyChoice: 2, replyDraft: "old" });
  const asking = q().start();
  expect(q()).toMatchObject({ threadId: "a", status: "writing" });
  await asking;
  expect(q()).toMatchObject({ threadId: "a", status: undefined });
  expect(s().threads[0].replyOptions?.map((o) => o.label)).toEqual(["Yes", "No", "Need more info"]);
  expect(s()).toMatchObject({ replyChoice: 0, replyDraft: undefined });
});

it("asks for replies that go above the signature the reply will get", async () => {
  const options = vi.fn(demoReplies().options);
  setReplies({ ...demoReplies(), options });
  useSignatures.setState({
    signatures: [{ id: "short", name: "Short", lines: ["Best", "Maya"] }],
    byAccount: { work: { newEmails: null, replies: "short" } },
    byAddress: {},
  });
  s().open("a");
  expect(replySignature(s().threads[0])?.lines).toEqual(["Best", "Maya"]);
  await q().start();
  expect(options).toHaveBeenCalledWith(...asked(["Best", "Maya"]));
  // No signature: none is sent.
  resetStore([thread("a")]);
  useSignatures.setState({ byAccount: { work: { newEmails: null, replies: null } } });
  expect(replySignature(s().threads[0])).toBeUndefined();
  s().open("a");
  await q().start();
  expect(options).toHaveBeenLastCalledWith(...asked([]));
});

it("in a back-and-forth it picks like the composer: here none, so replies are asked for without one", async () => {
  resetStore([thread("a", { messages: [message(), message({ id: "a-2", from: maya }), message({ id: "a-3" })] })]);
  useSignatures.setState({
    signatures: [{ id: "formal", name: "Formal", lines: ["Best regards", "Maya Ross"] }],
    byAccount: { work: { newEmails: "formal", replies: "formal" } },
    byAddress: {},
  });
  const options = vi.fn(demoReplies().options);
  setReplies({ ...demoReplies(), options });
  s().open("a");
  expect(quickSignature(s().threads[0])).toEqual({ signature: undefined, chosen: undefined, pick: { id: null, why: "quick" } });
  await q().start();
  expect(options).toHaveBeenCalledWith(...asked([]));
  // Editing it in the composer starts with the same pick.
  s().editInstantReply(quickSignature(s().threads[0]).chosen);
  expect(s().draft).toMatchObject({ signatureId: null });
  expect(s().draft?.signatureChosen).toBeFalsy();
});

it("you can pick another signature for an email's Quick reply, or none", async () => {
  useSignatures.setState({
    signatures: [
      { id: "short", name: "Short", lines: ["Best", "Maya"] },
      { id: "formal", name: "Formal", lines: ["Best regards", "Maya Ross"] },
    ],
    byAccount: { work: { newEmails: null, replies: "short" } },
    byAddress: {},
  });
  s().open("a");
  const a = () => s().threads.find((t) => t.id === "a")!;
  const b = () => s().threads.find((t) => t.id === "b")!;
  expect(quickSignature(a())).toEqual({ signature: expect.objectContaining({ id: "short" }), chosen: undefined, pick: { id: "short", why: "reply" } });
  q().toggleSignature();
  expect(q().signatureOpen).toBe(true);
  q().chooseSignature("formal");
  expect(q().signatureOpen).toBe(false);
  expect(quickSignature(a())).toEqual({ signature: expect.objectContaining({ id: "formal" }), chosen: "formal", pick: undefined });
  // Only on that email.
  expect(quickSignature(b()).chosen).toBeUndefined();
  q().chooseSignature(null);
  expect(quickSignature(a())).toEqual({ signature: undefined, chosen: null, pick: undefined });
  // New replies are asked for above the one you picked; Discard forgets it.
  const options = vi.fn(demoReplies().options);
  setReplies({ ...demoReplies(), options });
  await q().start();
  expect(options).toHaveBeenLastCalledWith(...asked([]));
  q().toggleSignature();
  q().discard();
  expect(q().signatureOpen).toBe(false);
  expect(quickSignature(a()).chosen).toBeUndefined();
  // With no email open there's nothing to pick it for.
  useStore.setState({ openThreadId: undefined });
  q().chooseSignature("formal");
  expect(q().signature).toBeUndefined();
});

it("Discard takes the replies away, with your changes, and Q writes new ones", async () => {
  const options = vi.fn(demoReplies().options);
  setReplies({ ...demoReplies(), options });
  s().open("a");
  await q().start();
  useStore.setState({ replyChoice: 1, replyDraft: "edited" });
  q().discard();
  expect(s().threads[0].replyOptions).toBeUndefined();
  expect(s()).toMatchObject({ replyChoice: 0, replyDraft: undefined });
  expect(q()).toMatchObject({ threadId: undefined, status: undefined });
  await q().start();
  expect(options).toHaveBeenCalledTimes(2);
  // On a Done email too; with no email open it does nothing.
  const done = thread("x", { doneAt: 5, replyOptions: [{ label: "Sure", draft: "Sure." }] });
  useStore.setState({ doneThreads: [done] });
  s().openDone();
  s().open("x");
  q().discard();
  expect(s().doneThreads[0].replyOptions).toBeUndefined();
  useStore.setState({ openThreadId: undefined });
  q().discard();
  expect(s().threads[0].replyOptions).toHaveLength(3);
});

it("an email that already has replies just shows them", async () => {
  resetStore([thread("a", { replyOptions: [{ label: "Sure", draft: "Sure." }] })]);
  s().open("a");
  await q().start();
  expect(q()).toMatchObject({ threadId: "a", status: undefined });
  expect(s().threads[0].replyOptions).toEqual([{ label: "Sure", draft: "Sure." }]);
});

it("works on a Done email too, and does nothing with no email open", async () => {
  const done = thread("x", { doneAt: 5 });
  useStore.setState({ doneThreads: [done] });
  s().openDone();
  s().open("x");
  await q().start();
  expect(s().doneThreads[0].replyOptions).toHaveLength(3);
  useStore.setState({ openThreadId: undefined });
  q().close();
  await q().start();
  expect(q().threadId).toBeUndefined();
});

it("says when the model is missing or failed, and Try again asks again", async () => {
  s().open("a");
  setReplies(demoReplies({ state: "missing" }));
  await q().start();
  expect(q().status).toBe("missing");
  setReplies(demoReplies({ state: "failed" }));
  await q().start();
  expect(q().status).toBe("failed");
  setReplies(demoReplies());
  await q().start();
  expect(q().status).toBeUndefined();
});

it("replies stream in as Moss writes them, and Stop keeps none of them", async () => {
  const demo = demoReplies({ state: "writing" });
  const options = vi.fn(demo.options);
  const stop = vi.fn(demo.stop);
  setReplies({ ...demo, options, stop });
  s().open("a");
  useStore.setState({ replyChoice: 2, replyDraft: "old" });
  const asking = q().start();
  await Promise.resolve();
  expect(q().status).toBe("writing");
  expect(s().threads[0].replyOptions).toEqual([{ label: "Yes", draft: "Hi,\n\nYes, that works for me." }]);
  expect(s()).toMatchObject({ replyChoice: 0, replyDraft: undefined });
  // Q again while it writes changes nothing.
  await q().start();
  expect(options).toHaveBeenCalledTimes(1);
  expect(q().status).toBe("writing");
  q().close();
  expect(stop).toHaveBeenCalledWith(options.mock.calls[0][0].id);
  await asking;
  expect(s().threads[0].replyOptions).toBeUndefined();
  expect(q()).toMatchObject({ threadId: undefined, status: undefined });
});

it("replies that fail part-way are taken away", async () => {
  setReplies({
    ...demoReplies(),
    options: async (_, onOptions) => {
      onOptions([{ label: "Yes", draft: "Hi" }]);
      throw { kind: "model", message: "x" };
    },
  });
  s().open("a");
  await q().start();
  expect(q().status).toBe("failed");
  expect(s().threads[0].replyOptions).toBeUndefined();
});

it("replies that arrive after you closed it or moved on are dropped", async () => {
  let answer: (v: { label: string; draft: string }[]) => void = () => {};
  let fail: (e: unknown) => void = () => {};
  let hear: (v: { label: string; draft: string }[]) => void = () => {};
  const slow: ReplyBackend = {
    ...demoReplies(),
    options: (_, onOptions) => new Promise((resolve, reject) => ((answer = resolve), (fail = reject), (hear = onOptions))),
  };
  setReplies(slow);
  s().open("a");
  const first = q().start();
  q().close();
  hear([{ label: "Late", draft: "La" }]);
  answer([{ label: "Late", draft: "Late." }]);
  await first;
  expect(s().threads[0].replyOptions).toBeUndefined();
  // Nor do the ones an earlier ask writes after you asked again.
  const again = q().start();
  hear([{ label: "Late", draft: "La" }]);
  expect(s().threads[0].replyOptions).toEqual([{ label: "Late", draft: "La" }]);
  const late = hear;
  q().close();
  const fresh = q().start();
  late([{ label: "Older", draft: "Ol" }]);
  expect(s().threads[0].replyOptions).toBeUndefined();
  q().close();
  answer([]);
  await fresh;
  void again;
  expect(q().threadId).toBeUndefined();
  const second = q().start();
  q().close();
  fail({ kind: "model", message: "x" });
  await second;
  expect(q().status).toBeUndefined();
});
