import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { SetupSteps } from "./SetupSteps";

const steps = [
  { title: "Turn on 2-Step Verification", sub: "Already on?", url: "https://a" },
  { title: "Create an app password", sub: "On myaccount…", url: "https://b" },
  { title: "Paste it below" },
];

it("numbers the steps, ticks the done ones, and highlights the next", async () => {
  const onOpen = vi.fn();
  render(<SetupSteps steps={steps} done={1} onOpen={onOpen} />);
  const items = screen.getAllByRole("listitem");
  expect(items[0]).toHaveAccessibleName(/done/);
  expect(items[1]).toHaveAttribute("aria-current", "step");
  expect(items[2]).toHaveTextContent("3Paste it below");
  // Several pages: ⌘1, ⌘2…
  await userEvent.click(screen.getByRole("button", { name: /Open.*2/ }));
  expect(onOpen).toHaveBeenCalledWith(1, "https://b");
});

it("uses ⌘O when only one step has a page, and can head the list and rename the action", () => {
  render(
    <SetupSteps
      label="How to fix it"
      steps={[{ title: "Send your admin the approval link", url: "https://c", action: "Copy link", keys: "⌘C" }, { title: "Try again" }]}
      done={0}
      onOpen={() => {}}
    />,
  );
  expect(screen.getByText("How to fix it")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Copy link/ })).toHaveTextContent(/Ctrl\+C|⌘C/);
});

it("gives the single page ⌘O", () => {
  render(<SetupSteps steps={[{ title: "Create", url: "https://d" }, { title: "Paste" }]} done={0} onOpen={() => {}} />);
  expect(screen.getByRole("button", { name: /Open/ })).toHaveTextContent(/O$/);
});
