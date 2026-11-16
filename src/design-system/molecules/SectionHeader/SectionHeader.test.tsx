import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { SectionHeader } from "./SectionHeader";

it("names a part of the page, with something on the right", () => {
  render(<SectionHeader label="Theme" right={<span>Change</span>} />);
  expect(screen.getByRole("heading", { name: "Theme" })).toBeInTheDocument();
  expect(screen.getByText("Change")).toBeInTheDocument();
});

it("can stand alone", () => {
  const { container } = render(<SectionHeader label="App" />);
  expect(container).toHaveTextContent("App");
});
