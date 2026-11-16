import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { SectionLabel } from "./SectionLabel";

describe("SectionLabel", () => {
  it("is an h2 by default", () => {
    render(<SectionLabel>Needs your reply</SectionLabel>);
    expect(screen.getByRole("heading", { level: 2, name: "Needs your reply" })).toBeInTheDocument();
  });

  it("can be an h3", () => {
    render(
      <SectionLabel as="h3" size="sm">
        Actions
      </SectionLabel>,
    );
    expect(screen.getByRole("heading", { level: 3, name: "Actions" })).toBeInTheDocument();
  });

  it("can be a plain div that is not a heading", () => {
    render(
      <SectionLabel as="div" className="extra">
        Suggestions
      </SectionLabel>,
    );
    expect(screen.queryByRole("heading")).toBeNull();
    const label = screen.getByText("Suggestions");
    expect(label.tagName).toBe("DIV");
    expect(label).toHaveClass("extra");
  });
});
