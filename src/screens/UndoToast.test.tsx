import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { sendBeatMs, undoSendMs, useStore } from "../state/store";
import { resetStore, thread } from "../test/fixtures";
import { UndoToast } from "./UndoToast";

const s = () => useStore.getState();

beforeEach(() => resetStore([thread("a"), thread("b"), thread("c")]));
afterEach(() => vi.useRealTimers());

describe("UndoToast", () => {
  it("shows nothing until there is something to undo", () => {
    render(<UndoToast />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("says where the email went, and Undo brings it back", async () => {
    render(<UndoToast />);
    act(() => s().markDone("a"));
    expect(screen.getByRole("status")).toHaveTextContent("Moved to Done");
    await userEvent.click(screen.getByRole("button", { name: "UndoCtrl+Z" }));
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("says when a change didn't save, with no Undo to offer", () => {
    render(<UndoToast />);
    act(() => useStore.setState({ toast: { id: 1, text: "Couldn’t save that, so Moss put it back", problem: true } }));
    expect(screen.getByRole("status")).toHaveTextContent("Couldn’t save that, so Moss put it back");
    expect(screen.queryByRole("button", { name: /Undo/ })).not.toBeInTheDocument();
    expect(screen.getByRole("status").querySelector("svg")).toHaveClass("lucide-circle-alert");
  });

  it("confirms a change ⌘Z can't take back with a tick and no Undo", () => {
    render(<UndoToast />);
    act(() => useStore.setState({ toast: { id: 1, text: "Blocked Jordan Blake", undo: false } }));
    expect(screen.getByRole("status")).toHaveTextContent("Blocked Jordan Blake");
    expect(screen.queryByRole("button", { name: /Undo/ })).not.toBeInTheDocument();
    expect(screen.getByRole("status").querySelector("svg")).toHaveClass("lucide-check");
  });

  it("offers a way to see where a change went, in place of Undo", async () => {
    const run = vi.fn();
    render(<UndoToast />);
    act(() => useStore.setState({ toast: { id: 1, text: "Blocked Jordan Blake", undo: false, show: { label: "Show Blocked", keys: "⌘⇧B", run } } }));
    expect(screen.queryByRole("button", { name: /Undo/ })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show BlockedCtrl+Shift+B" }));
    expect(run).toHaveBeenCalledOnce();
  });

  it("sits above the shortcut bar, but top right on inbox zero where that space is the hero's", () => {
    render(<UndoToast />);
    act(() => s().markDone("a"));
    const dock = () => screen.getByRole("status").parentElement;
    expect(dock()).toHaveAttribute("data-place", "bottom");
    act(() => useStore.setState({ view: "zero" }));
    expect(dock()).toHaveAttribute("data-place", "top-right");
  });

  it("goes away after a few seconds; a new action starts the wait again", () => {
    vi.useFakeTimers();
    render(<UndoToast />);
    act(() => s().markDone("a"));
    act(() => vi.advanceTimersByTime(5000));
    act(() => s().markDone("b"));
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByRole("status")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(s().undoSteps).toHaveLength(2);
  });

  it("counts down while a sent email waits, and Undo takes it back", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    render(<UndoToast />);
    act(() => {
      s().compose("new");
      s().updateDraft({ subject: "Lunch" });
      s().send();
      vi.advanceTimersByTime(sendBeatMs);
    });
    expect(screen.getByRole("status")).toHaveTextContent("Sending in 6s");
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByRole("status")).toHaveTextContent("Sending in 5s");
    await userEvent.click(screen.getByRole("button", { name: "UndoCtrl+Z" }));
    expect(s().view).toBe("compose");
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("stays up until the email has gone, saying it is sending once Undo is too late", () => {
    vi.useFakeTimers();
    render(<UndoToast />);
    act(() => useStore.setState({ toast: { id: 1, text: "Sending", until: Date.now() + undoSendMs, hold: "h" } }));
    act(() => vi.advanceTimersByTime(undoSendMs + 4000));
    expect(screen.getByRole("status")).toHaveTextContent(/^Sending…$/);
    expect(screen.queryByRole("button", { name: /Undo/ })).not.toBeInTheDocument();
  });

  it("a send that failed stays up and opens the email again", async () => {
    render(<UndoToast />);
    const open = vi.spyOn(s(), "undo");
    act(() =>
      useStore.setState({ toast: { id: 1, text: "Couldn’t send your email", problem: true, undoLabel: "Open", stays: true }, undo: open }),
    );
    expect(screen.getByRole("status").querySelector("svg")).toHaveClass("lucide-circle-alert");
    await userEvent.click(screen.getByRole("button", { name: "OpenCtrl+Z" }));
    expect(open).toHaveBeenCalledOnce();
  });

  it("a note that stays doesn't go away by itself", () => {
    vi.useFakeTimers();
    render(<UndoToast />);
    act(() => useStore.setState({ toast: { id: 1, text: "Couldn’t send your email", problem: true, undoLabel: "Open", stays: true } }));
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByRole("status")).toBeInTheDocument();
  });
});
