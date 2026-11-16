import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import { demoSummaries, setSummaries, type SummaryBackend } from "../../services/summaries";
import { useSummarySettings } from "../../state/summary";
import { SummariesSettings } from "./SummariesSettings";

let demo: SummaryBackend;

async function show() {
  render(<SummariesSettings />);
  await act(async () => {});
}

beforeEach(() => {
  // Rules exist, but they're set and stopped from the email, never listed here.
  demo = demoSummaries({ rules: [{ id: 1, sender: "dana@brightlane.com", name: "Dana Whitfield", kind: "actions", createdAt: 0 }] });
  setSummaries(demo);
  useSummarySettings.setState({ automatic: true, problem: undefined });
});

it("has only the automatic one-liners switch, and says rules live on the email", async () => {
  await show();
  const card = screen.getByLabelText("Summaries");
  expect(within(card).getByRole("switch", { name: "Auto summaries" })).toHaveAttribute("aria-checked", "true");
  expect(within(card).getAllByRole("switch")).toHaveLength(1);
  expect(screen.queryByText("Dana Whitfield")).not.toBeInTheDocument();
  expect(screen.queryByText("dana@brightlane.com")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Remove rule|Kind for/ })).not.toBeInTheDocument();
  expect(screen.getByText("Rules live on the email: press S to set one, Stop rule to end it.")).toBeInTheDocument();
});

it("shows the saved setting once loaded", async () => {
  await demo.setAutomatic(false);
  await show();
  expect(screen.getByRole("switch", { name: "Auto summaries" })).toHaveAttribute("aria-checked", "false");
});

it("turns the automatic one-liners off", async () => {
  await show();
  await userEvent.setup().click(screen.getByRole("switch", { name: "Auto summaries" }));
  expect(await demo.automatic()).toBe(false);
  expect(screen.getByRole("switch", { name: "Auto summaries" })).toHaveAttribute("aria-checked", "false");
});

it("says why a change didn't save", async () => {
  await show();
  act(() => useSummarySettings.setState({ problem: "disk full" }));
  expect(screen.getByText("disk full")).toBeInTheDocument();
});
