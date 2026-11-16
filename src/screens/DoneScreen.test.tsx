import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "../state/store";
import { personal, resetStore, thread, work } from "../test/fixtures";
import { DoneScreen } from "./DoneScreen";
import { changeWhatNoListShows, renderCounted } from "../test/renders";

const s = () => useStore.getState();
const now = Date.now();
const rows = () => screen.queryAllByRole("option");
const selected = () => rows().find((r) => r.getAttribute("aria-selected") === "true");

beforeEach(() => {
  resetStore([thread("a")], [work, personal]);
  useStore.setState({
    doneThreads: [
      thread("x", { subject: "Standup notes", doneAt: now - 60_000 }),
      thread("y", { subject: "Dinner on Sunday?", accountId: "personal", doneAt: now - 86_400_000 }),
      thread("z", { subject: "Q3 spend review", doneAt: now - 30 * 86_400_000 }),
    ],
  });
  s().openDone();
});

describe("DoneScreen", () => {
  it("lists what you marked done, newest first, by the day you did it", () => {
    render(<DoneScreen />);
    expect(screen.getByRole("heading", { name: "Done" })).toBeInTheDocument();
    expect(screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"))).toEqual(["Today", "Yesterday", "Older"]);
    expect(within(screen.getByRole("group", { name: "Today" })).getByRole("option")).toHaveTextContent("Standup notes");
    expect(selected()).toHaveTextContent("Standup notes");
  });

  it("says nothing is lost: done emails are archived, not deleted", () => {
    render(<DoneScreen />);
    expect(screen.getByText(/archived, not deleted/)).toBeInTheDocument();
  });

  it("follows the account you are looking at", () => {
    act(() => s().setAccountFilter("personal"));
    act(() => s().openDone());
    render(<DoneScreen />);
    expect(screen.getByRole("heading", { name: "Done in Personal" })).toBeInTheDocument();
    expect(rows()).toHaveLength(1);
  });

  it("the arrows move, J and K do nothing, ↵ opens", async () => {
    const user = userEvent.setup();
    render(<DoneScreen />);
    await user.keyboard("jJ");
    expect(selected()).toHaveTextContent("Standup notes");
    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent("Dinner on Sunday?");
    await user.keyboard("kK");
    expect(selected()).toHaveTextContent("Dinner on Sunday?");
    await user.keyboard("{ArrowDown}{ArrowUp}{ArrowUp}");
    expect(selected()).toHaveTextContent("Standup notes");
    await user.keyboard("{Enter}");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x" });
  });

  it("O opens too", async () => {
    render(<DoneScreen />);
    await userEvent.keyboard("o");
    expect(s().openThreadId).toBe("x");
  });

  it("E brings the selected email back to the inbox, and ⌘Z puts it back", async () => {
    const user = userEvent.setup();
    render(<DoneScreen />);
    await user.keyboard("e");
    expect(s().threads.map((t) => t.id)).toEqual(["x", "a"]);
    expect(selected()).toHaveTextContent("Dinner on Sunday?");
    await user.keyboard("{Control>}z{/Control}");
    expect(selected()).toHaveTextContent("Standup notes");
    expect(rows()).toHaveLength(3);
  });

  it("M opens the Move menu for the selected email, and the list's keys wait while it is open", async () => {
    const user = userEvent.setup();
    render(<DoneScreen />);
    await user.keyboard("m");
    expect(s().movingId).toBe("x");
    await user.keyboard("e");
    expect(rows()).toHaveLength(3);
  });

  it("⌫ or Delete moves the selected email to Trash, and ⌘Z puts it back", async () => {
    const user = userEvent.setup();
    render(<DoneScreen />);
    await user.keyboard("{Backspace}");
    expect(rows()).toHaveLength(2);
    expect(selected()).toHaveTextContent("Dinner on Sunday?");
    expect(s().threads.map((t) => t.id)).toEqual(["a"]);
    await user.keyboard("{Delete}");
    expect(rows()).toHaveLength(1);
    await user.keyboard("{Control>}z{/Control}");
    expect(selected()).toHaveTextContent("Dinner on Sunday?");
    expect(rows()).toHaveLength(2);
  });

  it("pointing leaves the selection alone; clicking opens and selects; the row has Open, Not done and Delete", async () => {
    const user = userEvent.setup();
    render(<DoneScreen />);
    await user.hover(rows()[2]);
    expect(selected()).toHaveTextContent("Standup notes");
    await user.click(rows()[2]);
    expect(s().openThreadId).toBe("z");
    act(() => s().back());
    expect(selected()).toHaveTextContent("Q3 spend review");
    await user.click(within(rows()[2]).getByRole("button", { name: "ENot done" }));
    expect(s().threads.map((t) => t.id)).toContain("z");
    await user.click(within(selected()!).getByRole("button", { name: "BackspaceDelete" }));
    expect(rows()).toHaveLength(1);
    await user.click(within(selected()!).getByRole("button", { name: "↵Open" }));
    expect(s().view).toBe("thread");
    act(() => s().back());
    await user.click(rows()[0]);
    expect(s().openThreadId).toBe("x");
  });

  it("Esc and the top bar go back to where you were", async () => {
    const user = userEvent.setup();
    render(<DoneScreen />);
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
    act(() => s().openDone());
    await user.click(screen.getByRole("button", { name: "EscBack" }));
    expect(s().view).toBe("inbox");
  });

  it("/ searches, other keys and ⌘ combinations are left alone", async () => {
    const user = userEvent.setup();
    render(<DoneScreen />);
    await user.keyboard("{Control>}j{/Control}q");
    expect(selected()).toHaveTextContent("Standup notes");
    await user.keyboard("/");
    expect(s().view).toBe("search");
  });

  it("points to Search once the list is as long as it gets", () => {
    useStore.setState({ doneThreads: Array.from({ length: 200 }, (_, i) => thread(`d${i}`, { doneAt: now })) });
    const { unmount } = render(<DoneScreen />);
    expect(screen.getByText("Older done emails are in Search.")).toBeInTheDocument();
    unmount();
    useStore.setState({ doneThreads: s().doneThreads.slice(1) });
    render(<DoneScreen />);
    expect(screen.queryByText("Older done emails are in Search.")).not.toBeInTheDocument();
  });

  it("C starts a new email, as the footer says", async () => {
    render(<DoneScreen />);
    expect(screen.getByRole("contentinfo")).toHaveTextContent("CNew email");
    await userEvent.keyboard("c");
    expect(s()).toMatchObject({ view: "compose", draft: { mode: "new" } });
  });

  it("says what belongs here when nothing is done yet", async () => {
    useStore.setState({ doneThreads: [] });
    const user = userEvent.setup();
    render(<DoneScreen />);
    expect(screen.getByText("Nothing done yet")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}e{Enter}");
    await user.click(within(screen.getByRole("status")).getByRole("button", { name: /Back/ }));
    expect(s().view).toBe("inbox");
  });
});

it("a store change it doesn't show doesn't redraw it", () => {
  const { renders } = renderCounted(<DoneScreen />);
  changeWhatNoListShows();
  expect(renders).not.toHaveBeenCalled();
});
