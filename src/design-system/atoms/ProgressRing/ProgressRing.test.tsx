import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ProgressRing } from "./ProgressRing";

/** Filled length / full circumference, read from the value circle's dash array. */
function fill(container: HTMLElement) {
  const [, value] = container.querySelectorAll("circle");
  const [filled, full] = value.getAttribute("stroke-dasharray")!.split(" ").map(Number);
  return filled / full;
}

describe("ProgressRing", () => {
  it("is an image named by its label", () => {
    render(<ProgressRing value={0.5} label="2 of 4 done" />);
    expect(screen.getByRole("img", { name: "2 of 4 done" })).toBeInTheDocument();
  });

  it("defaults to 40px", () => {
    const { container } = render(<ProgressRing value={0} />);
    const svg = container.querySelector("svg")!;
    expect(svg).toHaveAttribute("width", "40");
    expect(svg).toHaveAttribute("viewBox", "0 0 40 40");
  });

  it("fills in proportion to the value", () => {
    const { container } = render(<ProgressRing value={0.25} size={56} />);
    expect(fill(container)).toBeCloseTo(0.25);
  });

  it("starts at 12 o'clock", () => {
    const { container } = render(<ProgressRing value={0.5} size={40} />);
    expect(container.querySelectorAll("circle")[1]).toHaveAttribute("transform", "rotate(-90 20 20)");
  });

  it("spins while waiting", () => {
    const { container } = render(<ProgressRing value={0.3} spinning label="Waiting" />);
    expect(container.querySelector("svg")).toHaveAttribute("data-spinning", "true");
    const still = render(<ProgressRing value={0.3} />);
    expect(still.container.querySelector("svg")).not.toHaveAttribute("data-spinning");
  });

  it("clamps values outside 0 to 1", () => {
    const over = render(<ProgressRing value={1.7} />);
    expect(fill(over.container)).toBe(1);
    const under = render(<ProgressRing value={-0.3} />);
    expect(fill(under.container)).toBe(0);
  });
});
