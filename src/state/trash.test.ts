import { act } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { personal, resetStore, thread, work } from "../test/fixtures";
import { useStore } from "./store";
import { loadTrash, moveTrashCursor, moveTrashReading, openFromTrash, openTrash, restoreToInbox, trashCursor, useTrash, visibleTrash } from "./trash";

const s = () => useStore.getState();
const t = () => useTrash.getState();
const deleted = () => [
  thread("x", { subject: "Old invoice", doneAt: 5 }),
  thread("y", { subject: "Party", accountId: "personal" }),
  thread("z", { subject: "Newsletter" }),
];
const flush = () => act(async () => {});

beforeEach(() => {
  resetStore([thread("a")], [work, personal]);
  useTrash.setState({ threads: deleted(), cursorId: undefined });
});

describe("Trash", () => {
  it("opens on its first email and Esc goes back where you were", () => {
    openTrash();
    expect(s().view).toBe("trash");
    expect(t().cursorId).toBe("x");
    act(() => s().leave());
    expect(s().view).toBe("inbox");
  });

  it("shows the account you are looking at", () => {
    act(() => s().setAccountFilter("personal"));
    openTrash();
    expect(visibleTrash(t().threads, s().accountFilter).map((x) => x.id)).toEqual(["y"]);
    expect(t().cursorId).toBe("y");
  });

  it("the cursor moves and stops at either end", () => {
    openTrash();
    moveTrashCursor(-1);
    expect(t().cursorId).toBe("x");
    moveTrashCursor(1);
    moveTrashCursor(1);
    moveTrashCursor(1);
    expect(t().cursorId).toBe("z");
    useTrash.setState({ threads: [] });
    moveTrashCursor(1);
    expect(t().cursorId).toBeUndefined();
  });

  it("puts an email back in the inbox, open again, and moves on to the next", () => {
    openTrash();
    restoreToInbox();
    expect(t().threads.map((x) => x.id)).toEqual(["y", "z"]);
    expect(t().cursorId).toBe("y");
    expect(s().threads.map((x) => x.id)).toEqual(["x", "a"]);
    expect(s().threads[0].doneAt).toBeUndefined();
    expect(s().toast?.text).toBe("Back in your inbox");
    restoreToInbox("z");
    expect(t().cursorId).toBe("y");
    restoreToInbox("nope");
    expect(t().threads.map((x) => x.id)).toEqual(["y"]);
  });

  it("the last email out leaves no cursor", () => {
    useTrash.setState({ threads: [thread("x")] });
    openTrash();
    restoreToInbox();
    expect(t().cursorId).toBeUndefined();
    restoreToInbox();
    expect(s().threads.map((x) => x.id)).toEqual(["x", "a"]);
  });

  it("opens an email to read it, and E there puts the one you're reading back", () => {
    openTrash();
    openFromTrash("y");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "y" });
    expect(trashCursor()).toBe("y");
    expect(s().foundThreads.map((x) => x.id)).toEqual(["y"]);
    restoreToInbox();
    expect(s().foundThreads).toEqual([]);
    expect(s().threads.map((x) => x.id)).toEqual(["y", "a"]);
    // Opening one that is already found doesn't add it twice.
    openFromTrash("z");
    openFromTrash("z");
    expect(s().foundThreads.map((x) => x.id)).toEqual(["z"]);
    openFromTrash("nope");
    expect(s().openThreadId).toBe("z");
  });

  it("reading one the account you're looking at doesn't hold, ↑/↓ have nowhere to go", () => {
    useTrash.setState({ threads: [thread("x")] });
    openTrash();
    openFromTrash("x");
    useStore.setState({ accountFilter: "personal" });
    moveTrashReading(1);
    expect(s().openThreadId).toBe("x");
  });

  it("knows which email you mean: the open one, else the cursor's", () => {
    openTrash();
    expect(trashCursor()).toBe("x");
    act(() => s().open("a"));
    expect(trashCursor()).toBe("a");
  });

  describe("with real mail", () => {
    it("reads the list from the engine each time it opens, keeping the cursor where it can", async () => {
      const { backend } = fakeMail({ trashThreads: vi.fn(async () => deleted().slice(1)) });
      useTrash.setState({ threads: [], cursorId: undefined });
      openTrash();
      await flush();
      expect(backend.trashThreads).toHaveBeenCalledTimes(1);
      expect(t()).toMatchObject({ cursorId: "y" });
      moveTrashCursor(1);
      await act(() => loadTrash());
      expect(t().cursorId).toBe("z");
      vi.mocked(backend.trashThreads).mockResolvedValueOnce(deleted().slice(1, 2));
      await act(() => loadTrash());
      expect(t().cursorId).toBe("y");
      openTrash();
      await flush();
      expect(backend.trashThreads).toHaveBeenCalledTimes(4);
    });

    it("asks the engine to put it back, then reads the inbox again", async () => {
      const { backend } = fakeMail();
      openTrash();
      restoreToInbox();
      await flush();
      expect(backend.restoreToInbox).toHaveBeenCalledWith("x");
      expect(backend.threads).toHaveBeenCalled();
    });

    it("a refusal says so and reads the lists again", async () => {
      vi.spyOn(console, "warn").mockImplementation(() => {});
      const { backend } = fakeMail({ restoreToInbox: vi.fn(async () => Promise.reject(new Error("no"))), trashThreads: vi.fn(async () => deleted()) });
      openTrash();
      await flush();
      restoreToInbox();
      await flush();
      expect(s().toast).toMatchObject({ text: "Couldn’t put that back, so it is still in Trash", problem: true });
      expect(backend.trashThreads).toHaveBeenCalledTimes(2);
      expect(t().threads.map((x) => x.id)).toEqual(["x", "y", "z"]);
    });
  });
});
