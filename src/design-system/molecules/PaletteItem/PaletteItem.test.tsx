import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Archive } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { PaletteItem, type PaletteItemProps } from "./PaletteItem";

function setup(props: Partial<PaletteItemProps> = {}) {
  const onChoose = vi.fn();
  const ref = createRef<HTMLButtonElement>();
  render(<PaletteItem ref={ref} label="Mark done" highlighted={false} onChoose={onChoose} {...props} />);
  return { item: screen.getByRole("option", { name: /Mark done/ }), onChoose, ref };
}

describe("PaletteItem", () => {
  it("shows its label and sub line, and forwards its ref", () => {
    const { item, ref } = setup({ sub: "Moves it out of the inbox" });
    expect(item).toHaveTextContent("Moves it out of the inbox");
    expect(ref.current).toBe(item);
  });

  it("has no icon, sub line or key cap unless given them", () => {
    const { item } = setup();
    expect(item.textContent).toBe("Mark done");
    expect(item.querySelector("svg")).toBeNull();
    expect(item.querySelector("kbd")).toBeNull();
  });

  it("shows its shortcut when not highlighted", () => {
    const { item } = setup({ shortcut: "E", icon: Archive });
    expect(item).toHaveAttribute("aria-selected", "false");
    expect(item.querySelector("kbd")).toHaveTextContent("E");
    expect(item.querySelector("svg")).toHaveAttribute("stroke", "var(--ink-2)");
  });

  it("shows Enter instead of the shortcut when highlighted", () => {
    const { item } = setup({ shortcut: "E", highlighted: true, icon: Archive });
    expect(item).toHaveAttribute("aria-selected", "true");
    expect(item.querySelector("kbd")).toHaveTextContent("↵");
    expect(item.querySelector("svg")).toHaveAttribute("stroke", "var(--ink-2)");
  });

  it("a highlighted AI suggestion turns its icon AI-green", () => {
    const { item } = setup({ ai: true, highlighted: true, icon: Archive });
    expect(item.querySelector("svg")).toHaveAttribute("stroke", "var(--ai)");
  });

  it("an AI suggestion that is not highlighted keeps the normal icon colour", () => {
    const { item } = setup({ ai: true, icon: Archive });
    expect(item.querySelector("svg")).toHaveAttribute("stroke", "var(--ink-2)");
  });

  it("moving the mouse over it does nothing; a click chooses it", async () => {
    const { item, onChoose } = setup();
    await userEvent.hover(item);
    await userEvent.pointer({ target: item, coords: { x: 5, y: 5 } });
    expect(item).toHaveAttribute("aria-selected", "false");
    expect(onChoose).not.toHaveBeenCalled();
    await userEvent.click(item);
    expect(onChoose).toHaveBeenCalledOnce();
  });
});
