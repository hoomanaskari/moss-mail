import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { mossDev, useAppIdentity } from "../services/appIdentity";
import { pristineUpdates, useUpdates, type UpdateState } from "../state/updates";
import { resetStore } from "../test/fixtures";
import { AppStatus, percent } from "./AppStatus";

beforeEach(() => resetStore([]));

const show = (update: UpdateState) => act(() => useUpdates.setState({ update }));

it("says nothing while there's nothing to install", () => {
  const { container } = render(<AppStatus />);
  for (const update of [{ state: "idle" }, { state: "checking" }, { state: "current" }, { state: "failed", during: "check", message: "offline" }] as const) {
    show(update);
    expect(container).toHaveTextContent(/^$/);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  }
});

it("offers a new Moss until you act on it, and installs it when you click", async () => {
  const install = vi.fn(() => Promise.resolve());
  useUpdates.setState({ install });
  render(<AppStatus />);
  show({ state: "available", version: "0.2.0", notes: "Faster search" });
  const button = screen.getByRole("button", { name: "Update to Moss 0.2.0" });
  // Reachable from the keyboard, as every button.
  button.focus();
  await userEvent.setup().keyboard("{Enter}");
  expect(install).toHaveBeenCalledOnce();
});

it("shows the download with its progress once the size is known", () => {
  render(<AppStatus />);
  show({ state: "downloading", version: "0.2.0", done: 0, total: 0 });
  expect(screen.getByRole("status")).toHaveTextContent(/^Updating to Moss 0\.2\.0$/);
  expect(screen.getByRole("progressbar", { name: "Downloading Moss 0.2.0" })).toHaveAttribute("aria-valuenow", "0");
  show({ state: "downloading", version: "0.2.0", done: 4, total: 10 });
  expect(screen.getByRole("status")).toHaveTextContent("Updating to Moss 0.2.0 · 40%");
  expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "40");
});

it("offers Restart now once the new Moss is in", async () => {
  const restart = vi.fn(() => Promise.resolve());
  useUpdates.setState({ restart });
  render(<AppStatus />);
  show({ state: "ready", version: "0.2.0" });
  expect(screen.getByText("Moss 0.2.0 is ready")).toBeInTheDocument();
  await userEvent.setup().click(screen.getByRole("button", { name: "Restart now" }));
  expect(restart).toHaveBeenCalledOnce();
});

it("says a download failed, and tries again", async () => {
  const install = vi.fn(() => Promise.resolve());
  useUpdates.setState({ install });
  render(<AppStatus />);
  show({ state: "failed", during: "download", message: "cut off", version: "0.2.0" });
  expect(screen.getByRole("alert")).toHaveTextContent("Couldn’t download Moss 0.2.0");
  await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
  expect(install).toHaveBeenCalledOnce();
  useUpdates.setState(pristineUpdates);
});

it("marks Moss Dev, quietly and always", () => {
  render(<AppStatus />);
  expect(screen.queryByText("Dev")).not.toBeInTheDocument();
  act(() => useAppIdentity.setState(mossDev));
  expect(screen.getByText("Dev")).toHaveAttribute("title", "Moss Dev: the app for trying changes, apart from your Moss");
});

it("works out a percent only once the size is known", () => {
  expect(percent(1, 0)).toBeUndefined();
  expect(percent(1, 3)).toBe(33);
  expect(percent(3, 3)).toBe(100);
});
