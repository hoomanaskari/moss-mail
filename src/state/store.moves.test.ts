/** Moving an email to another tab yourself (M), and waiting for an answer to your own email. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { lena, maya, message, resetStore, thread } from "../test/fixtures";
import { findThread, splitCountsFor, useStore, visibleThreads } from "./store";

const s = () => useStore.getState();
const ids = (split: Parameters<typeof visibleThreads>[0]["split"]) => visibleThreads({ ...s(), split }).map((t) => t.id);

const tuesday = new Date(2026, 10, 3, 13, 20).getTime();
const saturday = new Date(2026, 10, 7, 8, 0).getTime();

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(tuesday);
});
afterEach(() => vi.useRealTimers());

/** Same cases as `moved_to_important_sits_on_top_of_what_needs_you…` in moss-store's moves.rs. */
describe("what you moved to Important", () => {
  it("comes first in its section, in Important only", () => {
    const threads = [
      thread("newer"),
      thread("moved", { movedByYou: true }),
      thread("info", { section: "info" }),
      thread("later", { section: "later" }),
      thread("movedLater", { section: "later", movedByYou: true }),
      thread("news", { split: "newsletters", section: "info" }),
      thread("movedNews", { split: "newsletters", section: "info", movedByYou: true }),
    ];
    expect(visibleThreads({ threads, accountFilter: "all", split: "important" }).map((t) => t.id)).toEqual([
      "moved",
      "newer",
      "info",
      "movedLater",
      "later",
    ]);
    expect(visibleThreads({ threads, accountFilter: "all", split: "newsletters" }).map((t) => t.id)).toEqual(["news", "movedNews"]);
  });
});

describe("moving to another tab", () => {
  it("to Important, it needs you: on top, counted, and Moss's sorting leaves it there", () => {
    const { backend } = fakeMail();
    resetStore([thread("r1", { split: "receipts", section: "info" }), thread("r2", { split: "receipts", section: "info" }), thread("a")]);
    s().setSplit("receipts");
    s().openMove();
    expect(s().movingId).toBe("r1");
    s().moveTo("important");
    expect(backend.moveThread).toHaveBeenCalledWith("r1", "important");
    expect(findThread(s(), "r1")).toMatchObject({ split: "important", section: "reply", movedByYou: true });
    expect(ids("receipts")).toEqual(["r2"]);
    expect(ids("important")).toEqual(["r1", "a"]);
    expect(splitCountsFor(s().threads, "all").important).toBe(2);
    expect(s()).toMatchObject({ movingId: undefined, cursorId: "r2", toast: { text: "Moved to Important" } });
  });

  it("out of Important, it's for your information there", () => {
    resetStore([thread("a"), thread("b")]);
    s().moveTo("newsletters", "a");
    expect(findThread(s(), "a")).toMatchObject({ split: "newsletters", section: "info", movedByYou: true });
    expect(s()).toMatchObject({ cursorId: "b", toast: { text: "Moved to Newsletters" } });
  });

  it("snoozed mail stays snoozed wherever it goes", () => {
    resetStore([thread("a", { section: "later" })]);
    s().moveTo("other", "a");
    expect(findThread(s(), "a")).toMatchObject({ split: "other", section: "later" });
    s().moveTo("important", "a");
    expect(findThread(s(), "a")).toMatchObject({ split: "important", section: "later" });
  });

  it("given back to Moss, it is no longer yours to place", () => {
    const { backend } = fakeMail();
    resetStore([thread("a", { split: "other", section: "info", movedByYou: true })]);
    s().moveTo(null, "a");
    expect(backend.moveThread).toHaveBeenCalledWith("a", null);
    expect(findThread(s(), "a")?.movedByYou).toBeUndefined();
    expect(s().toast).toMatchObject({ text: "Moss sorts it again" });
  });

  it("reading it, you stay on it", () => {
    resetStore([thread("a"), thread("b")]);
    s().open("a");
    s().openMove();
    expect(s().movingId).toBe("a");
    s().moveTo("other");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
  });

  it("from the Done list it stays done, in its new tab", () => {
    resetStore([]);
    useStore.setState({ doneThreads: [thread("d", { doneAt: 1 })], doneCursorId: "d" });
    s().openDone();
    s().openMove();
    expect(s().movingId).toBe("d");
    s().moveTo("important");
    expect(s().doneThreads[0]).toMatchObject({ id: "d", split: "important", movedByYou: true, doneAt: 1 });
    expect(s().threads).toEqual([]);
  });

  it("an email no list holds goes to the engine, which reloads the lists", () => {
    const { backend } = fakeMail();
    resetStore([]);
    s().moveTo("important", "found");
    expect(backend.moveThread).toHaveBeenCalledWith("found", "important");
    expect(s().toast).toMatchObject({ text: "Moved to Important" });
  });

  it("does nothing with no email, and the menu closes", () => {
    resetStore([]);
    s().openMove();
    expect(s().movingId).toBeUndefined();
    useStore.setState({ movingId: "x" });
    s().closeMove();
    expect(s().movingId).toBeUndefined();
    s().moveTo("other");
    expect(s().toast).toBeUndefined();
  });

  it("⌘Z puts it back as it was, and gives it back to Moss's sorting", () => {
    const { backend } = fakeMail();
    resetStore([thread("r", { split: "receipts", section: "info" }), thread("s", { split: "receipts", section: "info" })]);
    s().setSplit("receipts");
    s().moveTo("important", "r");
    s().undo();
    expect(backend.moveThread).toHaveBeenLastCalledWith("r", null);
    expect(findThread(s(), "r")).toEqual(thread("r", { split: "receipts", section: "info" }));
    expect(s()).toMatchObject({ cursorId: "r", undoSteps: [], toast: undefined });
  });

  it("⌘Z after moving one you had placed puts it back in your tab", () => {
    const { backend } = fakeMail();
    resetStore([thread("a", { split: "other", section: "info", movedByYou: true })]);
    s().moveTo("important", "a");
    s().undo();
    expect(backend.moveThread).toHaveBeenLastCalledWith("a", "other");
  });

  it("⌘Z after moving a done one puts it back in the Done list as it was", () => {
    resetStore([]);
    useStore.setState({ doneThreads: [thread("d", { doneAt: 1 })] });
    s().moveTo("other", "d");
    s().undo();
    expect(s().doneThreads).toEqual([thread("d", { doneAt: 1 })]);
  });
});

describe("waiting for an answer", () => {
  const mine = () => thread("sent", { split: "other", section: "info", messages: [message({ id: "s1", from: maya, to: [lena] })] });

  it("moving your own email to Important asks when to bring it back if no one replies", () => {
    resetStore([mine(), thread("b")]);
    s().moveTo("important", "sent");
    expect(s()).toMatchObject({ remindingId: "sent", remindNoReply: true });
  });

  it("the engine says whose the latest email is when its messages haven't loaded", () => {
    resetStore([thread("a", { split: "other", partial: true, youWroteLast: true }), thread("b", { partial: true })]);
    s().moveTo("important", "a");
    expect(s().remindNoReply).toBe(true);
    s().closeRemind();
    expect(s()).toMatchObject({ remindingId: undefined, remindNoReply: false });
    s().moveTo("important", "b");
    expect(s().remindingId).toBeUndefined();
  });

  it("picking a time hides it until then, and says so", () => {
    const { backend } = fakeMail();
    resetStore([mine(), thread("b")]);
    s().moveTo("important", "sent");
    s().waitForReply(saturday);
    expect(backend.waitForReply).toHaveBeenCalledWith("sent", saturday);
    expect(findThread(s(), "sent")).toMatchObject({
      split: "important",
      section: "later",
      time: "Sat",
      movedByYou: true,
      waiting: { until: saturday, why: { by: "you" } },
    });
    expect(s()).toMatchObject({ remindingId: undefined, remindNoReply: false, toast: { text: "Back Sat 8 AM if no one replies" } });
    s().undo();
    expect(backend.snooze).toHaveBeenLastCalledWith("sent", null);
  });

  it("an email of yours only Search found joins the list, out of sight until then", () => {
    resetStore([thread("b")]);
    useStore.setState({ foundThreads: [mine()], view: "search" });
    s().waitForReply(saturday, "sent");
    expect(s().foundThreads).toEqual([]);
    expect(findThread(s(), "sent")).toMatchObject({ split: "important", section: "later" });
    expect(s().view).toBe("search");
  });

  it("⇧H asks only when the latest email is yours", () => {
    resetStore([thread("a"), mine()]);
    s().openRemind("a", true);
    expect(s().remindingId).toBeUndefined();
    expect(s().toast).toMatchObject({ text: "The latest email isn’t yours, so there’s no answer to wait for" });
    s().openRemind("sent", true);
    expect(s()).toMatchObject({ remindingId: "sent", remindNoReply: true });
    s().openRemind("sent");
    expect(s().remindNoReply).toBe(false);
  });

  it("does nothing with no email", () => {
    resetStore([]);
    s().waitForReply(saturday);
    expect(s().toast).toBeUndefined();
  });
});
