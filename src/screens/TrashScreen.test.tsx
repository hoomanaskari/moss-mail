import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "../state/store";
import { openTrash, useTrash } from "../state/trash";
import { personal, resetStore, thread, work } from "../test/fixtures";
import { changeWhatNoListShows, renderCounted } from "../test/renders";
import { TrashScreen } from "./TrashScreen";

const s = () => useStore.getState();
const now = Date.now();
const rows = () => screen.queryAllByRole("option");
const selected = () => rows().find((r) => r.getAttribute("aria-selected") === "true");

beforeEach(() => {
  resetStore([thread("a")], [work, personal]);
  useTrash.setState({
    threads: [
      thread("x", { subject: "Old invoice", lastMessageAt: now - 60_000 }),
      thread("y", { subject: "Party", accountId: "personal", lastMessageAt: now - 3 * 86_400_000 }),
      thread("z", { subject: "Newsletter", lastMessageAt: now - 30 * 86_400_000 }),
    ],
  });
  openTrash();
});

describe("TrashScreen", () => {
  it("lists deleted mail, newest first, by when it came", () => {
    render(<TrashScreen />);
    expect(screen.getByRole("heading", { name: "Trash" })).toBeInTheDocument();
    expect(screen.getByText(/E puts one back in your inbox/)).toBeInTheDocument();
    expect(screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"))).toEqual(["Today", "Earlier this week", "Older"]);
    expect(selected()).toHaveTextContent("Old invoice");
  });

  it("follows the account you are looking at", () => {
    act(() => s().setAccountFilter("personal"));
    act(() => openTrash());
    render(<TrashScreen />);
    expect(screen.getByRole("heading", { name: "Trash in Personal" })).toBeInTheDocument();
    expect(rows()).toHaveLength(1);
  });

  it("the arrows move, ↵ and O open", async () => {
    const user = userEvent.setup();
    render(<TrashScreen />);
    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent("Party");
    await user.keyboard("{ArrowUp}{Enter}");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x" });
    act(() => s().back());
    expect(s().view).toBe("trash");
    await user.keyboard("o");
    expect(s().openThreadId).toBe("x");
  });

  it("E puts the email back in your inbox and moves on", async () => {
    const user = userEvent.setup();
    render(<TrashScreen />);
    await user.keyboard("e");
    expect(s().threads.map((t) => t.id)).toEqual(["x", "a"]);
    expect(rows()).toHaveLength(2);
    expect(selected()).toHaveTextContent("Party");
  });

  it("each row has Open and Back to inbox; clicking a row opens it", async () => {
    const user = userEvent.setup();
    render(<TrashScreen />);
    await user.click(within(selected()!).getByRole("button", { name: "↵Open" }));
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x" });
    act(() => s().back());
    await user.click(rows()[1]);
    expect(s().openThreadId).toBe("y");
    act(() => s().back());
    await user.keyboard("{ArrowDown}");
    await user.click(within(selected()!).getByRole("button", { name: "EBack to inbox" }));
    expect(s().threads.map((t) => t.id)).toContain("z");
  });

  it("Esc and the top bar go back; / searches; C writes; ⌘ keys are left alone", async () => {
    const user = userEvent.setup();
    render(<TrashScreen />);
    await user.keyboard("{Control>}e{/Control}q");
    expect(rows()).toHaveLength(3);
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
    act(() => openTrash());
    await user.click(screen.getByRole("button", { name: "EscBack" }));
    expect(s().view).toBe("inbox");
    act(() => openTrash());
    await user.keyboard("/");
    expect(s().view).toBe("search");
    act(() => openTrash());
    await user.keyboard("c");
    expect(s()).toMatchObject({ view: "compose", draft: { mode: "new" } });
  });

  it("says what belongs here when nothing is deleted", async () => {
    useTrash.setState({ threads: [], cursorId: undefined });
    const user = userEvent.setup();
    render(<TrashScreen />);
    expect(screen.getByText("Trash is empty")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}e{Enter}");
    expect(s().view).toBe("trash");
    await user.click(within(screen.getByRole("status")).getByRole("button", { name: /Back/ }));
    expect(s().view).toBe("inbox");
  });

  it("says where older deleted mail is when the list is full", () => {
    useTrash.setState({ threads: Array.from({ length: 200 }, (_, i) => thread(`t${i}`)) });
    render(<TrashScreen />);
    expect(screen.getByText(/Older deleted emails are in your provider’s Trash/)).toBeInTheDocument();
  });
});

it("a store change it doesn't show doesn't redraw it", () => {
  const { renders } = renderCounted(<TrashScreen />);
  changeWhatNoListShows();
  expect(renders).not.toHaveBeenCalled();
});
