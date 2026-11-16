import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "../state/store";
import { person, personal, resetStore, thread, work } from "../test/fixtures";
import { BlockedScreen } from "./BlockedScreen";
import { changeWhatNoListShows, renderCounted } from "../test/renders";

const s = () => useStore.getState();
const now = Date.now();
const rows = () => screen.queryAllByRole("option");
const selected = () => rows().find((r) => r.getAttribute("aria-selected") === "true");
const jordan = person("Jordan Blake", "jordan@growthleads.example");
const alex = person("Alex Rivera", "alex@seo-boost.example");

beforeEach(() => {
  resetStore([thread("a")], [work, personal]);
  useStore.setState({
    blockedThreads: [
      thread("x", { from: jordan, subject: "Quick question", sender: "blocked", lastMessageAt: now - 60_000 }),
      thread("y", { from: alex, subject: "Your ranking", sender: "blocked", accountId: "personal", lastMessageAt: now - 3 * 86_400_000 }),
      thread("z", { from: jordan, subject: "Following up", sender: "blocked", lastMessageAt: now - 30 * 86_400_000 }),
    ],
  });
  s().openBlocked();
});

describe("BlockedScreen", () => {
  it("lists blocked mail, newest first, by when it came", () => {
    render(<BlockedScreen />);
    expect(screen.getByRole("heading", { name: "Blocked" })).toBeInTheDocument();
    expect(screen.getByText(/each account’s Blocked folder/)).toBeInTheDocument();
    expect(screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"))).toEqual(["Today", "Earlier this week", "Older"]);
    expect(selected()).toHaveTextContent("Quick question");
  });

  it("follows the account you are looking at", () => {
    act(() => s().setAccountFilter("personal"));
    act(() => s().openBlocked());
    render(<BlockedScreen />);
    expect(screen.getByRole("heading", { name: "Blocked in Personal" })).toBeInTheDocument();
    expect(rows()).toHaveLength(1);
  });

  it("the arrows move, J and K do nothing, ↵ and O open", async () => {
    const user = userEvent.setup();
    render(<BlockedScreen />);
    await user.keyboard("jJ");
    expect(selected()).toHaveTextContent("Quick question");
    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent("Your ranking");
    await user.keyboard("kK");
    expect(selected()).toHaveTextContent("Your ranking");
    await user.keyboard("{ArrowDown}{ArrowUp}{ArrowUp}");
    expect(selected()).toHaveTextContent("Quick question");
    await user.keyboard("{Enter}");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x" });
    act(() => s().back());
    await user.keyboard("o");
    expect(s().openThreadId).toBe("x");
  });

  it("U unblocks the sender: all their mail goes back to the inbox", async () => {
    const user = userEvent.setup();
    render(<BlockedScreen />);
    await user.keyboard("u");
    expect(s().threads.map((t) => t.id)).toEqual(["x", "z", "a"]);
    expect(rows()).toHaveLength(1);
    expect(selected()).toHaveTextContent("Your ranking");
  });

  it("pointing leaves the selection alone; clicking opens and selects; the row has Open and Unblock", async () => {
    const user = userEvent.setup();
    render(<BlockedScreen />);
    await user.hover(rows()[1]);
    await user.hover(rows()[2]);
    expect(selected()).toHaveTextContent("Quick question");
    await user.click(within(selected()!).getByRole("button", { name: "↵Open" }));
    expect(s()).toMatchObject({ view: "thread", openThreadId: "x" });
    act(() => s().back());
    await user.click(rows()[1]);
    expect(s().openThreadId).toBe("y");
    act(() => s().back());
    expect(selected()).toHaveTextContent("Your ranking");
    await user.click(within(selected()!).getByRole("button", { name: "UUnblock" }));
    expect(s().threads.map((t) => t.id)).toContain("y");
  });

  it("Esc and the top bar go back; / searches; other keys are left alone", async () => {
    const user = userEvent.setup();
    render(<BlockedScreen />);
    await user.keyboard("{Control>}j{/Control}q");
    expect(selected()).toHaveTextContent("Quick question");
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
    act(() => s().openBlocked());
    await user.click(screen.getByRole("button", { name: "EscBack" }));
    expect(s().view).toBe("inbox");
    act(() => s().openBlocked());
    await user.keyboard("/");
    expect(s().view).toBe("search");
  });

  it("C starts a new email, as the footer says", async () => {
    render(<BlockedScreen />);
    expect(screen.getByRole("contentinfo")).toHaveTextContent("CNew email");
    await userEvent.keyboard("c");
    expect(s()).toMatchObject({ view: "compose", draft: { mode: "new" } });
  });

  it("says what belongs here when nothing is blocked", async () => {
    useStore.setState({ blockedThreads: [] });
    const user = userEvent.setup();
    render(<BlockedScreen />);
    expect(screen.getByText("No blocked mail")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}u{Enter}");
    await user.click(within(screen.getByRole("status")).getByRole("button", { name: /Back/ }));
    expect(s().view).toBe("inbox");
  });
});

it("a store change it doesn't show doesn't redraw it", () => {
  const { renders } = renderCounted(<BlockedScreen />);
  changeWhatNoListShows();
  expect(renders).not.toHaveBeenCalled();
});
