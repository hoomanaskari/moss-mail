import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { Radio } from "./Radio";

it("is one choice in a group", async () => {
  const onChange = vi.fn();
  render(
    <>
      <Radio name="theme" aria-label="System" checked onChange={onChange} />
      <Radio name="theme" aria-label="Light" checked={false} onChange={onChange} />
    </>,
  );
  expect(screen.getByRole("radio", { name: "System" })).toBeChecked();
  await userEvent.click(screen.getByRole("radio", { name: "Light" }));
  expect(onChange).toHaveBeenCalledOnce();
});

it("can stand in a list without being focusable on its own", () => {
  render(<Radio aria-label="Studio" checked readOnly tabIndex={-1} />);
  expect(screen.getByRole("radio")).toHaveAttribute("tabindex", "-1");
});
