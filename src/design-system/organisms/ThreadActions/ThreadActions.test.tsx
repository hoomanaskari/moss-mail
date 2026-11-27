import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ThreadActions, type ThreadActionsProps } from "./ThreadActions";

function setup(props: Partial<ThreadActionsProps> = {}) {
  const p = { done: false, onDone: vi.fn(), onRemind: vi.fn(), onRemindNoReply: vi.fn(), onDelete: vi.fn(), ...props };
  render(<ThreadActions {...p} />);
  return { p, user: userEvent.setup() };
}

// Tuesday 3 November 2026, 1:20 PM: every ready-made time is on offer.
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 10, 3, 13, 20));
});
afterEach(() => vi.useRealTimers());

describe("ThreadActions", () => {
  it("shows Done, Remind and Delete with their keys, each named for a tooltip", async () => {
    const { p, user } = setup();
    const done = screen.getByRole("button", { name: "DoneE" });
    expect(done).toHaveAttribute("title", "Done");
    expect(screen.getByRole("button", { name: "RemindH" })).toHaveAttribute("aria-expanded", "false");
    await user.click(done);
    expect(p.onDone).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "DeleteBackspace" }));
    expect(p.onDelete).toHaveBeenCalledOnce();
  });

  it("offers Move, with its key, when the email can move", async () => {
    const onMove = vi.fn();
    const { user } = setup({ onMove });
    const move = screen.getByRole("button", { name: "MoveM" });
    expect(move).toHaveAttribute("title", "Move to another tab");
    await user.click(move);
    expect(onMove).toHaveBeenCalledOnce();
  });

  it("offers Not done for an email that is done", () => {
    setup({ done: true });
    expect(screen.getByRole("button", { name: "Not doneE" })).toHaveAttribute("title", "Not done");
    expect(screen.queryByRole("button", { name: "DoneE" })).not.toBeInTheDocument();
  });

  it("deleted mail offers only Back to inbox", async () => {
    const onRestore = vi.fn();
    const { user } = setup({ onRestore, onMove: vi.fn() });
    expect(screen.getAllByRole("button")).toHaveLength(1);
    const back = screen.getByRole("button", { name: "Back to inboxE" });
    expect(back).toHaveAttribute("title", "Put it back in your inbox");
    await user.click(back);
    expect(onRestore).toHaveBeenCalledOnce();
  });

  it("leaves out Delete when the email can't be deleted", () => {
    setup({ onDelete: undefined });
    expect(screen.queryByRole("button", { name: /Delete/ })).not.toBeInTheDocument();
  });

  it("Remind opens a menu of times, and choosing one closes it", async () => {
    const { p, user } = setup();
    const remind = screen.getByRole("button", { name: "RemindH" });
    await user.click(remind);
    expect(remind).toHaveAttribute("aria-expanded", "true");
    const menu = within(screen.getByRole("group", { name: "Remind" }));
    await user.click(menu.getByRole("button", { name: "This weekendSat 8 AM" }));
    expect(p.onRemind).toHaveBeenCalledWith(new Date(2026, 10, 7, 8, 0).getTime());
    expect(screen.queryByRole("group")).not.toBeInTheDocument();
    await user.click(remind);
    await user.click(screen.getByRole("button", { name: "Remind me if no replyShift+H" }));
    expect(p.onRemindNoReply).toHaveBeenCalledOnce();
    expect(screen.queryByRole("group")).not.toBeInTheDocument();
  });

  it("↑ ↓ ↵ pick a time in the open menu, starting on Tomorrow, and no key reaches the app", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const { p, user } = setup();
    await user.click(screen.getByRole("button", { name: "RemindH" }));
    // Hovering never moves the mark: ↵ picks Tomorrow.
    await user.hover(screen.getByRole("button", { name: "Remind me if no replyShift+H" }));
    await user.keyboard("{Enter}");
    expect(p.onRemind).toHaveBeenCalledWith(new Date(2026, 10, 4, 8, 0).getTime());
    expect(screen.queryByRole("group")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "RemindH" }));
    await user.keyboard("{ArrowUp}{ArrowUp}{Enter}");
    expect(p.onRemindNoReply).toHaveBeenCalledOnce();
    expect(outer).not.toHaveBeenCalled();
    window.removeEventListener("keydown", outer);
  });

  it("picks your own date and time", async () => {
    const { p, user } = setup();
    await user.click(screen.getByRole("button", { name: "RemindH" }));
    await user.click(screen.getByRole("button", { name: "Pick a date and time…" }));
    await user.click(screen.getByRole("button", { name: "Remind me Wed 8 AM" }));
    expect(p.onRemind).toHaveBeenCalledWith(new Date(2026, 10, 4, 8, 0).getTime());
    expect(screen.queryByRole("group")).not.toBeInTheDocument();
  });

  it("opens and closes when its owner says, e.g. for the H key", async () => {
    const onRemindOpenChange = vi.fn();
    const { user } = setup({ remindOpen: true, onRemindOpenChange });
    expect(screen.getByRole("button", { name: "RemindH" })).toHaveAttribute("aria-expanded", "true");
    await user.keyboard("{Escape}");
    expect(onRemindOpenChange).toHaveBeenLastCalledWith(false);
  });
});
