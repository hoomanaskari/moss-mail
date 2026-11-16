import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ThreadRow, type ThreadRowProps } from "./ThreadRow";

function setup(props: Partial<ThreadRowProps> = {}) {
  const handlers = { onOpen: vi.fn(), onDone: vi.fn(), onLater: vi.fn(), onReply: vi.fn(), onDelete: vi.fn() };
  const ref = createRef<HTMLDivElement>();
  render(
    <ThreadRow
      ref={ref}
      account={{ letter: "W", color: "work" }}
      from="Lena Vogt"
      subject="Q4 budget"
      time="9:12 AM"
      section="reply"
      selected={false}
      {...handlers}
      {...props}
    />,
  );
  return { row: screen.getByRole("option"), ref, ...handlers };
}

describe("ThreadRow", () => {
  it("shows sender, subject, account and time when not selected", () => {
    const { row, ref } = setup();
    expect(ref.current).toBe(row);
    expect(row).toHaveAttribute("aria-selected", "false");
    expect(row).toHaveTextContent("Lena Vogt");
    expect(row).toHaveTextContent("Q4 budget");
    expect(row).toHaveTextContent("9:12 AM");
    expect(screen.getByText("W")).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("says when it is unread, for screen readers too", () => {
    expect(setup({ unread: true }).row).toHaveAccessibleName(/^Unread/);
  });

  it("says nothing extra when it is read", () => {
    expect(setup().row).not.toHaveAccessibleName(/Unread/);
  });

  it("shows the summary and due pill when given", () => {
    const { row } = setup({ summary: "Lena needs numbers by Wednesday", due: "Due Wed" });
    expect(row).toHaveTextContent("Lena needs numbers by Wednesday");
    expect(screen.getByText("Due Wed")).toBeInTheDocument();
    expect(screen.queryByTitle(/waiting/)).not.toBeInTheDocument();
  });

  it("a No reply chip says it waits on an answer, so it isn't read as a due day", () => {
    setup({ due: "No reply", noReply: true });
    expect(screen.getByTitle("You’re waiting for an answer")).toHaveTextContent("No reply");
  });

  it("the selected row swaps the time for Done, Later, Reply and Delete", async () => {
    const { row, onDone, onLater, onReply, onDelete, onOpen } = setup({ selected: true });
    expect(row).toHaveAttribute("aria-selected", "true");
    expect(row).not.toHaveTextContent("9:12 AM");
    await userEvent.click(screen.getByRole("button", { name: /Done/ }));
    await userEvent.click(screen.getByRole("button", { name: /Later/ }));
    await userEvent.click(screen.getByRole("button", { name: /Reply/ }));
    await userEvent.click(screen.getByRole("button", { name: "BackspaceDelete" }));
    expect(onDone).toHaveBeenCalledOnce();
    expect(onLater).toHaveBeenCalledOnce();
    expect(onReply).toHaveBeenCalledOnce();
    expect(onDelete).toHaveBeenCalledOnce();
    // Clicking a shortcut must not also open the thread.
    expect(onOpen).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Block/ })).toBeNull();
  });

  it("marks an email you moved to Important, for screen readers too", () => {
    const { row } = setup({ moved: true });
    expect(screen.getByRole("img", { name: "You moved it to Important" })).toBeInTheDocument();
    expect(row).toHaveTextContent("9:12 AM");
  });

  it("the selected row offers Move when it can move", async () => {
    const onMove = vi.fn();
    const { onOpen } = setup({ selected: true, onMove });
    await userEvent.click(screen.getByRole("button", { name: "MMove" }));
    expect(onMove).toHaveBeenCalledOnce();
    expect(onOpen).not.toHaveBeenCalled();
    expect(screen.queryByRole("img", { name: /moved/ })).toBeNull();
  });

  it("a new sender's selected row also offers Block", async () => {
    const onBlock = vi.fn();
    const { onOpen } = setup({ selected: true, onBlock });
    await userEvent.click(screen.getByRole("button", { name: "BBlock" }));
    expect(onBlock).toHaveBeenCalledOnce();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("a snoozed row shows its return time and no summary, even when selected", () => {
    const { row } = setup({ section: "later", selected: true, summary: "Hidden summary", time: "Mon 9 AM" });
    expect(row).toHaveTextContent("Q4 budget");
    expect(row).toHaveTextContent("Mon 9 AM");
    expect(row).not.toHaveTextContent("Hidden summary");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("moving the mouse over it does nothing; a click opens it", async () => {
    const { row, onOpen } = setup({ section: "info" });
    await userEvent.hover(row);
    await userEvent.pointer({ target: row, coords: { x: 5, y: 5 } });
    expect(row).toHaveAttribute("aria-selected", "false");
    expect(onOpen).not.toHaveBeenCalled();
    await userEvent.click(row);
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("can carry an attachment mark and its own actions in place of the usual ones", async () => {
    const onPick = vi.fn();
    const { row } = setup({
      selected: true,
      attachment: true,
      subject: <b>Budget</b>,
      actions: (
        <button type="button" onClick={onPick}>
          Open
        </button>
      ),
    });
    expect(screen.getByLabelText("Has an attachment")).toBeInTheDocument();
    expect(row).toHaveTextContent("Budget");
    expect(screen.queryByRole("button", { name: /Later/ })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(onPick).toHaveBeenCalled();
  });
});
