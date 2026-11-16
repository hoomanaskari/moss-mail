import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { Select } from "./Select";

const options = [
  { value: "all", label: "All addresses" },
  { value: "me", label: "Only me@mayaross.dev" },
];

it("shows the chosen option and reports a new choice", async () => {
  const onChange = vi.fn();
  render(<Select aria-label="Addresses" value="all" options={options} onChange={onChange} />);
  expect(screen.getByText("All addresses", { selector: "span" })).toBeInTheDocument();
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Addresses" }), "me");
  expect(onChange).toHaveBeenCalledWith("me");
});

it("can be turned off, and falls back to the first option for an unknown value", () => {
  render(<Select aria-label="Security" value="?" options={options} onChange={() => {}} disabled variant="field" />);
  expect(screen.getByRole("combobox")).toBeDisabled();
  expect(screen.getByText("All addresses", { selector: "span" })).toBeInTheDocument();
});
