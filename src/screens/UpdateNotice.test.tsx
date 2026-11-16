import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { demoUpdates, setUpdates } from "../services/updates";
import { pristineUpdates, useUpdates, type UpdateState } from "../state/updates";
import { UpdateNotice } from "./UpdateNotice";

beforeEach(() => useUpdates.setState(pristineUpdates));
afterEach(() => setUpdates(demoUpdates()));

const show = (update: UpdateState) => act(() => useUpdates.setState({ update }));

it("says nothing while there's nothing to say", () => {
  const { container } = render(<UpdateNotice />);
  expect(container).toBeEmptyDOMElement();
});

it("says how a check you asked for is going", () => {
  render(<UpdateNotice />);
  show({ state: "checking" });
  expect(screen.getByRole("status")).toHaveTextContent("Checking for updates…");
  show({ state: "current" });
  expect(screen.getByRole("status")).toHaveTextContent("Moss is up to date");
  show({ state: "failed", message: "Couldn’t check for updates. Try again later." });
  expect(screen.getByRole("alert")).toHaveTextContent("Couldn’t check for updates. Try again later.");
});

it("shows the download quietly, with how far it is once the size is known", () => {
  render(<UpdateNotice />);
  show({ state: "downloading", version: "0.2.0", done: 0, total: 0 });
  expect(screen.getByRole("status")).toHaveTextContent(/^Getting Moss 0\.2\.0$/);
  show({ state: "downloading", version: "0.2.0", done: 4, total: 10 });
  expect(screen.getByRole("status")).toHaveTextContent("Getting Moss 0.2.0 · 40%");
});

it("offers a restart once the new Moss is in", async () => {
  const restart = vi.fn(() => Promise.resolve());
  setUpdates({ ...demoUpdates(), restart });
  render(<UpdateNotice />);
  show({ state: "ready", version: "0.2.0" });
  expect(screen.getByText("Moss 0.2.0 is ready")).toBeInTheDocument();
  await userEvent.setup().click(screen.getByRole("button", { name: "Restart" }));
  expect(restart).toHaveBeenCalledOnce();
});
