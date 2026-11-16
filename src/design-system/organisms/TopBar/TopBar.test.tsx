import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TopBar } from "./TopBar";

describe("TopBar", () => {
  it("is a window drag handle with the traffic lights hidden from screen readers", () => {
    render(<TopBar />);
    const bar = screen.getByRole("banner");
    expect(bar).toHaveAttribute("data-tauri-drag-region");
    const dots = bar.firstElementChild!;
    expect(dots).toHaveAttribute("aria-hidden");
    expect(dots.children).toHaveLength(3);
  });

  it("with nothing passed, has no title, back link or extras", () => {
    render(<TopBar />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    // Traffic lights plus the drag spacer only.
    expect(screen.getByRole("banner").children).toHaveLength(2);
  });

  it("shows the title in a draggable area", () => {
    render(<TopBar title={<span>Inbox</span>} />);
    const title = screen.getByText("Inbox").parentElement!;
    expect(title).toHaveAttribute("data-tauri-drag-region");
  });

  it("shows a back link with its label and Esc key, and calls onBack", async () => {
    const user = userEvent.setup();
    const onBack = vi.fn();
    render(<TopBar back={{ label: "Inbox", onBack }} />);
    const back = screen.getByRole("button", { name: "InboxEsc" });
    expect(back.querySelector("kbd")).toHaveTextContent("Esc");
    await user.click(back);
    expect(onBack).toHaveBeenCalledOnce();
  });

  it("puts the right-hand content after the spacer", () => {
    render(<TopBar title="Inbox" right={<button type="button">Search</button>} />);
    const bar = screen.getByRole("banner");
    expect(bar.lastElementChild).toBe(screen.getByRole("button", { name: "Search" }));
  });
});
