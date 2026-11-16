import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { FlowLayout } from "./FlowLayout";

it("stacks a step’s content in one readable column", () => {
  render(
    <FlowLayout gap={24} tight>
      <p>One</p>
      <p>Two</p>
    </FlowLayout>,
  );
  const column = screen.getByText("One").parentElement!;
  expect(column).toContainElement(screen.getByText("Two"));
  expect(column.style.gap).toBe("24px");
});
