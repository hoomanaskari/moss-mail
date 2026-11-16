import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import type { Thread } from "../../../data/types";
import { mockRulerHeights, TestResizeObserver } from "../../../test/resize";
import { jonas, personal, thread, work } from "../../../test/fixtures";
import type { ThreadRowProps } from "../../molecules/ThreadRow/ThreadRow";
import { ThreadList, type ThreadListProps } from "./ThreadList";

// Counts each row's renders by subject, so tests can see which rows a change re-renders.
const rendered = vi.hoisted(() => ({ rows: [] as string[] }));
vi.mock("../../molecules", async (importOriginal) => {
  const real = await importOriginal<typeof import("../../molecules")>();
  const { createElement, forwardRef } = await import("react");
  const Counted = forwardRef<HTMLDivElement, ThreadRowProps>(function Counted(props, ref) {
    rendered.rows.push(String(props.subject));
    return createElement(real.ThreadRow, { ...props, ref });
  });
  return { ...real, ThreadRow: Counted };
});

const threads: Thread[] = [
  thread("a", { section: "info", subject: "Newsletter" }),
  thread("b", { section: "reply", subject: "Budget", summary: "Needs sign-off", due: "Due Wed" }),
  thread("c", { section: "later", subject: "Trip", accountId: "personal", from: jonas }),
  thread("d", { section: "reply", subject: "Hiring" }),
];

const accountOf = (id: string) => (id === "personal" ? personal : work);

function setup(overrides: Partial<ThreadListProps> = {}) {
  const props: ThreadListProps = {
    threads,
    accountOf,
    cursorId: "b",
    onOpen: vi.fn(),
    onDone: vi.fn(),
    onLater: vi.fn(),
    onReply: vi.fn(),
    onDelete: vi.fn(),
    onBlock: vi.fn(),
    ...overrides,
  };
  const view = render(<ThreadList {...props} />);
  return { ...view, props, rerender: (next: Partial<ThreadListProps>) => view.rerender(<ThreadList {...props} {...next} />) };
}

const row = (subject: string) => screen.getByRole("option", { name: new RegExp(subject) });

let scroll: MockInstance<Element["scrollIntoView"]>;
beforeEach(() => {
  scroll = vi.spyOn(Element.prototype, "scrollIntoView");
});
afterEach(() => scroll.mockRestore());

describe("ThreadList", () => {
  describe("re-rendering", () => {
    const many = Array.from({ length: 200 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));

    it("moving the cursor re-renders only the row it leaves and the row it lands on", async () => {
      const user = userEvent.setup();
      const view = setup({ threads: many, cursorId: "large-0" });
      const mounted = screen.getAllByRole("option").length;
      expect(mounted).toBeGreaterThan(10);
      rendered.rows = [];
      // Screens pass fresh callbacks on every render; rows must not care.
      const onDone = vi.fn();
      view.rerender({ cursorId: "large-1", onOpen: vi.fn(), onDone, onLater: vi.fn(), onReply: vi.fn(), onDelete: vi.fn() });
      expect(rendered.rows.sort()).toEqual(["Email 0", "Email 1"]);
      expect(screen.getByRole("option", { selected: true })).toHaveTextContent("Email 1");
      // The re-used rows still act through the newest callbacks.
      await user.click(within(screen.getByRole("option", { selected: true })).getByRole("button", { name: /Done/ }));
      expect(onDone).toHaveBeenCalledWith("large-1");
      expect(view.props.onDone).not.toHaveBeenCalled();
    });

    it("a changed email re-renders only its own row", () => {
      const view = setup({ threads: many, cursorId: "large-0" });
      rendered.rows = [];
      view.rerender({ threads: many.map((t) => (t.id === "large-3" ? { ...t, unread: true } : t)) });
      expect(rendered.rows).toEqual(["Email 3"]);
      expect(within(row("Email 3")).getByText("Unread")).toBeInTheDocument();
    });

    it("rows follow their account when its letter changes", () => {
      const view = setup({ threads: many, cursorId: "large-0" });
      view.rerender({ accountOf: () => ({ ...work, letter: "X" }) });
      expect(within(row("Email 5")).getByText("X")).toBeInTheDocument();
    });

    it("rows a newly given Move reaches act through it", async () => {
      const user = userEvent.setup();
      const view = setup({ threads: many, cursorId: "large-0" });
      expect(within(row("Email 0")).queryByRole("button", { name: "MMove" })).toBeNull();
      const onMove = vi.fn();
      view.rerender({ onMove });
      await user.click(within(row("Email 0")).getByRole("button", { name: "MMove" }));
      expect(onMove).toHaveBeenCalledWith("large-0");
    });
  });

  it("bounds a large inbox while keeping a distant keyboard selection mounted", () => {
    const many = Array.from({ length: 1000 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    const view = setup({ threads: many, cursorId: "large-0" });
    expect(screen.getAllByRole("option").length).toBeLessThan(40);
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("Email 0");
    view.rerender({ cursorId: "large-999" });
    expect(screen.getAllByRole("option").length).toBeLessThan(40);
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("Email 999");
    expect(scroll.mock.instances.at(-1)).toBe(screen.getByRole("option", { selected: true }));
  });

  it("exposes full group positions when only part of the inbox is mounted", () => {
    const many = Array.from({ length: 1000 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    const view = setup({ threads: many, cursorId: "large-999" });
    expect(screen.getByRole("option", { selected: true })).toHaveAttribute("aria-posinset", "1000");
    expect(screen.getByRole("option", { selected: true })).toHaveAttribute("aria-setsize", "1000");
    view.rerender({ threads: [...many, thread("later", { section: "later" })] });
    expect(within(screen.getByRole("group", { name: "Coming back later" })).getAllByRole("option")).toHaveLength(1);
  });

  it("renders scrolled rows without moving the cursor, and keeps their actions attached to the right email", () => {
    const many = Array.from({ length: 1000 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    const { props } = setup({ threads: many, cursorId: "large-0" });
    const list = screen.getByRole("listbox");
    fireEvent.scroll(list, { target: { scrollTop: 50 + 500 * 58 } });
    expect(row("Email 500")).toBeInTheDocument();
    expect(screen.queryByText("Email 100")).not.toBeInTheDocument();
    expect(screen.getAllByRole("option").length).toBeLessThan(30);
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("Email 0");
    expect(scroll).toHaveBeenCalledTimes(1);
    fireEvent.click(row("Email 500"));
    expect(props.onOpen).toHaveBeenCalledWith("large-500");
    fireEvent.scroll(list);
    expect(row("Email 500")).toBeInTheDocument();
  });

  it("measures reading-size and viewport changes, including a plain list without headings", () => {
    const many = Array.from({ length: 1000 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    const view = setup({ threads: many, cursorId: undefined });
    const list = screen.getByRole("listbox");
    act(() => {
      TestResizeObserver.resize(list, 140, true);
      TestResizeObserver.resize(screen.getByTestId("thread-row-height"), 70);
      TestResizeObserver.resize(screen.getByTestId("thread-heading-height"), 50);
    });
    expect(screen.getAllByRole("option")).toHaveLength(6);
    act(() => TestResizeObserver.resize(list, 140, true));
    expect(screen.getAllByRole("option")).toHaveLength(6);
    view.rerender({ grouped: false, cursorId: undefined });
    expect(screen.queryByRole("heading")).not.toBeInTheDocument();
    fireEvent.scroll(list, { target: { scrollTop: 70008 } });
    expect(row("Email 999")).toBeInTheDocument();
    fireEvent.scroll(list, { target: { scrollTop: 80000 } });
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(row("Email 0")).toBeInTheDocument();
    view.rerender({ threads: [], cursorId: undefined, empty: <p>Finished</p> });
    expect(screen.getByText("Finished")).toBeInTheDocument();
  });

  it("windows across group boundaries and preserves each group's own positions", () => {
    const many = Array.from({ length: 600 }, (_, i) => thread(`large-${i}`, {
      subject: `Email ${i}`, section: i < 200 ? "reply" : i < 400 ? "info" : "later",
    }));
    setup({ threads: many, cursorId: "large-205" });
    const list = screen.getByRole("listbox");
    fireEvent.scroll(list, { target: { scrollTop: 8 + 42 + 200 * 58 } });
    expect(row("Email 199")).toBeInTheDocument();
    expect(row("Email 200")).toHaveAttribute("aria-posinset", "1");
    expect(row("Email 205")).toHaveAttribute("aria-setsize", "200");
    expect(within(screen.getByRole("group", { name: "Coming back later" })).getAllByRole("option")).toHaveLength(1);
    fireEvent.scroll(list, { target: { scrollTop: 8 + 84 + 400 * 58 } });
    expect(row("Email 400")).toBeInTheDocument();
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("Email 205");
  });

  it("uses the shorter later-row geometry and preserves the viewport when reading size changes", () => {
    const many = Array.from({ length: 600 }, (_, i) => thread(`large-${i}`, {
      subject: `Email ${i}`, section: i < 400 ? "reply" : "later",
    }));
    setup({ threads: many, cursorId: "large-599" });
    const list = screen.getByRole("listbox");
    fireEvent.scroll(list, { target: { scrollTop: 8 + 42 + 400 * 58 + 42 + 190 * 52 } });
    expect(row("Email 590")).toBeInTheDocument();
    expect(screen.queryByText("Email 570")).not.toBeInTheDocument();
    act(() => {
      TestResizeObserver.resize(screen.getByTestId("thread-row-height"), 64);
      TestResizeObserver.resize(screen.getByTestId("thread-later-height"), 57);
    });
    expect(list.scrollTop).toBe(8 + 42 + 400 * 64 + 42 + 190 * 57);
    expect(scroll).toHaveBeenCalledTimes(2);
    expect(scroll.mock.instances.at(-1)).toBe(screen.getByRole("option", { selected: true }));
  });

  it("preserves manual scrolling through an ungrouped inbox when text grows", () => {
    const many = Array.from({ length: 500 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    setup({ threads: many, grouped: false, cursorId: "large-0" });
    const list = screen.getByRole("listbox");
    fireEvent.scroll(list, { target: { scrollTop: 8 + 200 * 58 } });
    act(() => TestResizeObserver.resize(screen.getByTestId("thread-row-height"), 70));
    expect(list.scrollTop).toBe(8 + 200 * 70);
    expect(row("Email 200")).toBeInTheDocument();
    expect(scroll).toHaveBeenCalledTimes(1);
  });

  it("switches at 101 rows and retains headings for offscreen sections", () => {
    const many = Array.from({ length: 101 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    const view = setup({ threads: many.slice(0, 100), cursorId: "large-0" });
    expect(screen.getAllByRole("option")).toHaveLength(100);
    fireEvent.scroll(screen.getByRole("listbox"));
    view.rerender({ threads: many });
    expect(screen.getAllByRole("option").length).toBeLessThan(40);
    const observers = [...TestResizeObserver.instances];
    fireEvent.scroll(screen.getByRole("listbox"), { target: { scrollTop: 1000 } });
    expect([...TestResizeObserver.instances]).toEqual(observers);
    view.rerender({ threads: [...many, thread("later", { section: "later" })] });
    expect(screen.getByRole("heading", { name: "Coming back later" })).toBeInTheDocument();
    view.rerender({ threads: many.slice(0, 100) });
    expect(screen.getAllByRole("option")).toHaveLength(100);
    expect([...TestResizeObserver.instances]).toEqual(observers);
  });

  it("keeps the actual scroll position when a differently sized list starts windowing", () => {
    const many = Array.from({ length: 101 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    const layout = mockRulerHeights({ "thread-row-height": 64, "thread-later-height": 57, "thread-heading-height": 43 });
    try {
      const view = setup({ threads: many.slice(0, 100), cursorId: "large-70" });
      const list = screen.getByRole("listbox");
      fireEvent.scroll(list, { target: { scrollTop: 3000 } });
      view.rerender({ threads: many });
      expect(list.scrollTop).toBe(3000);
      view.rerender({ threads: many.slice(0, 100) });
      fireEvent.scroll(list, { target: { scrollTop: 1000 } });
      view.rerender({ threads: many });
      expect(list.scrollTop).toBe(1000);
    } finally { layout.mockRestore(); }
  });

  it("keeps every option group present with a real first option outside the window", () => {
    const many = Array.from({ length: 600 }, (_, i) => thread(`large-${i}`, {
      subject: `Email ${i}`, section: i < 200 ? "reply" : i < 400 ? "info" : "later",
    }));
    setup({ threads: many, cursorId: "large-0" });
    const later = screen.getByRole("group", { name: "Coming back later" });
    expect(within(later).getAllByRole("option")).toHaveLength(1);
    expect(within(later).getByRole("option")).toHaveTextContent("Email 400");
    expect(within(later).getByRole("option")).toHaveAttribute("aria-setsize", "200");
  });

  it("updates the rendering window immediately after native selection scrolling", () => {
    const many = Array.from({ length: 1000 }, (_, i) => thread(`large-${i}`, { subject: `Email ${i}` }));
    scroll.mockImplementation(() => { screen.getByRole("listbox").scrollTop = 50 + 500 * 58; });
    setup({ threads: many, cursorId: "large-500" });
    expect(row("Email 499")).toBeInTheDocument();
    expect(row("Email 501")).toBeInTheDocument();
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("Email 500");
    expect(scroll).toHaveBeenCalledTimes(1);
  });

  it("does not re-render small inbox rows for ordinary scrolling", () => {
    const many = Array.from({ length: 50 }, (_, i) => thread(`small-${i}`));
    const account = vi.fn(accountOf);
    setup({ threads: many, accountOf: account });
    account.mockClear();
    const list = screen.getByRole("listbox");
    fireEvent.scroll(list, { target: { scrollTop: 300 } });
    fireEvent.scroll(list, { target: { scrollTop: 500 } });
    expect(account).not.toHaveBeenCalled();
  });

  it("scrolls to a distant initial selection only after its real row geometry is known", () => {
    const many = Array.from({ length: 1000 }, (_, i) => thread(`large-${i}`));
    const layout = mockRulerHeights({ "thread-row-height": 64, "thread-later-height": 57, "thread-heading-height": 42 });
    const rowOffsets: number[] = [];
    scroll.mockImplementation(function (this: Element) {
      let offset = 0;
      for (let sibling = this.previousElementSibling; sibling; sibling = sibling.previousElementSibling) {
        offset += sibling.getAttribute("role") === "option" ? 64 : parseFloat((sibling as HTMLElement).style.height) || 0;
      }
      rowOffsets.push(offset);
    });
    try {
      setup({ threads: many, cursorId: "large-500" });
      // Visible rows plus spacer rows must put option 500 at its measured position.
      expect(rowOffsets).toEqual([500 * 64]);
    } finally { layout.mockRestore(); }
  });

  it("marks unread emails", () => {
    setup({ threads: [thread("a", { subject: "Newsletter", unread: true }), thread("b", { subject: "Budget" })] });
    expect(row("Newsletter")).toHaveAccessibleName(/^Unread/);
    expect(row("Budget")).not.toHaveAccessibleName(/Unread/);
  });

  it("groups threads by section in a fixed order, keeping their order within each", () => {
    setup();
    expect(screen.getByRole("listbox", { name: "Emails" })).toBeInTheDocument();
    const groups = screen.getAllByRole("group");
    expect(groups.map((g) => g.getAttribute("aria-label"))).toEqual(["Needs your reply", "For your info", "Coming back later"]);
    expect(within(groups[0]).getAllByRole("option").map((o) => o.textContent)).toEqual([
      expect.stringContaining("Budget"),
      expect.stringContaining("Hiring"),
    ]);
    expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
      "Needs your reply",
      "For your info",
      "Coming back later",
    ]);
  });

  it("leaves out sections with no threads", () => {
    setup({ threads: [threads[0]], cursorId: "a" });
    expect(screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"))).toEqual(["For your info"]);
  });

  it("not grouped, it's one list, with reminded mail apart at the bottom under Coming back later", () => {
    setup({ grouped: false });
    expect(screen.getAllByRole("heading").map((h) => h.textContent)).toEqual(["Coming back later"]);
    const groups = screen.getAllByRole("group");
    expect(groups.map((g) => g.getAttribute("aria-label"))).toEqual(["Emails", "Coming back later"]);
    expect(within(groups[0]).getAllByRole("option").map((o) => o.textContent)).toEqual([
      expect.stringContaining("Budget"),
      expect.stringContaining("Hiring"),
      expect.stringContaining("Newsletter"),
    ]);
    expect(within(groups[1]).getAllByRole("option").map((o) => o.textContent)).toEqual([expect.stringContaining("Trip")]);
  });

  it("not grouped and nothing reminded: no headings at all", () => {
    setup({ grouped: false, threads: threads.filter((t) => t.section !== "later") });
    expect(screen.queryByRole("heading")).not.toBeInTheDocument();
  });

  it("shows each thread's sender, subject, summary, due date and account letter", () => {
    setup();
    const budget = row("Budget");
    expect(budget).toHaveTextContent("Lena Vogt");
    expect(budget).toHaveTextContent("Needs sign-off");
    expect(budget).toHaveTextContent("Due Wed");
    expect(budget).toHaveTextContent("W");
    expect(row("Trip")).toHaveTextContent("Jonas Weber");
    expect(row("Trip")).toHaveTextContent("P");
  });

  it("an email back because no one replied gets the No reply chip, not a due day", () => {
    const asked = thread("e", { section: "reply", subject: "Plan", due: "No reply", waiting: { until: 1, why: { by: "you" } } });
    const dated = thread("f", { section: "reply", subject: "Deck", due: "Due Fri", dueDate: "2026-11-06", waiting: { until: 1, why: { by: "you" } } });
    setup({ threads: [asked, dated] });
    expect(within(row("Plan")).getByTitle("You’re waiting for an answer")).toHaveTextContent("No reply");
    expect(within(row("Deck")).queryByTitle("You’re waiting for an answer")).not.toBeInTheDocument();
  });

  it("marks only the cursor row as selected", () => {
    setup({ cursorId: "d" });
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("Hiring");
    expect(screen.getAllByRole("option", { selected: false })).toHaveLength(3);
  });

  it("shows the empty state when there are no threads", () => {
    setup({ threads: [], cursorId: undefined, empty: <p>All done</p> });
    expect(screen.getByText("All done")).toBeInTheDocument();
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
  });

  it("hides the empty state when there are threads", () => {
    setup({ empty: <p>All done</p> });
    expect(screen.queryByText("All done")).not.toBeInTheDocument();
  });

  describe("pointer", () => {
    it("clicking a row opens it", async () => {
      const user = userEvent.setup();
      const { props } = setup();
      await user.click(row("Newsletter"));
      expect(props.onOpen).toHaveBeenCalledWith("a");
    });

    it("moving the mouse over rows never moves the selection", async () => {
      const user = userEvent.setup();
      const { props } = setup();
      fireEvent.mouseMove(row("Hiring"));
      await user.hover(row("Newsletter"));
      await user.hover(row("Budget"));
      expect(row("Budget")).toHaveAttribute("aria-selected", "true");
      expect(row("Hiring")).toHaveAttribute("aria-selected", "false");
      expect(row("Newsletter")).toHaveAttribute("aria-selected", "false");
      for (const handler of Object.values(props).filter(vi.isMockFunction)) expect(handler).not.toHaveBeenCalled();
    });

    it("the selected row's Done, Later, Reply and Delete act on it without opening it", async () => {
      const user = userEvent.setup();
      const { props } = setup();
      const budget = row("Budget");
      await user.click(within(budget).getByRole("button", { name: /Done/ }));
      expect(props.onDone).toHaveBeenCalledWith("b");
      await user.click(within(budget).getByRole("button", { name: /Later/ }));
      expect(props.onLater).toHaveBeenCalledWith("b");
      await user.click(within(budget).getByRole("button", { name: /Reply/ }));
      expect(props.onReply).toHaveBeenCalledWith("b");
      await user.click(within(budget).getByRole("button", { name: /Delete/ }));
      expect(props.onDelete).toHaveBeenCalledWith("b");
      expect(props.onOpen).not.toHaveBeenCalled();
      expect(within(budget).queryByRole("button", { name: /Block/ })).toBeNull();
    });

    it("marks what you moved to Important, and the selected row's Move acts on it", async () => {
      const user = userEvent.setup();
      const onMove = vi.fn();
      setup({ onMove, threads: [thread("b", { subject: "Budget", movedByYou: true }), thread("n", { subject: "News", split: "newsletters", movedByYou: true })] });
      expect(within(row("Budget")).getByRole("img", { name: "You moved it to Important" })).toBeInTheDocument();
      // Elsewhere it's only where you put it: no mark.
      expect(within(row("News")).queryByRole("img")).toBeNull();
      await user.click(within(row("Budget")).getByRole("button", { name: "MMove" }));
      expect(onMove).toHaveBeenCalledWith("b");
    });

    it("only a new sender's row offers Block", async () => {
      const user = userEvent.setup();
      const { props } = setup({ threads: [...threads, thread("e", { section: "info", subject: "Leads", sender: "new" })], cursorId: "e" });
      await user.click(within(row("Leads")).getByRole("button", { name: /Block/ }));
      expect(props.onBlock).toHaveBeenCalledWith("e");
    });
  });

  describe("keeping the cursor in view", () => {
    it("scrolls the selected row into view on first render", () => {
      setup();
      expect(scroll).toHaveBeenCalledOnce();
      expect(scroll).toHaveBeenCalledWith({ block: "nearest" });
      expect(scroll.mock.contexts[0]).toBe(row("Budget"));
    });

    it("scrolls when the cursor moves by keyboard", () => {
      const { rerender } = setup();
      scroll.mockClear();
      rerender({ cursorId: "d" });
      expect(scroll).toHaveBeenCalledOnce();
      expect(scroll.mock.contexts[0]).toBe(row("Hiring"));
    });

    it("moving the mouse never scrolls; the next key move still does", () => {
      const { rerender } = setup();
      scroll.mockClear();
      fireEvent.mouseMove(row("Hiring"));
      expect(scroll).not.toHaveBeenCalled();
      rerender({ cursorId: "d" });
      expect(scroll).toHaveBeenCalledOnce();
      expect(scroll.mock.contexts[0]).toBe(row("Hiring"));
    });

    it("does not scroll on re-renders that keep the same cursor", () => {
      const { rerender } = setup();
      scroll.mockClear();
      rerender({ empty: <p>Nothing here</p> });
      expect(scroll).not.toHaveBeenCalled();
    });

    it("does nothing when the cursor is not in the list", () => {
      setup({ cursorId: "gone" });
      expect(scroll).not.toHaveBeenCalled();
    });
  });
});
