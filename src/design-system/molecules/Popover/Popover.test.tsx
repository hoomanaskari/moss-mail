import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Popover } from "./Popover";

// Stands in for the app's shortcuts, which listen on window.
const onKey = vi.fn();
beforeEach(() => window.addEventListener("keydown", onKey));
afterEach(() => {
  window.removeEventListener("keydown", onKey);
  onKey.mockReset();
});

function setup() {
  render(
    <>
      <Popover
        role="dialog"
        label="Card"
        align="end"
        trigger={({ open, toggle }) => (
          <button type="button" aria-expanded={open} onClick={toggle}>
            Open
          </button>
        )}
      >
        {(close) => (
          <button type="button" onClick={close}>
            Inside
          </button>
        )}
      </Popover>
      <p>Outside</p>
    </>,
  );
  return { user: userEvent.setup() };
}

describe("Popover", () => {
  it("opens from its trigger and closes again", async () => {
    const { user } = setup();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("dialog", { name: "Card" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open" })).toHaveAttribute("aria-expanded", "true");
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("gives its content a way to close it", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: "Inside" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("closes on a click outside it, not inside", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Open" }));
    fireEvent.pointerDown(screen.getByRole("dialog"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    fireEvent.pointerDown(screen.getByText("Outside"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("Esc closes it without reaching the app's shortcuts, which get Esc again once it is closed", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Open" }));
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(onKey).not.toHaveBeenCalled();
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onKey).toHaveBeenCalledOnce();
  });

  it("lets other keys through while open", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Open" }));
    fireEvent.keyDown(document.body, { key: "h" });
    expect(onKey).toHaveBeenCalledOnce();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});

describe("opened from outside", () => {
  function controlled(open: boolean) {
    const onOpenChange = vi.fn();
    const ui = (o: boolean) => (
      <Popover
        role="group"
        label="Card"
        open={o}
        onOpenChange={onOpenChange}
        trigger={({ open, toggle }) => (
          <button type="button" aria-expanded={open} onClick={toggle}>
            Open
          </button>
        )}
      >
        {(close) => (
          <button type="button" onClick={close}>
            Inside
          </button>
        )}
      </Popover>
    );
    const r = render(ui(open));
    return { onOpenChange, rerender: (o: boolean) => r.rerender(ui(o)) };
  }

  it("shows while its owner says it is open", () => {
    const { rerender } = controlled(true);
    expect(screen.getByRole("group", { name: "Card" })).toBeInTheDocument();
    rerender(false);
    expect(screen.queryByRole("group")).not.toBeInTheDocument();
  });

  it("asks its owner to open or close it, and doesn't do it itself", async () => {
    const user = userEvent.setup();
    const { onOpenChange } = controlled(false);
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(onOpenChange).toHaveBeenLastCalledWith(true);
    expect(screen.queryByRole("group")).not.toBeInTheDocument();
  });

  it("Esc, a click outside, or a choice ask it to close", async () => {
    const user = userEvent.setup();
    const { onOpenChange } = controlled(true);
    await user.keyboard("{Escape}");
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
    onOpenChange.mockClear();
    fireEvent.pointerDown(document.body);
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
    onOpenChange.mockClear();
    await user.click(screen.getByRole("button", { name: "Inside" }));
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
  });
});

describe("floating", () => {
  /** Puts the trigger at `top`–`bottom`, 200–500 across, in a 1000 × 768 window. */
  function at(top: number, bottom: number) {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
      { top, bottom, left: 200, right: 500, width: 300, height: bottom - top, x: 200, y: top, toJSON: () => ({}) } as DOMRect,
    );
  }
  afterEach(() => vi.restoreAllMocks());

  function float(align: "start" | "end" = "start") {
    const { container } = render(
      <>
        <Popover
          role="dialog"
          label="Card"
          floating
          align={align}
          trigger={({ open, toggle }) => (
            <button type="button" aria-expanded={open} onClick={toggle}>
              Open
            </button>
          )}
        >
          {(close) => (
            <button type="button" onClick={close}>
              Inside
            </button>
          )}
        </Popover>
        <p>Outside</p>
      </>,
    );
    return { container, user: userEvent.setup(), trigger: screen.getByRole("button", { name: "Open" }) };
  }

  it("floats over the page, out of its card, under its trigger when there's room below", async () => {
    at(100, 136);
    const { container, user, trigger } = float();
    await user.click(trigger);
    const card = screen.getByRole("dialog", { name: "Card" });
    expect(container.contains(card)).toBe(false);
    expect(card.style.position).toBe("fixed");
    expect([card.style.top, card.style.left, card.style.maxHeight]).toEqual(["144px", "200px", "612px"]);
    expect(card).toHaveAttribute("data-placement", "down");
  });

  it("flips up when there isn't room below, and lines up its right edge when asked", async () => {
    at(600, 636);
    const { user, trigger } = float("end");
    await user.click(trigger);
    const card = screen.getByRole("dialog");
    expect(card).toHaveAttribute("data-placement", "up");
    expect([card.style.bottom, card.style.right, card.style.maxHeight, card.style.top]).toEqual(["176px", `${window.innerWidth - 500}px`, "580px", ""]);
  });

  it("follows its trigger when the window resizes or something scrolls", async () => {
    at(100, 136);
    const { user, trigger } = float();
    await user.click(trigger);
    at(200, 236);
    fireEvent.scroll(document);
    expect(screen.getByRole("dialog").style.top).toBe("244px");
    at(300, 336);
    fireEvent(window, new Event("resize"));
    expect(screen.getByRole("dialog").style.top).toBe("344px");
  });

  it("a click inside keeps it; one outside closes it; Esc and a choice give focus back to the trigger", async () => {
    at(100, 136);
    const { user, trigger } = float();
    await user.click(trigger);
    fireEvent.pointerDown(screen.getByRole("button", { name: "Inside" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.click(screen.getByText("Outside"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.click(trigger);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();

    await user.click(trigger);
    screen.getByRole("button", { name: "Inside" }).focus();
    await user.click(screen.getByRole("button", { name: "Inside" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });
});
