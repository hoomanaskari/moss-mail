import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { FlowSteps } from "./FlowSteps";

it("marks steps done, current and still to come", () => {
  render(<FlowSteps steps={["Provider", "Sign in", "Sync"]} current={1} />);
  const items = screen.getAllByRole("listitem");
  expect(screen.getByRole("list", { name: "Steps" })).toBeInTheDocument();
  expect(items[0]).toHaveTextContent("Provider");
  expect(items[0].querySelector("svg")).not.toBeNull();
  expect(items[1]).toHaveAttribute("aria-current", "step");
  expect(items[2]).not.toHaveAttribute("aria-current");
  expect(items[2].querySelector("svg")).toBeNull();
});
