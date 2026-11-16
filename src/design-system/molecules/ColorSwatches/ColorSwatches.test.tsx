import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { ColorSwatches } from "./ColorSwatches";

it("offers every account colour and marks the chosen one", async () => {
  const onChange = vi.fn();
  render(<ColorSwatches label="Account colour" value="green" onChange={onChange} />);
  const group = screen.getByRole("radiogroup", { name: "Account colour" });
  const swatches = screen.getAllByRole("radio");
  expect(group).toContainElement(swatches[0]);
  expect(swatches).toHaveLength(8);
  expect(screen.getByRole("radio", { name: "Green" })).toHaveAttribute("aria-checked", "true");
  await userEvent.click(screen.getByRole("radio", { name: "Violet" }));
  expect(onChange).toHaveBeenCalledWith("violet");
});
