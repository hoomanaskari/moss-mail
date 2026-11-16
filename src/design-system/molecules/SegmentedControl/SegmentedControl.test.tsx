import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { SegmentedControl } from "./SegmentedControl";

it("picks one of a few options", async () => {
  const onChange = vi.fn();
  render(
    <SegmentedControl
      label="Group by"
      value="date"
      onChange={onChange}
      options={[
        { value: "date", label: "Date" },
        { value: "account", label: "Account", lead: <b>Aa</b> },
      ]}
    />,
  );
  expect(screen.getByRole("radiogroup", { name: "Group by" })).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: "Date" })).toHaveAttribute("aria-checked", "true");
  expect(screen.getByRole("radio", { name: "Account" })).toHaveAttribute("aria-checked", "false");
  await userEvent.click(screen.getByRole("radio", { name: "Account" }));
  expect(onChange).toHaveBeenCalledWith("account");
});
