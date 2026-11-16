import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Avatar } from "./Avatar";

describe("Avatar", () => {
  it("shows initials, hidden from screen readers", () => {
    render(<Avatar initials="MR" />);
    expect(screen.getByText("MR")).toHaveAttribute("aria-hidden", "true");
  });

  it("keeps an extra class name for any size and tone", () => {
    render(<Avatar initials="LV" size={56} tone="external" className="extra" />);
    expect(screen.getByText("LV")).toHaveClass("extra");
  });
});
