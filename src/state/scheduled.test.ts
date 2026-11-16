import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ScheduleOutcome } from "../services/mail";
import { fakeMail } from "../test/fakeMail";
import { lena, resetStore, thread } from "../test/fixtures";
import { draftsSaved, useDrafts, type SavedDraft } from "./drafts";
import {
  cancelScheduled,
  closePicker,
  editScheduled,
  loadScheduled,
  openScheduled,
  openSendLater,
  pickTime,
  scheduleDraft,
  scheduledEvent,
  sendNow,
  useScheduled,
  visibleScheduled,
  type ScheduledEmail,
} from "./scheduled";
import { useStore, type Draft } from "./store";

const s = () => useStore.getState();
const q = () => useScheduled.getState();

// Tuesday 3 November 2026, 1:20 PM.
const now = new Date(2026, 10, 3, 13, 20).getTime();
const saturday8 = new Date(2026, 10, 7, 8, 0).getTime();
const tomorrow8 = new Date(2026, 10, 4, 8, 0).getTime();

const draft: Draft = {
  id: "d1",
  mode: "reply",
  threadId: "a",
  fromAccountId: "work",
  to: [lena],
  cc: [],
  bcc: [],
  subject: "Re: Subject a",
  body: "Yes.",
  bodyHtml: "<p>Yes.</p>",
  fieldsOpen: true,
  showCc: false,
  showBcc: false,
  remindIfNoReply: false,
  signatureId: null,
};

const email = (id: string, sendAt: number, extra: Partial<ScheduledEmail> = {}): ScheduledEmail => ({
  revision: 1,
  operationId: `op-${id}`,
  draft: { ...draft, id, subject: `Email ${id}`, updatedAt: 1 },
  sendAt,
  nextTry: sendAt,
  tries: 0,
  ...extra,
});

/** Writing a reply to thread a, in the composer. */
function composing(d: Draft = draft) {
  useStore.setState({ view: "compose", draft: d, openThreadId: "a" });
}

beforeEach(() => {
  vi.useFakeTimers({ now });
  resetStore([thread("a"), thread("b")]);
  useDrafts.setState({ drafts: [], status: {} });
  useScheduled.setState({ emails: [], latestOperation: {}, finishedOperations: {}, restoredOperations: {}, pendingOperations: {}, cursorId: undefined, picking: undefined, editing: undefined });
});
afterEach(() => vi.useRealTimers());

describe("Send later from the composer", () => {
  it("asks when, for an email with someone to send it to", () => {
    fakeMail();
    composing();
    openSendLater();
    expect(q().picking).toEqual({ kind: "compose" });
    closePicker();
    expect(q().picking).toBeUndefined();
  });

  it("says who is missing instead of asking, and does nothing outside the composer or while sending", () => {
    fakeMail();
    composing({ ...draft, to: [] });
    openSendLater();
    expect(q().picking).toBeUndefined();
    expect(s().sendError).toBe("add someone to send it to");
    useStore.setState({ draft: undefined });
    openSendLater();
    composing();
    useStore.setState({ sending: true });
    openSendLater();
    expect(q().picking).toBeUndefined();
    // Files too big to send: the composer already says so, so no time is asked for.
    useStore.setState({ sending: false });
    composing({ ...draft, attachments: [{ id: "1", name: "film.mov", bytes: 26 * 1024 * 1024, contentType: "video/quicktime" }] });
    openSendLater();
    expect(q().picking).toBeUndefined();
  });

  it("queues the email at the time picked, out of Drafts, back to the conversation, never Done", async () => {
    const { backend } = fakeMail();
    useDrafts.setState({ drafts: [{ ...draft, updatedAt: 1 }] });
    composing();
    openSendLater();
    pickTime(saturday8);
    expect(q().picking).toBeUndefined();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", draft: undefined, toast: { text: "Scheduled for Sat 8 AM" } });
    expect(s().threads.map((t) => t.id), "sending never marks mail Done").toEqual(["a", "b"]);
    expect(useDrafts.getState().drafts).toEqual([]);
    expect(q().emails).toMatchObject([{ draft: { id: "d1" }, sendAt: saturday8, tries: 0 }]);
    await draftsSaved();
    await vi.advanceTimersByTimeAsync(0);
    // The backend consumes the saved row and its removed files atomically.
    expect(backend.deleteDraft).not.toHaveBeenCalled();
    expect(backend.scheduleSend).toHaveBeenCalledWith(draft, saturday8, expect.any(String));
  });

  it("a new email goes back to the list", () => {
    fakeMail();
    composing({ ...draft, mode: "new", threadId: undefined });
    openSendLater();
    pickTime(tomorrow8);
    expect(s()).toMatchObject({ view: "inbox", toast: { text: "Scheduled for Wed 8 AM" } });
  });

  it("keeps the list soonest first", () => {
    fakeMail();
    useScheduled.setState({ emails: [email("x", tomorrow8 + 1)] });
    composing();
    openSendLater();
    pickTime(tomorrow8);
    expect(q().emails.map((e) => e.draft.id)).toEqual(["d1", "x"]);
  });

  it("⌘Z takes it back into the composer, as it was", async () => {
    const { backend } = fakeMail();
    composing();
    openSendLater();
    pickTime(saturday8);
    s().undo();
    expect(backend.unscheduleSend).toHaveBeenCalledWith("d1", expect.any(String));
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: { id: "d1", body: "Yes." }, toast: undefined });
    expect(q().emails).toEqual([]);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toBeUndefined();
  });

  it("a failed Undo confirmation reports uncertainty and leaves no draft behind", async () => {
    fakeMail({ unscheduleSend: vi.fn(() => Promise.reject(new Error("It’s sending now."))) });
    composing();
    openSendLater();
    pickTime(saturday8);
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Couldn’t confirm cancellation. The email may still send.", problem: true });
  });

  it("⌘Z does nothing while you write another email", () => {
    const { backend } = fakeMail();
    composing();
    openSendLater();
    pickTime(saturday8);
    composing({ ...draft, id: "d2" });
    s().undo();
    expect(backend.unscheduleSend).not.toHaveBeenCalled();
    expect(s().draft?.id).toBe("d2");
  });

  it("one the engine refuses goes back to Drafts, to open with why", async () => {
    fakeMail({ scheduleSend: vi.fn(() => Promise.reject(new Error("me@x.co isn't one of Work's addresses"))) });
    composing();
    openSendLater();
    pickTime(saturday8);
    await vi.advanceTimersByTimeAsync(0);
    expect(q().emails).toEqual([]);
    expect(useDrafts.getState().drafts.map((d) => d.id)).toEqual(["d1"]);
    expect(s().toast).toMatchObject({ text: "Not scheduled: me@x.co isn't one of Work's addresses", problem: true, undoLabel: "Open", stays: true });
    s().undo();
    expect(s()).toMatchObject({ view: "compose", draft: { id: "d1" }, sendError: "me@x.co isn't one of Work's addresses" });
  });

  it("does nothing when the composer closed meanwhile", () => {
    const { backend } = fakeMail();
    useScheduled.setState({ picking: { kind: "compose" } });
    pickTime(saturday8);
    pickTime(saturday8);
    expect(backend.scheduleSend).not.toHaveBeenCalled();
  });
});

describe("Send later from Ask Moss", () => {
  it("queues the email Moss wrote at the time you asked for, wherever you are", async () => {
    const { backend } = fakeMail();
    const saved = { ...draft, updatedAt: 1 };
    useDrafts.setState({ drafts: [saved] });
    useStore.setState({ view: "inbox" });
    scheduleDraft(saved, saturday8);
    expect(s()).toMatchObject({ view: "inbox", toast: { text: "Scheduled for Sat 8 AM" } });
    expect(useDrafts.getState().drafts).toEqual([]);
    expect(q().emails).toMatchObject([{ draft: { id: "d1" }, sendAt: saturday8, tries: 0 }]);
    await draftsSaved();
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.scheduleSend).toHaveBeenCalledWith(saved, saturday8, expect.any(String));
    // ⌘Z opens it once the backend confirms it cannot send.
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: { id: "d1" } });
  });
});

describe("the Scheduled list", () => {
  it("opens from anywhere, on the soonest email, and loads real mail", async () => {
    const { backend } = fakeMail({ scheduled: vi.fn(async () => [email("x", tomorrow8)]) });
    useStore.setState({ view: "inbox" });
    openScheduled();
    expect(s()).toMatchObject({ view: "scheduled", history: ["inbox"] });
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.scheduled).toHaveBeenCalled();
    expect(q()).toMatchObject({ emails: [{ draft: { id: "x" } }], cursorId: "x" });
  });

  it("shows one account's emails when you look at one account", () => {
    const a = email("a", 1);
    const b = { ...email("b", 2), draft: { ...email("b", 2).draft, fromAccountId: "personal" } };
    expect(visibleScheduled([a, b], "all")).toEqual([a, b]);
    expect(visibleScheduled([a, b], "personal")).toEqual([b]);
  });

  it("moves the cursor through what is on screen", () => {
    useScheduled.setState({ emails: [email("a", 1), email("b", 2), email("c", 3)], cursorId: "a" });
    q().moveCursor(1);
    expect(q().cursorId).toBe("b");
    q().moveCursor(1, [email("a", 1), email("b", 2)]);
    expect(q().cursorId).toBe("b");
    q().moveCursor(-1);
    q().moveCursor(-1);
    expect(q().cursorId).toBe("a");
    q().setCursor("c");
    expect(q().cursorId).toBe("c");
    useScheduled.setState({ cursorId: "gone" });
    q().moveCursor(1);
    expect(q().cursorId).toBe("b");
  });

  it("keeps the cursor on its email when the list reloads, or moves it to the first", async () => {
    fakeMail({ scheduled: vi.fn(async () => [email("a", 1), email("b", 2)]) });
    useScheduled.setState({ cursorId: "b" });
    await loadScheduled();
    expect(q().cursorId).toBe("b");
    useScheduled.setState({ cursorId: "gone" });
    await loadScheduled();
    expect(q().cursorId).toBe("a");
  });

  it("H picks a new time for one", async () => {
    const { backend } = fakeMail();
    useScheduled.setState({ emails: [email("a", tomorrow8, { tries: 2, error: "offline", nextTry: tomorrow8 + 5 })] });
    useScheduled.getState().openMove("a");
    expect(q().picking).toEqual({ kind: "move", id: "a", operationId: "op-a", revision: 1 });
    pickTime(saturday8);
    expect(q().emails).toEqual([email("a", saturday8)]);
    expect(backend.rescheduleSend).toHaveBeenCalledWith("a", saturday8, "op-a", 1);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Now set for Sat 8 AM" });
    // Sent while the picker was open: nothing to move.
    useScheduled.setState({ picking: { kind: "move", id: "gone", operationId: "op-gone", revision: 1 } });
    pickTime(saturday8);
    expect(backend.rescheduleSend).toHaveBeenCalledOnce();
  });

  it("a new time the engine refuses says why and shows the list as it is", async () => {
    fakeMail({
      rescheduleSend: vi.fn(() => Promise.reject(new Error("It’s sending now."))),
      scheduled: vi.fn(async () => []),
    });
    useScheduled.setState({ emails: [email("a", tomorrow8)], picking: { kind: "move", id: "a", operationId: "op-a", revision: 1 } });
    pickTime(saturday8);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "It’s sending now.", problem: true });
    expect(q().emails).toEqual([]);
  });

  it("send now goes at once", async () => {
    const { backend } = fakeMail();
    useScheduled.setState({ emails: [email("a", saturday8)] });
    sendNow("a");
    expect(backend.rescheduleSend).toHaveBeenCalledWith("a", now, "op-a", 1);
    expect(q().emails[0].sendAt).toBe(now);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Sending “Email a” now" });
    sendNow("unknown");
    expect(backend.rescheduleSend).toHaveBeenCalledOnce();
  });

  it("send now the engine refuses says why and shows the list as it is", async () => {
    fakeMail({ rescheduleSend: vi.fn(() => Promise.reject(new Error("It’s sending now."))), scheduled: vi.fn(async () => [email("a", saturday8)]) });
    useScheduled.setState({ emails: [email("a", saturday8)] });
    sendNow("a");
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "It’s sending now.", problem: true });
    expect(q().emails[0].sendAt).toBe(saturday8);
  });

  it("↵ opens one to edit: back in Drafts, in the composer, saying when it was set for", async () => {
    const kept: SavedDraft = { ...email("a", 1).draft, updatedAt: now };
    const other = { ...email("c", 1).draft };
    useDrafts.setState({ drafts: [{ ...kept, body: "Stale" }, other] });
    const { backend } = fakeMail({ unscheduleSend: vi.fn(async () => ({ status: "cancelled" as const, restored: kept })) });
    useStore.setState({ view: "scheduled" });
    useScheduled.setState({ emails: [email("a", saturday8), email("b", saturday8 + 1)], cursorId: "a" });
    await editScheduled("a");
    expect(backend.unscheduleSend).toHaveBeenCalledWith("a", "op-a");
    expect(s()).toMatchObject({ view: "compose", draft: kept });
    expect(q()).toMatchObject({ editing: { id: "a", sendAt: saturday8 }, cursorId: "b" });
    expect(q().emails.map((e) => e.draft.id)).toEqual(["b"]);
    expect(useDrafts.getState()).toMatchObject({ drafts: [kept, other], status: { a: "saved" } });
    // The demo mailbox keeps no drafts: the one on screen opens.
    fakeMail();
    await editScheduled("b");
    expect(s().draft).toMatchObject({ id: "b", updatedAt: now });
    await editScheduled("gone");
  });

  it("one already on its way can't be opened, and says so", async () => {
    fakeMail({ unscheduleSend: vi.fn(() => Promise.reject(new Error("It’s sending now."))) });
    useStore.setState({ view: "scheduled" });
    useScheduled.setState({ emails: [email("a", saturday8)] });
    await editScheduled("a");
    expect(s()).toMatchObject({ view: "scheduled", toast: { text: "It’s sending now.", problem: true } });
    expect(q().emails).toHaveLength(1);
  });

  it("⌫ puts one back in Drafts", async () => {
    fakeMail();
    useScheduled.setState({ emails: [email("a", saturday8)], cursorId: "a" });
    await cancelScheduled("a");
    expect(q()).toMatchObject({ emails: [], cursorId: undefined });
    expect(useDrafts.getState().drafts.map((d) => d.id)).toEqual(["a"]);
    expect(s().toast).toMatchObject({ text: "Back in Drafts. It won’t send." });
    await cancelScheduled("gone");
  });

  it("⌫ on one already on its way says so", async () => {
    fakeMail({ unscheduleSend: vi.fn(() => Promise.reject(new Error("It’s sending now."))) });
    useScheduled.setState({ emails: [email("a", saturday8)] });
    await cancelScheduled("a");
    expect(s().toast).toMatchObject({ text: "It’s sending now.", problem: true });
    expect(q().emails).toHaveLength(1);
  });
});

describe("what the engine says", () => {
  it("an email that went leaves the list and says so", () => {
    useScheduled.setState({ emails: [email("a", now), email("b", now + 1)], cursorId: "a" });
    scheduledEvent({ type: "scheduledSent", email: email("a", now), late: false });
    expect(q()).toMatchObject({ emails: [email("b", now + 1)], cursorId: "b" });
    expect(s().toast).toMatchObject({ text: "Sent “Email a”" });
  });

  it("one that went late says Moss was closed at its time", () => {
    const eight = new Date(2026, 10, 3, 8, 0).getTime();
    scheduledEvent({ type: "scheduledSent", email: { ...email("a", eight), draft: { ...email("a", eight).draft, subject: " " } }, late: true });
    expect(s().toast).toMatchObject({ text: "Sent “No subject” late. Moss was closed at 8 AM." });
  });

  it("a failed try shows on the list, and says so the first time", () => {
    useScheduled.setState({ emails: [email("a", now)] });
    const failed = email("a", now, { tries: 1, nextTry: now + 60_000, error: "can't reach the server" });
    scheduledEvent({ type: "scheduledFailed", email: failed });
    expect(q().emails).toEqual([failed]);
    expect(s().toast).toMatchObject({ text: "Couldn’t send “Email a”. Trying again at 1:21 PM.", problem: true });
    expect(s().toast?.show?.label).toBe("Show scheduled");
    s().toast!.show!.run();
    expect(s().view).toBe("scheduled");

    useStore.setState({ toast: undefined });
    scheduledEvent({ type: "scheduledFailed", email: { ...failed, tries: 2 } });
    expect(q().emails[0].tries).toBe(2);
    expect(s().toast, "not again each time").toBeUndefined();
  });

  it("ignores other events", () => {
    scheduledEvent({ type: "mailChanged", accountId: "work" });
    expect(s().toast).toBeUndefined();
  });
});

it("Undo does not reopen a sendable draft before schedule cancellation is confirmed", async () => {
  let finish!: (outcome: ScheduleOutcome) => void;
  fakeMail({ unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { finish = resolve; })) });
  composing();
  openSendLater();
  pickTime(saturday8);
  s().undo();
  try {
    expect(s().view).not.toBe("compose");
    expect(s().draft).toBeUndefined();
    expect(q().emails.map((email) => email.draft.id)).toEqual([draft.id]);
  } finally {
    useDrafts.setState({ drafts: [{ ...draft, body: "Stale list entry", updatedAt: 1 }, { ...draft, id: "other", updatedAt: 1 }] });
    finish({ status: "cancelled", restored: { ...draft, threadId: undefined, updatedAt: now } });
  }
  await vi.advanceTimersByTimeAsync(0);
  expect(s()).toMatchObject({ view: "compose", draft: { id: draft.id } });
  expect(s().draft?.threadId).toBeUndefined();
  expect(useDrafts.getState().drafts.map((d) => d.id)).toEqual([draft.id, "other"]);
  expect(useDrafts.getState().drafts[0].body).toBe(draft.body);
  expect(q().emails).toEqual([]);
});

it("a refused schedule Undo never leaves an editable copy behind", async () => {
  fakeMail({ unscheduleSend: vi.fn(async () => { throw new Error("Already claimed"); }) });
  composing();
  openSendLater();
  pickTime(saturday8);
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft).toBeUndefined();
  expect(s().view).not.toBe("compose");
  expect(q().emails.map((email) => email.draft.id)).toEqual([draft.id]);
});

it("a confirmed Undo keeps a composer opened while cancellation was pending", async () => {
  let finish!: (outcome: ScheduleOutcome) => void;
  fakeMail({ unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { finish = resolve; })) });
  composing();
  openSendLater();
  pickTime(saturday8);
  s().undo();
  composing({ ...draft, id: "another", body: "Keep these edits" });
  finish({ status: "cancelled", restored: { ...draft, threadId: undefined, updatedAt: now } });
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft).toMatchObject({ id: "another", body: "Keep these edits" });
  expect(useDrafts.getState().drafts).toContainEqual(expect.objectContaining({ id: "d1", threadId: undefined }));
  expect(s().toast).toMatchObject({ text: "Email restored to Drafts" });
});

it("keeps the operation identity through creation and Undo", async () => {
  const { backend } = fakeMail();
  scheduleDraft(draft, saturday8);
  const operation = q().emails[0].operationId;
  expect(operation).toEqual(expect.any(String));
  await vi.runAllTimersAsync();
  expect(backend.scheduleSend).toHaveBeenCalledWith(draft, saturday8, operation);
  s().undo();
  await vi.runAllTimersAsync();
  expect(backend.unscheduleSend).toHaveBeenCalledWith(draft.id, operation);
});

it("does not restore an editable copy when cancellation reports sending", async () => {
  fakeMail({ unscheduleSend: vi.fn(async () => ({ status: "sending" as const })) });
  scheduleDraft(draft, saturday8);
  await vi.runAllTimersAsync();
  s().undo();
  await vi.runAllTimersAsync();
  expect(s().draft).toBeUndefined();
  expect(q().emails).toHaveLength(1);
  expect(useDrafts.getState().drafts).toEqual([]);
  expect(s().toast?.text).toBe("This email is sending. It cannot be changed now.");
});

it("does not reopen a rejected creation until cancellation is confirmed", async () => {
  fakeMail({
    scheduleSend: vi.fn(async () => { throw new Error("Reply lost"); }),
    unscheduleSend: vi.fn(async () => ({ status: "sending" as const })),
  });
  scheduleDraft(draft, saturday8);
  await vi.runAllTimersAsync();
  expect(useDrafts.getState().drafts).toEqual([]);
  expect(s().draft).toBeUndefined();
  expect(q().emails).toHaveLength(1);
});

it("an old send event cannot remove or replace a newer schedule", () => {
  const newer = email("a", saturday8, { operationId: "new" });
  useScheduled.setState({ emails: [newer] });
  scheduledEvent({ type: "scheduledSent", email: email("a", now, { operationId: "old" }), late: false });
  expect(q().emails).toEqual([newer]);
  scheduledEvent({ type: "scheduledFailed", email: email("a", now, { operationId: "old", tries: 1, error: "offline" }) });
  expect(q().emails).toEqual([newer]);
});

it("restores a confirmed Undo even when creation reports cancellation first", async () => {
  let finish!: (outcome: ScheduleOutcome) => void;
  fakeMail({
    scheduleSend: vi.fn(async () => ({ status: "cancelled" as const })),
    unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { finish = resolve; })),
  });
  scheduleDraft(draft, saturday8);
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  expect(q().emails).toEqual([]);
  finish({ status: "cancelled", restored: { ...draft, updatedAt: now } });
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft).toMatchObject({ id: draft.id, body: draft.body });
});

it("ignores an old Undo reply after a new schedule has also been cancelled", async () => {
  let finish!: (outcome: ScheduleOutcome) => void;
  const { backend } = fakeMail({ unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { finish = resolve; })) });
  scheduleDraft(draft, saturday8);
  s().undo();
  const oldFinish = finish;
  scheduleDraft({ ...draft, body: "New content" }, tomorrow8);
  vi.mocked(backend.unscheduleSend).mockResolvedValueOnce({ status: "cancelled", restored: { ...draft, body: "New content", updatedAt: now } });
  await cancelScheduled(draft.id);
  oldFinish({ status: "cancelled", restored: { ...draft, updatedAt: now } });
  await vi.advanceTimersByTimeAsync(0);
  expect(s().draft).toBeUndefined();
  expect(useDrafts.getState().drafts[0].body).toBe("New content");
});

it("does not resurrect a cancelled operation from a delayed failure event", async () => {
  fakeMail();
  const waiting = email("a", saturday8);
  useScheduled.setState({ emails: [waiting] });
  await cancelScheduled("a");
  scheduledEvent({ type: "scheduledFailed", email: { ...waiting, tries: 1, error: "offline" } });
  expect(q().emails).toEqual([]);
});

it.each(["cancelled", "completed"] as const)("Send now reconciles a %s outcome", async (status) => {
  fakeMail({ rescheduleSend: vi.fn(async () => ({ status })), scheduled: vi.fn(async () => []) });
  useScheduled.setState({ emails: [email("a", saturday8)] });
  sendNow("a");
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([]);
  expect(s().toast?.text ?? "").not.toContain("now");
});

it("persists the fallback draft after list cancellation instead of merely marking it saved", async () => {
  const { backend } = fakeMail();
  useScheduled.setState({ emails: [email("a", saturday8)] });
  await cancelScheduled("a");
  await draftsSaved();
  expect(backend.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ id: "a", body: draft.body }));
});

it.each([
  [{ status: "sending" }, "This email is sending. It cannot be changed now."],
  [{ status: "completed" }, "This email has already been sent."],
  [{ status: "inDelivery", state: "uncertain" }, "This email has a delivery in progress. Check its status before sending again."],
] as const)("keeps list actions truthful for %j", async (outcome, message) => {
  fakeMail({ unscheduleSend: vi.fn(async () => outcome), rescheduleSend: vi.fn(async () => outcome), scheduled: vi.fn(async () => []) });
  useScheduled.setState({ emails: [email("a", saturday8)] });
  await cancelScheduled("a");
  expect(s().toast?.text).toBe(message);
  expect(useDrafts.getState().drafts).toEqual([]);
  // The first completed operation remains terminal. Exercise moving a new operation.
  useScheduled.setState({ emails: [email("a", saturday8, { operationId: "op-a-new" })] });
  q().openMove("a");
  pickTime(tomorrow8);
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([]);
  expect(s().toast?.text).toBe(message);
});

it("keeps the pending schedule when creation and cancellation replies are both lost", async () => {
  fakeMail({ scheduleSend: vi.fn(async () => { throw new Error("Lost reply"); }), unscheduleSend: vi.fn(async () => { throw new Error("Lost confirmation"); }) });
  scheduleDraft(draft, saturday8);
  await vi.runAllTimersAsync();
  expect(q().emails).toHaveLength(1);
  expect(useDrafts.getState().recoveries.size).toBe(1);
  expect(useDrafts.getState().drafts[0].body).toBe(draft.body);
  expect(s().toast?.text).toBe("Couldn’t confirm scheduling or cancellation. Check Scheduled before sending again.");
});

it("a creation failure from an older operation does not cancel a newer one", async () => {
  let reject!: (error: Error) => void;
  const create = vi.fn().mockImplementationOnce(() => new Promise<ScheduleOutcome>((_resolve, fail) => { reject = fail; })).mockResolvedValue({ status: "scheduled" });
  const { backend } = fakeMail({ scheduleSend: create });
  scheduleDraft(draft, saturday8);
  scheduleDraft({ ...draft, body: "New content" }, tomorrow8);
  const newer = q().emails[0];
  reject(new Error("Late failure"));
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([newer]);
  expect(backend.unscheduleSend).not.toHaveBeenCalled();
  expect(useDrafts.getState().drafts).toEqual([]);
});

it("records the revision returned by a confirmed move", async () => {
  fakeMail({ rescheduleSend: vi.fn(async () => ({ status: "rescheduled" as const, revision: 2 })) });
  useScheduled.setState({ emails: [email("a", saturday8)] });
  q().openMove("a");
  pickTime(tomorrow8);
  await vi.runAllTimersAsync();
  expect(q().emails).toMatchObject([{ sendAt: tomorrow8, revision: 2 }]);
  expect(s().toast?.text).toContain("Now set for");
});

it("a late failed-creation confirmation cannot overwrite a completed Undo", async () => {
  let rejectCreation!: (error: Error) => void;
  const cancellations: ((outcome: ScheduleOutcome) => void)[] = [];
  const { backend } = fakeMail({
    scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((_resolve, reject) => { rejectCreation = reject; })),
    unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { cancellations.push(resolve); })),
  });
  scheduleDraft(draft, saturday8);
  s().undo();
  rejectCreation(new Error("Lost creation reply"));
  await vi.advanceTimersByTimeAsync(0);
  expect(cancellations).toHaveLength(2);
  cancellations[0]({ status: "cancelled", restored: { ...draft, threadId: undefined, updatedAt: now } });
  await vi.advanceTimersByTimeAsync(0);
  const restored = s().draft;
  cancellations[1]({ status: "cancelled" });
  await vi.runAllTimersAsync();
  expect(s().draft).toBe(restored);
  expect(useDrafts.getState().drafts[0].threadId).toBeUndefined();
  expect(backend.saveDraft).not.toHaveBeenCalled();
  expect(s().toast?.text ?? "").not.toContain("Not scheduled");
});

it("retries a lost Undo reply and restores its confirmed cancellation", async () => {
  let finishCreation!: (outcome: ScheduleOutcome) => void;
  const cancel = vi.fn().mockRejectedValueOnce(new Error("Lost Undo reply")).mockResolvedValue({ status: "cancelled", restored: { ...draft, revision: "restored", updatedAt: now } });
  fakeMail({ scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { finishCreation = resolve; })), unscheduleSend: cancel });
  scheduleDraft(draft, saturday8);
  s().undo();
  await vi.advanceTimersByTimeAsync(0);
  finishCreation({ status: "cancelled" });
  await vi.runAllTimersAsync();
  expect(cancel).toHaveBeenCalledTimes(2);
  expect(s().draft).toMatchObject({ id: draft.id, body: draft.body });
  expect(useDrafts.getState().drafts[0].body).toBe(draft.body);
  expect(q().emails).toEqual([]);
  expect(s().toast?.text ?? "").not.toContain("may still send");
});

it.each(["move", "send", "cancel"] as const)("ignores stale %s results and errors after a newer schedule replaces the row", async (action) => {
  for (const failed of [false, true]) {
    let finish!: (outcome: ScheduleOutcome) => void;
    let fail!: (error: Error) => void;
    const pending = () => new Promise<ScheduleOutcome>((resolve, reject) => { finish = resolve; fail = reject; });
    const { backend } = fakeMail({ rescheduleSend: vi.fn(pending), unscheduleSend: vi.fn(pending) });
    useStore.setState({ toast: undefined });
    useScheduled.setState({ emails: [email("a", saturday8, { operationId: "old" })] });
    let cancelling: Promise<void> | undefined;
    if (action === "move") { q().openMove("a"); pickTime(tomorrow8); }
    else if (action === "send") sendNow("a");
    else cancelling = cancelScheduled("a");
    const newer = email("a", saturday8, { operationId: "new" });
    useScheduled.setState({ emails: [newer] });
    if (failed) fail(new Error("Old action failed"));
    else finish(action === "cancel" ? { status: "cancelled", restored: { ...draft, id: "a", updatedAt: now } } : { status: "completed" });
    await cancelling;
    await vi.runAllTimersAsync();
    expect(q().emails).toEqual([newer]);
    expect(s().toast).toBeUndefined();
    expect(useDrafts.getState().drafts).toEqual([]);
    expect(backend.scheduled).not.toHaveBeenCalled();
  }
});

it("ignores a lost cancellation confirmation after a newer schedule takes over", async () => {
  let rejectConfirmation!: (error: Error) => void;
  const create = vi.fn().mockRejectedValueOnce(new Error("Lost creation reply")).mockResolvedValue({ status: "scheduled" });
  fakeMail({ scheduleSend: create, unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((_resolve, reject) => { rejectConfirmation = reject; })) });
  scheduleDraft(draft, saturday8);
  await vi.advanceTimersByTimeAsync(0);
  scheduleDraft({ ...draft, body: "New content" }, tomorrow8);
  const newer = q().emails[0];
  const message = s().toast;
  rejectConfirmation(new Error("Lost cancellation reply"));
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([newer]);
  expect(s().toast).toBe(message);
  expect(useDrafts.getState().drafts).toEqual([]);
});

it("an open time picker cannot reschedule a replacement operation", () => {
  const { backend } = fakeMail();
  useScheduled.setState({ emails: [email("a", saturday8, { operationId: "old" })] });
  q().openMove("a");
  const replacement = email("a", saturday8, { operationId: "new" });
  useScheduled.setState({ emails: [replacement] });
  pickTime(tomorrow8);
  expect(backend.rescheduleSend).not.toHaveBeenCalled();
  expect(q().emails).toEqual([replacement]);
  expect(q().picking).toBeUndefined();
});

it("a delayed list response preserves actions made while it was loading", async () => {
  let finish!: (emails: ScheduledEmail[]) => void;
  fakeMail({ scheduled: vi.fn(() => new Promise<ScheduledEmail[]>((resolve) => { finish = resolve; })) });
  const old = email("a", saturday8);
  useScheduled.setState({ emails: [old] });
  const loading = loadScheduled();
  await cancelScheduled("a");
  scheduleDraft({ ...draft, id: "b" }, tomorrow8);
  const created = q().emails[0];
  finish([old, email("c", saturday8)]);
  await loading;
  expect(q().emails.map((e) => e.draft.id).sort()).toEqual(["b", "c"]);
  expect(q().emails.find((e) => e.draft.id === "b")).toEqual(created);
});

it("an older list request cannot overwrite a newer list response", async () => {
  const replies: ((emails: ScheduledEmail[]) => void)[] = [];
  fakeMail({ scheduled: vi.fn(() => new Promise<ScheduledEmail[]>((resolve) => { replies.push(resolve); })) });
  const older = loadScheduled();
  const newer = loadScheduled();
  const current = email("a", saturday8, { operationId: "new" });
  replies[1]([current]);
  await newer;
  replies[0]([email("a", tomorrow8, { operationId: "old" })]);
  await older;
  expect(q().emails).toEqual([current]);
});

it.each(["create", "move"] as const)("a refresh preserves a %s action already pending when loading began", async (action) => {
  let finishAction!: (outcome: ScheduleOutcome) => void;
  const pending = () => new Promise<ScheduleOutcome>((resolve) => { finishAction = resolve; });
  fakeMail({ scheduleSend: vi.fn(pending), rescheduleSend: vi.fn(pending), scheduled: vi.fn(async () => action === "create" ? [] : [email("a", saturday8)]) });
  if (action === "create") scheduleDraft(draft, tomorrow8);
  else { useScheduled.setState({ emails: [email("a", saturday8)] }); q().openMove("a"); pickTime(tomorrow8); }
  const optimistic = q().emails[0];
  await loadScheduled();
  expect(q().emails).toEqual([optimistic]);
  finishAction(action === "create" ? { status: "scheduled" } : { status: "rescheduled", revision: 2 });
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([{ ...optimistic, revision: action === "create" ? 1 : 2 }]);
});

it("Undo does not restore a stale snapshot after failed creation already restored normalized mail", async () => {
  let rejectCreation!: (error: Error) => void;
  const cancellations: ((outcome: ScheduleOutcome) => void)[] = [];
  const { backend } = fakeMail({
    scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((_resolve, reject) => { rejectCreation = reject; })),
    unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { cancellations.push(resolve); })),
  });
  scheduleDraft(draft, saturday8);
  s().undo();
  rejectCreation(new Error("Lost reply"));
  await vi.advanceTimersByTimeAsync(0);
  cancellations[1]({ status: "cancelled", restored: { ...draft, threadId: undefined, updatedAt: now } });
  await vi.runAllTimersAsync();
  vi.mocked(backend.saveDraft).mockClear();
  cancellations[0]({ status: "cancelled" });
  await vi.runAllTimersAsync();
  expect(useDrafts.getState().drafts[0].threadId).toBeUndefined();
  expect(backend.saveDraft).not.toHaveBeenCalled();
  expect(s().draft).toBeUndefined();
});

it("does not open a time picker for a schedule that no longer exists", () => {
  fakeMail();
  q().openMove("missing");
  expect(q().picking).toBeUndefined();
});

it("queues moves for one operation using the preceding confirmed revision", async () => {
  const replies: ((outcome: ScheduleOutcome) => void)[] = [];
  const { backend } = fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { replies.push(resolve); })) });
  useScheduled.setState({ emails: [email("a", saturday8)] });
  q().openMove("a");
  pickTime(tomorrow8);
  sendNow("a");
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.rescheduleSend).toHaveBeenCalledTimes(1);
  expect(backend.rescheduleSend).toHaveBeenNthCalledWith(1, "a", tomorrow8, "op-a", 1);
  replies[0]({ status: "rescheduled", revision: 2 });
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.rescheduleSend).toHaveBeenNthCalledWith(2, "a", now, "op-a", 2);
  expect(s().toast?.text ?? "").not.toContain("Now set for");
  replies[1]({ status: "rescheduled", revision: 3 });
  await vi.runAllTimersAsync();
  expect(q().emails).toMatchObject([{ sendAt: now, revision: 3 }]);
  expect(q().pendingOperations).toEqual({});
  expect(s().toast?.text).toBe("Sending “Email a” now");
});

it("ignores a failure event from an older schedule revision", () => {
  fakeMail();
  const current = email("a", tomorrow8, { revision: 2 });
  useScheduled.setState({ emails: [current] });
  scheduledEvent({ type: "scheduledFailed", email: email("a", saturday8, { revision: 1, tries: 1, error: "offline" }) });
  expect(q().emails).toEqual([current]);
  expect(s().toast).toBeUndefined();
});

it("uses the revision observed when the time picker opened", async () => {
  const current = email("a", tomorrow8, { revision: 2 });
  const { backend } = fakeMail({
    rescheduleSend: vi.fn(async () => ({ status: "changed" as const, current })),
    scheduled: vi.fn(async () => [current]),
  });
  useScheduled.setState({ emails: [email("a", saturday8)] });
  q().openMove("a");
  await loadScheduled();
  pickTime(now);
  await vi.runAllTimersAsync();
  expect(backend.rescheduleSend).toHaveBeenCalledWith("a", now, "op-a", 1);
  expect(q().emails).toEqual([current]);
  expect(s().toast?.text).toBe("This schedule changed. Check the new time.");
});

it.each(["changed", "sending", "completed", "rejected"] as const)("stops queued moves after %s and reconciles once", async (status) => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  let reject!: (error: Error) => void;
  const current = email("a", saturday8, { revision: 4 });
  const { backend } = fakeMail({
    rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes, no) => { resolve = yes; reject = no; })),
    scheduled: vi.fn(async () => status === "completed" ? [] : [current]),
  });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  sendNow("a");
  sendNow("a");
  if (status === "rejected") reject(new Error("Reply lost"));
  else resolve(status === "changed" ? { status, current } : { status });
  await vi.runAllTimersAsync();
  expect(backend.rescheduleSend).toHaveBeenCalledTimes(1);
  expect(backend.scheduled).toHaveBeenCalledTimes(1);
  expect(q().pendingOperations).toEqual({});
  expect(q().emails).toEqual(status === "completed" ? [] : [current]);
  expect(s().toast?.text).toBe({
    changed: "This schedule changed. Check the new time.",
    sending: "This email is sending. It cannot be changed now.",
    completed: "This email has already been sent.",
    rejected: "Reply lost",
  }[status]);
});

it("does not submit a queued move after cancellation finishes", async () => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  const { backend } = fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })) });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  sendNow("a");
  sendNow("a");
  await cancelScheduled("a");
  resolve({ status: "rescheduled", revision: 2 });
  await vi.runAllTimersAsync();
  expect(backend.rescheduleSend).toHaveBeenCalledTimes(1);
  expect(q().pendingOperations).toEqual({});
  expect(q().emails).toEqual([]);
  expect(s().toast?.text).toBe("Back in Drafts. It won’t send.");
});

it.each(["rescheduled", "changed"] as const)("preserves a newer event when an older %s reply arrives", async (status) => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  const newer = email("a", saturday8, { revision: 4, tries: 2, error: "offline" });
  let finishLoad!: (rows: ScheduledEmail[]) => void;
  fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })), scheduled: vi.fn(() => new Promise<ScheduledEmail[]>((yes) => { finishLoad = yes; })) });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  sendNow("a");
  scheduledEvent({ type: "scheduledFailed", email: newer });
  resolve(status === "rescheduled" ? { status, revision: 2 } : { status, current: email("a", tomorrow8, { revision: 3 }) });
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await vi.advanceTimersByTimeAsync(0);
  expect(q().emails).toEqual([newer]);
  if (status === "changed") finishLoad([newer]);
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([newer]);
  expect(s().toast?.text).toBe("This schedule changed. Check the new time.");
});

it("keeps a newer revision when a later list request returns an older row", async () => {
  const newer = email("a", saturday8, { revision: 3 });
  fakeMail({ scheduled: vi.fn(async () => [email("a", tomorrow8, { revision: 2 })]) });
  useScheduled.setState({ emails: [newer] });
  await loadScheduled();
  expect(q().emails).toEqual([newer]);
});

it("applies a confirmed move time after an older failure event", async () => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })) });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  sendNow("a");
  scheduledEvent({ type: "scheduledFailed", email: email("a", saturday8, { revision: 2, tries: 2, error: "offline" }) });
  resolve({ status: "rescheduled", revision: 3 });
  await vi.runAllTimersAsync();
  expect(q().emails).toMatchObject([{ revision: 3, sendAt: now, nextTry: now, tries: 0, error: undefined }]);
  expect(s().toast?.text).toBe("Sending “Email a” now");
});

it.each(["rescheduled", "sending"] as const)("preserves the queued time during an equal-revision failure until %s", async (status) => {
  const replies: ((outcome: ScheduleOutcome) => void)[] = [];
  const failed = email("a", tomorrow8, { revision: 2, tries: 1, error: "offline", nextTry: saturday8 });
  fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { replies.push(yes); })), scheduled: vi.fn(async () => [failed]) });
  useScheduled.setState({ emails: [email("a", saturday8)] });
  q().openMove("a");
  pickTime(tomorrow8);
  sendNow("a");
  replies[0]({ status: "rescheduled", revision: 2 });
  await vi.advanceTimersByTimeAsync(0);
  scheduledEvent({ type: "scheduledFailed", email: failed });
  expect(q().emails).toMatchObject([{ revision: 2, sendAt: now, tries: 0 }]);
  expect(s().toast).toBeUndefined();
  replies[1](status === "rescheduled" ? { status, revision: 3 } : { status });
  await vi.runAllTimersAsync();
  if (status === "sending") {
    expect(q().emails).toEqual([failed]);
    expect(s().toast?.text).toBe("This email is sending. It cannot be changed now.");
  } else {
    expect(q().emails).toMatchObject([{ revision: 3, sendAt: now, tries: 0 }]);
    expect(s().toast?.text).toBe("Sending “Email a” now");
  }
});

it("does not announce a move after its row has been cleared", async () => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })) });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  sendNow("a");
  useScheduled.setState({ emails: [] });
  resolve({ status: "rescheduled", revision: 2 });
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([]);
  expect(s().toast).toBeUndefined();
});

it("retains equal-revision retry metadata after a delayed success", async () => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })) });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  sendNow("a");
  const failed = email("a", now, { revision: 2, nextTry: tomorrow8, tries: 2, error: "offline" });
  scheduledEvent({ type: "scheduledFailed", email: failed });
  resolve({ status: "rescheduled", revision: 2 });
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([failed]);
  expect(s().toast).toBeUndefined();
  scheduledEvent({ type: "scheduledFailed", email: { ...failed, tries: 3, nextTry: saturday8 } });
  expect(q().emails).toEqual([{ ...failed, tries: 3, nextTry: saturday8 }]);
});

it("keeps a failure received before the creation reply", async () => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  fakeMail({ scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })) });
  scheduleDraft(draft, now);
  const failed = { ...q().emails[0], tries: 1, error: "offline", nextTry: tomorrow8 };
  scheduledEvent({ type: "scheduledFailed", email: failed });
  resolve({ status: "scheduled" });
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([failed]);
  expect(s().toast?.text).toContain("Couldn’t send");
});

it.each(["scheduled", "cancelled"] as const)("waits for creation before moving, then handles %s", async (status) => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  const { backend } = fakeMail({ scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })) });
  scheduleDraft(draft, tomorrow8);
  const operation = q().emails[0].operationId;
  sendNow(draft.id);
  expect(backend.rescheduleSend).not.toHaveBeenCalled();
  resolve({ status });
  await vi.runAllTimersAsync();
  if (status === "scheduled") {
    expect(backend.rescheduleSend).toHaveBeenCalledWith(draft.id, now, operation, 1);
    expect(q().emails).toMatchObject([{ sendAt: now, revision: 2 }]);
    expect(s().toast?.text).toBe("Sending “Re: Subject a” now");
  } else {
    expect(backend.rescheduleSend).not.toHaveBeenCalled();
    expect(q().emails).toEqual([]);
  }
  expect(q().pendingOperations).toEqual({});
});

it("leaves failed creation recovery to the creation handler when a move is queued", async () => {
  let reject!: (error: Error) => void;
  let confirm!: (outcome: ScheduleOutcome) => void;
  const { backend } = fakeMail({
    scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((_, no) => { reject = no; })),
    unscheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { confirm = yes; })),
  });
  scheduleDraft(draft, tomorrow8);
  sendNow(draft.id);
  reject(new Error("Creation reply lost"));
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.rescheduleSend).not.toHaveBeenCalled();
  expect(backend.scheduled).not.toHaveBeenCalled();
  expect(s().toast?.text).not.toBe("Creation reply lost");
  confirm({ status: "cancelled", restored: { ...draft, updatedAt: now } });
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([]);
  expect(s().toast?.text).toContain("Not scheduled");
  expect(q().pendingOperations).toEqual({});
});

it("checks the captured revision after an already scheduled creation replay", async () => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  const current = email(draft.id, saturday8, { revision: 2 });
  const { backend } = fakeMail({
    scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })),
    rescheduleSend: vi.fn(async () => ({ status: "changed" as const, current })),
    scheduled: vi.fn(async () => [current]),
  });
  scheduleDraft(draft, tomorrow8);
  current.operationId = q().emails[0].operationId;
  sendNow(draft.id);
  resolve({ status: "alreadyScheduled" });
  await vi.runAllTimersAsync();
  expect(backend.rescheduleSend).toHaveBeenCalledWith(draft.id, now, current.operationId, 1);
  expect(q().emails).toEqual([current]);
  expect(s().toast?.text).toBe("This schedule changed. Check the new time.");
});

it("keeps an older queued reply silent after a newer external event", async () => {
  const replies: ((outcome: ScheduleOutcome) => void)[] = [];
  const current = email("a", saturday8, { revision: 4, tries: 2, error: "offline" });
  fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { replies.push(yes); })), scheduled: vi.fn(async () => [current]) });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  sendNow("a");
  sendNow("a");
  scheduledEvent({ type: "scheduledFailed", email: current });
  replies[0]({ status: "rescheduled", revision: 2 });
  await vi.advanceTimersByTimeAsync(0);
  expect(q().emails).toEqual([current]);
  expect(s().toast).toBeUndefined();
  replies[1]({ status: "changed", current });
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([current]);
  expect(s().toast?.text).toBe("This schedule changed. Check the new time.");
});

it("does not revive an externally removed schedule from a delayed failure", async () => {
  const waiting = email("a", tomorrow8);
  const { backend } = fakeMail({ scheduled: vi.fn(async () => []) });
  useScheduled.setState({ emails: [waiting] });
  await loadScheduled();
  scheduledEvent({ type: "scheduledFailed", email: { ...waiting, tries: 1, error: "offline" } });
  expect(q().emails).toEqual([]);
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  expect(q().emails).toEqual([]);
  expect(s().toast).toBeUndefined();
});

it("loads an unfamiliar operation instead of inserting its event snapshot", async () => {
  let resolve!: (rows: ScheduledEmail[]) => void;
  fakeMail({ scheduled: vi.fn(() => new Promise<ScheduledEmail[]>((yes) => { resolve = yes; })) });
  const stale = email("a", tomorrow8, { tries: 1, error: "offline" });
  const current = email("a", saturday8, { revision: 2 });
  scheduledEvent({ type: "scheduledFailed", email: stale });
  expect(q().emails).toEqual([]);
  await vi.advanceTimersByTimeAsync(0);
  resolve([current]);
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([current]);
  expect(s().toast).toBeUndefined();
});

it("coalesces schedule-change notifications and refreshes again after an in-flight change", async () => {
  const replies: ((rows: ScheduledEmail[]) => void)[] = [];
  const { backend } = fakeMail({ scheduled: vi.fn(() => new Promise<ScheduledEmail[]>((yes) => { replies.push(yes); })) });
  scheduledEvent({ type: "scheduledChanged" });
  scheduledEvent({ type: "scheduledChanged" });
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.scheduled).toHaveBeenCalledTimes(1);
  scheduledEvent({ type: "scheduledChanged" });
  replies[0]([email("a", tomorrow8)]);
  await vi.advanceTimersByTimeAsync(1);
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  replies[1]([]);
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([]);
});

it("refreshes external changes after a pending move settles", async () => {
  let resolve!: (outcome: ScheduleOutcome) => void;
  const current = email("a", saturday8, { revision: 3 });
  const { backend } = fakeMail({ rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { resolve = yes; })), scheduled: vi.fn(async () => [current]) });
  useScheduled.setState({ emails: [email("a", tomorrow8), email("b", saturday8)] });
  sendNow("a");
  scheduledEvent({ type: "scheduledChanged" });
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(1);
  expect(q().emails).toMatchObject([{ draft: { id: "a" }, sendAt: now }]);
  resolve({ status: "rescheduled", revision: 2 });
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  expect(q().emails).toEqual([current]);
});

it("retries a failed event refresh with backoff", async () => {
  const current = email("a", tomorrow8);
  const { backend } = fakeMail({ scheduled: vi.fn().mockRejectedValueOnce(new Error("IPC unavailable")).mockResolvedValueOnce([current]) });
  scheduledEvent({ type: "scheduledChanged" });
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.scheduled).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(999);
  expect(backend.scheduled).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  expect(q().emails).toEqual([current]);
});

it("catches up an action that starts while an event refresh is in flight", async () => {
  let finishLoad!: (rows: ScheduledEmail[]) => void;
  let finishMove!: (outcome: ScheduleOutcome) => void;
  const current = email("a", saturday8, { revision: 3 });
  const { backend } = fakeMail({ scheduled: vi.fn().mockImplementationOnce(() => new Promise<ScheduledEmail[]>((yes) => { finishLoad = yes; })).mockResolvedValue([current]), rescheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((yes) => { finishMove = yes; })) });
  useScheduled.setState({ emails: [email("a", tomorrow8)] });
  scheduledEvent({ type: "scheduledChanged" });
  await vi.advanceTimersByTimeAsync(0);
  sendNow("a");
  finishLoad([current]);
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(1);
  expect(q().emails[0].sendAt).toBe(now);
  finishMove({ status: "rescheduled", revision: 2 });
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  expect(q().emails).toEqual([current]);
});

it("bounds failed refresh retries and recovers on a later event", async () => {
  const { backend } = fakeMail({ scheduled: vi.fn(async () => { throw new Error("IPC unavailable"); }) });
  scheduledEvent({ type: "scheduledChanged" });
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(4);
  expect(s().toast?.text).toBe("Couldn’t refresh Scheduled. Open Scheduled to try again.");
  const current = email("a", tomorrow8);
  vi.mocked(backend.scheduled).mockResolvedValueOnce([current]);
  scheduledEvent({ type: "scheduledChanged" });
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([current]);
});

it("times out a hung refresh and ignores its eventual stale response", async () => {
  let oldReply!: (rows: ScheduledEmail[]) => void;
  const current = email("a", saturday8, { revision: 2 });
  const { backend } = fakeMail({ scheduled: vi.fn().mockImplementationOnce(() => new Promise<ScheduledEmail[]>((yes) => { oldReply = yes; })).mockResolvedValue([current]) });
  scheduledEvent({ type: "scheduledChanged" });
  await vi.advanceTimersByTimeAsync(11_000);
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  expect(q().emails).toEqual([current]);
  oldReply([email("a", tomorrow8)]);
  await vi.runAllTimersAsync();
  expect(q().emails).toEqual([current]);
});

it("does not lose an event refresh superseded by a failing manual load", async () => {
  let first!: (rows: ScheduledEmail[]) => void;
  const current = email("a", tomorrow8);
  const { backend } = fakeMail({ scheduled: vi.fn().mockImplementationOnce(() => new Promise<ScheduledEmail[]>((yes) => { first = yes; })).mockRejectedValueOnce(new Error("Manual load failed")).mockResolvedValue([current]) });
  scheduledEvent({ type: "scheduledChanged" });
  await vi.advanceTimersByTimeAsync(0);
  await loadScheduled().catch(() => {});
  first([]);
  await vi.advanceTimersByTimeAsync(1);
  expect(backend.scheduled).toHaveBeenCalledTimes(3);
  expect(q().emails).toEqual([current]);
});

it("a new event accelerates a pending retry", async () => {
  const current = email("a", tomorrow8);
  const { backend } = fakeMail({ scheduled: vi.fn().mockRejectedValueOnce(new Error("one")).mockRejectedValueOnce(new Error("two")).mockRejectedValueOnce(new Error("three")).mockResolvedValue([current]) });
  scheduledEvent({ type: "scheduledChanged" });
  await vi.advanceTimersByTimeAsync(3000);
  expect(backend.scheduled).toHaveBeenCalledTimes(3);
  scheduledEvent({ type: "scheduledChanged" });
  await vi.advanceTimersByTimeAsync(1);
  expect(backend.scheduled).toHaveBeenCalledTimes(4);
  expect(q().emails).toEqual([current]);
});

it("retries reconciliation after a refused move", async () => {
  const current = email("a", tomorrow8, { tries: 1, error: "offline" });
  const { backend } = fakeMail({ rescheduleSend: vi.fn(async () => ({ status: "sending" as const })), scheduled: vi.fn().mockRejectedValueOnce(new Error("IPC unavailable")).mockResolvedValueOnce([current]) });
  useScheduled.setState({ emails: [email("a", saturday8)] });
  sendNow("a");
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  expect(q().emails).toEqual([current]);
});

it("retries a failed load when opening Scheduled", async () => {
  const current = email("a", tomorrow8);
  const { backend } = fakeMail({ live: true, scheduled: vi.fn().mockRejectedValueOnce(new Error("IPC unavailable")).mockResolvedValueOnce([current]) });
  openScheduled();
  await vi.runAllTimersAsync();
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  expect(q().emails).toEqual([current]);
});

it.each(["edit", "undo"] as const)("does not recreate an unavailable prepared schedule on %s", async (action) => {
  const { backend } = fakeMail({ live: true, unscheduleSend: vi.fn(async () => ({ status: "cancelled" as const })) });
  if (action === "edit") {
    useScheduled.setState({ emails: [email("a", saturday8)] });
    useDrafts.setState({ retired: { a: true } });
    await editScheduled("a");
    expect(useDrafts.getState().retired.a).toBeUndefined();
  } else {
    scheduleDraft(draft, saturday8);
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(useDrafts.getState().retired[draft.id]).toBeUndefined();
  }
  await draftsSaved();
  expect(backend.saveDraft).not.toHaveBeenCalled();
  expect(s().draft).toBeUndefined();
  expect(s().toast?.text).toContain("already removed or handed off");
});

it("does not recreate a removed restoration when failed creation is confirmed cancelled", async () => {
  const { backend } = fakeMail({
    scheduleSend: vi.fn(async () => { throw new Error("Lost creation reply"); }),
    unscheduleSend: vi.fn(async () => ({ status: "cancelled" as const })),
  });
  scheduleDraft(draft, saturday8);
  await vi.runAllTimersAsync();
  await draftsSaved();
  expect(backend.saveDraft).not.toHaveBeenCalled();
  expect(useDrafts.getState().drafts).toEqual([]);
  expect(s().draft).toBeUndefined();
  expect(s().toast?.text).toContain("already removed or handed off");
});

it.each(["undo", "edit", "cancel"].flatMap((action) => [false, true].map((anotherComposer) => ({ action, anotherComposer }))))("recovers $action after lost cancellation replies (another composer: $anotherComposer)", async ({ action, anotherComposer }) => {
  let finish!: (outcome: ScheduleOutcome) => void;
  const { backend } = fakeMail({
    scheduleSend: vi.fn(() => new Promise<ScheduleOutcome>((resolve) => { finish = resolve; })),
    unscheduleSend: vi.fn(async () => { throw new Error("Lost cancellation reply"); }),
  });
  scheduleDraft(draft, saturday8);
  if (action === "undo") s().undo();
  else if (action === "edit") await editScheduled(draft.id);
  else await cancelScheduled(draft.id);
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.saveDraft).not.toHaveBeenCalled();
  if (anotherComposer) { s().compose("new"); s().updateDraft({ body: "Another email" }); }
  const newer = s().draft;
  finish({ status: "cancelled", early: true });
  await vi.runAllTimersAsync();
  await draftsSaved();
  expect(backend.saveDraft).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ id: draft.id, body: draft.body }));
  expect(useDrafts.getState().drafts[0]).toMatchObject({ id: draft.id, body: draft.body });
  if (anotherComposer) expect(s().draft).toBe(newer);
  else if (action !== "cancel") expect(s().draft).toMatchObject({ id: draft.id, body: draft.body });
  else expect(s().draft).toBeUndefined();
  if (action === "edit" && !anotherComposer) expect(q().editing).toEqual({ id: draft.id, sendAt: saturday8 });
});

it("shows a recovery wait without inventing a provider retry notification", () => {
  fakeMail();
  const waiting = email("a", now, { tries: 1, nextTry: now, error: "offline" });
  useScheduled.setState({ emails: [waiting] });
  useStore.setState({ toast: undefined });
  scheduledEvent({ type: "scheduledWaiting", email: { ...waiting, error: "Sending is waiting for delivery recovery." } });
  expect(q().emails[0]).toMatchObject({ tries: 1, nextTry: now, error: "Sending is waiting for delivery recovery." });
  expect(s().toast).toBeUndefined();
});
