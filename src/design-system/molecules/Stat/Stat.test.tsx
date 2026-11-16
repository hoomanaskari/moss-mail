import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Stat } from "./Stat";

describe("Stat", () => {
  it("shows a number and its label", () => {
    render(<Stat value={42} label="emails done" />);
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText("emails done")).toBeInTheDocument();
  });

  it("accepts a string value", () => {
    render(<Stat value="12 min" label="saved" />);
    expect(screen.getByText("12 min")).toBeInTheDocument();
  });
});
