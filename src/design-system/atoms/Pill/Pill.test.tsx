import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Pill } from "./Pill";

describe("Pill", () => {
  it("shows its text and keeps an extra class name", () => {
    render(
      <Pill variant="due" className="extra">
        Due Wed
      </Pill>,
    );
    expect(screen.getByText("Due Wed")).toHaveClass("extra");
  });
});
