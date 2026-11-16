import { useCredentialCleanup } from "./credentialCleanup";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetServerChanges, useServerChanges } from "./serverChanges";
import type { Thread } from "../data/types";
import { useDrafts } from "../state/drafts";
import { useSignatures } from "../state/signatures";
import { useStore } from "../state/store";
import { fakeMail } from "../test/fakeMail";
import { draft, message, personal, resetStore, thread, work } from "../test/fixtures";
import { useScheduled } from "../state/scheduled";
import { resetInterruptedSends } from "../state/interruptedSends";
import { handleEvent, preloadAround, reloadThreads, startLive, useSync } from "./live";

const s = () => useStore.getState();

beforeEach(() => {
  resetStore([]);
  useSync.setState({ progress: {}, problems: {} });
  resetServerChanges();
  resetInterruptedSends();
});
afterEach(() => vi.useRealTimers());

describe("starting with real mail", () => {
  it("with no accounts yet, opens on Add account instead of the demo inbox", async () => {
    const { backend, listening } = fakeMail();
    resetStore([thread("demo")]);
    useStore.setState({ blockedThreads: [thread("demo-blocked")], doneThreads: [thread("demo-done")] });
    await startLive();
    expect(s()).toMatchObject({ accounts: [], threads: [], doneThreads: [], blockedThreads: [], view: "add-account" });
    expect(backend.threads).not.toHaveBeenCalled();
    expect(listening()).toBe(1);
  });

  it("with accounts, opens on their inbox", async () => {
    const t = thread("a", { partial: true });
    fakeMail({ accounts: vi.fn(async () => [work]), threads: vi.fn(async () => [t]) });
    const stop = await startLive();
    expect(s()).toMatchObject({ accounts: [work], view: "inbox", cursorId: "a" });
    expect(s().threads).toEqual([t]);
    stop();
  });

  it("learns why an account isn't syncing, even when sync said so before the window was listening", async () => {
    const connected = { ...work, connection: { provider: "gmail" as const, summary: "Gmail · App password" } };
    fakeMail({
      accounts: vi.fn(async () => [connected]),
      syncProblems: vi.fn(async () => [{ accountId: "work", problem: "signedOut" as const, message: "no saved sign-in" }]),
    });
    const stop = await startLive();
    expect(useSync.getState().problems.work).toEqual({ problem: "signedOut", message: "no saved sign-in" });
    expect(s().accounts[0].connection?.signedOut).toBe(true);
    stop();
  });

  it("loads the Done and Blocked lists too, so today’s progress is real from the start", async () => {
    const done = thread("d", { doneAt: 1 });
    const blocked = thread("x", { sender: "blocked" });
    fakeMail({
      accounts: vi.fn(async () => [work]),
      threads: vi.fn(async () => [thread("a"), thread("d"), thread("x")]),
      doneThreads: vi.fn(async () => [done]),
      blockedThreads: vi.fn(async () => [blocked]),
    });
    const stop = await startLive();
    expect(s().threads.map((t) => t.id)).toEqual(["a"]);
    expect(s().doneThreads).toEqual([done]);
    expect(s().blockedThreads).toEqual([blocked]);
    stop();
  });
});

describe("signatures in the desktop app", () => {
  it("loads the saved ones in place of the demo ones, so real mail is never signed with Maya’s", async () => {
    const saved = {
      signatures: [{ id: "s1", name: "Work", lines: ["Sam Lee"] }],
      byAccount: { gmail: { newEmails: "s1", replies: null } },
      letMossChoose: false,
      recent: ["s1"],
    };
    fakeMail({ signatures: vi.fn(async () => saved) });
    await startLive();
    expect(useSignatures.getState()).toMatchObject(saved);
  });

  it("starts with none when nothing is saved yet", async () => {
    fakeMail();
    await startLive();
    expect(useSignatures.getState()).toMatchObject({ signatures: [], byAccount: {}, letMossChoose: true, recent: [] });
  });
});

describe("people to write to", () => {
  it("loads the ones Moss knows from all your mail", async () => {
    const priya = { name: "Priya", email: "priya@acme.co", initials: "P" };
    const picks = [{ typed: "pri", email: "priya@acme.co" }];
    fakeMail({ accounts: vi.fn(async () => [work]), contacts: vi.fn(async () => [priya]), recipientPicks: vi.fn(async () => picks) });
    const stop = await startLive();
    expect(useStore.getState().knownPeople).toEqual([priya]);
    expect(useStore.getState().recipientPicks).toEqual(picks);
    stop();
  });
});

describe("drafts in the desktop app", () => {
  it("loads the ones kept on this Mac", async () => {
    const kept = { id: "d1", mode: "new" as const, fromAccountId: "gmail", to: [], cc: [], bcc: [], subject: "Lunch?", body: "", bodyHtml: "", fieldsOpen: true, showCc: false, showBcc: false, remindIfNoReply: false, signatureId: null, updatedAt: 1 };
    fakeMail({ drafts: vi.fn(async () => [kept]) });
    await startLive();
    expect(useDrafts.getState().drafts).toEqual([kept]);
  });
});

describe("reloading", () => {
  it("keeps loaded messages of unchanged threads, and the cursor when it can", async () => {
    const loaded = thread("a", { lastMessageAt: 1, messages: [message({ id: "a-1" }), message({ id: "a-2" })], fullSubject: "Re: A", partial: false });
    resetStore([loaded, thread("b", { lastMessageAt: 1 })]);
    useStore.setState({ cursorId: "b" });
    const fresh: Thread[] = [
      thread("a", { lastMessageAt: 1, partial: true }),
      thread("b", { lastMessageAt: 2, partial: true }),
    ];
    fakeMail({ threads: vi.fn(async () => fresh) });
    await reloadThreads();
    expect(s().threads[0]).toMatchObject({ partial: false, fullSubject: "Re: A" });
    expect(s().threads[0].messages).toHaveLength(2);
    expect(s().threads[1].partial).toBe(true);
    expect(s().cursorId).toBe("b");
  });

  it("moves the cursor to the top when its thread went away", async () => {
    resetStore([thread("gone")]);
    fakeMail({ threads: vi.fn(async () => [thread("new", { lastMessageAt: 3, partial: true })]) });
    await reloadThreads();
    expect(s().cursorId).toBe("new");
    // A newer copy of a loaded thread is taken as it comes.
    resetStore([thread("x", { lastMessageAt: 1, partial: false })]);
    fakeMail({ threads: vi.fn(async () => [thread("x", { lastMessageAt: 2, partial: true })]) });
    await reloadThreads();
    expect(s().threads[0].partial).toBe(true);
  });
});

describe("engine events", () => {
  it("new mail reloads the list once the batch settles", async () => {
    vi.useFakeTimers();
    const { backend } = fakeMail();
    handleEvent({ type: "mailChanged", accountId: "work" });
    handleEvent({ type: "mailChanged", accountId: "work" });
    await vi.advanceTimersByTimeAsync(300);
    expect(backend.threads).toHaveBeenCalledTimes(1);
  });

  it("new mail also reloads the Done list while it is on screen", async () => {
    vi.useFakeTimers();
    const { backend } = fakeMail({ doneThreads: vi.fn(async () => [thread("x", { doneAt: 1 })]) });
    handleEvent({ type: "mailChanged", accountId: "work" });
    await vi.advanceTimersByTimeAsync(300);
    expect(backend.doneThreads).not.toHaveBeenCalled();
    useStore.setState({ view: "done" });
    handleEvent({ type: "mailChanged", accountId: "work" });
    await vi.advanceTimersByTimeAsync(300);
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x"]);
  });

  it("new mail also reloads the Blocked list while it is on screen", async () => {
    vi.useFakeTimers();
    const { backend } = fakeMail({ blockedThreads: vi.fn(async () => [thread("x", { sender: "blocked" as const })]) });
    handleEvent({ type: "mailChanged", accountId: "work" });
    await vi.advanceTimersByTimeAsync(300);
    expect(backend.blockedThreads).not.toHaveBeenCalled();
    useStore.setState({ view: "blocked" });
    handleEvent({ type: "mailChanged", accountId: "work" });
    await vi.advanceTimersByTimeAsync(300);
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["x"]);
  });

  it("says at launch, and when the engine finds one later, that an email you sent didn't go", async () => {
    const { backend } = fakeMail({ interruptedSends: vi.fn().mockResolvedValueOnce([{ id: "a", subject: "At launch", at: 1 }]).mockResolvedValueOnce([{ id: "b", subject: "Later", at: 2 }]) });
    const stop = await startLive();
    await vi.waitFor(() => expect(s().toast?.text).toBe("Moss stopped before sending “At launch”. It is back in Drafts."));
    handleEvent({ type: "sendsInterrupted" });
    await vi.waitFor(() => expect(s().toast?.text).toBe("Moss stopped before sending “Later”. It is back in Drafts."));
    expect(backend.dismissInterruptedSends).toHaveBeenCalledTimes(2);
    stop();
  });

  it("drafts from the server reload the Drafts list", async () => {
    const kept = { id: "server:p1", mode: "new" as const, fromAccountId: "work", to: [], cc: [], bcc: [], subject: "Lunch?", body: "", bodyHtml: "", fieldsOpen: true, showCc: false, showBcc: false, remindIfNoReply: false, signatureId: null, updatedAt: 1 };
    const { backend } = fakeMail({ drafts: vi.fn(async () => [kept]) });
    handleEvent({ type: "draftsChanged", accountId: "work" });
    await vi.waitFor(() => expect(useDrafts.getState().drafts).toEqual([kept]));
    expect(backend.drafts).toHaveBeenCalledTimes(1);
  });

  it("progress fills the Syncing screen", () => {
    handleEvent({ type: "syncProgress", accountId: "work", done: 200, folders: 4, recent: 0.4, oldestYear: 2026, more: true });
    expect(useSync.getState().progress.work).toEqual({ done: 200, total: 400, folders: 4, recent: 0.4, oldestYear: 2026 });
  });

  it("a refused password marks the account signed out until sync works again", () => {
    const connected = { ...work, connection: { provider: "gmail" as const, summary: "Gmail · App password" } };
    resetStore([], [connected, personal]);
    handleEvent({ type: "syncError", accountId: "work", problem: "signedOut", message: "invalid credentials" });
    expect(s().accounts[0].connection?.signedOut).toBe(true);
    expect(s().accounts[1].connection).toBeUndefined();
    expect(useSync.getState().problems.work).toEqual({ problem: "signedOut", message: "invalid credentials" });
    handleEvent({ type: "syncError", accountId: "personal", problem: "offline", message: "no network" });
    expect(useSync.getState().problems.personal.problem).toBe("offline");
    handleEvent({ type: "syncOk", accountId: "work" });
    expect(s().accounts[0].connection?.signedOut).toBe(false);
    expect(useSync.getState().problems.work).toBeUndefined();
  });

  it("an updated account replaces the one shown", () => {
    resetStore([], [work, personal]);
    handleEvent({
      type: "accountUpdated",
      account: { id: "work", name: "Work", letter: "W", address: work.address, senderName: "Maya R. Ross", color: "work", provider: "gmail", connection: null },
    });
    expect(s().accounts[0].senderName).toBe("Maya R. Ross");
    expect(s().accounts[1]).toEqual(personal);
  });
});

describe("send later", () => {
  it("loads what waits to send at launch, and hears when one goes", async () => {
    const draft = { id: "d1", mode: "new", fromAccountId: "work", to: [], cc: [], bcc: [], subject: "Lunch?", body: "", bodyHtml: "", fieldsOpen: true, showCc: false, showBcc: false, remindIfNoReply: false, signatureId: null, updatedAt: 1 } as const;
    const email = { revision: 1, operationId: "op-d1", draft: { ...draft, to: [] as never[], cc: [] as never[], bcc: [] as never[] }, sendAt: 5, nextTry: 5, tries: 0 };
    const { backend } = fakeMail({ accounts: vi.fn(async () => [work]), scheduled: vi.fn(async () => [email]) });
    const stop = await startLive();
    expect(useScheduled.getState().emails).toEqual([email]);
    handleEvent({ type: "scheduledWaiting", email: { ...email, error: "Waiting for recovery" } });
    expect(useScheduled.getState().emails[0]).toMatchObject({ tries: 0, error: "Waiting for recovery" });
    handleEvent({ type: "scheduledSent", email, late: false });
    expect(useScheduled.getState().emails).toEqual([]);
    expect(s().toast).toMatchObject({ text: "Sent “Lunch?”" });
    handleEvent({ type: "scheduledFailed", email: { ...email, tries: 2, error: "offline" } });
    expect(useScheduled.getState().emails).toEqual([]); // A delayed failure cannot revive sent mail.
    const failed = { ...email, operationId: "op-d2", draft: { ...email.draft, id: "d2" }, tries: 2, error: "offline" };
    vi.mocked(backend.scheduled).mockResolvedValueOnce([failed]);
    handleEvent({ type: "scheduledFailed", email: failed });
    expect(useScheduled.getState().emails).toEqual([]);
    await vi.waitFor(() => expect(useScheduled.getState().emails).toEqual([failed]));
    vi.mocked(backend.scheduled).mockResolvedValueOnce([]);
    handleEvent({ type: "scheduledChanged" });
    await vi.waitFor(() => expect(useScheduled.getState().emails).toEqual([]));
    stop();
    useScheduled.setState({ emails: [] });
  });
});

describe("preloading", () => {
  it("fetches the bodies of the thread under the cursor and the next few, each once, as the cursor moves", async () => {
    const list = ["a", "b", "c", "d", "e", "f"].map((id, i) => thread(id, { lastMessageAt: 100 - i, section: "reply" }));
    const { backend } = fakeMail({ accounts: vi.fn(async () => [work]), threads: vi.fn(async () => list) });
    const stop = await startLive();
    expect(backend.preload).toHaveBeenLastCalledWith(["a", "b", "c", "d"]);
    s().moveCursor(1);
    expect(backend.preload).toHaveBeenLastCalledWith(["e"]);
    s().moveCursor(1);
    s().moveCursor(-1);
    expect(backend.preload).toHaveBeenCalledTimes(3);
    expect(backend.preload).toHaveBeenLastCalledWith(["f"]);
    stop();
    s().moveCursor(1);
    expect(backend.preload).toHaveBeenCalledTimes(3);
  });

  it("starts from the top when the cursor isn't in the list", () => {
    const { backend } = fakeMail();
    resetStore([thread("a"), thread("b")]);
    useStore.setState({ cursorId: "gone" });
    preloadAround(s(), 1);
    expect(backend.preload).toHaveBeenCalledWith(["a", "b"]);
  });
});

it("opens the inbox while Scheduled is slow and recovers after its deadline", async () => {
  vi.useFakeTimers();
  let late!: (rows: never[]) => void;
  const { backend } = fakeMail({ accounts: vi.fn(async () => [work]), scheduled: vi.fn().mockImplementationOnce(() => new Promise<never[]>((yes) => { late = yes; })).mockResolvedValue([]) });
  let stop: (() => void) | undefined;
  void startLive().then((done) => { stop = done; });
  await vi.advanceTimersByTimeAsync(0);
  expect(s().accounts).toEqual([work]);
  expect(s().view).toBe("inbox");
  expect(stop).toBeTypeOf("function");
  await vi.advanceTimersByTimeAsync(10_001);
  expect(backend.scheduled).toHaveBeenCalledTimes(2);
  late([]);
  await vi.runAllTimersAsync();
  expect(useScheduled.getState().emails).toEqual([]);
  stop?.();
});

it("hears schedule changes while the initial inbox data is loading", async () => {
  vi.useFakeTimers();
  const waiting = { operationId: "startup-gap", revision: 1, draft: draft("startup-gap"), sendAt: 5, nextTry: 5, tries: 0 };
  const connection = fakeMail({ accounts: vi.fn(async () => [work]), scheduled: vi.fn().mockResolvedValueOnce([waiting]).mockResolvedValue([]), contacts: vi.fn(async () => {
    connection.emit({ type: "scheduledChanged" });
    return [];
  }) });
  const stop = await startLive();
  await vi.runAllTimersAsync();
  expect(connection.backend.scheduled).toHaveBeenCalledTimes(2);
  expect(useScheduled.getState().emails).toEqual([]);
  stop();
});

it.each(["contacts", "syncProblems"] as const)("removes the early event listener if %s prevents startup", async (method) => {
  const failure = new Error("Startup query failed");
  const connection = fakeMail({ accounts: vi.fn(async () => [work]), [method]: vi.fn(async () => { throw failure; }) });
  await expect(startLive()).rejects.toBe(failure);
  expect(connection.listening()).toBe(0);
});

it.each(["syncOk", "syncError"] as const)("does not replay a stale sync problem over newer %s", async (type) => {
  vi.useFakeTimers();
  let snapshot!: (issues: { accountId: string; problem: "signedOut"; message: string }[]) => void;
  const connection = fakeMail({ accounts: vi.fn(async () => [work]), syncProblems: vi.fn(() => new Promise<{ accountId: string; problem: "signedOut"; message: string }[]>((yes) => { snapshot = yes; })) });
  const starting = startLive();
  await vi.advanceTimersByTimeAsync(0);
  connection.emit(type === "syncOk" ? { type, accountId: "work" } : { type, accountId: "work", problem: "offline", message: "New offline failure" });
  snapshot([{ accountId: "work", problem: "signedOut", message: "Old failure" }]);
  const stop = await starting;
  expect(useSync.getState().problems.work).toEqual(type === "syncOk" ? undefined : { problem: "offline", message: "New offline failure" });
  expect(s().accounts[0].connection?.signedOut).not.toBe(true);
  stop();
});

it("clears sync problems from a previous live session", async () => {
  useSync.setState({ problems: { work: { problem: "signedOut", message: "Old session" } }, progress: {} });
  fakeMail({ accounts: vi.fn(async () => [work]), syncProblems: vi.fn(async () => []) });
  const stop = await startLive();
  expect(useSync.getState().problems).toEqual({});
  stop();
});


it("loads retained server status separately from transient sync problems and refreshes on events", async () => {
  const retained = { accountId: "work", change: "0123456789abcdef0123456789abcdef", revision: 1, state: "blocked" as const, failure: "unreadable" as const,
    kind: "legacy" as const, completed: 0, pending: 1, discarded: 0, dropped: 0 };
  const get = vi.fn().mockResolvedValueOnce(retained).mockResolvedValueOnce(null);
  const { emit } = fakeMail({ accounts: vi.fn(async () => [work]), serverChangeStatus: get });
  const stop = await startLive();
  expect(useServerChanges.getState().byAccount.work).toEqual({ status: retained, unavailable: false });
  emit({ type: "syncOk", accountId: "work" });
  expect(useServerChanges.getState().byAccount.work.status).toEqual(retained);
  emit({ type: "serverChangesChanged", accountId: "work" });
  await vi.waitFor(() => expect(useServerChanges.getState().byAccount.work.status).toBeNull());
  stop();
});

it("a new live session ignores a pending removal from the previous session", async () => {
  let finish!: () => void;
  fakeMail({ accounts: vi.fn(async () => [work]), removeAccount: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finish = () => resolve({ pending: false }); })) });
  const pending = s().removeAccount("work");
  const stop = await startLive();
  expect(s().accountRemovals).toEqual({});
  finish();
  expect(await pending).toBe(false);
  expect(s().accounts).toEqual([work]);
  stop();
});

it("invalidates old removal responses before loading the new live session", async () => {
  let finishRemoval!: () => void;
  let finishAccounts!: (value: typeof work[]) => void;
  fakeMail({ accounts: vi.fn(() => new Promise<typeof work[]>(resolve => { finishAccounts = resolve; })), removeAccount: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finishRemoval = () => resolve({ pending: false }); })) });
  const removal = s().removeAccount("work");
  const loading = startLive();
  finishRemoval();
  expect(await removal).toBe(false);
  expect(s().accounts).toContainEqual(work);
  finishAccounts([work]);
  const stop = await loading;
  expect(s().accounts).toEqual([work]);
  stop();
});

it("loads pending credential cleanup when the live session starts without accounts", async () => {
  const { backend } = fakeMail({ credentialCleanupStatus: vi.fn(async () => ({ pending: true })) });
  const stop = await startLive();
  expect(backend.credentialCleanupStatus).toHaveBeenCalledOnce();
  expect(useCredentialCleanup.getState()).toEqual({ pending: true, busy: false, unavailable: false });
  expect(s().view).toBe("add-account");
  stop();
});
