/** Files in the composer: chosen or dropped, kept with the draft, forwarded, too big, sent. */
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { message, person, resetStore, thread, work } from "../test/fixtures";
import type { DraftFile } from "../data/types";
import { draftsSaved, hasContent, saveDelay, useDrafts, watchDrafts } from "./drafts";
import { filesTooBig, undoSendMs, useStore } from "./store";

import { openSendLater, pickTime, scheduleDraft, useScheduled } from "./scheduled";

const s = () => useStore.getState();
const MB = 1024 * 1024;

const plan: DraftFile = { id: "1", name: "Plan.pdf", bytes: 2048, contentType: "application/pdf" };
const notes: DraftFile = { id: "2", name: "notes.txt", bytes: 2, contentType: "text/plain" };

let stop = () => {};
beforeEach(() => {
  vi.useFakeTimers();
  stop = watchDrafts(useStore);
  const q4 = { index: 3, name: "Q4.pdf", size: "2.0 MB", bytes: 2 * MB, contentType: "application/pdf" };
  resetStore([
    thread("a", { messages: [message({ id: "a-1", attachments: [q4] })] }),
    // Demo mail's files have no index or size.
    thread("b", { messages: [message({ id: "b-1", attachments: [{ name: "logo.png", size: "3 KB" }] })] }),
  ]);
});
afterEach(() => {
  stop();
  vi.useRealTimers();
});

it("Attach opens the file picker and adds what you choose, with thumbnails", async () => {
  const { backend } = fakeMail({
    pickFiles: vi.fn(async () => ["/tmp/Plan.pdf", "/tmp/notes.txt"]),
    attachFiles: vi.fn(async () => [plan, notes]),
    draftPreview: vi.fn(async () => "data:image/jpeg;base64,AA=="),
  });
  s().compose("new");
  const id = s().draft!.id;
  s().attach();
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.attachFiles).toHaveBeenCalledWith(id, ["/tmp/Plan.pdf", "/tmp/notes.txt"]);
  expect(s().draft!.attachments).toEqual([plan, notes]);
  // A PDF gets a thumbnail; a text file shows its icon.
  expect(backend.draftPreview).toHaveBeenCalledTimes(1);
  expect(s().previews[`draft:${id}:1`]).toBe("data:image/jpeg;base64,AA==");
});

it("choosing nothing adds nothing", async () => {
  const { backend } = fakeMail({ pickFiles: vi.fn(async () => []) });
  s().compose("new");
  s().attach();
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.attachFiles).not.toHaveBeenCalled();
  expect(s().draft!.attachments).toBeUndefined();
});

it("dropped files skip the picker, and add to what's there", async () => {
  const dropped = [new File(["hi"], "notes.txt")];
  const { backend } = fakeMail({ attachFiles: vi.fn(async () => [notes]) });
  s().compose("new");
  s().updateDraft({ attachments: [plan] });
  s().attach(dropped);
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.pickFiles).not.toHaveBeenCalled();
  expect(backend.attachFiles).toHaveBeenCalledWith(s().draft!.id, dropped);
  expect(s().draft!.attachments).toEqual([plan, notes]);
});

it("files that arrive after you left that email stay out of the next one", async () => {
  let finish = (_: DraftFile[]) => {};
  fakeMail({ attachFiles: vi.fn(() => new Promise<DraftFile[]>((done) => (finish = done))) });
  s().compose("new");
  s().attach(["/tmp/Plan.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  s().discardDraft();
  s().compose("new");
  finish([plan]);
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft!.attachments).toBeUndefined();
});

it("a file that can't be attached says why, until you try again", async () => {
  fakeMail({ attachFiles: vi.fn().mockRejectedValueOnce(new Error("can’t read gone.pdf")).mockResolvedValue([plan]) });
  s().compose("new");
  s().attach(["/tmp/gone.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  expect(s().attachProblem).toBe("Couldn’t attach: can’t read gone.pdf");
  s().attach(["/tmp/Plan.pdf"]);
  expect(s().attachProblem).toBeUndefined();
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft!.attachments).toEqual([plan]);
  // A new email starts without the problem.
  s().attach(["/tmp/gone.pdf"]);
  s().compose("new");
  expect(s().attachProblem).toBeUndefined();
});

it("removing a staged file releases its backend storage, but not forwarded files", async () => {
  const discardDraftFile = vi.fn(async () => {});
  fakeMail({ discardDraftFile });
  s().compose("new");
  const id = s().draft!.id;
  const forwarded = { ...notes, id: "message:1", forwarded: { messageId: "message", index: 1 } };
  s().updateDraft({ attachments: [plan, forwarded] });
  s().removeAttachment(plan.id);
  s().removeAttachment(forwarded.id);
  s().removeAttachment("missing");
  await draftsSaved();
  expect(discardDraftFile).toHaveBeenCalledExactlyOnceWith(id, plan.id);
  expect(s().draft!.attachments).toEqual([]);
});

it("releases files that finish staging after their composer was left", async () => {
  let finish!: (files: DraftFile[]) => void;
  const discardDraftFile = vi.fn(async () => {});
  fakeMail({ attachFiles: vi.fn(() => new Promise<DraftFile[]>((resolve) => { finish = resolve; })), discardDraftFile });
  s().compose("new");
  const id = s().draft!.id;
  s().attach(["/tmp/Plan.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  s().discardDraft();
  s().compose("new");
  finish([plan]);
  await vi.advanceTimersByTimeAsync(0);
  await draftsSaved();
  expect(discardDraftFile).toHaveBeenCalledExactlyOnceWith(id, plan.id);
  expect(s().draft!.attachments).toBeUndefined();
});

it("orders staging and removal after an already queued draft save", async () => {
  let finish!: () => void;
  const discardDraftFile = vi.fn(async () => {});
  const { backend } = fakeMail({
    saveDraft: vi.fn((draft) => new Promise<typeof draft>((resolve) => { finish = () => resolve(draft); })),
    attachFiles: vi.fn(async () => [plan]), discardDraftFile,
  });
  s().compose("new");
  s().updateDraft({ body: "Keep my text" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  s().attach(["/tmp/Plan.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  try { expect(backend.attachFiles).not.toHaveBeenCalled(); }
  finally { finish(); }
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft!.attachments).toEqual([plan]);
  s().removeAttachment(plan.id);
  await draftsSaved();
  expect(discardDraftFile).toHaveBeenCalledExactlyOnceWith(s().draft!.id, plan.id);
  expect(s().draft!.body).toBe("Keep my text");
});

it("a file can be taken off again", () => {
  fakeMail();
  s().compose("new");
  s().updateDraft({ attachments: [plan, notes] });
  s().removeAttachment("1");
  expect(s().draft!.attachments).toEqual([notes]);
  s().discardDraft();
  s().removeAttachment("2");
  expect(s().draft).toBeUndefined();
});

it("Attach does nothing without an email being written", async () => {
  const { backend } = fakeMail();
  s().attach(["/tmp/Plan.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.attachFiles).not.toHaveBeenCalled();
});

it("a forward keeps the files of the email it forwards", () => {
  fakeMail();
  s().compose("forward", "a");
  expect(s().draft!.attachments).toEqual([
    { id: "a-1:3", name: "Q4.pdf", bytes: 2 * MB, contentType: "application/pdf", forwarded: { messageId: "a-1", index: 3 } },
  ]);
  // Demo mail's: their place in the list stands in for the index.
  s().compose("forward", "b");
  expect(s().draft!.attachments).toEqual([
    { id: "b-1:0", name: "logo.png", bytes: 0, contentType: "", forwarded: { messageId: "b-1", index: 0 } },
  ]);
  s().compose("reply", "a");
  expect(s().draft!.attachments).toBeUndefined();
});

it("a draft opened again, or a forward, asks for its files' thumbnails", async () => {
  const { backend } = fakeMail({
    draftPreview: vi.fn(async () => "data:image/jpeg;base64,AA=="),
    attachmentPreview: vi.fn(async () => "data:image/jpeg;base64,BB=="),
  });
  useDrafts.setState({ drafts: [{ ...s(), id: "d9", mode: "new", fromAccountId: "work", to: [], cc: [], bcc: [], subject: "x", body: "", bodyHtml: "", fieldsOpen: true, showCc: false, showBcc: false, remindIfNoReply: false, signatureId: null, attachments: [plan, notes], updatedAt: 1 } as never] });
  s().openDraft("d9");
  s().compose("forward", "a");
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.draftPreview).toHaveBeenCalledWith("d9", "1");
  expect(backend.attachmentPreview).toHaveBeenCalledWith("a-1", 3);
  expect(s().previews).toMatchObject({ "draft:d9:1": "data:image/jpeg;base64,AA==", "a-1:3": "data:image/jpeg;base64,BB==" });
  // Asked once: opening it again doesn't ask again.
  s().openDraft("d9");
  expect(backend.draftPreview).toHaveBeenCalledTimes(1);
});

it("files alone are worth keeping as a draft", () => {
  fakeMail();
  s().compose("reply", "a");
  const d = s().draft!;
  expect(hasContent({ ...d, attachments: [] })).toBe(false);
  expect(hasContent({ ...d, attachments: [plan] })).toBe(true);
});

it("too many files for the account say so, and Send waits", () => {
  const { backend } = fakeMail();
  const gmail = { ...work, connection: { provider: "gmail" as const, summary: "Gmail" } };
  resetStore([thread("a")], [gmail]);
  s().compose("new");
  s().updateDraft({ subject: "Film", to: [work as never], attachments: [{ ...plan, bytes: 26 * MB }] });
  expect(filesTooBig(s().draft!)).toBe("These files are 26 MB. Gmail sends up to 25 MB.");
  s().send();
  expect(backend.sendAfter).not.toHaveBeenCalled();
  expect(s().sending).toBeFalsy();
  s().updateDraft({ attachments: [plan] });
  expect(filesTooBig(s().draft!)).toBeUndefined();
});

it("with several files, too big names the biggest", () => {
  fakeMail();
  const gmail = { ...work, connection: { provider: "gmail" as const, summary: "Gmail" } };
  resetStore([thread("a")], [gmail]);
  s().compose("new");
  const film = { ...plan, id: "film", name: "film.mov", bytes: 24 * MB };
  s().updateDraft({ attachments: [{ ...plan, bytes: 2 * MB }, film] });
  expect(filesTooBig(s().draft!)).toBe("These files are 26 MB. Gmail sends up to 25 MB. The biggest is film.mov (24.0 MB).");
});

it("sending keeps the files until the email has gone, so Undo still has them", async () => {
  const { backend } = fakeMail({ sendAfter: vi.fn(() => new Promise<boolean>(() => {})) });
  s().compose("new");
  s().updateDraft({ subject: "Plan", attachments: [plan] });
  await vi.advanceTimersByTimeAsync(saveDelay);
  s().send();
  await vi.advanceTimersByTimeAsync(1000);
  await draftsSaved();
  expect(backend.deleteDraft).not.toHaveBeenCalled();
  expect(backend.sendAfter).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ attachments: [plan] }), undoSendMs);
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft!.attachments).toEqual([plan]);
});

it("discarding lets the files go with the draft", async () => {
  const { backend } = fakeMail();
  s().compose("new");
  s().updateDraft({ attachments: [plan] });
  await vi.advanceTimersByTimeAsync(saveDelay);
  s().discardDraft();
  await draftsSaved();
  expect(backend.deleteDraft).toHaveBeenCalledWith(expect.any(String), false);
});

 it.each(["send", "sendDraft", "openSchedule", "pickSchedule", "scheduleDraft"] as const)("%s refuses an email while selected files are still importing", async (action) => {
  let finish!: (files: DraftFile[]) => void;
  const { backend } = fakeMail({ attachFiles: vi.fn(() => new Promise<DraftFile[]>((resolve) => { finish = resolve; })) });
  useScheduled.setState({ emails: [], picking: undefined });
  s().compose("new");
  s().updateDraft({ subject: "With the plan", to: [person("Recipient", "recipient@example.com")] });
  const draft = s().draft!;
  if (action === "pickSchedule") openSendLater();
  s().attach(["/tmp/Plan.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  try {
    if (action === "send") s().send();
    if (action === "sendDraft") s().sendDraft(draft);
    if (action === "openSchedule") openSendLater();
    if (action === "pickSchedule") pickTime(Date.now() + 60_000);
    if (action === "scheduleDraft") scheduleDraft(draft, Date.now() + 60_000);
    expect(backend.sendAfter).not.toHaveBeenCalled();
    expect(useScheduled.getState().emails).toEqual([]);
    expect(useScheduled.getState().picking).toBeUndefined();
    expect(s().draft?.id).toBe(draft.id);
    expect(s().sendError).toBe("Still attaching files. Try again when they are ready.");
  } finally { finish([plan]); }
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.scheduleSend).not.toHaveBeenCalled();
  expect(s().draft?.attachments).toEqual([plan]);
  expect(s().sendError).toBeUndefined();
});

it("waits for every concurrent import and sends the completed attachment list", async () => {
  const finishes: ((files: DraftFile[]) => void)[] = [];
  const { backend } = fakeMail({ attachFiles: vi.fn(() => new Promise<DraftFile[]>((resolve) => finishes.push(resolve))) });
  s().compose("new");
  s().updateDraft({ subject: "Both files" });
  s().attach(["/tmp/Plan.pdf"]);
  s().attach(["/tmp/notes.txt"]);
  s().send();
  expect(backend.sendAfter).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(0);
  finishes[0]([plan]);
  await vi.advanceTimersByTimeAsync(0);
  s().send();
  try {
    expect(backend.sendAfter).not.toHaveBeenCalled();
    expect(s().sendError).toBe("Still attaching files. Try again when they are ready.");
  } finally { finishes[1]([notes]); }
  await vi.advanceTimersByTimeAsync(0);
  s().send();
  expect(backend.sendAfter).toHaveBeenCalledExactlyOnceWith(expect.any(String), expect.objectContaining({ attachments: [plan, notes] }), undoSendMs);
});

it("blocks while the picker is open and releases the guard when it is cancelled", async () => {
  let finish!: (files: string[]) => void;
  const { backend } = fakeMail({ pickFiles: vi.fn(() => new Promise<string[]>((resolve) => { finish = resolve; })) });
  s().compose("new");
  s().updateDraft({ subject: "No files after all" });
  s().attach();
  s().send();
  try { expect(backend.sendAfter).not.toHaveBeenCalled(); }
  finally { finish([]); }
  await vi.advanceTimersByTimeAsync(0);
  s().send();
  expect(backend.sendAfter).toHaveBeenCalledTimes(1);
  expect(backend.attachFiles).not.toHaveBeenCalled();
});

it.each([false, true])("a failed import cannot replace another composer's error (new composer: %s)", async (openNext) => {
  let fail!: (error: Error) => void;
  fakeMail({ attachFiles: vi.fn(() => new Promise<DraftFile[]>((_, reject) => { fail = reject; })) });
  s().compose("new");
  s().attach(["/tmp/gone.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  s().discardDraft();
  if (openNext) s().compose("new");
  useStore.setState({ attachProblem: "Keep this error" });
  fail(new Error("old file is gone"));
  await vi.advanceTimersByTimeAsync(0);
  expect(s().attachProblem).toBe("Keep this error");
});

it.each(["thread", "otherComposer"] as const)("pending imports show a visible refusal from %s without changing another composer's error", async (surface) => {
  let finish!: (files: DraftFile[]) => void;
  const { backend } = fakeMail({ attachFiles: vi.fn(() => new Promise<DraftFile[]>((resolve) => { finish = resolve; })) });
  s().compose("new");
  s().updateDraft({ subject: "Still importing" });
  const draft = s().draft!;
  s().attach(["/tmp/Plan.pdf"]);
  await vi.advanceTimersByTimeAsync(0);
  if (surface === "otherComposer") s().compose("new");
  else useStore.setState({ view: "thread" });
  useStore.setState({ sendError: "Keep another error" });
  s().sendDraft(draft);
  try {
    expect(s().toast).toMatchObject({ text: "Still attaching files. Try again when they are ready.", problem: true, undo: false });
    expect(s().sendError).toBe("Keep another error");
    expect(backend.sendAfter).not.toHaveBeenCalled();
  } finally { finish([plan]); }
  await vi.advanceTimersByTimeAsync(0);
});
