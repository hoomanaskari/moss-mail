import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Paperclip } from "lucide-react";
import { expect, it, vi } from "vitest";
import { FilterChip } from "./FilterChip";

it("is a toggle button showing its key", async () => {
  const onClick = vi.fn();
  render(<FilterChip icon={Paperclip} label="Has attachment" keys="⌥P" onClick={onClick} />);
  const chip = screen.getByRole("button", { name: /Has attachment/ });
  expect(chip).toHaveAttribute("aria-pressed", "false");
  expect(chip.querySelector("kbd")).toHaveTextContent("Alt+P");
  await userEvent.click(chip);
  expect(onClick).toHaveBeenCalled();
});

it("an active person filter shows their initials and a remove mark", () => {
  render(<FilterChip person="DO" label="From: Daniel Okafor" keys="⌥F" active removable onClick={() => {}} />);
  const chip = screen.getByRole("button", { name: /From: Daniel Okafor/ });
  expect(chip).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByText("DO")).toBeInTheDocument();
  expect(chip.querySelectorAll("svg")).toHaveLength(1);
});

it("a menu filter has a chevron", () => {
  render(<FilterChip icon={Paperclip} label="Date: Any time" keys="⌥D" menu onClick={() => {}} />);
  expect(screen.getByRole("button").querySelectorAll("svg")).toHaveLength(2);
});
