import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { SyncProgress } from "./SyncProgress";

it("shows how much mail is down and what happens next", () => {
  render(
    <SyncProgress
      done={1240}
      total={1930}
      items={[
        { label: "Signed in", state: "done" },
        { label: "Folders and labels", state: "done", status: "14 found" },
        { label: "Recent mail, last 30 days", state: "now", status: "64%" },
        { label: "Older mail, back to 2016", state: "next", status: "Next, in the background" },
      ]}
    />,
  );
  expect(screen.getByText("1,240 of 1,930 emails")).toBeInTheDocument();
  expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "64");
  expect(screen.getByRole("img", { name: "In progress" })).toBeInTheDocument();
  expect(screen.getByText("Next, in the background")).toBeInTheDocument();
});

it("before any mail has come down, the bar is empty rather than broken", () => {
  render(<SyncProgress done={0} total={0} items={[]} />);
  expect(screen.getByText("0 of 0 emails")).toBeInTheDocument();
  expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "0");
});
