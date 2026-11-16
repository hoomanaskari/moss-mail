import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Spinner } from "./Spinner";

describe("Spinner", () => {
  it("turns in the colour around it, hidden from screen readers: the words beside it say what is happening", () => {
    const { container } = render(<Spinner size={14} />);
    const svg = container.querySelector("svg")!;
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).toHaveAttribute("width", "14");
    expect(svg.querySelectorAll("circle")[1]).toHaveAttribute("stroke", "currentColor");
  });

  it("is 16px unless told otherwise", () => {
    const { container } = render(<Spinner />);
    expect(container.querySelector("svg")).toHaveAttribute("width", "16");
  });
});
