import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Archive } from "lucide-react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandPalette, type PaletteGroup } from "./CommandPalette";

const runs = { summarize: vi.fn(), archive: vi.fn(), reply: vi.fn() };

/** Small palette: an AI group that echoes the query, and actions filtered by it. */
const groups = vi.fn((query: string): PaletteGroup[] => {
  const q = query.toLowerCase();
  return [
    { title: "Ask AI", items: q ? [] : [{ id: "summarize", label: "Summarize this email", ai: true, run: runs.summarize }] },
    {
      title: "Actions",
      items: [
        { id: "archive", label: "Mark done", sub: "Archive it", icon: Archive, shortcut: "E", run: runs.archive },
        { id: "reply", label: "Reply", shortcut: "R", run: runs.reply },
      ].filter((c) => c.label.toLowerCase().includes(q)),
    },
  ];
});

afterEach(() => vi.clearAllMocks());

function setup(props: { context?: string; initialQuery?: string } = {}) {
  const onClose = vi.fn();
  render(<CommandPalette groups={groups} onClose={onClose} {...props} />);
  const input = screen.getByRole("textbox", { name: "Ask AI or type a command" });
  const highlighted = () => screen.getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true");
  return { onClose, input, highlighted, user: userEvent.setup() };
}

describe("CommandPalette", () => {
  it("opens as a labelled dialog with the query field focused", () => {
    const { input } = setup();
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeInTheDocument();
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute("aria-controls", "palette-results");
    expect(screen.getByRole("listbox")).toHaveAttribute("id", "palette-results");
  });

  it("lists commands by group and highlights the first", () => {
    const { highlighted } = setup();
    expect(within(screen.getByRole("group", { name: "Ask AI" })).getByRole("option")).toHaveTextContent("Summarize this email");
    expect(within(screen.getByRole("group", { name: "Actions" })).getAllByRole("option")).toHaveLength(2);
    expect(highlighted()).toHaveTextContent("Summarize this email");
    expect(screen.getByRole("option", { name: /Mark done/ })).toHaveTextContent("Archive it");
  });

  it("starts from the initial query and filters as you type, hiding empty groups", async () => {
    const { input, user, highlighted } = setup({ initialQuery: "re" });
    expect(input).toHaveValue("re");
    expect(screen.queryByRole("group", { name: "Ask AI" })).not.toBeInTheDocument();
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["Reply↵"]);

    await user.clear(input);
    await user.type(input, "mark");
    expect(groups).toHaveBeenLastCalledWith("mark");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(highlighted()).toHaveTextContent("Mark done");
  });

  it("says when nothing matches", async () => {
    const { input, user } = setup();
    await user.type(input, "zzz");
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
    expect(screen.getByText("Nothing matches “zzz”.")).toBeInTheDocument();
  });

  it("moves the highlight with the arrow keys, wrapping at both ends", async () => {
    const { user, highlighted } = setup();
    await user.keyboard("{ArrowDown}");
    expect(highlighted()).toHaveTextContent("Mark done");
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(highlighted()).toHaveTextContent("Summarize this email");
    await user.keyboard("{ArrowUp}");
    expect(highlighted()).toHaveTextContent("Reply");
  });

  it("shows the shortcut on other items and the Enter key on the highlighted one", async () => {
    const { user } = setup();
    expect(screen.getByRole("option", { name: /Mark done/ })).toHaveTextContent(/E$/);
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("option", { name: /Mark done/ })).toHaveTextContent(/↵$/);
  });

  it("resets the highlight to the top when the query changes", async () => {
    const { input, user, highlighted } = setup();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    await user.type(input, "r");
    expect(highlighted()).toHaveTextContent("Mark done");
  });

  it("Enter closes the palette, then runs the highlighted command", async () => {
    const order: string[] = [];
    const { onClose, user } = setup();
    onClose.mockImplementation(() => order.push("close"));
    runs.archive.mockImplementation(() => order.push("run"));
    await user.keyboard("{ArrowDown}{Enter}");
    expect(order).toEqual(["close", "run"]);
    expect(runs.summarize).not.toHaveBeenCalled();
  });

  it("keeps the keys it uses, so the screen a command opens doesn't act on the same ↵", async () => {
    // A command can open a screen whose own ↵ opens an email: that ↵ must not reach it.
    const app = vi.fn();
    window.addEventListener("keydown", app);
    const { user } = setup();
    await user.keyboard("{ArrowDown}{ArrowUp}{Escape}{ArrowDown}{Enter}");
    expect(runs.archive).toHaveBeenCalledOnce();
    window.removeEventListener("keydown", app);
    expect(app.mock.calls.map(([e]) => (e as KeyboardEvent).key)).toEqual([]);
  });

  it("arrows and Enter do nothing when there are no results", async () => {
    const { input, onClose, user } = setup();
    await user.type(input, "zzz");
    await user.keyboard("{ArrowDown}{ArrowUp}{Enter}");
    expect(onClose).not.toHaveBeenCalled();
    await user.clear(input);
    expect(screen.getByRole("option", { name: /Summarize/ })).toHaveAttribute("aria-selected", "true");
  });

  it("⇧↵ runs the command whose key is ⇧↵, wherever the highlight is", async () => {
    const thisEmail = vi.fn();
    const onClose = vi.fn();
    const asking = (): PaletteGroup[] => [
      { title: "Ask AI", items: [{ id: "all", label: "Ask Moss", shortcut: "↵", run: runs.summarize }, { id: "one", label: "Ask this email only", shortcut: "⇧↵", run: thisEmail }] },
    ];
    render(<CommandPalette groups={asking} onClose={onClose} />);
    await userEvent.setup().keyboard("{Shift>}{Enter}{/Shift}");
    expect(thisEmail).toHaveBeenCalledOnce();
    expect(runs.summarize).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("⇧↵ with no such command does nothing", async () => {
    const { onClose, user } = setup();
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(onClose).not.toHaveBeenCalled();
    expect(runs.summarize).not.toHaveBeenCalled();
  });

  it("its footer shows the key for Ask Moss", () => {
    setup();
    expect(screen.getByText("Ask Moss").previousElementSibling).toHaveTextContent("Ctrl+J");
  });

  it("clicking a command closes and runs it", async () => {
    const { onClose, user } = setup();
    await user.click(screen.getByRole("option", { name: /Reply/ }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(runs.reply).toHaveBeenCalledOnce();
  });

  it("closes on Escape, on the Esc hint and on the backdrop, without running anything", async () => {
    const { onClose, user } = setup();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: /Close/ }));
    await user.click(screen.getByRole("dialog").previousElementSibling as Element);
    expect(onClose).toHaveBeenCalledTimes(3);
    expect(Object.values(runs).every((r) => r.mock.calls.length === 0)).toBe(true);
  });

  it("ignores other keys", async () => {
    const { onClose, user, highlighted } = setup();
    await user.keyboard("{ArrowLeft}");
    expect(onClose).not.toHaveBeenCalled();
    expect(highlighted()).toHaveTextContent("Summarize this email");
  });

  it("hovering never moves the highlight or scrolls; arrow keys move it and scroll it into view", async () => {
    const scroll = vi.spyOn(Element.prototype, "scrollIntoView");
    const { user, highlighted, onClose } = setup();
    scroll.mockClear();

    fireEvent.mouseMove(screen.getByRole("option", { name: /Reply/ }));
    await user.hover(screen.getByRole("option", { name: /Mark done/ }));
    expect(highlighted()).toHaveTextContent("Summarize this email");
    expect(scroll).not.toHaveBeenCalled();
    await user.keyboard("{ArrowDown}");
    expect(highlighted()).toHaveTextContent("Mark done");
    expect(scroll).toHaveBeenCalledWith({ block: "nearest" });
    // ↵ runs the highlighted item, not the one under the mouse.
    await user.hover(screen.getByRole("option", { name: /Reply/ }));
    await user.keyboard("{Enter}");
    expect(runs.archive).toHaveBeenCalledOnce();
    expect(runs.reply).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledOnce();
    scroll.mockRestore();
  });

  it("shows what it acts on when given a context", () => {
    setup({ context: "Q4 planning" });
    expect(screen.getByText("Q4 planning")).toBeInTheDocument();
  });

  it("says AI runs on this device", () => {
    setup();
    expect(screen.getByText(/AI runs on this (Mac|computer)/)).toBeInTheDocument();
  });
});
