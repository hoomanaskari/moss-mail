import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { lena, personal, resetStore, thread, work } from "../test/fixtures";
import { acknowledgedDraftRevision, adoptDraft, draftsSaved, forgetDraft, handoffDraft, hasContent, keepDraft, loadDrafts, reloadDrafts, saveDelay, useDrafts, visibleDrafts, watchDrafts, type SavedDraft } from "./drafts";
import { sendBeatMs, useStore, type Draft } from "./store";

const s = () => useStore.getState();
const d = () => useDrafts.getState();

const blank: Draft = {
  id: "d1",
  mode: "new",
  fromAccountId: "work",
  to: [],
  cc: [],
  bcc: [],
  subject: "",
  body: "",
  bodyHtml: "",
  fieldsOpen: true,
  showCc: false,
  showBcc: false,
  remindIfNoReply: false,
  signatureId: null,
};

const saved = (id: string, extra: Partial<SavedDraft> = {}): SavedDraft => ({ ...blank, id, subject: `Draft ${id}`, updatedAt: 1, ...extra });

let stop = () => {};
beforeEach(() => {
  vi.useFakeTimers();
  resetStore([thread("q4", { subject: "Q4 budget" })], [work, personal]);
  stop = watchDrafts(useStore);
});
afterEach(() => {
  stop();
  vi.useRealTimers();
});

/** Waits out the pause after typing, then for the save to finish. */
async function settle() {
  await vi.advanceTimersByTimeAsync(saveDelay);
  await draftsSaved();
}

describe("what counts as a draft worth keeping", () => {
  it("a new email needs words, a subject or someone to send to", () => {
    expect(hasContent(blank)).toBe(false);
    expect(hasContent({ ...blank, body: "  \n" })).toBe(false);
    expect(hasContent({ ...blank, body: "Hi" })).toBe(true);
    expect(hasContent({ ...blank, subject: "Lunch" })).toBe(true);
    expect(hasContent({ ...blank, to: [lena] })).toBe(true);
    expect(hasContent({ ...blank, bcc: [lena] })).toBe(true);
  });

  it("a reply comes with its people and subject, so only what you write counts", () => {
    const reply: Draft = { ...blank, mode: "reply", threadId: "q4", to: [lena], subject: "Re: Q4 budget" };
    expect(hasContent(reply)).toBe(false);
    expect(hasContent({ ...reply, body: "Yes" })).toBe(true);
  });

  it("a forward counts once it goes to someone", () => {
    const forward: Draft = { ...blank, mode: "forward", threadId: "q4", subject: "Fwd: Q4 budget" };
    expect(hasContent(forward)).toBe(false);
    expect(hasContent({ ...forward, cc: [lena] })).toBe(true);
  });
});

describe("saving drafts", () => {
  it("saves a draft once you pause typing, not before", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    s().updateDraft({ body: "Fri" });
    s().updateDraft({ body: "Friday?" });
    expect(backend.saveDraft).not.toHaveBeenCalled();
    expect(d().status[s().draft!.id]).toBeUndefined();
    await settle();
    expect(backend.saveDraft).toHaveBeenCalledTimes(1);
    expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ id: s().draft!.id, body: "Friday?", updatedAt: expect.any(Number) }));
    expect(d().status[s().draft!.id]).toBe("saved");
    expect(d().drafts.map((x) => x.body)).toEqual(["Friday?"]);
  });

  it("an empty composer saves nothing, open or closed", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    s().updateDraft({ fieldsOpen: false });
    s().back();
    await settle();
    expect(backend.saveDraft).not.toHaveBeenCalled();
    expect(backend.deleteDraft).not.toHaveBeenCalled();
    expect(d().drafts).toEqual([]);
  });

  it("closing the composer saves at once", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    s().back();
    await draftsSaved();
    expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ subject: "Lunch" }));
    await settle();
    expect(backend.saveDraft).toHaveBeenCalledTimes(1);
  });

  it("starting another email saves the one before it", async () => {
    s().compose("new");
    const first = s().draft!.id;
    s().updateDraft({ subject: "First" });
    s().compose("reply", "q4");
    s().updateDraft({ body: "Yes" });
    await draftsSaved();
    expect(d().drafts.map((x) => x.id)).toEqual([first]);
    await settle();
    expect(d().drafts.map((x) => x.subject)).toEqual(["Re: Q4 budget", "First"]);
    expect(s().draft!.id).not.toBe(first);
  });

  it("a draft emptied again is deleted", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    s().updateDraft({ body: "Hi" });
    await settle();
    const id = s().draft!.id;
    s().updateDraft({ body: "" });
    await draftsSaved();
    expect(backend.deleteDraft).toHaveBeenCalledWith(id, false);
    expect(d().drafts).toEqual([]);
    expect(d().status[id]).toBeUndefined();
  });

  it("says when a draft couldn’t be saved", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    fakeMail({ saveDraft: vi.fn(async () => Promise.reject(new Error("disk full"))) });
    s().compose("new");
    s().updateDraft({ body: "Hi" });
    await settle();
    expect(d().status[s().draft!.id]).toBe("failed");
    expect(warn).toHaveBeenCalled();
  });

  it("a failed save of a draft deleted meanwhile says nothing", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    let fail = (_: Error) => {};
    fakeMail({ saveDraft: vi.fn(() => new Promise<SavedDraft>((_, reject) => (fail = reject))) });
    s().compose("new");
    s().updateDraft({ body: "Hi" });
    await vi.advanceTimersByTimeAsync(saveDelay);
    const id = s().draft!.id;
    s().discardDraft();
    fail(new Error("disk full"));
    await draftsSaved();
    expect(d().status[id]).toBeUndefined();
  });

  it("a draft deleted while it was being saved stays deleted", async () => {
    let finish = () => {};
    const { backend } = fakeMail({ saveDraft: vi.fn((draft) => new Promise<SavedDraft>((resolve) => (finish = () => resolve(draft)))) });
    s().compose("new");
    s().updateDraft({ body: "Hi" });
    await vi.advanceTimersByTimeAsync(saveDelay);
    const id = s().draft!.id;
    s().discardDraft();
    finish();
    await draftsSaved();
    expect(backend.deleteDraft).toHaveBeenCalledWith(id, false);
    expect(d().drafts).toEqual([]);
    expect(d().status[id]).toBeUndefined();
  });

  it("stops listening when asked, and drops what was waiting", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    s().updateDraft({ body: "Hi" });
    stop();
    await settle();
    expect(backend.saveDraft).not.toHaveBeenCalled();
  });
});

describe("going back to a draft", () => {
  it("reopens a saved draft in the composer, without saving it again", async () => {
    const { backend } = fakeMail();
    useDrafts.setState({ drafts: [saved("a", { body: "Friday?" })], status: { a: "saved" } });
    s().openDraft("a");
    expect(s()).toMatchObject({ view: "compose", draft: { id: "a", body: "Friday?" } });
    await settle();
    expect(backend.saveDraft).not.toHaveBeenCalled();
    s().updateDraft({ body: "Friday at 1?" });
    await settle();
    expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ id: "a", body: "Friday at 1?" }));
    expect(d().drafts).toHaveLength(1);
  });

  it("a reply reopens on its thread", () => {
    useDrafts.setState({ drafts: [saved("a", { mode: "reply", threadId: "q4", body: "Yes" })] });
    s().openDraft("a");
    expect(s()).toMatchObject({ view: "compose", openThreadId: "q4" });
    s().back();
    expect(s().view).toBe("thread");
  });

  it("does nothing for a draft that isn’t there", () => {
    s().openDraft("nope");
    expect(s().view).toBe("inbox");
    expect(s().draft).toBeUndefined();
  });

  it("opens the Drafts list on its newest draft, and Esc goes back", () => {
    useDrafts.setState({ drafts: [saved("a"), saved("b")] });
    s().openDrafts();
    expect(s().view).toBe("drafts");
    expect(d().cursorId).toBe("a");
    s().leave();
    expect(s().view).toBe("inbox");
  });

  it("moves the cursor through the list, and stops at either end", () => {
    useDrafts.setState({ drafts: [saved("a"), saved("b")] });
    s().openDrafts();
    d().moveCursor(1);
    expect(d().cursorId).toBe("b");
    d().moveCursor(1);
    expect(d().cursorId).toBe("b");
    d().moveCursor(-1);
    d().moveCursor(-1);
    expect(d().cursorId).toBe("a");
    d().setCursor("b");
    expect(d().cursorId).toBe("b");
  });

  it("shows the drafts of the account you are looking at", () => {
    const list = [saved("a"), saved("b", { fromAccountId: "personal" })];
    expect(visibleDrafts(list, "all").map((x) => x.id)).toEqual(["a", "b"]);
    expect(visibleDrafts(list, "personal").map((x) => x.id)).toEqual(["b"]);
  });
});

describe("deleting and sending", () => {
  it("deletes a draft from the list, and the cursor moves to the next one", async () => {
    const { backend } = fakeMail();
    useDrafts.setState({ drafts: [saved("a"), saved("b"), saved("c")], status: { a: "saved", b: "saved", c: "saved" } });
    s().openDrafts();
    d().setCursor("b");
    s().deleteDraft();
    await draftsSaved();
    expect(backend.deleteDraft).toHaveBeenCalledWith("b", false);
    expect(d().drafts.map((x) => x.id)).toEqual(["a", "c"]);
    expect(d().cursorId).toBe("c");
    expect(s().toast?.text).toBe("Draft deleted");
    s().deleteDraft("c");
    expect(d().cursorId).toBe("a");
    s().deleteDraft("a");
    expect(d().cursorId).toBeUndefined();
    s().deleteDraft();
    await draftsSaved();
    expect(backend.deleteDraft).toHaveBeenCalledTimes(3);
  });

  it("Discard deletes the draft and closes the composer", async () => {
    const { backend } = fakeMail();
    s().compose("reply", "q4");
    s().updateDraft({ body: "Yes" });
    await settle();
    const id = s().draft!.id;
    s().discardDraft();
    await settle();
    expect(s()).toMatchObject({ view: "thread", draft: undefined });
    expect(backend.deleteDraft).toHaveBeenCalledWith(id, false);
    expect(backend.saveDraft).toHaveBeenCalledTimes(1);
    expect(d().drafts).toEqual([]);
  });

  it("Discard on an empty composer just closes it", () => {
    s().discardDraft();
    expect(s().view).toBe("inbox");
    s().compose("new");
    s().discardDraft();
    expect(s()).toMatchObject({ view: "inbox", draft: undefined });
  });

  it("sending a draft removes it", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    s().updateDraft({ to: [lena], subject: "Lunch", body: "Hi" });
    await settle();
    const id = s().draft!.id;
    s().updateDraft({ body: "Hi there" });
    s().send();
    await vi.waitFor(() => expect(s().draft).toBeUndefined());
    await settle();
    expect(backend.deleteDraft).not.toHaveBeenCalled();
    expect(backend.sendAfter).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ id }), expect.any(Number));
    expect(d().drafts).toEqual([]);
  });

  it("keeps the latest text when validation refuses Send before preparation", async () => {
    const { backend } = fakeMail({ sendAfter: vi.fn(async () => { throw new Error("Add a recipient"); }) });
    s().compose("new");
    s().updateDraft({ subject: "Unsent", body: "Last unsaved edit" });
    s().send();
    await vi.advanceTimersByTimeAsync(0);
    await draftsSaved();
    expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ body: "Last unsaved edit" }));
    expect(s()).toMatchObject({ view: "compose", sendError: "Add a recipient" });
  });

  it("stops a nearly due autosave before starting the send handoff", async () => {
    const { backend } = fakeMail({ sendAfter: vi.fn(() => new Promise<boolean>(() => {})) });
    s().compose("new");
    s().updateDraft({ to: [lena], subject: "Lunch", body: "Newest text" });
    await vi.advanceTimersByTimeAsync(saveDelay - 100);
    vi.mocked(backend.saveDraft).mockClear();
    s().send();
    await vi.advanceTimersByTimeAsync(150);
    expect(backend.saveDraft).not.toHaveBeenCalled();
    expect(backend.sendAfter).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ body: "Newest text" }), expect.any(Number));
  });

  it("a held send leaves the drafts list, and Undo makes it a draft again", async () => {
    // Still held when Undo comes.
    const { backend } = fakeMail({ sendAfter: vi.fn(() => new Promise<boolean>(() => {})) });
    s().compose("new");
    s().updateDraft({ to: [lena], subject: "Lunch", body: "Hi" });
    await settle();
    const id = s().draft!.id;
    s().send();
    await vi.advanceTimersByTimeAsync(sendBeatMs);
    await draftsSaved();
    expect(s().draft).toBeUndefined();
    expect(backend.deleteDraft).not.toHaveBeenCalled();
    expect(backend.sendAfter).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ id }), expect.any(Number));
    expect(d().drafts).toEqual([]);
    s().undo();
  await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: expect.objectContaining({ id, body: "Hi" }) });
    await settle();
    expect(d().drafts.map((x) => x.id)).toEqual([id]);
  });

  it("a send that fails keeps the draft", async () => {
    const { backend } = fakeMail({ sendAfter: vi.fn(async () => Promise.reject(new Error("offline"))) });
    s().compose("new");
    s().updateDraft({ to: [lena], subject: "Lunch", body: "Hi" });
    await settle();
    s().send();
    await vi.waitFor(() => expect(s().sendError).toBe("offline"));
    expect(backend.deleteDraft).not.toHaveBeenCalled();
    expect(d().drafts).toHaveLength(1);
  });
});

describe("drafts kept on this Mac", () => {
  it("loads them, all saved", async () => {
    fakeMail({ drafts: vi.fn(async () => [saved("a"), saved("b")]) });
    await loadDrafts();
    expect(d().drafts.map((x) => x.id)).toEqual(["a", "b"]);
    expect(d().status).toEqual({ a: "saved", b: "saved" });
  });
});

describe("drafts from your phone and webmail", () => {
  it("come into the list; ones sent or deleted elsewhere leave it", async () => {
    useDrafts.setState({ drafts: [saved("a"), saved("gone")], status: { a: "saved", gone: "saved" }, cursorId: "a" });
    fakeMail({ drafts: vi.fn(async () => [saved("phone", { updatedAt: 3 }), saved("a")]) });
    await reloadDrafts();
    expect(d().drafts.map((x) => x.id)).toEqual(["phone", "a"]);
    expect(d().status).toEqual({ phone: "saved", a: "saved" });
    expect(d().cursorId).toBe("a");
  });

  it("preserves explicitly unsaved edits without trusting wall-clock order", async () => {
    const a = saved("a", { body: "newer", updatedAt: 5 });
    const typing = saved("typing", { updatedAt: 6 });
    useDrafts.setState({
      drafts: [a, typing, saved("b", { body: "mine", updatedAt: 1 })],
      unsaved: { a, typing },
      status: { a: "failed", b: "saved" },
    });
    fakeMail({ drafts: vi.fn(async () => [saved("b", { body: "from phone", updatedAt: 4 }), saved("a", { body: "older", updatedAt: 2 })]) });
    await reloadDrafts();
    expect(d().drafts.map((x) => [x.id, x.body])).toEqual([["typing", ""], ["a", "newer"], ["b", "from phone"]]);
    expect(d().status).toEqual({ a: "failed", b: "saved" });
  });

  it("a cursor on a draft that left moves to the top", async () => {
    useDrafts.setState({ drafts: [saved("gone")], status: { gone: "saved" }, cursorId: "gone" });
    fakeMail({ drafts: vi.fn(async () => [saved("phone")]) });
    await reloadDrafts();
    expect(d().cursorId).toBe("phone");
    useDrafts.setState({ drafts: [saved("gone")], status: { gone: "saved" }, cursorId: undefined });
    await reloadDrafts();
    expect(d().cursorId).toBeUndefined();
  });

  it("opening Drafts asks every account for its drafts now", () => {
    const { backend } = fakeMail();
    s().openDrafts();
    expect(backend.syncNow).toHaveBeenCalledWith();
  });
});

it("chains queued edits through the last acknowledged revision without replacing newer text", async () => {
  const first = Object.assign(saved("revision-chain", { body: "First edit" }), { revision: "base" });
  let acknowledge!: (draft: SavedDraft) => void;
  const save = vi.fn<(draft: SavedDraft) => Promise<SavedDraft>>()
    .mockImplementationOnce(() => new Promise((resolve) => { acknowledge = resolve; }))
    .mockImplementation(async (draft) => Object.assign({}, draft, { revision: "second-ack" }));
  fakeMail({ saveDraft: save });
  useStore.setState({ draft: first, view: "compose" });
  keepDraft(first);
  await vi.advanceTimersByTimeAsync(0);
  s().updateDraft({ body: "Second edit" });
  keepDraft(s().draft!);
  acknowledge(Object.assign({}, first, { revision: "first-ack" }));
  await draftsSaved();
  expect(save).toHaveBeenNthCalledWith(2, expect.objectContaining({ body: "Second edit", revision: "first-ack" }));
  expect(d().drafts[0]).toMatchObject({ body: "Second edit", revision: "second-ack" });
});

it("does not give a separately loaded draft the revision of an older local acknowledgement", async () => {
  const first = saved("remote-revision", { body: "Local edit", revision: "base" });
  let acknowledge!: (draft: SavedDraft) => void;
  fakeMail({ saveDraft: vi.fn(() => new Promise<SavedDraft>((resolve) => { acknowledge = resolve; })) });
  keepDraft(first);
  await vi.advanceTimersByTimeAsync(0);
  const remote = saved(first.id, { body: "Remote edit", revision: "remote" });
  useDrafts.setState({ drafts: [remote] });
  acknowledge({ ...first, revision: "local-ack" });
  await draftsSaved();
  expect(d().drafts).toEqual([remote]);
});

it("never advances a foreign edit through an unrelated local acknowledgement", async () => {
  const local = saved("foreign-revision", { body: "Local", revision: "base" });
  const save = vi.fn(async (draft: SavedDraft) => ({ ...draft, revision: "ack" }));
  fakeMail({ saveDraft: save });
  keepDraft(local);
  await draftsSaved();
  keepDraft({ ...local, body: "Separately opened", revision: "base" });
  await draftsSaved();
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ revision: "base" }));
});

it("keeps acknowledging edits after reopening a previously saved draft", async () => {
  const initial = saved("reopened-chain", { body: "Initial", revision: "original" });
  let revision = 0;
  const save = vi.fn(async (draft: SavedDraft) => ({ ...draft, revision: `ack-${++revision}` }));
  fakeMail({ saveDraft: save });
  keepDraft(initial);
  await draftsSaved();
  s().openDraft(initial.id);
  s().updateDraft({ body: "Reopened edit" });
  keepDraft(s().draft!);
  await draftsSaved();
  s().updateDraft({ body: "Still typing" });
  keepDraft(s().draft!);
  await draftsSaved();
  expect(save).toHaveBeenNthCalledWith(3, expect.objectContaining({ revision: "ack-2", body: "Still typing" }));
});

it.each(["handoff", "delete"] as const)("ignores a pre-%s reload snapshot", async (action) => {
  const row = saved("retired", { revision: "old" });
  let resolve!: (drafts: SavedDraft[]) => void;
  fakeMail({ drafts: vi.fn(() => new Promise<SavedDraft[]>((done) => { resolve = done; })) });
  useDrafts.setState({ drafts: [row], status: { retired: "saved" } });
  const loading = reloadDrafts();
  await vi.advanceTimersByTimeAsync(0);
  if (action === "handoff") handoffDraft(row.id);
  else forgetDraft(row.id);
  resolve([row]);
  await loading;
  await draftsSaved();
  expect(d().drafts).toEqual([]);
});

it("keeps unsaved local content across a reload with equal timestamps", async () => {
  const older = saved("dirty-reload", { body: "Previously saved", revision: "base", updatedAt: Date.now() });
  let fail!: (error: Error) => void;
  fakeMail({
    saveDraft: vi.fn(() => new Promise<SavedDraft>((_, reject) => { fail = reject; })),
    drafts: vi.fn(async () => [older]),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  keepDraft({ ...older, body: "Unsaved local work" });
  await vi.advanceTimersByTimeAsync(0);
  const loading = reloadDrafts();
  fail(new Error("disk full"));
  await loading;
  await draftsSaved();
  expect(d().drafts[0]).toMatchObject({ body: "Unsaved local work", revision: "base" });
  expect(d().status[older.id]).toBe("failed");
});

it("updates a clean open composer when its saved draft changes remotely", async () => {
  const before = saved("clean-reload", { body: "Before", revision: "before" });
  const after = saved(before.id, { body: "Remote edit", revision: "after" });
  const { backend } = fakeMail({ drafts: vi.fn(async () => [after]) });
  useDrafts.setState({ drafts: [before], status: { [before.id]: "saved" } });
  s().openDraft(before.id);
  await reloadDrafts();
  expect(s().draft).toBe(after);
  expect(backend.saveDraft).not.toHaveBeenCalled();
});

it("adopts its own save revision in the composer without replacing newer typing or scheduling another save", async () => {
  let acknowledge!: (draft: SavedDraft) => void;
  const save = vi.fn((draft: SavedDraft) => new Promise<SavedDraft>((resolve) => { acknowledge = () => resolve({ ...draft, revision: "composer-ack" }); }));
  fakeMail({ saveDraft: save });
  s().compose("new");
  s().updateDraft({ body: "First text" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  s().updateDraft({ body: "Newer typing" });
  acknowledge(save.mock.calls[0][0]);
  await draftsSaved();
  expect(s().draft).toMatchObject({ body: "Newer typing", revision: "composer-ack" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ body: "Newer typing", revision: "composer-ack" }));
  acknowledge(save.mock.calls[1][0]);
  await draftsSaved();
  await vi.advanceTimersByTimeAsync(saveDelay);
  expect(save).toHaveBeenCalledTimes(2);
});

it("does not let initial loading erase a draft composed while the read was pending", async () => {
  let resolve!: (drafts: SavedDraft[]) => void;
  fakeMail({ drafts: vi.fn(() => new Promise<SavedDraft[]>((done) => { resolve = done; })) });
  const loading = loadDrafts();
  await vi.advanceTimersByTimeAsync(0);
  keepDraft(saved("during-startup", { body: "Keep this" }));
  resolve([]);
  await loading;
  await draftsSaved();
  expect(d().drafts).toEqual([expect.objectContaining({ id: "during-startup", body: "Keep this" })]);
});

it("advances the composer through two saves queued from the same original revision", async () => {
  const original = saved("composer-queue", { body: "Original", revision: "base" });
  const acknowledgements: Array<() => void> = [];
  const save = vi.fn((draft: SavedDraft) => new Promise<SavedDraft>((resolve) => {
    const revision = `ack-${save.mock.calls.length}`;
    acknowledgements.push(() => resolve({ ...draft, revision }));
  }));
  fakeMail({ saveDraft: save });
  useDrafts.setState({ drafts: [original] });
  s().openDraft(original.id);
  s().updateDraft({ body: "First edit" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  s().updateDraft({ body: "Queued" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  acknowledgements[0]();
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft?.revision).toBe("ack-1");
  const frozen = s().draft!;
  acknowledgements[1]();
  await draftsSaved();
  expect(s().draft?.revision).toBe("ack-2");
  expect(acknowledgedDraftRevision(frozen)).toBe("ack-2");
});

it("never promotes a separately opened composer through a foreign save acknowledgement", async () => {
  const remote = saved("foreign-composer", { body: "Remote text", revision: "remote" });
  const { backend } = fakeMail({ saveDraft: vi.fn(async (draft) => ({ ...draft, revision: "foreign-ack" })) });
  useDrafts.setState({ drafts: [remote] });
  s().openDraft(remote.id);
  keepDraft({ ...remote, body: "Foreign text" });
  await draftsSaved();
  expect(s().draft).toBe(remote);
  s().updateDraft({ body: "My separate edit" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  await draftsSaved();
  expect(backend.saveDraft).toHaveBeenLastCalledWith(expect.objectContaining({ body: "My separate edit", revision: "remote" }));
});

it("discards with the revision acknowledged by a preceding queued save", async () => {
  const original = saved("discard-ack", { body: "Queued save", revision: "base" });
  let acknowledge!: () => void;
  const remove = vi.fn(async (..._args: unknown[]) => {});
  fakeMail({
    saveDraft: vi.fn((draft) => new Promise<SavedDraft>((resolve) => { acknowledge = () => resolve({ ...draft, revision: "last-ack" }); })),
    deleteDraft: remove,
  });
  keepDraft(original);
  await vi.advanceTimersByTimeAsync(0);
  forgetDraft(original.id);
  acknowledge();
  await draftsSaved();
  expect(remove).toHaveBeenCalledExactlyOnceWith(original.id, false, "last-ack");
});

it("Undo adopts the backend-restored draft without re-saving the retired snapshot", async () => {
  const cancel = vi.fn();
  const { backend } = fakeMail({
    saveDraft: vi.fn(async (draft) => ({ ...draft, revision: "before-send" })),
    sendAfter: vi.fn(() => new Promise<boolean>(() => {})),
    cancelSend: cancel,
  });
  s().compose("new");
  s().updateDraft({ to: [lena], subject: "Lunch", body: "Before send" });
  await settle();
  const id = s().draft!.id;
  s().send();
  await vi.advanceTimersByTimeAsync(sendBeatMs);
  await draftsSaved();
  const restored = saved(id, { body: "Backend restored current text", revision: "restored-revision" });
  cancel.mockResolvedValue({ cancelled: true, early: false, restored });
  const writes = vi.mocked(backend.saveDraft).mock.calls.length;
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  await draftsSaved();
  expect(s().draft).toBe(restored);
  expect(d().drafts[0]).toBe(restored);
  expect(d().acknowledgements[id]).toBeUndefined();
  expect(backend.saveDraft).toHaveBeenCalledTimes(writes);
});

it("does not recreate a prepared draft that disappeared after cancellation", async () => {
  const { backend } = fakeMail({
    sendAfter: vi.fn(() => new Promise<boolean>(() => {})),
    cancelSend: vi.fn(async () => ({ cancelled: true, early: false })),
  });
  s().compose("new");
  s().updateDraft({ to: [lena], subject: "Lunch", body: "Old snapshot" });
  await settle();
  s().send();
  await vi.advanceTimersByTimeAsync(sendBeatMs);
  await draftsSaved();
  const writes = vi.mocked(backend.saveDraft).mock.calls.length;
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft).toBeUndefined();
  expect(d().drafts).toEqual([]);
  expect(s().toast?.text).toBe("This draft was already removed or handed off elsewhere.");
  expect(backend.saveDraft).toHaveBeenCalledTimes(writes);
});


it.each([false, true])("preserves a restored row adopted during a reload (old row: %s)", async (oldRow) => {
  const before = saved("restore-race", { revision: "before" });
  const restored = saved(before.id, { revision: "restored", body: "Restored content" });
  let resolve!: (rows: SavedDraft[]) => void;
  fakeMail({ drafts: vi.fn(() => new Promise<SavedDraft[]>((done) => { resolve = done; })) });
  const loading = reloadDrafts();
  await vi.advanceTimersByTimeAsync(0);
  adoptDraft(restored);
  s().openDraft(restored.id);
  resolve(oldRow ? [before] : []);
  await loading;
  expect(d().drafts).toEqual([restored]);
  expect(s().draft).toBe(restored);
});

it("does not replace composer presentation or unsaved text with the same revision", async () => {
  const row = saved("same-revision", { revision: "same", body: "Saved" });
  fakeMail({ drafts: vi.fn(async () => [{ ...row }]) });
  useDrafts.setState({ drafts: [row] });
  s().openDraft(row.id);
  s().updateDraft({ showCc: true });
  await settle();
  const composer = s().draft;
  await reloadDrafts();
  expect(s().draft).toBe(composer);
  expect(s().draft?.showCc).toBe(true);
});

it("uses a fresh identity after emptying a saved draft", async () => {
  const { backend } = fakeMail({ saveDraft: vi.fn(async (draft) => ({ ...draft, revision: "saved" })) });
  s().compose("new");
  s().updateDraft({ body: "First" });
  await settle();
  const oldId = s().draft!.id;
  s().updateDraft({ body: "", bodyHtml: "" });
  await draftsSaved();
  s().updateDraft({ body: "Again" });
  await settle();
  expect(s().draft!.id).not.toBe(oldId);
  expect(backend.saveDraft).toHaveBeenLastCalledWith(expect.objectContaining({ body: "Again", revision: undefined }));
});

it("preserves queued edit ancestry when the pending draft is reopened", async () => {
  const original = saved("reopen-pending", { revision: "base", body: "Original" });
  const acknowledgements: Array<() => void> = [];
  const save = vi.fn((draft: SavedDraft) => new Promise<SavedDraft>((resolve) => {
    const revision = `ack-${save.mock.calls.length}`;
    acknowledgements.push(() => resolve({ ...draft, revision }));
  }));
  fakeMail({ saveDraft: save });
  useDrafts.setState({ drafts: [original] });
  s().openDraft(original.id);
  s().updateDraft({ body: "First" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  s().updateDraft({ body: "Second" });
  await vi.advanceTimersByTimeAsync(saveDelay);
  s().back();
  s().openDraft(original.id);
  acknowledgements[0]();
  await vi.advanceTimersByTimeAsync(0);
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ body: "Second", revision: "ack-1" }));
  acknowledgements[1]();
  await draftsSaved();
  expect(s().draft?.revision).toBe("ack-2");
});

it("early Undo retains its save lineage and does not save the restored snapshot twice", async () => {
  let current: string | undefined;
  const save = vi.fn(async (draft: SavedDraft) => {
    if (draft.revision !== current) throw new Error("stale revision");
    current = `ack-${save.mock.calls.length}`;
    return { ...draft, revision: current };
  });
  fakeMail({ saveDraft: save, sendAfter: vi.fn(() => new Promise<boolean>(() => {})), cancelSend: vi.fn(async () => ({ cancelled: true, early: true })) });
  s().compose("new");
  s().updateDraft({ to: [lena], subject: "Lunch", body: "Before" });
  await settle();
  s().updateDraft({ body: "Latest" });
  s().send();
  await vi.advanceTimersByTimeAsync(sendBeatMs);
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  await settle();
  expect(save).toHaveBeenCalledTimes(2);
  expect(s().draft).toMatchObject({ body: "Latest", revision: current });
  expect(d().status[s().draft!.id]).toBe("saved");
});

it("early scheduled Undo preserves its revision lineage with one restoration save", async () => {
  let current: string | undefined;
  const save = vi.fn(async (draft: SavedDraft) => {
    if (draft.revision !== current) throw new Error("stale revision");
    current = `ack-${save.mock.calls.length}`;
    return { ...draft, revision: current };
  });
  const { openSendLater, pickTime } = await import("./scheduled");
  fakeMail({ saveDraft: save, scheduleSend: vi.fn(() => new Promise<never>(() => {})), unscheduleSend: vi.fn(async () => ({ status: "cancelled" as const, early: true })) });
  s().compose("new");
  s().updateDraft({ to: [lena], subject: "Lunch", body: "Latest" });
  await settle();
  openSendLater();
  pickTime(Date.now() + 60000);
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  await settle();
  expect(save).toHaveBeenCalledTimes(2);
  expect(s().draft).toMatchObject({ body: "Latest", revision: current });
  expect(d().status[s().draft!.id]).toBe("saved");
});

it("cancels an optimistic Scheduled row through its original pending save ancestry", async () => {
  let revision: string | undefined;
  let release!: () => void;
  const save = vi.fn(async (draft: SavedDraft) => {
    if (save.mock.calls.length === 2) await new Promise<void>((resolve) => { release = resolve; });
    if (draft.revision !== revision) throw new Error("stale revision");
    revision = `ack-${save.mock.calls.length}`;
    return { ...draft, revision };
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const { openSendLater, pickTime, editScheduled } = await import("./scheduled");
  fakeMail({ saveDraft: save, scheduleSend: vi.fn(() => new Promise<never>(() => {})), unscheduleSend: vi.fn(async () => ({ status: "cancelled" as const, early: true })) });
  s().compose("new");
  s().updateDraft({ to: [lena], subject: "Lunch", body: "Before" });
  await settle();
  const id = s().draft!.id;
  s().updateDraft({ body: "Latest" });
  openSendLater();
  pickTime(Date.now() + 60000);
  await vi.advanceTimersByTimeAsync(0);
  await editScheduled(id);
  release();
  await draftsSaved();
  await settle();
  expect(save).toHaveBeenCalledTimes(3);
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ revision: "ack-2", body: "Latest" }));
  expect(s().draft).toMatchObject({ body: "Latest", revision: "ack-3" });
  expect(d().status[id]).toBe("saved");
});


it("rechecks an unconfirmed save before writing newer content in the same editing chain", async () => {
  const attempt = saved("unconfirmed-save", { body: "First edit", revision: "base" });
  let writes = 0;
  const { backend } = fakeMail({
    saveDraft: vi.fn(async (draft) => {
      if (++writes === 1) throw new Error("save and receipt replies both lost");
      expect(draft.revision).toBe("committed-first");
      return { ...draft, revision: "committed-latest" };
    }),
    draftSaveReceipt: vi.fn(async (draft) => {
      expect(draft.body).toBe("First edit");
      return { ...draft, revision: "committed-first" };
    }),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  useStore.setState({ draft: attempt });
  keepDraft(attempt);
  s().updateDraft({ body: "Latest edit" });
  keepDraft(s().draft!);
  await draftsSaved();
  expect(backend.draftSaveReceipt).toHaveBeenCalledTimes(1);
  expect(d().status[attempt.id]).toBe("saved");
  expect(s().draft).toMatchObject({ body: "Latest edit", revision: "committed-latest" });
});

it("keeps the first unconfirmed snapshot and blocks newer writes while its receipt is unreadable", async () => {
  const attempt = saved("receipt-unreadable", { body: "First", revision: "base" });
  const { backend } = fakeMail({
    saveDraft: vi.fn(async () => { throw new Error("lost reply"); }),
    draftSaveReceipt: vi.fn(async () => { throw new Error("read unavailable"); }),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  useStore.setState({ draft: attempt });
  keepDraft(attempt);
  s().updateDraft({ body: "Second" }); keepDraft(s().draft!);
  s().updateDraft({ body: "Third" }); keepDraft(s().draft!);
  await draftsSaved();
  expect(backend.saveDraft).toHaveBeenCalledTimes(1);
  expect(backend.draftSaveReceipt).toHaveBeenCalledTimes(2);
  expect(backend.draftSaveReceipt).toHaveBeenLastCalledWith(expect.objectContaining({ body: "First", revision: "base" }));
  expect([...d().unconfirmed.values()][0].saved.body).toBe("First");
  expect(d().unsaved[attempt.id].body).toBe("Third");
  expect(d().status[attempt.id]).toBe("failed");
});

it("uses the original precondition when read-back proves the earlier content is absent", async () => {
  const attempt = saved("receipt-absent", { body: "First", revision: "base" });
  let writes = 0;
  const { backend } = fakeMail({
    saveDraft: vi.fn(async (draft) => {
      if (++writes === 1) throw new Error("write failed");
      expect(draft.revision).toBe("base");
      return { ...draft, revision: "latest" };
    }),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  useStore.setState({ draft: attempt }); keepDraft(attempt);
  s().updateDraft({ body: "Latest" }); keepDraft(s().draft!);
  await draftsSaved();
  expect(backend.draftSaveReceipt).toHaveBeenCalledTimes(1);
  expect(d().unconfirmed.size).toBe(0);
  expect(d().status[attempt.id]).toBe("saved");
});

it("does not borrow an unconfirmed receipt from a separate editing chain", async () => {
  const attempt = saved("separate-receipt", { body: "First", revision: "base" });
  let writes = 0;
  const { backend } = fakeMail({ saveDraft: vi.fn(async (draft) => {
    if (++writes === 1) throw new Error("lost reply");
    expect(draft.revision).toBe(writes === 2 ? "separate" : "base");
    return { ...draft, revision: "latest" };
  }) });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  keepDraft(attempt);
  keepDraft({ ...attempt, body: "Separate", revision: "separate" });
  await draftsSaved();
  expect(backend.draftSaveReceipt).not.toHaveBeenCalled();
  keepDraft(attempt); await draftsSaved();
  expect(backend.draftSaveReceipt).toHaveBeenCalledTimes(1);
  expect(d().status[attempt.id]).toBe("saved");
});

it("settles a doubly lost save reply before discarding the saved draft", async () => {
  const attempt = saved("discard-unconfirmed", { body: "Saved text", revision: "base" });
  const { backend } = fakeMail({
    saveDraft: vi.fn(async () => { throw new Error("lost save and receipt replies"); }),
    draftSaveReceipt: vi.fn(async (draft) => ({ ...draft, revision: "committed" })),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  keepDraft(attempt); await draftsSaved();
  forgetDraft(attempt.id); await draftsSaved();
  expect(backend.deleteDraft).toHaveBeenCalledWith(attempt.id, false, "committed");
  expect(d().drafts).toEqual([]);
});

it("keeps a refused discard visible instead of hiding a row that remains on disk", async () => {
  const row = saved("refused-discard", { revision: "base" });
  const { backend } = fakeMail({ deleteDraft: vi.fn(async () => { throw new Error("delete refused"); }) });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  useDrafts.setState({ drafts: [row] });
  forgetDraft(row.id); await draftsSaved();
  expect(backend.deleteDraft).toHaveBeenCalledWith(row.id, false, "base");
  expect(d().drafts).toEqual([row]);
  expect(d().retired[row.id]).toBeUndefined();
  expect(d().status[row.id]).toBe("failed");
});

it("a refused discard does not replace a newer local edit already in the list", async () => {
  const row = saved("discard-edit-race", { body: "Before", revision: "base" });
  let reject!: (error: Error) => void;
  fakeMail({ deleteDraft: vi.fn(() => new Promise<void>((_, fail) => { reject = fail; })) });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  useDrafts.setState({ drafts: [row] });
  forgetDraft(row.id);
  await vi.advanceTimersByTimeAsync(0);
  keepDraft({ ...row, body: "Newer local text" });
  reject(new Error("delete refused")); await draftsSaved();
  expect(d().drafts).toHaveLength(1);
  expect(d().drafts[0].body).toBe("Newer local text");
  expect(d().retired[row.id]).toBeUndefined();
});

it("authoritative adoption clears only that draft's unconfirmed editing chains", async () => {
  fakeMail({ saveDraft: vi.fn(async () => { throw new Error("lost reply"); }) });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  keepDraft(saved("first", { body: "First chain" }));
  keepDraft(saved("first", { body: "Separate chain" }));
  keepDraft(saved("second", { body: "Other draft" }));
  await draftsSaved();
  expect(d().unconfirmed.size).toBe(3);
  adoptDraft(saved("first", { body: "Authoritative", revision: "restored" }));
  expect([...d().unconfirmed.values()].map((entry) => entry.saved.id)).toEqual(["second"]);
  expect(d().drafts[0]).toMatchObject({ body: "Authoritative", revision: "restored" });
});
