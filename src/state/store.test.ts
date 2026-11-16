import { useRemoteImages } from "./remoteImages";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ai, setAi } from "../services/ai";
import type { Person, Thread } from "../data/types";
import type { CancelSendOutcome, MailBackend, Sent } from "../services/mail";
import { fakeMail } from "../test/fakeMail";
import { draft, jonas, lena, maya, message, person, personal, resetStore, thread, work } from "../test/fixtures";
import { useSignatures } from "./signatures";
import { useDrafts } from "./drafts";
import { useSearch } from "./search";
import type { SearchHit } from "../lib/search";
import { sendBeatMs, signaturePick, undoSendMs } from "./store";
import { accountCountsFor, accountFor, unreadCountsFor, contactsFrom, findThread, loadBlocked, loadDone, loadThread, openThread, progressFor, reloadThreads, shownView, splitCountsFor, useStore, visibleBlocked, visibleDone, visibleThreads } from "./store";

const s = () => useStore.getState();

describe("replying where you have a draft", () => {
  beforeEach(() => resetStore([thread("a")]));
  afterEach(() => useDrafts.setState({ drafts: [] }));

  it("Reply and Reply all open the draft you have there; Forward starts a new email", () => {
    const store = () => useStore.getState();
    useDrafts.setState({ drafts: [draft("d1", { mode: "reply", threadId: "a", body: "Half written" })] });
    store().compose("reply", "a");
    expect(store().draft).toMatchObject({ id: "d1", body: "Half written" });
    store().compose("reply-all", "a");
    expect(store().draft?.id).toBe("d1");
    store().compose("forward", "a");
    expect(store().draft).toMatchObject({ mode: "forward" });
    expect(store().draft?.id).not.toBe("d1");
    // A forward you left there isn't a reply.
    useDrafts.setState({ drafts: [draft("f1", { mode: "forward", threadId: "a" })] });
    store().compose("reply", "a");
    expect(store().draft?.id).not.toBe("f1");
  });
});

describe("visibleThreads", () => {
  const threads = [
    thread("later", { section: "later" }),
    thread("info", { section: "info" }),
    thread("reply", { section: "reply" }),
    thread("home", { accountId: "personal" }),
    thread("news", { split: "newsletters" }),
  ];

  it("shows the current split for all accounts, replies first and snoozed last", () => {
    const ids = visibleThreads({ threads, accountFilter: "all", split: "important" }).map((t) => t.id);
    expect(ids).toEqual(["reply", "home", "info", "later"]);
  });

  it("narrows to one account", () => {
    const ids = visibleThreads({ threads, accountFilter: "personal", split: "important" }).map((t) => t.id);
    expect(ids).toEqual(["home"]);
  });
});

describe("splitCountsFor", () => {
  it("counts every email waiting in each tab, For your info included", () => {
    const threads = [
      thread("a"),
      thread("b", { section: "info" }),
      thread("c", { split: "receipts", section: "info" }),
      thread("d", { split: "receipts", section: "info", accountId: "personal" }),
    ];
    expect(splitCountsFor(threads, "all")).toEqual({ important: 2, other: 0, calendar: 0, newsletters: 0, receipts: 2 });
    expect(splitCountsFor(threads, "work").receipts).toBe(1);
  });

  /** Same cases as `split_counts` in moss-store's query.rs. */
  it("leaves out reminded mail in every tab until its time comes", () => {
    const threads = [
      thread("a"),
      thread("fyi", { section: "info" }),
      thread("later", { section: "later" }),
      thread("weekly", { split: "newsletters", section: "info" }),
      thread("monthly", { split: "newsletters", section: "later" }),
      thread("receipt", { split: "receipts", section: "info" }),
    ];
    expect(splitCountsFor(threads, "all")).toEqual({ important: 2, other: 0, calendar: 0, newsletters: 1, receipts: 1 });
  });
});

describe("unreadCountsFor", () => {
  it("counts unread mail in each split, leaving out snoozed mail and other accounts", () => {
    const threads = [
      thread("a", { unread: true }),
      thread("b", { unread: true, section: "later" }),
      thread("c", { unread: true, split: "newsletters", section: "info" }),
      thread("d", { unread: true, split: "newsletters", section: "info", accountId: "personal" }),
      thread("e", { split: "newsletters", section: "info" }),
    ];
    expect(unreadCountsFor(threads, "all")).toEqual({ important: 1, other: 0, calendar: 0, newsletters: 2, receipts: 0 });
    expect(unreadCountsFor(threads, "work").newsletters).toBe(1);
  });
});

describe("accountCountsFor", () => {
  it("counts each account by the same rule as the split tabs, and all of them together", () => {
    const threads = [
      thread("reply"),
      thread("fyi", { section: "info" }),
      thread("snoozed", { section: "later" }),
      thread("news", { split: "newsletters", section: "info", accountId: "personal" }),
      thread("bill", { split: "receipts", section: "info", accountId: "personal" }),
    ];
    expect(accountCountsFor(threads, ["work", "personal", "empty"])).toEqual({ all: 4, work: 2, personal: 2, empty: 0 });
  });
});

describe("progressFor", () => {
  const now = new Date(2026, 10, 2, 15, 0).getTime();
  const earlierToday = new Date(2026, 10, 2, 8, 0).getTime();
  const yesterday = new Date(2026, 10, 1, 23, 0).getTime();
  const threads = [thread("a"), thread("b", { split: "newsletters", section: "info", accountId: "personal" })];
  const done = [
    thread("d1", { doneAt: earlierToday }),
    thread("d2", { doneAt: earlierToday, accountId: "personal" }),
    thread("d3", { doneAt: yesterday }),
    thread("d4"),
  ];

  it("counts what was done today against what is still waiting", () => {
    expect(progressFor(threads, done, "all", now)).toEqual({ done: 2, total: 4, eta: "2 left" });
    expect(progressFor(threads, done, "personal", now)).toEqual({ done: 1, total: 2, eta: "1 left" });
  });

  it("isn't inbox zero while an email for your info still waits", () => {
    expect(progressFor([thread("fyi", { section: "info" })], [], "all", now)).toEqual({ done: 0, total: 1, eta: "1 left" });
  });

  it("says so at inbox zero", () => {
    expect(progressFor([], done, "work", now)).toEqual({ done: 1, total: 1, eta: "Inbox zero" });
    expect(progressFor([], [], "all", now)).toEqual({ done: 0, total: 0, eta: "Inbox zero" });
  });
});

describe("contactsFrom", () => {
  const dan = person("Dan Ito", "Dan@x.io");
  const ana = person("Ana Lee", "ana@y.io");
  const bot = person("Shop", "no-reply@shop.example");
  const bot2 = person("Bank", "noreply@bank.example");

  it("lists the people in your mail, most frequent first, once each, without you or no-reply senders", () => {
    const threads = [
      thread("a", { messages: [message({ from: dan, to: [maya], cc: [ana] }), message({ from: maya, to: [person("Dan", "dan@x.io")] })] }),
      thread("b", { messages: [message({ from: bot, to: [maya] }), message({ from: bot2, to: [maya] })] }),
    ];
    expect(contactsFrom(threads, [work]).map((p) => p.email)).toEqual(["Dan@x.io", "ana@y.io"]);
  });

  it("puts the people Moss knows from all your mail first, then the rest, each once", () => {
    const threads = [thread("a", { messages: [message({ from: dan, to: [maya], cc: [ana] })] })];
    const known = [person("Priya", "priya@acme.co"), person("Dan", "dan@x.io")];
    expect(contactsFrom(threads, [work], known).map((p) => p.email)).toEqual(["priya@acme.co", "dan@x.io", "ana@y.io"]);
  });
});

describe("findThread", () => {
  // The lists given, and ones that fail the test if they're read: findThread runs in selectors on every store change.
  type Lists = Parameters<typeof findThread>[0];
  const only = (given: Partial<Lists> = {}) =>
    Object.defineProperties({} as Lists, Object.fromEntries((["threads", "doneThreads", "blockedThreads", "foundThreads"] as const).map((k) => [k, { get: () => given[k] ?? (() => { throw new Error(`read ${k}`); })() }])));

  it("looks in no list without an id", () => {
    expect(findThread(only(), undefined)).toBeUndefined();
  });

  it("stops at the first list that holds it, in order: inbox, Done, Blocked, then opened from Search", () => {
    const a = thread("a");
    expect(findThread(only({ threads: [a] }), "a")).toBe(a);
    expect(findThread(only({ threads: [], doneThreads: [a] }), "a")).toBe(a);
    expect(findThread({ threads: [], doneThreads: [], blockedThreads: [], foundThreads: [a] }, "a")).toBe(a);
    expect(findThread({ threads: [thread("b")], doneThreads: [], blockedThreads: [], foundThreads: [] }, "a")).toBeUndefined();
  });
});

describe("shownView", () => {
  it("falls back to the inbox when there is nothing to draw", () => {
    expect(shownView({ blockedThreads: [], foundThreads: [], view: "thread", threads: [], doneThreads: [], openThreadId: "gone" })).toBe("inbox");
    expect(shownView({ blockedThreads: [], foundThreads: [], view: "compose", threads: [], doneThreads: [], draft: undefined })).toBe("inbox");
    expect(shownView({ blockedThreads: [], foundThreads: [], view: "thread", threads: [thread("a")], doneThreads: [], openThreadId: "a" })).toBe("thread");
    expect(shownView({ blockedThreads: [], foundThreads: [], view: "thread", threads: [], doneThreads: [thread("a")], openThreadId: "a" })).toBe("thread");
    // One opened that no list holds (only you sent it) can be drawn too.
    expect(shownView({ blockedThreads: [], foundThreads: [thread("a")], view: "thread", threads: [], doneThreads: [], openThreadId: "a" })).toBe("thread");
  });
});

describe("moving around", () => {
  beforeEach(() => resetStore([thread("a"), thread("b"), thread("c"), thread("n", { split: "newsletters" })]));

  it("moves the cursor and stops at the ends", () => {
    s().moveCursor(-1);
    expect(s().cursorId).toBe("a");
    s().moveCursor(1);
    s().moveCursor(1);
    s().moveCursor(1);
    expect(s().cursorId).toBe("c");
  });

  it("moves to the next email while reading", () => {
    s().open("a");
    s().moveCursor(1);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b", cursorId: "b" });
  });

  it("opens the email under the cursor and goes back to the list", () => {
    s().setCursor("b");
    s().open();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b" });
    s().back();
    expect(s().view).toBe("inbox");
  });

  it("cycles split tabs in both directions, wrapping around", () => {
    s().cycleSplit(-1);
    expect(s().split).toBe("receipts");
    s().cycleSplit(1);
    expect(s().split).toBe("important");
    s().setSplit("newsletters");
    expect(s().cursorId).toBe("n");
  });

  it("picking an account returns to the inbox at its first email", () => {
    resetStore([thread("a"), thread("home", { accountId: "personal" })]);
    s().open("a");
    s().setAccountFilter("personal");
    expect(s()).toMatchObject({ view: "inbox", accountFilter: "personal", cursorId: "home" });
  });
});

describe("markDone", () => {
  beforeEach(() => resetStore([thread("a"), thread("b"), thread("c")]));

  it("removes the email, counts it, and keeps the cursor in place", () => {
    s().setCursor("b");
    s().markDone();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "c"]);
    expect(s()).toMatchObject({ cursorId: "c", handled: 1, view: "inbox" });
  });

  it("moves on to the next email while reading", () => {
    s().open("b");
    s().markDone();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "c" });
  });

  it("moves up after the bottom email while reading", () => {
    s().open("c");
    s().markDone();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b" });
  });

  it("goes back to the list when the split has nothing left to read", () => {
    resetStore([thread("n", { split: "newsletters" })]);
    s().setSplit("newsletters");
    s().open("n");
    s().markDone();
    expect(s()).toMatchObject({ view: "inbox", cursorId: undefined, openThreadId: undefined });
  });

  it("reaches inbox zero when nothing in Important is waiting", () => {
    resetStore([thread("a"), thread("snoozed", { section: "later" })]);
    s().markDone("a");
    expect(s().view).toBe("zero");
  });

  it("shows the empty list, not inbox zero, in other splits", () => {
    resetStore([thread("n", { split: "newsletters" })]);
    s().setSplit("newsletters");
    s().markDone("n");
    expect(s().view).toBe("inbox");
  });
});

describe("deleteThread", () => {
  beforeEach(() => {
    resetStore([thread("a"), thread("b"), thread("c")]);
    useStore.setState({ doneThreads: [thread("x", { doneAt: 300 }), thread("y", { doneAt: 200 })] });
  });

  it("moves the email to Trash, not to Done, and keeps the cursor in place", () => {
    s().setCursor("b");
    s().deleteThread();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "c"]);
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x", "y"]);
    expect(s()).toMatchObject({ cursorId: "c", handled: 1, view: "inbox", toast: { text: "Moved to Trash" } });
  });

  it("moves on to the next email while reading", () => {
    s().open("b");
    s().deleteThread();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "c" });
  });

  it("reaches inbox zero when nothing in Important is waiting", () => {
    resetStore([thread("a")]);
    s().deleteThread("a");
    expect(s().view).toBe("zero");
  });

  it("takes an email out of the Done list, onto the next done one", () => {
    s().openDone();
    s().deleteThread();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["y"]);
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(s()).toMatchObject({ view: "done", doneCursorId: "y", handled: 0, toast: { text: "Moved to Trash" } });
  });

  it("reading a done email moves on to the next one, then back to the Done list", () => {
    s().openDone();
    s().open("x");
    s().deleteThread();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "y", doneCursorId: "y" });
    s().deleteThread();
    expect(s()).toMatchObject({ view: "done", doneThreads: [] });
  });

  it("does nothing for blocked mail, or without an email", () => {
    useStore.setState({ blockedThreads: [thread("z", { sender: "blocked" })] });
    s().deleteThread("z");
    s().deleteThread("nope");
    useStore.setState({ cursorId: undefined });
    s().deleteThread();
    expect(s().threads).toHaveLength(3);
    expect(s().blockedThreads).toHaveLength(1);
    expect(s().undoSteps).toEqual([]);
    expect(s().toast).toBeUndefined();
  });
});

describe("deleteThread from Search", () => {
  /** A search result for the thread `threadId`. */
  const found = (threadId: string): SearchHit => ({
    doc: { id: `m-${threadId}`, threadId, accountId: "work", from: maya, subject: threadId, summary: "", when: "Aug 30", date: "2023-08-30", hasAttachment: false, relevance: 1 },
    subject: [],
    summary: [],
    best: false,
  });
  const hits = () => useSearch.getState().hits.map((h) => h.doc.threadId);
  const pristine = useSearch.getState();

  beforeEach(() => {
    resetStore([thread("a"), thread("b")]);
    useStore.setState({ doneThreads: [thread("x", { doneAt: 300 })] });
    useSearch.setState(pristine, true);
    s().openSearch();
  });

  it("deletes a result no list holds, such as one only you sent; ⌘Z brings it back to the results", () => {
    const { backend } = fakeMail();
    const sent = found("sent");
    // Search took it out of the results itself.
    useSearch.setState({ hits: [found("b")], cursorId: "m-b" });
    s().deleteThread("sent", { hit: sent, index: 0 });
    expect(backend.deleteThread).toHaveBeenCalledWith("sent");
    expect(s()).toMatchObject({ view: "search", toast: { text: "Moved to Trash" } });
    expect(s().undoSteps.at(-1)).toMatchObject({ kind: "deletedFound", thread: { id: "sent" }, view: "search" });
    s().undo();
    expect(backend.restoreThread).toHaveBeenCalledWith("sent");
    expect(hits()).toEqual(["sent", "b"]);
    expect(useSearch.getState().cursorId).toBe("m-sent");
    expect(s()).toMatchObject({ view: "search", undoSteps: [], toast: undefined });
  });

  it("an inbox or done result leaves its list too, and ⌘Z puts it back in both, staying in Search", () => {
    const { backend } = fakeMail();
    s().deleteThread("b", { hit: found("b"), index: 1 });
    s().deleteThread("x", { hit: found("x"), index: 0 });
    expect(s().threads.map((t) => t.id)).toEqual(["a"]);
    expect(s().doneThreads).toEqual([]);
    expect(s().view).toBe("search");
    s().undo();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x"]);
    expect(hits()).toEqual(["x"]);
    s().undo();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
    expect(hits()).toEqual(["x", "b"]);
    expect(backend.restoreThread).toHaveBeenCalledTimes(2);
    expect(s().view).toBe("search");
  });

  it("a result already back in the results isn't added twice; one put back past the end goes last", () => {
    useSearch.setState({ hits: [found("sent")] });
    s().deleteThread("sent", { hit: found("sent"), index: 0 });
    s().undo();
    expect(hits()).toEqual(["sent"]);
    useSearch.setState({ hits: [] });
    s().deleteThread("sent", { hit: found("sent"), index: 4 });
    useSearch.setState({ hits: [found("b")] });
    s().undo();
    expect(hits()).toEqual(["b", "sent"]);
  });

  it("an email opened from Search that no list holds goes back to Search once deleted; ⌘Z keeps you there", () => {
    const { backend } = fakeMail();
    useStore.setState({ foundThreads: [thread("sent")] });
    s().open("sent");
    s().deleteThread();
    expect(backend.deleteThread).toHaveBeenCalledWith("sent");
    expect(s().foundThreads).toEqual([]);
    expect(s()).toMatchObject({ view: "search", toast: { text: "Moved to Trash" } });
    s().undo();
    expect(backend.restoreThread).toHaveBeenCalledWith("sent");
    expect(s().view).toBe("search");
  });

  it("still does nothing for blocked mail, or an email that isn't there", () => {
    useStore.setState({ blockedThreads: [thread("z", { sender: "blocked" })] });
    s().deleteThread("z", { hit: found("z"), index: 0 });
    s().deleteThread("nope");
    expect(s().undoSteps).toEqual([]);
  });
});

describe("remindLater", () => {
  it("snoozes the email until tomorrow and clears its due date", () => {
    resetStore([thread("a", { due: "Today" }), thread("b")]);
    s().remindLater("a");
    expect(s().threads[0]).toMatchObject({ section: "later", time: "Tomorrow", due: undefined });
    expect(s()).toMatchObject({ view: "inbox", cursorId: "b" });
  });

  it("keeps your place: the selection moves to the next email, not back to the top", () => {
    resetStore([thread("a"), thread("b"), thread("c")]);
    s().setCursor("b");
    s().remindLater();
    expect(s().cursorId).toBe("c");
  });

  it("snoozing the last waiting email selects the one above it", () => {
    resetStore([thread("a"), thread("b"), thread("c")]);
    s().setCursor("c");
    s().remindLater();
    expect(s().cursorId).toBe("b");
  });

  it("reaches inbox zero when the last waiting email is snoozed", () => {
    resetStore([thread("a")]);
    s().remindLater("a");
    expect(s().view).toBe("zero");
  });

  describe("at a time you pick", () => {
    const tuesday = new Date(2026, 10, 3, 13, 20).getTime();
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(tuesday);
    });
    afterEach(() => vi.useRealTimers());

    it("snoozes until then, shows when it comes back, and says so", () => {
      const { backend } = fakeMail();
      resetStore([thread("a"), thread("b")]);
      const saturday = new Date(2026, 10, 7, 8, 0).getTime();
      s().remindLater("a", saturday);
      expect(backend.snooze).toHaveBeenCalledWith("a", saturday);
      expect(s().threads[0]).toMatchObject({ section: "later", time: "Sat" });
      expect(s().toast).toMatchObject({ text: "Coming back Sat 8 AM" });
    });

    it("is tomorrow at 8 AM when no time is given", () => {
      const { backend } = fakeMail();
      resetStore([thread("a")]);
      s().remindLater("a");
      expect(backend.snooze).toHaveBeenCalledWith("a", new Date(2026, 10, 4, 8, 0).getTime());
      expect(s().toast).toMatchObject({ text: "Coming back Wed 8 AM" });
    });

    it("later today shows the hour it comes back", () => {
      resetStore([thread("a")]);
      s().remindLater("a", new Date(2026, 10, 3, 17, 0).getTime());
      expect(s().threads[0].time).toBe("5:00 PM");
      expect(s().toast).toMatchObject({ text: "Coming back 5 PM" });
    });
  });
});

describe("correcting what Moss decided", () => {
  const DAY = 86_400_000;

  it("moves an email between Needs your reply and For your information, dropping the AI's reason", () => {
    resetStore([thread("a", { aiSure: 91 }), thread("b")]);
    const { backend } = fakeMail();
    s().moveToSection("info", "a");
    expect(s().threads[0]).toMatchObject({ section: "info", aiSure: undefined });
    expect(s().toast?.text).toBe("Moved to For your information");
    expect(backend.setSection).toHaveBeenCalledWith("a", "info");
    useStore.setState({ view: "thread", openThreadId: "a" });
    s().moveToSection("reply");
    expect(s().threads[0].section).toBe("reply");
    expect(s().toast?.text).toBe("Moved to Needs your reply");
  });

  it("No deadline takes the due chip away", () => {
    resetStore([thread("a", { due: "Due Fri", askDue: "Due Fri", dueDate: "2026-11-06", dueWhy: { by: "ai", sure: 90 } })]);
    const { backend } = fakeMail();
    s().clearDeadline("a");
    expect(s().threads[0]).toMatchObject({ due: undefined, askDue: undefined, dueDate: undefined, dueWhy: undefined });
    expect(s().toast?.text).toBe("No deadline");
    expect(backend.clearDeadline).toHaveBeenCalledWith("a");
  });

  it("Remind me if no reply waits on your latest email, out of sight until then", () => {
    const now = Date.now();
    resetStore([thread("a", { youWroteLast: true, lastMessageAt: now - DAY }), thread("b")]);
    const { backend } = fakeMail();
    s().remindIfNoReply("a");
    const until = now - DAY + 3 * DAY;
    expect(s().threads[0]).toMatchObject({ section: "later", waiting: { until, why: { by: "you" } } });
    expect(s().toast?.text).toMatch(/^Back .* if Lena doesn’t reply$/);
    expect(backend.remindIfNoReply).toHaveBeenCalledWith("a", true);
    expect(s().cursorId).toBe("b");
    // Sent long ago: it waits from now.
    resetStore([thread("old", { youWroteLast: true, lastMessageAt: now - 9 * DAY })]);
    fakeMail();
    s().remindIfNoReply("old");
    expect(s().threads[0].waiting!.until).toBeGreaterThanOrEqual(now + 3 * DAY);
  });

  it("acts on the open email, and names whoever you wrote to", () => {
    const now = Date.now();
    resetStore([thread("a", { from: maya, youWroteLast: true, due: "Due Fri", dueDate: "2026-11-06" }), thread("b")]);
    fakeMail();
    useStore.setState({ view: "thread", openThreadId: "a" });
    s().clearDeadline();
    expect(s().threads[0].dueDate).toBeUndefined();
    s().remindIfNoReply();
    // Demo mail has no time: it waits from now.
    expect(s().threads[0].waiting!.until).toBeGreaterThanOrEqual(now + 3 * DAY);
    expect(s().toast?.text).toMatch(/if Maya Ross doesn’t reply$/);
    s().moveTo("other");
    expect(s().threads[0].split).toBe("other");
  });

  it("Don’t remind me stops waiting", () => {
    resetStore([thread("a", { section: "later", youWroteLast: true, waiting: { until: Date.now() + DAY, why: { by: "ai", sure: 90 } } })]);
    const { backend } = fakeMail();
    s().remindIfNoReply("a", false);
    expect(s().threads[0]).toMatchObject({ section: "info", waiting: undefined });
    expect(s().toast?.text).toBe("Moss won’t remind you");
    expect(backend.remindIfNoReply).toHaveBeenCalledWith("a", false);
  });

  it("someone else wrote last: there's nothing of yours to wait on", () => {
    resetStore([thread("a")]);
    const { backend } = fakeMail();
    s().remindIfNoReply("a");
    expect(s().threads[0].section).toBe("reply");
    expect(s().toast?.text).toBe("Reply first, then Moss can wait for an answer");
    expect(backend.remindIfNoReply).not.toHaveBeenCalled();
  });

  it("does nothing without an email", () => {
    resetStore([]);
    const { backend } = fakeMail();
    s().moveToSection("info");
    s().clearDeadline();
    s().remindIfNoReply();
    expect(backend.setSection).not.toHaveBeenCalled();
    expect(backend.clearDeadline).not.toHaveBeenCalled();
    expect(backend.remindIfNoReply).not.toHaveBeenCalled();
  });
});

describe("the Remind menu", () => {
  it("opens for the email you're on, and closes again", () => {
    resetStore([thread("a"), thread("b")]);
    s().setCursor("b");
    s().openRemind();
    expect(s().remindingId).toBe("b");
    s().closeRemind();
    expect(s().remindingId).toBeUndefined();
  });

  it("opens for the open email, or the one it is given", () => {
    resetStore([thread("a"), thread("b")]);
    s().open("a");
    s().openRemind();
    expect(s().remindingId).toBe("a");
    s().openRemind("b");
    expect(s().remindingId).toBe("b");
  });

  it("doesn't open with no email to remind you about", () => {
    resetStore([]);
    s().openRemind();
    expect(s().remindingId).toBeUndefined();
  });

  it("closes once you pick a time", () => {
    resetStore([thread("a"), thread("b")]);
    s().openRemind("a");
    s().remindLater("a", Date.now() + 3_600_000);
    expect(s().remindingId).toBeUndefined();
  });
});

describe("remote images", () => {
  const alex = person("Alex Morgan Studio Build", "alex@studio-build.example.test");
  const fromAlex = (id: string, accountId = "work") =>
    thread(id, { accountId, messages: [message({ id: `${id}-1`, from: { ...alex, email: "Alex@STUDIO-BUILD.example.test" } })] });

  it("just this once shows one message's images and remembers nothing", () => {
    resetStore([fromAlex("a")]);
    s().showImages("a-1");
    s().showImages("a-1");
    expect(s().imagesShown).toEqual(["a-1"]);
    expect(s().threads[0].messages[0].imageTrust).toBeUndefined();
  });

  it("showing images from a sender covers their other emails in that account, and is saved", () => {
    const { backend } = fakeMail();
    const mixed = thread("e", { messages: [message({ id: "e-1" }), message({ id: "e-2", from: alex })] });
    resetStore([fromAlex("a"), fromAlex("b"), fromAlex("c", "personal"), thread("d"), mixed]);
    s().allowImages("a-1");
    expect(s().threads.map((t) => t.messages[0].imageTrust)).toEqual(["you-allowed", "you-allowed", undefined, undefined, undefined]);
    expect(s().threads[4].messages[1].imageTrust).toBe("you-allowed");
    expect(backend.allowImages).toHaveBeenCalledWith("work", "Alex@STUDIO-BUILD.example.test");
    s().allowImages("missing");
    expect(backend.allowImages).toHaveBeenCalledOnce();
  });

  it("showing images from a sender covers their done emails too, even from a done email", () => {
    const { backend } = fakeMail();
    resetStore([fromAlex("a")]);
    useStore.setState({ doneThreads: [fromAlex("old"), thread("other")] });
    s().allowImages("old-1");
    expect(s().threads[0].messages[0].imageTrust).toBe("you-allowed");
    expect(s().doneThreads.map((t) => t.messages[0].imageTrust)).toEqual(["you-allowed", undefined]);
    expect(backend.allowImages).toHaveBeenCalledOnce();
  });
});

describe("compose", () => {
  const original = thread("q4", {
    subject: "Q4 budget",
    fullSubject: "Re: Q4 budget review",
    messages: [message({ to: [maya, jonas], cc: [maya, lena] })],
  });
  beforeEach(() => resetStore([original]));

  it("replies to the sender with Re: added once", () => {
    s().compose("reply", "q4");
    expect(s().view).toBe("compose");
    expect(s().draft).toMatchObject({
      mode: "reply",
      threadId: "q4",
      fromAccountId: "work",
      to: [lena],
      cc: [],
      subject: "Re: Q4 budget review",
      showCc: false,
    });
  });

  it("replies with one Re: however many the subject already has", () => {
    resetStore([thread("re", { subject: "Lunch", fullSubject: "Re: RE:re: Lunch" })]);
    s().compose("reply", "re");
    expect(s().draft?.subject).toBe("Re: Lunch");
  });

  it("reply-all includes everyone except you and shows CC", () => {
    s().compose("reply-all", "q4");
    expect(s().draft).toMatchObject({ to: [lena, jonas], cc: [lena], showCc: true });
  });

  it("forwards with nobody filled in", () => {
    s().compose("forward", "q4");
    expect(s().draft).toMatchObject({ to: [], subject: "Fwd: Q4 budget review" });
  });

  it("a new email sends from the account you are looking at", () => {
    s().compose("new");
    expect(s().draft).toMatchObject({ mode: "new", to: [], subject: "", fromAccountId: "work" });
    s().setAccountFilter("personal");
    s().compose("new");
    expect(s().draft?.fromAccountId).toBe("personal");
  });

  it("back from a reply returns to its thread", () => {
    s().compose("reply", "q4");
    s().back();
    expect(s().view).toBe("thread");
  });

  it("only patches a draft that exists", () => {
    s().updateDraft({ subject: "Ignored" });
    expect(s().draft).toBeUndefined();
    s().compose("new");
    s().updateDraft({ subject: "Hi" });
    expect(s().draft?.subject).toBe("Hi");
  });
});

describe("sending", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it.each(["reply", "reply-all", "forward"] as const)("a %s leaves its email in the inbox: only you mark it done", (mode) => {
    resetStore([thread("a"), thread("b")]);
    s().compose(mode, "a");
    s().updateDraft({ to: [jonas] });
    s().send();
    vi.advanceTimersByTime(sendBeatMs);
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", draft: undefined, handled: 0, doneThreads: [] });
  });

  it("a new email with no subject asks first; Send again, or a subject, lets it go", () => {
    resetStore([thread("a")]);
    s().compose("new");
    s().send();
    expect(s()).toMatchObject({ noSubjectAsked: true, view: "compose" });
    expect(s().sending).toBeFalsy();
    s().updateDraft({ body: "Hi" });
    expect(s().noSubjectAsked).toBe(true);
    s().updateDraft({ subject: "" });
    expect(s().noSubjectAsked).toBe(false);
    s().send();
    s().send();
    expect(s()).toMatchObject({ noSubjectAsked: false, sending: true });
    // The next email asks again.
    vi.advanceTimersByTime(sendBeatMs);
    s().compose("new");
    s().send();
    expect(s().noSubjectAsked).toBe(true);
    // A reply always has one.
    s().compose("reply", "a");
    expect(s().noSubjectAsked).toBe(false);
    s().updateDraft({ subject: "" });
    s().send();
    expect(s().sending).toBe(true);
  });

  it("a new email closes the composer", () => {
    resetStore([thread("a")]);
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    s().send();
    vi.advanceTimersByTime(sendBeatMs);
    expect(s()).toMatchObject({ view: "inbox", draft: undefined, handled: 0 });
    // With no draft there is nothing to send.
    s().send();
    expect(s().sending).toBeFalsy();
  });

  it("the people you send to are suggested next time, at once", () => {
    resetStore([thread("a")]);
    const priya = person("Priya", "priya@acme.co");
    useStore.setState({ knownPeople: [person("Dan", "dan@x.io")] });
    s().compose("new");
    s().updateDraft({ subject: "Lunch", to: [priya], cc: [person("Dan", "Dan@x.io")] });
    s().send();
    vi.advanceTimersByTime(sendBeatMs);
    expect(s().knownPeople.map((p) => p.email)).toEqual(["priya@acme.co", "Dan@x.io"]);
  });

  it("editing an instant reply opens it in the composer, fields collapsed", () => {
    resetStore([
      thread("a", { replyOptions: [{ label: "Yes", draft: "Sounds good." }, { label: "No", draft: "Can’t this week." }] }),
    ]);
    s().open("a");
    s().setReplyChoice(1);
    s().editInstantReply();
    expect(s().draft).toMatchObject({ mode: "reply", body: "Can’t this week.", fieldsOpen: false });
  });
});

describe("theme", () => {
  it("pins a choice, or follows the system again", () => {
    resetStore([]);
    s().setTheme("light");
    expect(s()).toMatchObject({ themeChoice: "light", theme: "light" });
    s().setTheme("system");
    expect(s().themeChoice).toBe("system");
  });
});

describe("accounts", () => {
  it("adds an account once", () => {
    resetStore([], [work]);
    s().addAccount(personal);
    s().addAccount(personal);
    expect(s().accounts.map((a) => a.id)).toEqual(["work", "personal"]);
  });

  it("falls back to the first account for an unknown id", () => {
    resetStore([]);
    expect(accountFor("personal").id).toBe("personal");
    expect(accountFor("gone").id).toBe("work");
  });
});

describe("instant replies", () => {
  const options = [
    { label: "Yes", draft: "Sounds good." },
    { label: "No", draft: "Can’t this week." },
  ];

  it("sending one sends the reply as you changed it, and leaves the open email where it is", () => {
    vi.useFakeTimers();
    resetStore([thread("a", { replyOptions: options }), thread("b")]);
    s().open("a");
    s().setReplyDraft("Sounds good, see you Thursday.");
    s().sendInstantReply();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", sending: true });
    vi.advanceTimersByTime(sendBeatMs);
    vi.useRealTimers();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", draft: undefined, replyDraft: undefined, toast: { text: "Sending" } });
    const step = s().undoSteps[0];
    expect(step).toMatchObject({ kind: "send", draft: { mode: "reply", threadId: "a", body: "Sounds good, see you Thursday." } });
    expect(step.kind === "send" && step.draft.bodyHtml).toBe("<p>Sounds good, see you Thursday.</p>");
    // Sending never marks it Done.
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
  });

  it("it goes from the address the email came to", () => {
    vi.useFakeTimers();
    const withStudio = { ...personal, addresses: [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 0, blocked: false }] };
    resetStore([thread("a", { accountId: "personal", toAddress: "hi@rossstudio.art", replyOptions: options })], [work, withStudio]);
    s().open("a");
    s().sendInstantReply();
    vi.advanceTimersByTime(sendBeatMs);
    vi.useRealTimers();
    expect(s().undoSteps[0]).toMatchObject({ draft: { fromAccountId: "personal", fromAddress: "hi@rossstudio.art", body: "Sounds good." } });
  });

  it("without a change, sends the chosen option; with nothing to send, does nothing", () => {
    vi.useFakeTimers();
    resetStore([thread("a", { replyOptions: options }), thread("b")]);
    s().open("b");
    s().sendInstantReply();
    expect(s().sending).toBeFalsy();
    s().open("a");
    s().setReplyChoice(1);
    s().sendInstantReply();
    vi.advanceTimersByTime(sendBeatMs);
    vi.useRealTimers();
    expect(s().undoSteps[0]).toMatchObject({ draft: { body: "Can’t this week." } });
  });

  it("choosing another option drops your changes", () => {
    resetStore([thread("a", { replyOptions: options })]);
    s().open("a");
    s().setReplyDraft("Mine");
    s().setReplyChoice(1);
    expect(s().replyDraft).toBeUndefined();
  });

  it("editing keeps your changes", () => {
    resetStore([thread("a", { replyOptions: options })]);
    s().open("a");
    s().setReplyDraft("Sounds good!");
    s().editInstantReply();
    expect(s().draft).toMatchObject({ mode: "reply", body: "Sounds good!", bodyHtml: "<p>Sounds good!</p>", fieldsOpen: false });
  });

  it("editing or sending goes with the signature you picked on the Quick reply, or none", () => {
    resetStore([thread("a", { replyOptions: options })]);
    s().open("a");
    s().compose("reply");
    const signature = s().draft?.signatureId;
    s().back();
    s().editInstantReply("studio");
    expect(s().draft).toMatchObject({ signatureId: "studio", signatureChosen: true });
    s().back();
    s().editInstantReply(null);
    expect(s().draft).toMatchObject({ signatureId: null, signatureChosen: true });
    s().back();
    s().editInstantReply();
    expect(s().draft?.signatureId).toBe(signature);
    s().back();
    s().sendInstantReply("studio");
    expect(s().draft).toMatchObject({ signatureId: "studio", signatureChosen: true });
  });

  it("editing without reply options opens a blank reply", () => {
    resetStore([thread("a")]);
    s().open("a");
    s().editInstantReply();
    expect(s().draft).toMatchObject({ mode: "reply", body: "", fieldsOpen: true });
  });
});

describe("nothing to act on", () => {
  it("open, done and snooze do nothing without an email", () => {
    resetStore([]);
    s().open();
    s().markDone();
    s().remindLater();
    s().moveCursor(1);
    expect(s()).toMatchObject({ view: "inbox", handled: 0 });
  });

  it("a reply with no email to reply to becomes a new email", () => {
    resetStore([]);
    s().compose("reply");
    expect(s().draft?.mode).toBe("new");
  });

  it("opens the palette, and has no font choice", () => {
    resetStore([]);
    s().setPaletteOpen(true);
    expect(s()).toMatchObject({ paletteOpen: true, paletteQuery: "" });
    expect(s()).not.toHaveProperty("font");
    expect(s()).not.toHaveProperty("setFont");
  });
});

describe("snoozing while reading", () => {
  it("snoozes the open email, not the one under the cursor", () => {
    resetStore([thread("a"), thread("b")]);
    s().open("b");
    s().setCursor("a");
    s().remindLater();
    expect(s().threads.find((t) => t.id === "b")?.section).toBe("later");
  });

  it("moves you on to the next email, like Done, instead of back to the list", () => {
    resetStore([thread("a"), thread("b"), thread("c")]);
    s().open("b");
    s().remindLater();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "c", cursorId: "c" });
    // The last one waiting: on to the one above.
    s().remindLater();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
  });

  it("reaches inbox zero once the last waiting email is snoozed", () => {
    resetStore([thread("a")]);
    s().open("a");
    s().remindLater();
    expect(s()).toMatchObject({ view: "zero", openThreadId: undefined, history: [] });
  });

  it("stays in Search when snoozed from there", () => {
    resetStore([thread("a"), thread("b")]);
    s().openSearch();
    s().remindLater("a");
    expect(s().view).toBe("search");
  });
});

describe("inbox zero counts every account", () => {
  const both = () => [thread("w"), thread("h", { accountId: "home" })];

  it("finishing one account while another still waits shows that account's empty list", () => {
    resetStore(both());
    s().setAccountFilter("work");
    s().markDone("w");
    expect(s().view).toBe("inbox");
    resetStore(both());
    s().setAccountFilter("work");
    s().remindLater("w");
    expect(s().view).toBe("inbox");
  });

  it("once you leave inbox zero yourself, handling what's left keeps you on the list", () => {
    resetStore([thread("a"), thread("l1", { section: "later" }), thread("l2", { section: "later" }), thread("l3", { section: "later" })]);
    s().markDone("a");
    expect(s().view).toBe("zero");
    s().back();
    s().markDone("l1");
    expect(s().view).toBe("inbox");
    s().deleteThread("l2");
    expect(s().view).toBe("inbox");
    s().open("l3");
    s().remindLater();
    expect(s().view).not.toBe("zero");
  });

  it("Z shows inbox zero when nothing waits, and says what does when something still does", () => {
    resetStore([thread("w"), thread("h", { accountId: "home" }), thread("l", { section: "later" })]);
    s().setAccountFilter("work");
    s().showInboxZero();
    expect(s()).toMatchObject({ view: "inbox", toast: { text: "2 emails still waiting" } });
    s().markDone("h");
    s().showInboxZero();
    expect(s().toast).toMatchObject({ text: "1 email still waiting" });
    s().markDone("w");
    s().back();
    s().showInboxZero();
    expect(s()).toMatchObject({ view: "zero", history: [] });
  });

  it("finishing the last one in every account reaches inbox zero", () => {
    resetStore(both());
    s().setAccountFilter("work");
    s().markDone("h");
    s().markDone("w");
    expect(s().view).toBe("zero");
  });
});

describe("screens over the mail", () => {
  beforeEach(() => resetStore([thread("a")]));

  it("add account, settings and search open from where you are and go back there", () => {
    s().open("a");
    s().openSearch();
    expect(s().view).toBe("search");
    s().leave();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
  });

  it("settings opens at a section, Accounts by default", () => {
    s().openSettings();
    expect(s()).toMatchObject({ view: "settings", settingsSection: "accounts" });
    s().openSettings("ai");
    expect(s().settingsSection).toBe("ai");
  });

  it("stack: add account from settings goes back to settings, then to the inbox", () => {
    s().openSettings("accounts");
    s().openAddAccount();
    expect(s().view).toBe("add-account");
    s().leave();
    expect(s().view).toBe("settings");
    s().leave();
    expect(s().view).toBe("inbox");
  });

  it("reopening the screen you are on doesn’t pile up history", () => {
    s().openSettings();
    s().openSettings("keyboard");
    s().leave();
    expect(s().view).toBe("inbox");
    s().leave();
    expect(s().view).toBe("inbox");
  });

  it("going to an account from settings drops the history", () => {
    s().openSettings();
    s().setAccountFilter("personal");
    expect(s()).toMatchObject({ view: "inbox", history: [] });
  });
});

describe("managing accounts", () => {
  const studio = { ...personal, id: "studio", name: "Studio" };
  beforeEach(() => resetStore([thread("w"), thread("p", { accountId: "personal" })], [work, personal, studio]));

  it("renames and recolours an account", () => {
    s().updateAccount("personal", { name: "Home", color: "sky" });
    expect(s().accounts[1]).toMatchObject({ name: "Home", color: "sky", address: personal.address });
  });

  it("moves an account up or down, and stops at the ends", () => {
    s().moveAccount("personal", -1);
    expect(s().accounts.map((a) => a.id)).toEqual(["personal", "work", "studio"]);
    s().moveAccount("personal", -1);
    expect(s().accounts.map((a) => a.id)).toEqual(["personal", "work", "studio"]);
    s().moveAccount("work", 1);
    expect(s().accounts.map((a) => a.id)).toEqual(["personal", "studio", "work"]);
    s().moveAccount("gone", 1);
    expect(s().accounts).toHaveLength(3);
  });

  it("removes an account with its mail, and stops filtering by it", async () => {
    s().setAccountFilter("personal");
    await s().removeAccount("personal");
    expect(s().accounts.map((a) => a.id)).toEqual(["work", "studio"]);
    expect(s().threads.map((t) => t.id)).toEqual(["w"]);
    expect(s().accountFilter).toBe("all");
    await s().removeAccount("studio");
    expect(s().accountFilter).toBe("all");
  });

  it("keeps the filter when another account is removed", async () => {
    s().setAccountFilter("work");
    await s().removeAccount("studio");
    expect(s().accountFilter).toBe("work");
  });

  it("sets the reading size", () => {
    expect(s().readingSize).toBe("default");
    s().setReadingSize("larger");
    expect(s().readingSize).toBe("larger");
  });
});

describe("signatures in a draft", () => {
  beforeEach(() => resetStore([thread("q4")], [work, personal, { ...personal, id: "studio", name: "Studio" }]));

  it("starts with the account’s signature for new emails or for replies", () => {
    s().compose("new");
    expect(s().draft?.signatureId).toBe("work-full");
    s().compose("reply", "q4");
    expect(s().draft?.signatureId).toBe("work-short");
    s().setAccountFilter("personal");
    s().compose("new");
    expect(s().draft?.signatureId).toBeNull();
  });

  it("follows the From account until you pick one yourself", () => {
    s().compose("new");
    s().updateDraft({ fromAccountId: "studio" });
    expect(s().draft?.signatureId).toBe("studio");
    s().chooseSignature("work-full");
    expect(s().draft).toMatchObject({ signatureId: "work-full", signatureChosen: true, signatureOpen: false });
    expect(useSignatures.getState().recent[0]).toBe("work-full");
    s().updateDraft({ fromAccountId: "personal" });
    expect(s().draft?.signatureId).toBe("work-full");
  });

  it("in a back-and-forth a reply gets a one-line one or none; a forward the one for new emails", () => {
    // You already wrote in this thread.
    const yours = message({ id: "q4-2", from: maya });
    resetStore([thread("q4", { messages: [message(), yours] })], [work, personal, { ...personal, id: "studio", name: "Studio" }]);
    s().compose("reply", "q4");
    expect(s().draft?.signatureId).toBe("work-short");
    expect(signaturePick(s().draft!)).toEqual({ id: "work-short", why: "short" });
    // Changing From picks again, with the thread in mind: Studio's is a full one.
    s().updateDraft({ fromAccountId: "studio" });
    expect(s().draft?.signatureId).toBeNull();
    s().compose("forward", "q4");
    expect(s().draft?.signatureId).toBe("work-full");
    // Off: each account's choice, as it is.
    useSignatures.setState({ letMossChoose: false });
    s().compose("forward", "q4");
    expect(s().draft?.signatureId).toBe("work-short");
    s().updateDraft({ fromAccountId: "studio" });
    expect(s().draft?.signatureId).toBe("studio");
  });

  it("keeps a signature set along with the From account", () => {
    s().compose("new");
    s().updateDraft({ fromAccountId: "studio", signatureId: null });
    expect(s().draft?.signatureId).toBeNull();
  });

  it("opens and closes the picker, and does nothing without a draft", () => {
    s().toggleSignaturePicker();
    expect(s().draft).toBeUndefined();
    s().chooseSignature(null);
    expect(s().draft).toBeUndefined();
    s().compose("new");
    s().toggleSignaturePicker();
    expect(s().draft?.signatureOpen).toBe(true);
    s().chooseSignature(null);
    expect(s().draft).toMatchObject({ signatureId: null, signatureOpen: false });
  });
});

describe("remote images", () => {
  beforeEach(() => resetStore([thread("a")]));

  it("are hidden until shown, per message", () => {
    expect(useStore.getState().imagesShown).toEqual([]);
    useStore.getState().showImages("a-1");
    useStore.getState().showImages("a-1");
    expect(useStore.getState().imagesShown).toEqual(["a-1"]);
  });
});

describe("attachment thumbnails", () => {
  beforeEach(() => resetStore([thread("a")]));
  const flush = () => new Promise((r) => setTimeout(r));
  const email = message({
    id: "m",
    attachments: [
      { index: 0, name: "Roof.jpg", size: "1.9 MB" },
      { index: 1, name: "Assets.zip", size: "12 MB" },
      { name: "Demo.pdf", size: "2 MB" },
      { index: 3, name: "Drawn.pdf", size: "2 MB", preview: "data:,drawn" },
    ],
  });

  it("asks once for each file that can have one", async () => {
    const { backend } = fakeMail({ attachmentPreview: vi.fn(async () => "data:,roof") });
    s().loadPreviews(email);
    s().loadPreviews(email);
    expect(backend.attachmentPreview).toHaveBeenCalledOnce();
    expect(backend.attachmentPreview).toHaveBeenCalledWith("m", 0);
    await flush();
    expect(s().previews).toEqual({ "m:0": "data:,roof" });
    s().loadPreviews(email);
    expect(backend.attachmentPreview).toHaveBeenCalledOnce();
  });

  it("opening an email asks for its thumbnails, once its messages are here", async () => {
    const latest = message({ id: "b-2", attachments: [{ index: 0, name: "Plan.docx", size: "84 KB" }] });
    const { backend } = fakeMail({ thread: vi.fn(async () => thread("b", { messages: [message({ id: "b-1" }), latest] })) });
    resetStore([thread("a", { messages: [message({ id: "a-1", attachments: [{ index: 2, name: "Roof.jpg", size: "1 MB" }] })] }), thread("b", { partial: true })]);
    s().open("a");
    expect(backend.attachmentPreview).toHaveBeenCalledWith("a-1", 2);
    s().open("b");
    await flush();
    expect(backend.attachmentPreview).toHaveBeenLastCalledWith("b-2", 0);
  });

  it("remembers a file that has none, and doesn't ask again", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { backend } = fakeMail({ attachmentPreview: vi.fn(async () => Promise.reject(new Error("offline"))) });
    s().loadPreviews(email);
    await flush();
    expect(s().previews).toEqual({ "m:0": null });
    expect(warn).toHaveBeenCalledWith("Moss:", expect.any(Error));
    s().loadPreviews(email);
    expect(backend.attachmentPreview).toHaveBeenCalledOnce();
  });
});

describe("with real mail", () => {
  beforeEach(() => resetStore([thread("a"), thread("b", { partial: true })]));
  const flush = () => new Promise((r) => setTimeout(r));

  it("done, remind and account changes reach the mail engine", async () => {
    const { backend } = fakeMail();
    s().markDone("a");
    expect(backend.markDone).toHaveBeenCalledWith("a");
    s().remindLater("b");
    expect(backend.snooze).toHaveBeenCalledWith("b", expect.any(Number));
    expect(vi.mocked(backend.snooze).mock.lastCall![1]).toBeGreaterThan(Date.now());
    s().updateAccount("work", { color: "sky" });
    expect(backend.updateAccount).toHaveBeenCalledWith("work", { color: "sky" });
    s().moveAccount("personal", -1);
    expect(backend.reorderAccounts).toHaveBeenCalledWith(["personal", "work"]);
    await s().removeAccount("personal");
    expect(backend.removeAccount).toHaveBeenCalledWith("personal");
  });

  it("delete and its undo reach the mail engine", () => {
    const { backend } = fakeMail();
    s().deleteThread("a");
    expect(backend.deleteThread).toHaveBeenCalledWith("a");
    s().undo();
    expect(backend.restoreThread).toHaveBeenCalledWith("a");
    useStore.setState({ doneThreads: [thread("x", { doneAt: 5 })] });
    s().deleteThread("x");
    expect(backend.deleteThread).toHaveBeenLastCalledWith("x");
    s().undo();
    expect(backend.restoreThread).toHaveBeenLastCalledWith("x");
  });

  it("opens a thread no list has (a forward you sent), fetching it first", async () => {
    const forward = thread("f", { subject: "Fwd: Q4 budget", doneAt: 1 });
    const { backend } = fakeMail({ thread: vi.fn(async () => forward) });
    expect(await s().openFound("f")).toBe(true);
    expect(backend.thread).toHaveBeenCalledWith("f");
    expect(s().view).toBe("thread");
    expect(openThread(s())?.subject).toBe("Fwd: Q4 budget");
    // No list gains it: it isn't waiting on you, and you didn't mark it done.
    expect(s().foundThreads.map((t) => t.id)).toEqual(["f"]);
    expect(s().doneThreads.map((t) => t.id)).not.toContain("f");

    // One a list has already opens straight away.
    expect(await s().openFound("a")).toBe(true);
    expect(s().openThreadId).toBe("a");
    expect(backend.thread).toHaveBeenCalledTimes(1);

    // One that's gone (deleted since) leaves you where you are.
    vi.mocked(backend.thread).mockResolvedValueOnce(undefined);
    expect(await s().openFound("gone")).toBe(false);
    expect(s().openThreadId).toBe("a");
  });

  it("an email only you sent, opened from Search, stays open while the lists reload", async () => {
    // Its only message is yours, so neither the list nor the Done list holds it.
    const sent = thread("sent", { subject: "Council Parking Permit Renewal Portal", messages: [message({ id: "sent-1" })] });
    resetStore([thread("a")]);
    const { backend } = fakeMail({
      thread: vi.fn(async (id: string) => (id === "sent" ? sent : thread(id))),
      doneThreads: vi.fn(async () => [thread("old", { doneAt: 1 })]),
    });
    s().openSearch();
    await s().openFound("long-done");
    s().leave();
    await s().openFound("sent");
    expect(shownView(s())).toBe("thread");
    expect(openThread(s())?.subject).toBe("Council Parking Permit Renewal Portal");
    // The list's cursor stays on its own thread.
    expect(s().cursorId).toBe("a");

    await Promise.all([loadDone(), reloadThreads(), loadBlocked()]);
    expect(shownView(s())).toBe("thread");
    expect(openThread(s())?.id).toBe("sent");
    // Its body and read state are kept up to date like any other.
    vi.mocked(backend.thread).mockResolvedValueOnce({ ...sent, messages: [message({ id: "sent-1" }), message({ id: "sent-2" })] });
    await loadThread("sent");
    expect(openThread(s())?.messages).toHaveLength(2);
    expect(findThread(s(), "long-done")?.messages).toHaveLength(1);
    // Marking it unread leaves it, like Esc: back to Search.
    s().toggleRead();
    expect(findThread(s(), "sent")?.unread).toBe(true);
    expect(s().view).toBe("search");
  });

  it("opening a thread loads its messages and marks it read", async () => {
    const full = thread("b", { messages: [message({ id: "b-1" }), message({ id: "b-2" })] });
    const { backend } = fakeMail({ thread: vi.fn(async () => full) });
    s().open("b");
    expect(backend.thread).toHaveBeenCalledWith("b");
    expect(backend.markRead).toHaveBeenCalledWith("b", true);
    await flush();
    expect(s().threads.find((t) => t.id === "b")?.messages).toHaveLength(2);
    // A loaded thread isn't fetched again; ↑/↓ in the thread view loads the next one.
    s().open("a");
    expect(backend.thread).toHaveBeenCalledTimes(1);
    s().moveCursor(1);
    expect(backend.markRead).toHaveBeenLastCalledWith("b", true);
  });

  it.each([
    ["Done", () => s().markDone()],
    ["Delete", () => s().deleteThread()],
    ["Block", () => s().blockSender()],
  ])("%s on an open email loads the next one's body and marks it read", async (_, act) => {
    // The engine lists it without its body; by the time it answers, it has saved it as read.
    const listed = thread("b", { partial: true, unread: false, lastMessageAt: 5 });
    const full = thread("b", { lastMessageAt: 5, messages: [message({ id: "b-1", html: "<p>Welcome</p>" })] });
    resetStore([thread("a", { from: maya }), { ...listed, unread: true }]);
    const { backend } = fakeMail({ thread: vi.fn(async () => full), threads: vi.fn(async () => [listed]) });
    s().open("a");
    act();
    expect(s().openThreadId).toBe("b");
    await flush();
    await flush();
    expect(backend.thread).toHaveBeenCalledWith("b");
    expect(backend.markRead).toHaveBeenCalledWith("b", true);
    expect(openThread(s())).toMatchObject({ unread: false, messages: [{ html: "<p>Welcome</p>" }] });
  });

  it("deleting a done email you're reading loads the next done one", async () => {
    const full = thread("y", { messages: [message({ id: "y-1", html: "<p>Receipt</p>" })] });
    resetStore([]);
    const listed = thread("y", { doneAt: 1, partial: true });
    useStore.setState({ doneThreads: [thread("x", { doneAt: 2 }), listed] });
    fakeMail({ thread: vi.fn(async () => full), doneThreads: vi.fn(async () => [listed]) });
    s().open("x");
    s().deleteThread();
    await flush();
    expect(openThread(s())?.messages[0].html).toBe("<p>Receipt</p>");
  });

  it.each([
    ["Done", "doneThreads", loadDone],
    ["Blocked", "blockedThreads", loadBlocked],
  ] as const)("reading the %s list again keeps the bodies already loaded", async (_, list, load) => {
    const listed = thread("y", { partial: true, lastMessageAt: 5 });
    resetStore([]);
    useStore.setState({ [list]: [thread("y", { lastMessageAt: 5, messages: [message({ id: "y-1", html: "<p>Hi</p>" })] })] });
    fakeMail({ [list]: vi.fn(async () => [listed]) });
    await load();
    expect(s()[list][0]).toMatchObject({ partial: false, messages: [{ html: "<p>Hi</p>" }] });
  });

  it.each([
    ["inbox", "threads", reloadThreads],
    ["Done", "doneThreads", loadDone],
    ["Blocked", "blockedThreads", loadBlocked],
  ] as const)("a new message in the email you're reading (%s) loads in full, never as the list's one-line preview", async (_, list, load) => {
    // What you sent, as the list row shows it: one line, its breaks gone.
    const row = (id: string) => thread(id, { partial: true, lastMessageAt: 9, messages: [message({ id: `${id}-preview`, paragraphs: ["Hi Dana, Thanks. Maya"] })] });
    const reading = thread("a", { lastMessageAt: 5, messages: [message({ id: "a-1", text: "Can you sign off?" })] });
    let finish: (t: Thread) => void = () => {};
    const { backend } = fakeMail({ [list]: vi.fn(async () => [row("a"), row("b")]), thread: vi.fn(() => new Promise<Thread>((r) => (finish = r))) });
    resetStore([]);
    useStore.setState({ [list]: [reading, thread("b", { lastMessageAt: 5 })] });
    s().open("a");
    vi.mocked(backend.thread).mockClear();
    await load();
    // Until the new message loads, the email shows what it showed.
    expect(openThread(s())).toMatchObject({ partial: false, lastMessageAt: 9, messages: [{ id: "a-1" }] });
    expect(backend.thread).toHaveBeenCalledExactlyOnceWith("a");
    // An email you aren't reading takes the list's row, and loads when you open it.
    expect(s()[list].find((t) => t.id === "b")).toMatchObject({ partial: true });
    finish(thread("a", { lastMessageAt: 9, messages: [message({ id: "a-1", text: "Can you sign off?" }), message({ id: "a-2", text: "Hi Dana,\n\nThanks.\n\nMaya" })] }));
    await flush();
    expect(openThread(s())?.messages.map((m) => m.text)).toEqual(["Can you sign off?", "Hi Dana,\n\nThanks.\n\nMaya"]);
  });

  it("undo back to an email that hadn't loaded loads it", async () => {
    const full = thread("a", { messages: [message({ id: "a-1", html: "<p>Agenda</p>" })] });
    let finish: (t: Thread) => void = () => {};
    const thread_ = vi.fn<() => Promise<Thread | undefined>>(() => new Promise((r) => (finish = r)));
    resetStore([thread("a", { partial: true }), thread("b")]);
    fakeMail({ thread: thread_, threads: vi.fn(async () => [thread("a", { partial: true }), thread("b")]) });
    s().open("a");
    s().markDone();
    finish(full);
    await flush();
    thread_.mockResolvedValue(full);
    s().undo();
    await flush();
    expect(s().openThreadId).toBe("a");
    expect(openThread(s())?.messages[0].html).toBe("<p>Agenda</p>");
  });

  it("an opened email stays read when its messages load before the engine has saved that", async () => {
    let saved: () => void = () => {};
    fakeMail({
      thread: vi.fn(async () => thread("b", { unread: true, messages: [message({ id: "b-1" }), message({ id: "b-2" })] })),
      markRead: vi.fn(() => new Promise<void>((r) => (saved = r))),
    });
    useStore.setState((st) => ({ threads: st.threads.map((t) => (t.id === "b" ? { ...t, unread: true } : t)) }));
    s().open("b");
    await flush();
    expect(s().threads.find((t) => t.id === "b")).toMatchObject({ unread: false, messages: [{ id: "b-1" }, { id: "b-2" }] });
    s().back();
    expect(s().threads.find((t) => t.id === "b")?.unread).toBe(false);
    saved();
    await flush();
  });

  it("a list reload while a read change is saving keeps it, and takes the engine's word after", async () => {
    let saved: () => void = () => {};
    const { backend } = fakeMail({ markRead: vi.fn(() => new Promise<void>((r) => (saved = r))) });
    vi.mocked(backend.threads).mockResolvedValue([thread("a", { unread: false }), thread("b")]);
    s().toggleRead("a");
    await reloadThreads();
    expect(s().threads.find((t) => t.id === "a")?.unread).toBe(true);
    saved();
    await flush();
    await reloadThreads();
    expect(s().threads.find((t) => t.id === "a")?.unread).toBe(false);
  });

  it("when U is pressed twice, the first save finishing doesn't undo the second on a reload", async () => {
    const saves: (() => void)[] = [];
    const { backend } = fakeMail({ markRead: vi.fn(() => new Promise<void>((r) => saves.push(r))) });
    vi.mocked(backend.threads).mockResolvedValue([thread("a", { unread: true }), thread("b")]);
    s().toggleRead("a");
    s().toggleRead("a");
    saves[0]();
    await flush();
    await reloadThreads();
    expect(s().threads.find((t) => t.id === "a")?.unread).toBe(false);
    saves[1]();
    await flush();
  });

  it("U marks the email under the cursor unread, then read again", () => {
    const { backend } = fakeMail();
    useStore.setState({ cursorId: "b" });
    s().toggleRead();
    expect(s().threads.find((t) => t.id === "b")?.unread).toBe(true);
    expect(backend.markRead).toHaveBeenLastCalledWith("b", false);
    s().toggleRead("b");
    expect(s().threads.find((t) => t.id === "b")?.unread).toBe(false);
    expect(backend.markRead).toHaveBeenLastCalledWith("b", true);
  });

  it("marking the open email unread goes back to the list, and opening it reads it again", () => {
    const { backend } = fakeMail();
    s().open("a");
    s().toggleRead();
    expect(s().view).toBe("inbox");
    expect(s().threads.find((t) => t.id === "a")?.unread).toBe(true);
    expect(backend.markRead).toHaveBeenLastCalledWith("a", false);
    s().open("a");
    expect(s().threads.find((t) => t.id === "a")?.unread).toBe(false);
  });

  it("can mark done and blocked mail unread too, and does nothing without an email", () => {
    const { backend } = fakeMail();
    useStore.setState({ doneThreads: [thread("d")], blockedThreads: [thread("x")] });
    s().toggleRead("d");
    s().toggleRead("x");
    expect(s().doneThreads[0].unread).toBe(true);
    expect(s().blockedThreads[0].unread).toBe(true);
    useStore.setState({ cursorId: undefined });
    s().toggleRead();
    s().toggleRead("gone");
    expect(backend.markRead).toHaveBeenCalledTimes(2);
  });

  it("opening mail that needs your reply asks the writing model what it wants of you, once", async () => {
    fakeMail();
    // A model's Markdown is taken out: the line has room for words only.
    const ask = vi.fn(async (id: string) => (id === "a" ? "Send **Dana** the deck by *Friday*." : null));
    setAi({ ...ai(), ask });
    resetStore([thread("a", { section: "reply" }), thread("i", { section: "info" }), thread("n", { section: "reply" })]);
    s().open("a");
    await flush();
    expect(s().threads.find((t) => t.id === "a")?.ask).toBe("Send Dana the deck by Friday.");
    // It stays through a reload of the list.
    fakeMail({ threads: vi.fn(async () => [thread("a", { section: "reply" })]) });
    await reloadThreads();
    expect(s().threads.find((t) => t.id === "a")?.ask).toBe("Send Dana the deck by Friday.");
    s().open("a");
    expect(ask).toHaveBeenCalledOnce();
    // Mail for your information never wakes the writing model.
    s().open("i");
    expect(ask).toHaveBeenCalledOnce();
  });

  it("a new sender's email asks about them from the moment it opens, and a reload doesn't take that away", async () => {
    const at = Date.now();
    const row = () => thread("n", { partial: true, lastMessageAt: at, sender: "new" });
    let finish: (t: ReturnType<typeof thread>) => void = () => {};
    fakeMail({ threads: vi.fn(async () => [row()]), thread: vi.fn(() => new Promise<ReturnType<typeof thread>>((r) => (finish = r))) });
    resetStore([row()]);
    s().open("n");
    // Decided before its messages load.
    expect(openThread(s())?.sender).toBe("new");
    finish(thread("n", { lastMessageAt: at, sender: "new" }));
    await flush();
    expect(openThread(s())?.partial).toBeFalsy();
    expect(openThread(s())?.sender).toBe("new");
    // The list read again in the background still says so, and keeps the loaded messages.
    await reloadThreads();
    expect(openThread(s())).toMatchObject({ partial: false, sender: "new" });
    // Accepted somewhere else: the next reload says so.
    fakeMail({ threads: vi.fn(async () => [thread("n", { partial: true, lastMessageAt: at, sender: "known" })]) });
    await reloadThreads();
    expect(openThread(s())?.sender).toBe("known");
  });

  it("a reload keeps the replies Moss wrote for an email, until a new message comes", async () => {
    const replyOptions = [{ label: "Yes", draft: "Yes." }];
    resetStore([thread("a", { lastMessageAt: 5, replyOptions }), thread("b", { lastMessageAt: 5, replyOptions })]);
    fakeMail({ threads: vi.fn(async () => [thread("a", { lastMessageAt: 5 }), thread("b", { lastMessageAt: 9 })]) });
    await reloadThreads();
    expect(s().threads.find((t) => t.id === "a")?.replyOptions).toEqual(replyOptions);
    expect(s().threads.find((t) => t.id === "b")?.replyOptions).toBeUndefined();
  });

  it("a reload that drops the selected email selects the one that took its place", async () => {
    resetStore([thread("a"), thread("b"), thread("c")]);
    s().setCursor("b");
    fakeMail({ threads: vi.fn(async () => [thread("a"), thread("c")]) });
    await reloadThreads();
    expect(s().cursorId).toBe("c");
    // Dropping the last one selects the one above it.
    fakeMail({ threads: vi.fn(async () => [thread("a")]) });
    await reloadThreads();
    expect(s().cursorId).toBe("a");
  });

  it("a reload keeps the ask of a thread whose messages haven't loaded", async () => {
    const at = Date.now();
    resetStore([thread("p", { section: "reply", partial: true, lastMessageAt: at, ask: "Reply to Dana." })]);
    fakeMail({ threads: vi.fn(async () => [thread("p", { section: "reply", partial: true, lastMessageAt: at })]) });
    await reloadThreads();
    expect(s().threads[0]).toMatchObject({ ask: "Reply to Dana.", partial: true });
  });

  it("the ask stays when the messages load after it", async () => {
    let finish: (t: ReturnType<typeof thread>) => void = () => {};
    fakeMail({ thread: vi.fn(() => new Promise<ReturnType<typeof thread>>((r) => (finish = r))) });
    setAi({ ...ai(), ask: vi.fn(async () => "Reply to Dana.") });
    resetStore([thread("b", { section: "reply", partial: true })]);
    s().open("b");
    await flush();
    finish(thread("b", { section: "reply", messages: [message({ id: "b-1" }), message({ id: "b-2" })] }));
    await flush();
    const b = s().threads[0];
    expect([b.ask, b.messages.length]).toEqual(["Reply to Dana.", 2]);
  });

  it("no ask when there is nothing to do, the model isn't there, or the mail is the demo", async () => {
    fakeMail();
    resetStore([thread("n", { section: "reply" }), thread("x", { section: "reply" })]);
    setAi({ ...ai(), ask: vi.fn(async (id: string) => (id === "n" ? null : Promise.reject({ kind: "notReady" }))) });
    s().open("n");
    s().open("x");
    await flush();
    expect(s().threads.map((t) => t.ask)).toEqual([undefined, undefined]);

    const { setMail, demoMail } = await import("../services/mail");
    setMail(demoMail);
    const ask = vi.fn(async () => "Reply.");
    setAi({ ...ai(), ask });
    s().open("n");
    expect(ask).not.toHaveBeenCalled();
  });

  it("opening a thread fetches the body of a message stored with headers only", async () => {
    const shallow = thread("b", { messages: [message({ id: "b-1" }), message({ id: "b-2", headersOnly: true })] });
    const whole = thread("b", { messages: [message({ id: "b-1" }), message({ id: "b-2", paragraphs: ["Signed."] })] });
    const thread_ = vi.fn<() => Promise<Thread | undefined>>().mockResolvedValueOnce(shallow).mockResolvedValueOnce(whole);
    const { backend } = fakeMail({ thread: thread_, loadMessage: vi.fn(async () => true) });
    s().open("b");
    await flush();
    await flush();
    expect(backend.loadMessage).toHaveBeenCalledWith("b-2");
    expect(backend.loadMessage).toHaveBeenCalledTimes(1);
    expect(s().threads.find((t) => t.id === "b")?.messages[1].paragraphs).toEqual(["Signed."]);
  });

  it("opening a thread fetches the images its HTML shows that aren't here", async () => {
    const shown = thread("b", { messages: [message({ id: "b-1", imagesMissing: true })] });
    const { backend } = fakeMail({ thread: vi.fn(async () => shown), loadMessage: vi.fn(async () => true) });
    s().open("b");
    await flush();
    await flush();
    expect(backend.loadMessage).toHaveBeenCalledWith("b-1");
  });

  it("a body the server no longer has leaves the headers as they were", async () => {
    const shallow = thread("b", { messages: [message({ id: "b-2", headersOnly: true })] });
    const { backend } = fakeMail({ thread: vi.fn(async () => shallow) });
    s().open("b");
    await flush();
    await flush();
    expect(backend.thread).toHaveBeenCalledTimes(1);
    expect(s().threads.find((t) => t.id === "b")?.messages[0].headersOnly).toBe(true);
  });

  it("opening an attachment hands it to the mail engine", () => {
    const { backend } = fakeMail();
    s().openAttachment("b-1", 2);
    expect(backend.openAttachment).toHaveBeenCalledWith("b-1", 2);
  });

  it("a thread that vanished on the way stays as it was", async () => {
    fakeMail();
    s().open("b");
    await flush();
    expect(s().threads.find((t) => t.id === "b")?.partial).toBe(true);
  });

  it("a background failure is only logged", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    fakeMail({ markDone: vi.fn(async () => Promise.reject(new Error("offline"))) });
    s().markDone("a");
    await flush();
    expect(warn).toHaveBeenCalledWith("Moss:", expect.any(Error));
    warn.mockRestore();
  });
});

describe("without accounts", () => {
  it("removing the last account opens Add account", async () => {
    resetStore([thread("a")], [work]);
    s().openSettings();
    await s().removeAccount("work");
    expect(s()).toMatchObject({ accounts: [], threads: [], view: "add-account", history: [] });
  });
});

describe("the Done list", () => {
  const doneA = thread("x", { doneAt: 300 });
  const doneB = thread("y", { doneAt: 200, accountId: "personal" });
  const doneC = thread("z", { doneAt: 100 });
  beforeEach(() => {
    resetStore([thread("a"), thread("b")]);
    useStore.setState({ doneThreads: [doneA, doneB, doneC] });
  });

  it("starts empty in tests, so demo edits can't leak in", () => {
    resetStore([]);
    expect(s().doneThreads).toEqual([]);
  });

  it("marking done puts the email at the top, with when it was done", () => {
    s().markDone("b");
    expect(s().doneThreads.map((t) => t.id)).toEqual(["b", "x", "y", "z"]);
    expect(s().doneThreads[0].doneAt).toBeGreaterThan(Date.now() - 1000);
  });

  it("opens over the current screen at the newest done email, and goes back", () => {
    s().openDone();
    expect(s()).toMatchObject({ view: "done", doneCursorId: "x", history: ["inbox"] });
    s().leave();
    expect(s().view).toBe("inbox");
  });

  it("follows the account you are looking at", () => {
    s().setAccountFilter("personal");
    expect(visibleDone(s()).map((t) => t.id)).toEqual(["y"]);
    s().openDone();
    expect(s().doneCursorId).toBe("y");
  });

  it("moves its own cursor and stops at the ends", () => {
    s().openDone();
    s().moveDoneCursor(-1);
    expect(s().doneCursorId).toBe("x");
    s().moveDoneCursor(1);
    s().moveDoneCursor(1);
    s().moveDoneCursor(1);
    expect(s().doneCursorId).toBe("z");
    s().setDoneCursor("y");
    expect(s().doneCursorId).toBe("y");
  });

  it("not done brings the email back to the inbox and moves to the next done one", () => {
    s().openDone();
    s().markNotDone("x");
    expect(s().doneThreads.map((t) => t.id)).toEqual(["y", "z"]);
    expect(s().threads[0]).toEqual(thread("x"));
    expect(s()).toMatchObject({ view: "done", doneCursorId: "y", toast: { text: "Back in your inbox" } });
    s().setDoneCursor("z");
    s().markNotDone();
    expect(s().doneCursorId).toBe("y");
  });

  it("E on a done email means not done", () => {
    s().openDone();
    s().markDone();
    expect(s().threads.map((t) => t.id)).toContain("x");
    expect(s().doneThreads.map((t) => t.id)).toEqual(["y", "z"]);
  });

  it("does nothing for an email that isn't done", () => {
    s().markNotDone("a");
    s().openDone();
    useStore.setState({ doneCursorId: undefined });
    s().markNotDone();
    expect(s().doneThreads).toHaveLength(3);
    expect(s().undoSteps).toEqual([]);
  });

  it("opens a done email, reads through the list with ↑ and ↓, and goes back to it", () => {
    s().openDone();
    s().open("x");
    expect(shownView(s())).toBe("thread");
    s().moveCursor(1);
    expect(s()).toMatchObject({ openThreadId: "y", doneCursorId: "y" });
    s().back();
    expect(s().view).toBe("done");
  });

  it("not done while reading keeps the email open", () => {
    s().openDone();
    s().open("x");
    s().markDone();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x" });
    expect(shownView(s())).toBe("thread");
  });

  it("an email opened from search goes back to search", () => {
    s().openSearch();
    s().open("a");
    s().back();
    expect(s().view).toBe("search");
    s().leave();
    expect(s()).toMatchObject({ view: "inbox", history: [] });
  });

  it("done from search stays in search", () => {
    s().openSearch();
    s().markDone("a");
    expect(s().view).toBe("search");
  });

  it("landing back on the list forgets where the email was opened from", () => {
    s().openSearch();
    s().open("a");
    s().markDone();
    s().markDone();
    expect(s()).toMatchObject({ view: "zero", history: [] });
  });

  it("loads the list from the mail engine with real mail", async () => {
    const { backend } = fakeMail({ doneThreads: vi.fn(async () => [doneC]) });
    s().openDone();
    await new Promise((r) => setTimeout(r));
    expect(backend.doneThreads).toHaveBeenCalled();
    expect(s()).toMatchObject({ doneThreads: [doneC], doneCursorId: "z" });
    s().markNotDone("z");
    expect(backend.markNotDone).toHaveBeenCalledWith("z");
  });
});

describe("with real mail, the mail engine has the last word", () => {
  const tick = () => new Promise((r) => setTimeout(r));
  beforeEach(() => resetStore([thread("a"), thread("b")]));

  it("after done or not done, both lists are read again from the engine", async () => {
    const { backend } = fakeMail({
      threads: vi.fn(async () => [thread("a")]),
      doneThreads: vi.fn(async () => [thread("b", { doneAt: 9 })]),
    });
    s().markDone("b");
    await tick();
    expect(backend.threads).toHaveBeenCalledOnce();
    expect(backend.doneThreads).toHaveBeenCalledOnce();
    expect(s().threads.map((t) => t.id)).toEqual(["a"]);
    expect(s().doneThreads.map((t) => t.id)).toEqual(["b"]);
  });

  it("an email is never in both lists: the newest read of either wins", async () => {
    useStore.setState({ doneThreads: [thread("b", { doneAt: 1 })] });
    fakeMail({ threads: vi.fn(async () => [thread("a"), thread("b")]), doneThreads: vi.fn(async () => [thread("a", { doneAt: 2 })]) });
    await reloadThreads();
    expect(s().doneThreads).toEqual([]);
    await loadDone();
    expect(s().threads.map((t) => t.id)).toEqual(["b"]);
    expect(s().doneThreads.map((t) => t.id)).toEqual(["a"]);
  });

  it("a change the engine refuses is put back, and a note says so", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    useStore.setState({ doneThreads: [thread("x", { doneAt: 1 })] });
    fakeMail({
      markNotDone: vi.fn(async () => Promise.reject(new Error("thread not found"))),
      threads: vi.fn(async () => [thread("a"), thread("b")]),
      doneThreads: vi.fn(async () => [thread("x", { doneAt: 1 })]),
    });
    s().markNotDone("x");
    expect(s().threads.map((t) => t.id)).toEqual(["x", "a", "b"]);
    await tick();
    await tick();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x"]);
    expect(s().toast).toMatchObject({ text: "Couldn’t save that, so Moss put it back", problem: true });
    // Undo would reverse a change that never happened.
    expect(s().undoSteps).toEqual([]);
    warn.mockRestore();
  });

  it("a list read that started before a change is read again, so the email doesn't come back", async () => {
    let answer: (t: Thread[]) => void = () => {};
    // The engine is still saving the done, so only the read that started first can bring it back.
    const { backend } = fakeMail({ markDone: vi.fn(() => new Promise<void>(() => {})) });
    vi.mocked(backend.threads).mockImplementationOnce(() => new Promise<Thread[]>((r) => (answer = r)));
    vi.mocked(backend.threads).mockResolvedValue([thread("b")]);
    const reload = reloadThreads();
    s().markDone("a");
    answer([thread("a"), thread("b")]);
    await reload;
    expect(s().threads.map((t) => t.id)).toEqual(["b"]);
    expect(backend.threads).toHaveBeenCalledTimes(2);
  });

  it("a snooze or an accept the engine refuses is put back, and a note says so", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { backend } = fakeMail({
      snooze: vi.fn(async () => Promise.reject(new Error("thread not found"))),
      acceptSender: vi.fn(async () => Promise.reject(new Error("thread not found"))),
    });
    vi.mocked(backend.threads).mockResolvedValue([thread("a"), thread("b", { sender: "new" })]);
    s().remindLater("a");
    expect(s().threads.find((t) => t.id === "a")?.section).toBe("later");
    await tick();
    await tick();
    expect(s().threads.find((t) => t.id === "a")?.section).not.toBe("later");
    expect(s().toast).toMatchObject({ text: "Couldn’t save that, so Moss put it back", problem: true });
    useStore.setState({ toast: undefined });
    s().acceptSender(false, "b");
    expect(s().threads.find((t) => t.id === "b")?.sender).toBe("known");
    await tick();
    await tick();
    expect(s().threads.find((t) => t.id === "b")?.sender).toBe("new");
    expect(s().toast).toMatchObject({ problem: true });
    warn.mockRestore();
  });

  it("the demo mailbox has no engine to ask", async () => {
    s().markDone("b");
    await tick();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["b"]);
  });
});

describe("answering a done email", () => {
  const done = thread("x", { doneAt: 5, from: jonas, replyOptions: [{ label: "Thanks", draft: "Thanks!" }] });
  beforeEach(() => {
    resetStore([thread("a")]);
    useStore.setState({ doneThreads: [done] });
    s().openDone();
    s().open("x");
  });

  it("replies to it like any other, and sending leaves it done", () => {
    vi.useFakeTimers();
    s().compose("reply");
    expect(s().draft).toMatchObject({ mode: "reply", threadId: "x", to: [jonas] });
    s().send();
    vi.advanceTimersByTime(sendBeatMs);
    vi.useRealTimers();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x"]);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x", draft: undefined });
    expect(s().threads.map((t) => t.id)).toEqual(["a"]);
  });

  it("edits and sends its instant reply, and it stays done", () => {
    s().editInstantReply();
    expect(s().draft?.body).toBe("Thanks!");
    s().back();
    s().sendInstantReply();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x"]);
    expect(s().undoSteps).toEqual([]);
  });

  it("not done with no email named means the one you are reading", () => {
    s().markNotDone();
    expect(s().threads.map((t) => t.id)).toEqual(["x", "a"]);
  });

  it("loading its messages keeps when it was done, and leaves other done emails alone", async () => {
    const other = thread("y", { doneAt: 3 });
    useStore.setState((st) => ({ doneThreads: [...st.doneThreads, other] }));
    fakeMail({ thread: vi.fn(async () => thread("x", { messages: [message({ id: "x-1" }), message({ id: "x-2" })] })) });
    await loadThread("x");
    expect(s().doneThreads[0]).toMatchObject({ doneAt: 5, messages: [{ id: "x-1" }, { id: "x-2" }] });
    expect(s().doneThreads[1]).toBe(other);
  });

  it("an instant reply with no email open changes nothing", () => {
    useStore.setState({ openThreadId: undefined, toast: undefined });
    s().sendInstantReply();
    expect(s().toast).toBeUndefined();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x"]);
  });

  it("reloading the list keeps the cursor on it while it is still there", async () => {
    const other = thread("w", { doneAt: 9 });
    fakeMail({ doneThreads: vi.fn(async () => [other, done]) });
    s().openDone();
    s().setDoneCursor("x");
    await loadDone();
    expect(s().doneCursorId).toBe("x");
  });
});

describe("undo", () => {
  beforeEach(() => resetStore([thread("a"), thread("b", { due: "Today" }), thread("c")]));

  it("brings a done email back where it was, with the cursor on it", () => {
    s().markDone("b");
    expect(s().toast).toMatchObject({ text: "Moved to Done" });
    s().undo();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(s()).toMatchObject({ doneThreads: [], cursorId: "b", view: "inbox", handled: 0, toast: undefined, undoSteps: [] });
  });

  it("goes back to reading the email you marked done", () => {
    s().open("b");
    s().markDone();
    expect(s().openThreadId).toBe("c");
    s().undo();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b" });
  });

  it("leaves inbox zero for the list", () => {
    resetStore([thread("a")]);
    s().markDone("a");
    expect(s().view).toBe("zero");
    s().undo();
    expect(s()).toMatchObject({ view: "inbox", cursorId: "a" });
  });

  it("brings back a snoozed email as it was", () => {
    s().remindLater("b");
    expect(s().toast!.text).toMatch(/^Coming back \w{3} 8 AM$/);
    s().undo();
    expect(s().threads[1]).toEqual(thread("b", { due: "Today" }));
    expect(s()).toMatchObject({ cursorId: "b", view: "inbox" });
  });

  it("puts an email you marked not done back in the Done list", () => {
    useStore.setState({ doneThreads: [thread("x", { doneAt: 5 }), thread("y", { doneAt: 4 })] });
    s().openDone();
    s().markNotDone("x");
    s().undo();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x", "y"]);
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(s()).toMatchObject({ view: "done", doneCursorId: "x" });
  });

  it("brings a deleted email back where it was", () => {
    s().open("b");
    s().deleteThread();
    s().undo();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b", cursorId: "b", handled: 0, toast: undefined });
  });

  it("puts an email deleted from the Done list back there", () => {
    useStore.setState({ doneThreads: [thread("x", { doneAt: 5 }), thread("y", { doneAt: 4 })] });
    s().openDone();
    s().deleteThread("x");
    s().undo();
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x", "y"]);
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(s()).toMatchObject({ view: "done", doneCursorId: "x" });
  });

  it("stays on the screen you moved to", () => {
    s().markDone("a");
    s().openSettings();
    s().undo();
    expect(s().view).toBe("settings");
  });

  it("undoes one step at a time, and does nothing with nothing left", () => {
    s().markDone("a");
    s().markDone("b");
    s().undo();
    expect(s().threads.map((t) => t.id)).toEqual(["b", "c"]);
    s().undo();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    s().undo();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
  });

  it("remembers the last 20 steps", () => {
    resetStore(Array.from({ length: 25 }, (_, i) => thread(`t${i}`)));
    for (let i = 0; i < 25; i++) s().markDone(`t${i}`);
    expect(s().undoSteps).toHaveLength(20);
    expect(s().undoSteps[0]).toMatchObject({ thread: { id: "t5" } });
  });

  it("an email done that wasn't in the list (an old search result) has nothing to undo", () => {
    s().markDone("old");
    expect(s()).toMatchObject({ undoSteps: [], handled: 1 });
    expect(s().toast).toBeUndefined();
  });

  it("the message can be put away", () => {
    s().markDone("a");
    s().dismissToast();
    expect(s().toast).toBeUndefined();
    expect(s().undoSteps).toHaveLength(1);
  });

  it("tells the mail engine", () => {
    const { backend } = fakeMail();
    useStore.setState({ doneThreads: [thread("x", { doneAt: 1 })] });
    s().markDone("a");
    s().undo();
    expect(backend.markNotDone).toHaveBeenCalledWith("a");
    s().remindLater("b");
    s().undo();
    expect(backend.snooze).toHaveBeenLastCalledWith("b", null);
    s().markNotDone("x");
    s().undo();
    expect(backend.markDone).toHaveBeenLastCalledWith("x");
  });
});

describe("blocking senders", () => {
  const jordan = person("Jordan Blake", "jordan@growthleads.example");
  const sam = person("Sam Lee", "Sam@GrowthLeads.example");
  const pat = person("Pat Kim", "pat@gmail.com");
  const tick = () => new Promise((r) => setTimeout(r));
  beforeEach(() => {
    resetStore([
      thread("a", { from: jordan, sender: "new" }),
      thread("b", { from: lena }),
      thread("c", { from: sam }),
      thread("d", { from: jordan, accountId: "personal" }),
      thread("e", { from: pat, sender: "new" }),
    ]);
    useStore.setState({ doneThreads: [thread("f", { from: jordan, doneAt: 1 })] });
  });

  it("starts empty in tests", () => {
    expect(s().blockedThreads).toEqual([]);
  });

  it("blocking moves their mail in that account to Blocked and reads on", () => {
    s().open("a");
    s().blockSender();
    expect(s().threads.map((t) => t.id)).toEqual(["b", "c", "d", "e"]);
    expect(s().doneThreads).toEqual([]);
    expect(s().blockedThreads.map((t) => [t.id, t.sender])).toEqual([
      ["a", "blocked"],
      ["f", "blocked"],
    ]);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b", toast: { text: "Blocked Jordan Blake", undo: false } });
  });

  it("blocking again keeps what is already in Blocked", () => {
    useStore.setState({ blockedThreads: [thread("old", { from: jordan, sender: "blocked" }), thread("other", { sender: "blocked" })] });
    s().blockSender(false, "a");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["a", "f", "old", "other"]);
  });

  it("opening blocked mail loads all of it", async () => {
    const full = thread("x", { from: jordan, sender: "blocked", messages: [message({ id: "x-1" }), message({ id: "x-2" })] });
    fakeMail({ thread: vi.fn(async () => full) });
    useStore.setState({ blockedThreads: [thread("x", { from: jordan, sender: "blocked", partial: true }), thread("y", { sender: "blocked" })] });
    await loadThread("x");
    expect(s().blockedThreads[0].messages).toHaveLength(2);
    expect(s().blockedThreads[1]).toEqual(thread("y", { sender: "blocked" }));
  });

  it("blocking the whole domain catches everyone there", () => {
    s().blockSender(true, "a");
    expect(s().threads.map((t) => t.id)).toEqual(["b", "d", "e"]);
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["a", "c", "f"]);
    expect(s()).toMatchObject({ view: "inbox", cursorId: "b", toast: { text: "Blocked everyone at growthleads.example" } });
  });

  it("in the list, B and A act on the email under the cursor", () => {
    s().acceptSender();
    expect(s().threads[0].sender).toBe("known");
    s().blockSender();
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["a", "f"]);
    expect(s()).toMatchObject({ view: "inbox", cursorId: "b" });
  });

  it("blocking says how to find the Blocked list, and its button opens it", () => {
    s().blockSender(false, "a");
    expect(s().toast).toMatchObject({ text: "Blocked Jordan Blake", undo: false, show: { label: "Show Blocked", keys: "⌘⇧B" } });
    s().toast?.show?.run();
    expect(s().view).toBe("blocked");
    s().back();
    s().blockSender(true, "c");
    expect(s().toast?.show?.label).toBe("Show Blocked");
  });

  it("never blocks a public domain as a whole, or an email that isn't there", () => {
    s().blockSender(true, "e");
    s().blockSender(false, "nope");
    expect(s().blockedThreads).toEqual([]);
    expect(s().toast).toBeUndefined();
  });

  it("blocking the last email you were reading goes back to the list", () => {
    resetStore([thread("a", { from: jordan, sender: "new" })]);
    s().open("a");
    s().blockSender();
    expect(s()).toMatchObject({ view: "inbox", cursorId: undefined, openThreadId: undefined });
  });

  it("accepting stops the question for that sender in that account", () => {
    s().open("a");
    s().acceptSender();
    expect(s().threads.find((t) => t.id === "a")?.sender).toBe("known");
    expect(s().threads.find((t) => t.id === "d")?.sender).toBeUndefined();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", toast: { text: "Accepted Jordan Blake", undo: false } });
    s().acceptSender(false, "nope");
    expect(s().toast?.text).toBe("Accepted Jordan Blake");
  });

  it("accepting the whole domain accepts everyone there in that account, and shows their images", () => {
    const { backend } = fakeMail();
    const from = (who: Person, id: string) => message({ id, from: who });
    resetStore([
      thread("a", { from: jordan, sender: "new", messages: [from(jordan, "a-1")] }),
      thread("c", { from: sam, sender: "new", messages: [from(sam, "c-1")] }),
      thread("d", { from: jordan, accountId: "personal", sender: "new", messages: [from(jordan, "d-1")] }),
      thread("e", { from: pat, sender: "new", messages: [from(pat, "e-1")] }),
    ]);
    s().acceptSender(true, "a");
    expect(s().threads.map((t) => [t.id, t.sender, t.messages[0].imageTrust])).toEqual([
      ["a", "known", "you-allowed"],
      ["c", "known", "you-allowed"],
      ["d", "new", undefined],
      ["e", "new", undefined],
    ]);
    expect(s().toast).toMatchObject({ text: "Accepted everyone at growthleads.example", undo: false });
    expect(backend.acceptSender).toHaveBeenCalledWith("a", true);
    // gmail.com is everyone's: it can't be accepted as a whole.
    s().acceptSender(true, "e");
    expect(s().threads.find((t) => t.id === "e")?.sender).toBe("new");
    expect(backend.acceptSender).toHaveBeenCalledOnce();
  });

  it("accepting or unblocking a sender shows their images in that account at once", () => {
    const { backend } = fakeMail();
    const from = (who: Person, id: string) => message({ id, from: who });
    resetStore([
      thread("a", { from: jordan, sender: "new", messages: [from(jordan, "a-1"), from(lena, "a-2")] }),
      thread("d", { from: jordan, accountId: "personal", messages: [from(jordan, "d-1")] }),
    ]);
    useStore.setState({ doneThreads: [thread("f", { from: jordan, doneAt: 1, messages: [from(jordan, "f-1")] })] });
    const trust = (list: Thread[]) => list.flatMap((t) => t.messages.map((m) => [m.id, m.imageTrust]));
    s().acceptSender(false, "a");
    expect(trust(s().threads)).toEqual([["a-1", "you-allowed"], ["a-2", undefined], ["d-1", undefined]]);
    expect(trust(s().doneThreads)).toEqual([["f-1", "you-allowed"]]);
    expect(backend.allowImages).not.toHaveBeenCalled();

    resetStore([thread("a", { from: jordan, sender: "new", messages: [from(jordan, "a-1")] })]);
    s().blockSender(false, "a");
    s().unblockSender("a");
    expect(trust(s().threads)).toEqual([["a-1", "you-allowed"]]);
  });

  it("opens the Blocked list over the current screen, follows the account, and moves its cursor", () => {
    s().blockSender(true, "a");
    s().openBlocked();
    expect(s()).toMatchObject({ view: "blocked", blockedCursorId: "a", history: ["inbox"] });
    s().moveBlockedCursor(-1);
    expect(s().blockedCursorId).toBe("a");
    s().moveBlockedCursor(1);
    s().moveBlockedCursor(1);
    s().moveBlockedCursor(1);
    expect(s().blockedCursorId).toBe("f");
    s().setBlockedCursor("c");
    expect(s().blockedCursorId).toBe("c");
    s().setAccountFilter("personal");
    expect(visibleBlocked(s())).toEqual([]);
    s().openBlocked();
    expect(s().blockedCursorId).toBeUndefined();
    s().moveBlockedCursor(1);
    expect(s().blockedCursorId).toBeUndefined();
  });

  it("opens the Senders list over the current screen, and Back returns there", () => {
    s().open("a");
    s().openSenders();
    expect(s()).toMatchObject({ view: "senders", history: ["thread"] });
    s().leave();
    expect(s().view).toBe("thread");
  });

  it("unblocking brings that sender's mail back to the inbox and moves on", () => {
    s().blockSender(true, "a");
    s().openBlocked();
    s().unblockSender();
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["c"]);
    expect(s().threads.map((t) => [t.id, t.sender])).toEqual([
      ["a", "known"],
      ["f", "known"],
      ["b", undefined],
      ["d", undefined],
      ["e", "new"],
    ]);
    expect(s().threads.find((t) => t.id === "f")).not.toHaveProperty("doneAt");
    expect(s()).toMatchObject({ view: "blocked", blockedCursorId: "c", toast: { text: "Jordan Blake’s mail is back in your inbox" } });
    s().unblockSender("nope");
    expect(s().blockedThreads).toHaveLength(1);
  });

  it("reads blocked mail with ↑ and ↓, unblocks it in place, and goes back to Blocked", () => {
    s().blockSender(true, "a");
    s().openBlocked();
    s().open("a");
    expect(shownView(s())).toBe("thread");
    s().moveCursor(1);
    expect(s()).toMatchObject({ openThreadId: "c", blockedCursorId: "c" });
    s().unblockSender();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "c" });
    expect(s().threads[0]).toMatchObject({ id: "c", sender: "known" });
    s().back();
    expect(s().view).toBe("blocked");
  });

  it("E or H on blocked mail does nothing on the server", () => {
    const { backend } = fakeMail();
    useStore.setState({ blockedThreads: [thread("x", { from: jordan, sender: "blocked" })] });
    s().openBlocked();
    s().open("x");
    s().markDone();
    expect(backend.markDone).not.toHaveBeenCalled();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x", handled: 0 });
  });

  it("sends each change to the mail engine, then reads the lists again", async () => {
    const { backend } = fakeMail({
      threads: vi.fn(async () => [thread("b")]),
      blockedThreads: vi.fn(async () => [thread("a", { from: jordan, sender: "blocked" as const })]),
    });
    s().blockSender(false, "a");
    expect(backend.blockSender).toHaveBeenCalledWith("a", false);
    await tick();
    expect(backend.blockedThreads).toHaveBeenCalled();
    expect(s().threads.map((t) => t.id)).toEqual(["b"]);
    s().acceptSender(false, "b");
    expect(backend.acceptSender).toHaveBeenCalledWith("b", false);
    s().unblockSender("a");
    expect(backend.unblockSender).toHaveBeenCalledWith("a");
  });

  it("loads the Blocked list from the mail engine, and it leaves the inbox", async () => {
    const blocked = thread("a", { from: jordan, sender: "blocked" as const });
    fakeMail({ blockedThreads: vi.fn(async () => [blocked]) });
    s().openBlocked();
    await tick();
    expect(s()).toMatchObject({ blockedThreads: [blocked], blockedCursorId: "a" });
    expect(s().threads.map((t) => t.id)).not.toContain("a");
    useStore.setState({ blockedCursorId: "a" });
    await loadBlocked();
    expect(s().blockedCursorId).toBe("a");
  });
});

describe("undo send", async () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetStore([thread("a"), thread("b"), thread("c")]);
  });
  afterEach(() => vi.useRealTimers());

  /** Presses Send and lets the pressed button show. */
  const send = () => {
    s().send();
    vi.advanceTimersByTime(sendBeatMs);
  };
  /** Waits out the undo time, and lets the send that follows finish. */
  const waitItOut = () => vi.advanceTimersByTimeAsync(undoSendMs);
  /** A send the engine hasn't finished. */
  const pending = (overrides: Partial<MailBackend> = {}) => {
    let finish!: (sent: Sent) => void;
    let fail!: (error: Error) => void;
    const sendAfter = vi.fn(() => new Promise<Sent>((resolve, reject) => ((finish = resolve), (fail = reject))));
    const { backend } = fakeMail({ sendAfter, ...overrides });
    return { backend, finish: (sent: Sent = true) => finish(sent), fail: (why: string, kind?: string) => fail(Object.assign(new Error(why), { kind })) };
  };

  it("waits a few seconds, long enough to read the note: one more than Gmail's 5", async () => {
    expect(undoSendMs).toBe(6000);
  });

  it("Send shows it is pressed at once, then the composer gives way to the email you answered", async () => {
    const { backend } = fakeMail();
    s().compose("reply", "a");
    const draft = s().draft!;
    s().send();
    expect(s()).toMatchObject({ sending: true, view: "compose" });
    // A second press while it shows is ignored.
    s().send();
    expect(backend.sendAfter).toHaveBeenCalledTimes(1);
    expect(backend.sendAfter).toHaveBeenCalledWith(expect.any(String), draft, undoSendMs);
    vi.advanceTimersByTime(sendBeatMs);
    expect(s()).toMatchObject({ sending: false, view: "thread", openThreadId: "a", draft: undefined, handled: 0 });
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(backend.markDone).not.toHaveBeenCalled();
    expect(s().toast).toMatchObject({ text: "Sending", until: Date.now() + undoSendMs - sendBeatMs });
  });

  it("holds the email until the time is up, then it really goes", async () => {
    const { finish } = pending();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    vi.advanceTimersByTime(undoSendMs);
    // Still on its way: the note stays.
    expect(s().toast).toMatchObject({ text: "Sending" });
    finish();
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Sent", undo: false });
    expect(s().undoSteps).toEqual([]);
    // Gone, but where to is not known (the demo mailbox): you stay where you are.
    expect(s().view).toBe("inbox");
  });

  /** Presses View on the Sent note. */
  const view = async () => {
    s().toast!.show!.run();
    await vi.advanceTimersByTimeAsync(0);
  };

  it.each([
    ["the inbox", () => {}, { view: "inbox" }],
    ["an email", () => s().open("b"), { view: "thread", openThreadId: "b" }],
    ["Drafts", () => s().openDrafts(), { view: "drafts" }],
    ["Search", () => s().openSearch(), { view: "search" }],
  ] as const)("a new email started from %s goes back there; Sent offers View, and nothing jumps", async (_, start, there) => {
    const lunch = thread("lunch", { messages: [message({ id: "lunch-1", text: "Lunch on Friday?\n\nMaya" })] });
    const { finish, backend } = pending({ thread: vi.fn(async () => lunch) });
    start();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    expect(s()).toMatchObject(there);
    finish("lunch");
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ ...there, toast: { text: "Sent", show: { label: "View" } } });
    expect(backend.thread).not.toHaveBeenCalled();
    await view();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "lunch" });
    expect(openThread(s())?.messages[0].text).toBe("Lunch on Friday?\n\nMaya");
  });

  it("a new email started from an email that has since gone goes back to the inbox", async () => {
    pending();
    s().open("b");
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    useStore.setState({ threads: s().threads.filter((t) => t.id !== "b") });
    send();
    expect(s().view).toBe("inbox");
  });

  it("View opens a conversation the list already holds, without reading it again", async () => {
    const { finish, backend } = pending();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    finish("c");
    await vi.advanceTimersByTimeAsync(0);
    await view();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "c", cursorId: "c" });
    expect(s().foundThreads).toEqual([]);
    expect(backend.thread).not.toHaveBeenCalledWith("c");
  });

  it("Sent offers nothing to view when where it went isn't known (the demo mailbox)", async () => {
    const { finish } = pending();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    finish(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Sent" });
    expect(s().toast?.show).toBeUndefined();
  });

  it("a sent reply stays in the email until its sent copy has loaded, so nothing flickers", async () => {
    let load!: (t: Thread) => void;
    const { finish, backend } = pending({ thread: vi.fn(() => new Promise<Thread>((r) => (load = r))) });
    s().compose("reply", "a");
    send();
    finish("a");
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.thread).toHaveBeenCalledWith("a");
    // Still shown as sending, until the email has what you sent.
    expect(s().undoSteps).toHaveLength(1);
    expect(s().toast).toMatchObject({ text: "Sending" });
    load(thread("a", { messages: [message({ id: "a-1" }), message({ id: "a-2", text: "Yes.\n\nMaya" })] }));
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", undoSteps: [], toast: { text: "Sent" } });
    expect(s().toast?.show).toBeUndefined();
    expect(openThread(s())?.messages.map((m) => m.id)).toEqual(["a-1", "a-2"]);
  });

  it("a sent reply's Sent note offers View once you've moved on", async () => {
    const { finish, backend } = pending();
    s().compose("reply", "a");
    send();
    s().open("b");
    finish("a");
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.thread).not.toHaveBeenCalledWith("a");
    expect(s()).toMatchObject({ openThreadId: "b", undoSteps: [], toast: { text: "Sent", show: { label: "View" } } });
    await view();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
  });

  it("a forward goes back to the email you forwarded; View opens the new conversation", async () => {
    const { finish } = pending({ thread: vi.fn(async () => thread("fwd")) });
    s().compose("forward", "a");
    s().updateDraft({ to: [lena] });
    send();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
    finish("fwd");
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", toast: { text: "Sent", show: { label: "View" } } });
    await view();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "fwd" });
  });

  it("a reply whose conversation no list holds opens it, then shows what you sent", async () => {
    const far = (...ids: string[]) => thread("far", { messages: ids.map((id) => message({ id })) });
    const conversation = vi.fn(async () => far("far-1"));
    const { finish } = pending({ thread: conversation });
    s().compose("reply", "a");
    s().updateDraft({ threadId: "far" });
    send();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "far" });
    conversation.mockImplementation(async () => far("far-1", "far-2"));
    finish("far");
    await vi.advanceTimersByTimeAsync(0);
    expect(openThread(s())?.messages.map((m) => m.id)).toEqual(["far-1", "far-2"]);
  });

  it("a reply whose conversation loads after you moved on leaves you there", async () => {
    let load!: (t: Thread) => void;
    pending({ thread: vi.fn((id: string) => new Promise<Thread>((r) => { if (id === "far") load = r; })) });
    s().compose("reply", "a");
    s().updateDraft({ threadId: "far" });
    send();
    s().open("b");
    load(thread("far"));
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b" });
    expect(findThread(s(), "far")).toBeUndefined();
  });

  it("a reply whose conversation Moss no longer has goes back to where you started it", async () => {
    const { finish } = pending();
    s().openDrafts();
    s().compose("reply", "a");
    s().updateDraft({ threadId: "far" });
    send();
    await vi.advanceTimersByTimeAsync(0);
    expect(s().view).toBe("drafts");
    finish("elsewhere");
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "drafts", toast: { text: "Sent", show: { label: "View" } } });
    // Moss can't read it either: View leaves you where you are.
    await view();
    expect(s().view).toBe("drafts");
  });

  it("a saved draft sent from Drafts stays in Drafts; View opens its conversation, and Esc comes back", async () => {
    const { finish } = pending({ thread: vi.fn(async () => thread("lunch")) });
    s().openDrafts();
    s().sendDraft(draft("lunch-draft", { mode: "new", subject: "Lunch", to: [lena] }));
    expect(s().view).toBe("drafts");
    finish("lunch");
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "drafts", toast: { text: "Sent", show: { label: "View" } } });
    await view();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "lunch" });
    s().back();
    expect(s().view).toBe("drafts");
  });

  it("a saved draft sent from the end of its email stays there and shows what you sent", async () => {
    const { finish, backend } = pending({ thread: vi.fn(async () => thread("a", { messages: [message({ id: "a-1" }), message({ id: "a-2" })] })) });
    s().open("a");
    s().sendDraft(draft("reply-draft", { mode: "reply", threadId: "a", subject: "Re: a", to: [lena] }));
    finish("a");
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.thread).toHaveBeenCalledWith("a");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", toast: { text: "Sent" } });
    expect(s().toast?.show).toBeUndefined();
    expect(openThread(s())?.messages).toHaveLength(2);
  });

  it("a sent reply says Sent and leaves nothing to take back", async () => {
    s().compose("reply", "a");
    send();
    await waitItOut();
    expect(s().toast).toMatchObject({ text: "Sent", undo: false });
    expect(s().undoSteps).toEqual([]);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a", draft: undefined });
  });

  it("⌘Z in time takes it back and opens it again, exactly as it was", async () => {
    const { backend } = fakeMail();
    s().compose("reply-all", "a");
    s().updateDraft({ bcc: [jonas], body: "See you there", bodyHtml: "<p>See you there</p>", signatureId: "work", signatureChosen: true });
    const draft = s().draft!;
    send();
    vi.advanceTimersByTime(3000);
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.cancelSend).toHaveBeenCalledWith(vi.mocked(backend.sendAfter).mock.lastCall![0]);
    expect(s()).toMatchObject({ view: "compose", draft, openThreadId: "a", toast: undefined, undoSteps: [], handled: 0 });
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(backend.markNotDone).not.toHaveBeenCalled();
    // Esc from there goes back to the email, as from any reply.
    s().back();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
  });

  it("a new email taken back opens in the composer; it never goes", async () => {
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    s().updateDraft({ to: [lena], subject: "Lunch?" });
    send();
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: { mode: "new", to: [lena], subject: "Lunch?" } });
    await waitItOut();
    expect(s().toast).toBeUndefined();
    expect(s().view).toBe("compose");
  });

  it("once it is on its way, ⌘Z can't take it back", async () => {
    const { backend } = pending();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    vi.advanceTimersByTime(undoSendMs);
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.cancelSend).not.toHaveBeenCalled();
    expect(s()).toMatchObject({ view: "inbox", draft: undefined });
  });

  it.each(["deliveryUnknown", "deliveryAccepted", "deliveryBusy"])("never offers an uncertain or accepted send as a sendable draft: %s", async (kind) => {
    const { fail, backend } = pending();
    s().compose("new");
    s().updateDraft({ subject: "Do not duplicate" });
    send();
    fail("Check the saved delivery status", kind);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Check the saved delivery status", problem: true, undo: false });
    expect(s().undoSteps.filter((step) => step.kind === "send")).toEqual([]);
    s().undo();
    expect(s().draft).toBeUndefined();
    expect(backend.cancelSend).not.toHaveBeenCalled();
  });

  it("does not reopen a held draft before cancellation is confirmed", async () => {
    let answer!: (cancelled: boolean) => void;
    pending({ cancelSend: vi.fn(() => new Promise<CancelSendOutcome>((resolve) => { answer = (cancelled) => resolve({ cancelled, early: cancelled }); })) });
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "inbox", draft: undefined });
    answer(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: { subject: "Lunch" } });
  });

  it("keeps a different composer open while confirmed Undo restores the first draft", async () => {
    let answer!: (cancelled: boolean) => void;
    pending({ cancelSend: vi.fn(() => new Promise<CancelSendOutcome>((resolve) => { answer = (cancelled) => resolve({ cancelled, early: cancelled }); })) });
    s().compose("new");
    s().updateDraft({ subject: "First" });
    const first = s().draft!;
    send();
    s().undo();
    s().compose("new");
    s().updateDraft({ subject: "Second" });
    answer(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().draft?.subject).toBe("Second");
    expect(useDrafts.getState().drafts.some((d) => d.id === first.id && d.subject === "First")).toBe(true);
    expect(s().toast?.text).toBe("Email restored to Drafts");
  });

  it("ignores a late send failure after Undo was confirmed", async () => {
    const { fail } = pending();
    s().compose("new");
    s().updateDraft({ subject: "Keep this draft" });
    send();
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    fail("late transport failure");
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: { subject: "Keep this draft" }, sendError: undefined, toast: undefined });
  });

  it("keeps a send closed when Undo cannot be confirmed", async () => {
    pending({ cancelSend: vi.fn(async () => { throw new Error("connection lost"); }) });
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "inbox", draft: undefined });
    expect(s().toast).toMatchObject({ text: "Couldn’t confirm Undo. Your email may still send.", problem: true });
  });

  it("keeps a recovery copy when Undo stopped sending but could not restore the draft", async () => {
    pending({ cancelSend: vi.fn(async () => ({ cancelled: true, early: false, restoreFailed: true })) });
    s().compose("new");
    s().updateDraft({ subject: "Keep after failed restore" });
    send();
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "inbox", draft: undefined });
    expect(useDrafts.getState().recoveries.size).toBeGreaterThan(0);
    expect(s().toast).toMatchObject({ text: "Send cancelled. The draft could not be restored. Its saved data is retained for recovery.", problem: true, stays: true });
  });

  it("says so when Undo came a moment too late", async () => {
    fakeMail({ cancelSend: vi.fn(async () => ({ cancelled: false, early: false })) });
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Too late: it had already gone", problem: true });
    expect(s()).toMatchObject({ view: "inbox", draft: undefined });
  });

  it("another action meanwhile takes the note; ⌘Z undoes that first, then the send", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    s().markDone("b");
    expect(s().toast).toMatchObject({ text: "Moved to Done" });
    s().undo();
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.cancelSend).toHaveBeenCalled();
    expect(s().view).toBe("compose");
  });

  it("going out while another note shows leaves that note alone", async () => {
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    send();
    s().markDone("b");
    await waitItOut();
    expect(s().toast).toMatchObject({ text: "Moved to Done" });
    expect(s().undoSteps).toMatchObject([{ kind: "done" }]);
  });

  it("a send that fails later says so, and ⌘Z opens the email with why", async () => {
    const { fail } = pending();
    s().markDone("c");
    s().compose("reply", "a");
    const draft = s().draft!;
    send();
    vi.advanceTimersByTime(undoSendMs);
    fail("can't reach the server");
    await vi.advanceTimersByTimeAsync(0);
    expect(s().toast).toMatchObject({ text: "Couldn’t send your email", problem: true, undoLabel: "Open", stays: true });
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft, sendError: "can't reach the server" });
    // Only the send is taken back; marking the other email done stays.
    expect(s().undoSteps).toMatchObject([{ kind: "done", thread: { id: "c" } }]);
  });

  it("cancels the durable held entry before reopening a refused send", async () => {
    let confirm!: (cancelled: boolean) => void;
    const { fail, backend } = pending({ cancelSend: vi.fn(() => new Promise<CancelSendOutcome>((resolve) => { confirm = (cancelled) => resolve({ cancelled, early: cancelled }); })) });
    s().compose("new");
    s().updateDraft({ subject: "Retry after offline" });
    send();
    fail("offline");
    await vi.advanceTimersByTimeAsync(0);
    s().undo();
    expect(backend.cancelSend).toHaveBeenCalled();
    expect(s().draft).toBeUndefined();
    confirm(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(s().draft?.subject).toBe("Retry after offline");
  });

  it("a send refused at once keeps you in the composer and says why", async () => {
    const { fail } = pending();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    s().send();
    fail("no one to send it to");
    await vi.advanceTimersByTimeAsync(sendBeatMs);
    expect(s()).toMatchObject({ sending: false, view: "compose", sendError: "no one to send it to" });
    expect(s().undoSteps).toEqual([]);
  });

  it("doesn't open a sent email over one you are writing", async () => {
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    s().updateDraft({ subject: "First" });
    send();
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    s().updateDraft({ subject: "Second" });
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s().draft?.subject).toBe("Second");
  });

  it("a reply to a done email leaves it done, and Undo opens the reply again", async () => {
    resetStore([thread("a")]);
    useStore.setState({ doneThreads: [thread("x", { doneAt: 5 })] });
    s().compose("reply", "x");
    send();
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x" });
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: { threadId: "x" } });
    expect(s().doneThreads.map((t) => t.id)).toEqual(["x"]);
  });

  it("another email failing to save leaves a held reply as it was", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    pending({ markDone: vi.fn(async (id: string) => (id === "b" ? Promise.reject(new Error("offline")) : undefined)) });
    s().markDone("c");
    s().compose("reply", "a");
    send();
    s().markDone("b");
    await vi.advanceTimersByTimeAsync(0);
    expect(s().undoSteps).toMatchObject([{ kind: "done", thread: { id: "c" } }, { kind: "send", draft: { threadId: "a" } }]);
    vi.mocked(console.warn).mockRestore();
  });

  it("a send that fails after Undo came too late can be reopened after the failure", async () => {
    const { fail } = pending({ cancelSend: vi.fn().mockResolvedValueOnce({ cancelled: false, early: false }).mockResolvedValue({ cancelled: true, early: true }) });
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    s().updateDraft({ subject: "Lunch?" });
    send();
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    fail("can't reach the server");
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "inbox", draft: undefined });
    expect(s().toast).toMatchObject({ text: "Couldn’t send your email" });
    s().undo();
    await vi.advanceTimersByTimeAsync(0);
    expect(s()).toMatchObject({ view: "compose", draft: { subject: "Lunch?" }, sendError: "can't reach the server" });
  });

});


it("strict image mode cannot be bypassed by the always-allow shortcut", () => {
  const { backend } = fakeMail();
  resetStore([thread("strict", { messages: [message({ id: "strict-image" })] })]);
  useRemoteImages.setState({ policy: "ask" });
  try {
    s().allowImages("strict-image");
    expect(backend.allowImages).not.toHaveBeenCalled();
    expect(s().threads[0].messages[0].imageTrust).toBeUndefined();
    expect(s().imagesShown).toEqual([]);
    s().showImages("strict-image");
    expect(s().imagesShown).toEqual(["strict-image"]);
  } finally { useRemoteImages.setState({ policy: "familiar", saved: true }); }
});

describe("recipient picks", () => {
  it("remembers who you chose for what you typed, newest first and once each", () => {
    const { backend } = fakeMail();
    useStore.setState({ recipientPicks: [{ typed: "ask", email: "a@x.io" }, { typed: "jo", email: "jo@x.io" }] });
    s().pickRecipient(" Jo ", "JO@x.io");
    expect(s().recipientPicks).toEqual([{ typed: "jo", email: "jo@x.io" }, { typed: "ask", email: "a@x.io" }]);
    expect(backend.pickRecipient).toHaveBeenCalledWith("jo", "jo@x.io");
  });
});
