import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { SummaryMenu } from "./SummaryMenu";

it("offers the five kinds with their keys, marks the one shown, and closes", async () => {
  const onPick = vi.fn();
  const onClose = vi.fn();
  render(<SummaryMenu current="facts" onPick={onPick} onClose={onClose} />);
  const items = screen.getAllByRole("menuitemradio");
  expect(items.map((i) => i.textContent)).toEqual([
    "ShortOne or two lines1",
    "DetailedThe main points, in order2",
    "Action pointsWhat you need to do, with dates3",
    "Key factsPeople, dates, amounts, links4",
    "Custom…Say how you want it5",
  ]);
  expect(items.map((i) => i.getAttribute("aria-checked"))).toEqual(["false", "false", "false", "true", "false"]);
  expect(screen.getByText("Runs on this Mac")).toBeInTheDocument();
  const user = userEvent.setup();
  await user.click(items[2]);
  expect(onPick).toHaveBeenCalledWith("actions");
  await user.click(screen.getByRole("button", { name: "EscClose" }));
  expect(onClose).toHaveBeenCalled();
});

it("↑ ↓ ↵ pick from the kind shown; hovering never moves the mark", async () => {
  const onPick = vi.fn();
  render(<SummaryMenu current="facts" onPick={onPick} onClose={vi.fn()} />);
  const user = userEvent.setup();
  await user.keyboard("{ArrowDown}{Enter}");
  expect(onPick).toHaveBeenLastCalledWith("custom");
  await user.hover(screen.getByRole("menuitemradio", { name: /Detailed/ }));
  await user.keyboard("{ArrowUp}{Enter}");
  expect(onPick).toHaveBeenLastCalledWith("facts");
});

it("with no kind shown yet, starts on the first", async () => {
  const onPick = vi.fn();
  render(<SummaryMenu onPick={onPick} onClose={vi.fn()} />);
  await userEvent.setup().keyboard("{Enter}");
  expect(onPick).toHaveBeenCalledWith("short");
});
