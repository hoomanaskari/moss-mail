import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { DomainList } from "./DomainList";

const domains = [
  { name: "rossstudio.art", detail: "Verified · 3 addresses", addresses: ["hello", "maya"], active: true, chosen: true, only: null },
  { name: "old-portfolio.net", detail: "Verified · no mail", addresses: [], active: false, chosen: false, only: null },
];

it("ticks domains on and off and narrows one to a single address", async () => {
  const onToggle = vi.fn();
  const onAddresses = vi.fn();
  render(<DomainList domains={domains} cursor={0} onToggle={onToggle} onAddresses={onAddresses} onCursor={() => {}} />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("checkbox", { name: "old-portfolio.net" }));
  expect(onToggle).toHaveBeenCalledWith(1);
  await user.selectOptions(screen.getByRole("combobox", { name: "Addresses for rossstudio.art" }), "maya");
  expect(onAddresses).toHaveBeenCalledWith(0, "maya");
  await user.selectOptions(screen.getByRole("combobox", { name: "Addresses for rossstudio.art" }), "all");
  expect(onAddresses).toHaveBeenLastCalledWith(0, null);
  expect(screen.getByRole("combobox", { name: "Addresses for old-portfolio.net" })).toBeDisabled();
});

it("leaves the cursor alone when the mouse points at a row, and moves it to a row you click", async () => {
  const onCursor = vi.fn();
  render(<DomainList domains={domains} cursor={0} onToggle={() => {}} onAddresses={() => {}} onCursor={onCursor} />);
  const user = userEvent.setup();
  await user.hover(screen.getByText("old-portfolio.net"));
  expect(onCursor).not.toHaveBeenCalled();
  await user.click(screen.getByText("old-portfolio.net"));
  expect(onCursor).toHaveBeenCalledWith(1);
});
