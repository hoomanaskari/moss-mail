import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AccountBadge } from "./AccountBadge";

describe("AccountBadge", () => {
  it("shows the account letter in its colour, hidden from screen readers", () => {
    render(<AccountBadge letter="W" color="work" />);
    const badge = screen.getByText("W");
    expect(badge).toHaveAttribute("aria-hidden", "true");
    expect(badge.style.background).toBe("var(--work)");
  });

  it("keeps an extra class name", () => {
    render(<AccountBadge letter="P" color="personal" size={28} className="extra" />);
    expect(screen.getByText("P")).toHaveClass("extra");
  });
});
