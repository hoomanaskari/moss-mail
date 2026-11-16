import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { people } from "../data/demo";
import { noFilters, type SearchHit } from "../lib/search";
import { useSearch } from "../state/search";
import { openThread, shownView, useStore } from "../state/store";
import { fakeMail } from "../test/fakeMail";
import { resetDemoStore, thread } from "../test/fixtures";
import { SearchScreen } from "./SearchScreen";

const s = () => useStore.getState();
const q = () => useSearch.getState();
const pristine = useSearch.getState();

beforeEach(() => {
  resetDemoStore();
  useSearch.setState(pristine, true);
  s().openSearch();
});

/** A result from the engine, for the thread `threadId`. */
function hit(id: string, threadId: string, subject: string): SearchHit {
  const doc = { id, threadId, accountId: "personal", from: people.maya, subject, summary: "", when: "Aug 30", date: "2023-08-30", hasAttachment: false, relevance: 0.9 };
  return { doc, subject: [{ text: subject, match: false }], summary: [], best: true };
}

const rows = () => screen.queryAllByRole("option");
const selected = () => rows().find((r) => r.getAttribute("aria-selected") === "true");

async function searchFor(text: string) {
  const user = userEvent.setup();
  render(<SearchScreen />);
  await user.type(screen.getByRole("searchbox", { name: "Search all mail" }), text);
  await screen.findAllByRole("option");
  return user;
}

describe("SearchScreen", () => {
  it("starts empty, with the field ready to type in", () => {
    render(<SearchScreen />);
    expect(screen.getByRole("heading", { name: "Search" })).toBeInTheDocument();
    expect(screen.getByRole("searchbox")).toHaveFocus();
    expect(screen.getByText("Search all your mail")).toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).toHaveTextContent("Ask AI about these");
  });

  it("finds mail as you type, best matches first, then by date", async () => {
    await searchFor("budget");
    expect(screen.getByText("8 results")).toBeInTheDocument();
    const groups = screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"));
    expect(groups).toEqual(["Best matches", "This week", "October"]);
    expect(within(screen.getByRole("group", { name: "Best matches" })).getAllByRole("option")).toHaveLength(3);
    expect(selected()).toHaveTextContent("Q4 design budget: sign-off needed");
    expect(screen.getAllByText("budget", { selector: "mark" }).length).toBeGreaterThan(3);
  });

  it("says when nothing matches, offering to clear", async () => {
    const user = userEvent.setup();
    render(<SearchScreen />);
    await user.type(screen.getByRole("searchbox"), "zzz");
    expect(await screen.findByText("Nothing found")).toBeInTheDocument();
    await user.click(within(screen.getByRole("status")).getByRole("button", { name: /Clear search/ }));
    expect(q().query).toBe("");
  });

  it("↓ ↑ move while typing and once the focus has left the field; J and K do nothing", async () => {
    const user = await searchFor("budget");
    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent("Q3 spend review");
    await user.keyboard("{ArrowUp}");
    expect(selected()).toHaveTextContent("Q4 design budget");
    screen.getByRole("searchbox").blur();
    await user.keyboard("jj");
    expect(selected()).toHaveTextContent("Q4 design budget");
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(selected()).toHaveTextContent("Budget template for 2027");
    await user.keyboard("k");
    expect(selected()).toHaveTextContent("Budget template for 2027");
    await user.keyboard("{ArrowUp}");
    expect(selected()).toHaveTextContent("Q3 spend review");
  });

  it("pointing at a result leaves the selection alone; clicking selects it and opens it", async () => {
    const user = await searchFor("budget");
    await user.hover(rows()[1]);
    await user.hover(rows()[2]);
    expect(selected()).toHaveTextContent("Q4 design budget");
    // Not in the mailbox: there is nothing to open, so the click only selects it.
    await user.click(rows()[1]);
    expect(selected()).toHaveTextContent("Q3 spend review");
    expect(s().view).toBe("search");
    await user.click(rows()[0]);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "q4-budget" });
  });

  it("↵ opens the selected email when it is in the mailbox", async () => {
    const user = await searchFor("budget");
    await user.keyboard("{ArrowDown}{Enter}");
    expect(s().view).toBe("search");
    await user.keyboard("{ArrowUp}{Enter}");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "q4-budget" });
  });

  it("opens a result no list holds, such as an email only you sent, reading it from the engine", async () => {
    const portal = "Council Parking Permit Renewal Portal";
    const sent = thread("sent", { subject: portal, accountId: "personal" });
    const { backend } = fakeMail({ search: vi.fn(async () => [hit("m-sent", "sent", portal)]), thread: vi.fn(async () => sent) });
    const user = await searchFor("Council");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(shownView(s())).toBe("thread"));
    expect(backend.thread).toHaveBeenCalledWith("sent");
    expect(openThread(s())?.subject).toBe(portal);
  });

  it("a result that can't be shown any more leaves the results instead of doing nothing", async () => {
    fakeMail({
      search: vi.fn(async () => [hit("m-gone", "gone", "Council letter"), hit("m-kept", "kept", "Council reply")]),
      thread: vi.fn(async () => undefined),
    });
    const user = await searchFor("Council");
    await user.click(rows()[0]);
    await waitFor(() => expect(rows()).toHaveLength(1));
    expect(selected()).toHaveTextContent("Council reply");
    expect(s().view).toBe("search");
  });

  it("offers no Done: E types into the search, wherever the focus is", async () => {
    const user = await searchFor("budget");
    expect(screen.getByRole("contentinfo")).not.toHaveTextContent("Done");
    expect(within(selected()!).queryByRole("button", { name: /Done/ })).not.toBeInTheDocument();
    screen.getByRole("searchbox").blur();
    await user.keyboard("e");
    expect(rows()).toHaveLength(8);
    expect(s().threads.some((t) => t.id === "q4-budget")).toBe(true);
    expect(s().view).toBe("search");
    expect(s().toast).toBeUndefined();
    await user.click(screen.getByRole("searchbox"));
    await user.keyboard("e");
    expect(q().query).toBe("budgete");
  });

  it("⌘⌫ deletes the selected result, even while typing, keeping the words; ⌘Z brings it back", async () => {
    const user = await searchFor("budget");
    expect(screen.getByRole("contentinfo")).toHaveTextContent("Delete");
    await user.keyboard("{Control>}{Backspace}{/Control}");
    expect(rows()).toHaveLength(7);
    expect(selected()).not.toHaveTextContent("Q4 design budget");
    expect(q().query).toBe("budget");
    expect(s().threads.some((t) => t.id === "q4-budget")).toBe(false);
    expect(s()).toMatchObject({ view: "search", toast: { text: "Moved to Trash" } });
    await user.keyboard("{Control>}z{/Control}");
    expect(rows()).toHaveLength(8);
    expect(selected()).toHaveTextContent("Q4 design budget");
    expect(s().threads.some((t) => t.id === "q4-budget")).toBe(true);
    expect(s().view).toBe("search");
  });

  it("M opens the Move menu for the selected result once the focus has left the field; in the field it types", async () => {
    fakeMail({ search: vi.fn(async () => [hit("m-a", "a", "Council letter"), hit("m-b", "b", "Council reply")]) });
    const user = await searchFor("Council");
    await user.keyboard("m");
    expect(q().query).toBe("Councilm");
    expect(s().movingId).toBeUndefined();
    await user.keyboard("{Backspace}");
    screen.getByRole("searchbox").blur();
    await user.keyboard("m");
    expect(s().movingId).toBe("a");
  });

  it("⌫ and Delete delete the selected result once the focus has left the field; in the field ⌫ edits the words", async () => {
    const { backend } = fakeMail({
      search: vi.fn(async () => [hit("m-a", "a", "Council letter"), hit("m-b", "b", "Council reply"), hit("m-c", "c", "Council form")]),
    });
    const user = await searchFor("Council");
    screen.getByRole("searchbox").blur();
    await user.keyboard("{Backspace}");
    expect(backend.deleteThread).toHaveBeenCalledWith("a");
    expect(selected()).toHaveTextContent("Council reply");
    await user.keyboard("{Delete}");
    expect(backend.deleteThread).toHaveBeenCalledWith("b");
    expect(rows()).toHaveLength(1);
    await user.click(screen.getByRole("searchbox"));
    await user.keyboard("{Backspace}");
    expect(q().query).toBe("Counci");
    expect(backend.deleteThread).toHaveBeenCalledTimes(2);
  });

  it("the selected row's Delete button works too; a result with no email behind it has none, and ⌘⌫ leaves it", async () => {
    const user = await searchFor("budget");
    await user.click(within(selected()!).getByRole("button", { name: /Delete/ }));
    expect(rows()).toHaveLength(7);
    expect(s().toast).toMatchObject({ text: "Moved to Trash" });
    useStore.setState({ toast: undefined });
    // Q3 spend review isn't in the mailbox.
    expect(selected()).toHaveTextContent("Q3 spend review");
    expect(within(selected()!).queryByRole("button", { name: /Delete/ })).not.toBeInTheDocument();
    await user.keyboard("{Control>}{Backspace}{/Control}");
    expect(rows()).toHaveLength(7);
    expect(s().toast).toBeUndefined();
  });

  it("⌘Z only takes back what you deleted here; anything older is left for its own screen", async () => {
    useStore.setState({ view: "inbox" });
    s().markDone("q4-budget");
    s().openSearch();
    const user = await searchFor("budget");
    await user.keyboard("{Control>}z{/Control}");
    expect(s().undoSteps).toHaveLength(1);
    expect(s().view).toBe("search");
    // A held send is taken back from the toast or its own screen, not from Search's field.
    const held = { kind: "send" as const, hold: "h1", draft: {} as never, until: Date.now() + 5000 };
    useStore.setState({ undoSteps: [held] });
    await user.keyboard("{Control>}z{/Control}");
    expect(s().undoSteps).toEqual([held]);
    expect(s().view).toBe("search");
    // So is an email set to send later.
    const takeBack = vi.fn();
    const scheduled = { kind: "schedule" as const, draft: {} as never, takeBack };
    useStore.setState({ undoSteps: [scheduled] });
    await user.keyboard("{Control>}z{/Control}");
    expect(s().undoSteps).toEqual([scheduled]);
    expect(takeBack).not.toHaveBeenCalled();
  });

  it("the selected row's Open button works too", async () => {
    const user = await searchFor("budget");
    await user.keyboard("{ArrowDown}");
    await user.click(within(selected()!).getByRole("button", { name: /Open/ }));
    expect(s().view).toBe("search");
    await user.keyboard("{ArrowUp}");
    await user.click(within(selected()!).getByRole("button", { name: /Open/ }));
    expect(s()).toMatchObject({ view: "thread", openThreadId: "q4-budget" });
  });

  it("⌥ keys set the filters, and the chips do the same", async () => {
    const user = await searchFor("budget");
    await user.keyboard("{Alt>}f{/Alt}");
    expect(q().filters.from?.name).toBe("Daniel Okafor");
    expect(await screen.findByText("7 results")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /From: Daniel Okafor/ })).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: /From: Daniel Okafor/ }));
    expect(q().filters.from).toBeUndefined();
    fireEvent.keyDown(window, { key: "å", code: "KeyA", altKey: true });
    expect(screen.getByRole("button", { name: /Account: Work/ })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Account: Work/ }));
    expect(q().filters.accountId).toBe("personal");
    await user.keyboard("{Alt>}p{/Alt}");
    expect(q().filters.hasAttachment).toBe(true);
    await user.click(screen.getByRole("button", { name: /Has attachment/ }));
    expect(q().filters.hasAttachment).toBe(false);
    await user.keyboard("{Alt>}d{/Alt}");
    expect(screen.getByRole("button", { name: /Date: This week/ })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Date: This week/ }));
    expect(q().filters.date).toBe("month");
    await user.keyboard("{Alt>}x{/Alt}");
    expect(q().filters.date).toBe("month");
  });

  it("From with nothing selected and no results does nothing", async () => {
    const user = userEvent.setup();
    render(<SearchScreen />);
    await user.click(screen.getByRole("button", { name: /From: Anyone/ }));
    expect(q().filters.from).toBeUndefined();
  });

  it("⌥G and the switch group results by account", async () => {
    const user = await searchFor("budget");
    await user.keyboard("{Alt>}g{/Alt}");
    expect(screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"))).toEqual(["Best matches", "Work"]);
    await user.click(screen.getByRole("radio", { name: "Date" }));
    expect(q().groupBy).toBe("date");
  });

  it("Esc clears the search, then closes Search", async () => {
    const user = await searchFor("budget");
    await user.keyboard("{Escape}");
    expect(q()).toMatchObject({ query: "", filters: noFilters });
    expect(s().view).toBe("search");
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
  });

  it("the top bar's Clear search works like Esc, and other keys pass through", async () => {
    useSearch.setState({ filters: { ...noFilters, from: people.daniel } });
    const user = userEvent.setup();
    render(<SearchScreen />);
    await user.keyboard("{Meta>}x{/Meta}");
    const clear = screen.getAllByRole("button", { name: /Clear search/ })[0];
    await user.click(clear);
    expect(q().filters.from).toBeUndefined();
    await user.click(clear);
    expect(s().view).toBe("inbox");
  });

  it("shows one result in the singular", async () => {
    await searchFor("lunch");
    expect(screen.getByText("1 result")).toBeInTheDocument();
    await act(() => Promise.resolve());
  });
});

describe("SearchScreen: searching while you type", () => {
  afterEach(() => vi.useRealTimers());

  /** Search with the engine's own backend, which can run the embedding model, under fake timers. */
  function typing() {
    vi.useFakeTimers();
    const search = vi.fn(async () => [hit("m-q4", "q4-budget", "Q4 design budget")]);
    fakeMail({ search });
    render(<SearchScreen />);
    const box = screen.getByRole("searchbox", { name: "Search all mail" });
    const type = (text: string) => fireEvent.change(box, { target: { value: text } });
    const wait = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));
    return { search, box, type, wait };
  }

  it("waits until you stop typing for a moment, then searches once for the words you ended on", async () => {
    const { search, type, wait } = typing();
    await wait(0);
    expect(search).toHaveBeenCalledOnce();
    type("b");
    await wait(150);
    type("bu");
    await wait(150);
    type("bud");
    await wait(199);
    expect(search).toHaveBeenCalledOnce();
    await wait(1);
    expect(search).toHaveBeenCalledTimes(2);
    expect(search).toHaveBeenLastCalledWith("bud", noFilters);
    expect(screen.getByText("1 result")).toBeInTheDocument();
  });

  it("searches at once when a filter changes, taking the words typed so far with it", async () => {
    const { search, type, wait } = typing();
    await wait(0);
    type("budget");
    fireEvent.keyDown(window, { key: "p", code: "KeyP", altKey: true });
    await wait(0);
    expect(search).toHaveBeenCalledTimes(2);
    expect(search).toHaveBeenLastCalledWith("budget", { ...noFilters, hasAttachment: true });
    await wait(500);
    expect(search).toHaveBeenCalledTimes(2);
  });

  it("↵ right after typing searches at once and opens the top result for those words", async () => {
    const { search, box, type, wait } = typing();
    await wait(0);
    type("budget");
    fireEvent.keyDown(box, { key: "Enter", code: "Enter" });
    await wait(0);
    expect(search).toHaveBeenLastCalledWith("budget", noFilters);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "q4-budget" });
    await wait(500);
    expect(search).toHaveBeenCalledTimes(2);
  });
});

describe("search by meaning", () => {
  it("without its model, offers it in one quiet line, and Get it starts the small download", async () => {
    const { demoAi, setAi } = await import("../services/ai");
    const { useAiSettings } = await import("../state/aiSettings");
    const demo = demoAi({ plan: "skip" });
    setAi(demo);
    act(() => useAiSettings.setState({ status: undefined }));
    await act(() => useAiSettings.getState().loadStatus());
    render(<SearchScreen />);
    expect(screen.getByText(/Search by meaning needs AI/)).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Get it (1.4 GB)" }));
    expect(useAiSettings.getState().status?.plan).toBe("essentials");
    expect(screen.queryByText(/Search by meaning needs AI/)).not.toBeInTheDocument();
  });
});
