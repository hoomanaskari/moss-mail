import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { Checkbox } from "./Checkbox";

it("ticks and unticks", async () => {
  const onChange = vi.fn();
  const { rerender } = render(<Checkbox aria-label="rossstudio.art" checked={false} onChange={onChange} />);
  const box = screen.getByRole("checkbox", { name: "rossstudio.art" });
  expect(box).not.toBeChecked();
  expect(box.parentElement!.querySelector("svg")).toBeNull();
  await userEvent.click(box);
  expect(onChange).toHaveBeenCalledWith(true);
  rerender(<Checkbox aria-label="rossstudio.art" checked onChange={onChange} />);
  expect(box).toBeChecked();
  expect(box.parentElement!.querySelector("svg")).not.toBeNull();
});
