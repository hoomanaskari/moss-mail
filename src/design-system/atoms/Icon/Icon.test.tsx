import { render } from "@testing-library/react";
import { Inbox } from "lucide-react";
import { describe, expect, it } from "vitest";
import { Icon } from "./Icon";

const svg = (c: HTMLElement) => c.querySelector("svg")!;

describe("Icon", () => {
  it("defaults to 16px in ink-2, hidden from screen readers", () => {
    const { container } = render(<Icon icon={Inbox} />);
    expect(svg(container)).toHaveAttribute("width", "16");
    expect(svg(container)).toHaveAttribute("height", "16");
    expect(svg(container)).toHaveAttribute("stroke", "var(--ink-2)");
    expect(svg(container)).toHaveAttribute("aria-hidden", "true");
  });

  it("uses the given size, tone and class", () => {
    const { container } = render(<Icon icon={Inbox} size={22} tone="ai" className="extra" />);
    expect(svg(container)).toHaveAttribute("width", "22");
    expect(svg(container)).toHaveAttribute("stroke", "var(--ai)");
    expect(svg(container)).toHaveClass("extra");
  });

  it("is filled with its colour when asked, and hollow by default", () => {
    const { container, rerender } = render(<Icon icon={Inbox} tone="due" filled />);
    expect(svg(container)).toHaveAttribute("fill", "var(--due)");
    rerender(<Icon icon={Inbox} />);
    expect(svg(container)).toHaveAttribute("fill", "none");
  });

  it("inherits the text colour when asked", () => {
    const { container } = render(<Icon icon={Inbox} tone="inherit" />);
    expect(svg(container)).toHaveAttribute("stroke", "currentColor");
  });
});
