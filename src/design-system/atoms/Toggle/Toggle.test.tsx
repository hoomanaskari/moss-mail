import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { Toggle } from "./Toggle";

it("is a switch that flips", async () => {
  const onChange = vi.fn();
  const { rerender } = render(<Toggle label="Let Moss choose" on onChange={onChange} />);
  const sw = screen.getByRole("switch", { name: "Let Moss choose" });
  expect(sw).toHaveAttribute("aria-checked", "true");
  await userEvent.click(sw);
  expect(onChange).toHaveBeenCalledWith(false);
  rerender(<Toggle label="Let Moss choose" on={false} onChange={onChange} />);
  expect(sw).toHaveAttribute("aria-checked", "false");
  await userEvent.click(sw);
  expect(onChange).toHaveBeenLastCalledWith(true);
});
