/** Sending a draft from the Ask panel: with Undo, without leaving the screen you are on. */
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore, thread } from "../test/fixtures";
import type { SavedDraft } from "./drafts";
import { useDrafts } from "./drafts";
import { undoSendMs, useStore } from "./store";

const s = () => useStore.getState();

const draft: SavedDraft = {
  id: "ai-1",
  mode: "reply",
  threadId: "a",
  fromAccountId: "work",
  to: [],
  cc: [],
  bcc: [],
  subject: "Re: Subject a",
  body: "Yes.",
  bodyHtml: "Yes.",
  fieldsOpen: true,
  showCc: false,
  showBcc: false,
  remindIfNoReply: false,
  signatureId: null,
  updatedAt: 1,
};

beforeEach(() => {
  vi.useFakeTimers();
  resetStore([thread("a"), thread("b")]);
  useDrafts.setState({ drafts: [draft] });
});
afterEach(() => vi.useRealTimers());

it("sends with Undo and stays where you are; the draft leaves Drafts", async () => {
  const { backend } = fakeMail();
  useStore.setState({ view: "inbox", cursorId: "b" });
  s().sendDraft(draft);
  expect(backend.sendAfter).toHaveBeenCalledWith(expect.any(String), draft, undoSendMs);
  expect(s()).toMatchObject({ view: "inbox", cursorId: "b", toast: { text: "Sending" } });
  expect(s().threads.map((t) => t.id), "sending never marks mail Done").toEqual(["a", "b"]);
  await vi.advanceTimersByTimeAsync(0);
  expect(s().toast).toMatchObject({ text: "Sent" });
  expect(useDrafts.getState().drafts).toEqual([]);
});

it("⌘Z in time takes it back into the composer", async () => {
  const { backend } = fakeMail({ sendAfter: vi.fn(() => new Promise<boolean>(() => {})) });
  s().sendDraft(draft);
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.cancelSend).toHaveBeenCalled();
  expect(s()).toMatchObject({ view: "compose", draft: { id: "ai-1", body: "Yes." } });
});

it("a send that fails keeps it, to open again with why", async () => {
  fakeMail({ sendAfter: vi.fn(() => Promise.reject(new Error("No connection"))) });
  s().sendDraft(draft);
  await vi.advanceTimersByTimeAsync(0);
  expect(s().toast).toMatchObject({ text: "Couldn’t send your email", problem: true });
});
