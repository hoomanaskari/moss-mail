import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { mossDev, useAppIdentity } from "../../services/appIdentity";
import { useUpdates, type UpdateState } from "../../state/updates";
import { resetStore } from "../../test/fixtures";
import { AboutMoss } from "./AboutMoss";

beforeEach(() => resetStore([]));

const now = () => new Date(2026, 10, 16, 15, 30).getTime();
const show = (update: UpdateState) => act(() => useUpdates.setState({ update }));
const status = () => screen.getByRole("status");

it("shows the version, a way to check, and that it hasn't checked yet", async () => {
  const check = vi.fn(() => Promise.resolve());
  useUpdates.setState({ version: "0.1.0", check });
  render(<AboutMoss now={now} />);
  expect(screen.getByText("Moss 0.1.0 · Free and open source")).toBeInTheDocument();
  expect(screen.getByText("Not checked yet")).toBeInTheDocument();
  expect(status()).toBeEmptyDOMElement();
  await userEvent.setup().click(screen.getByRole("button", { name: "Check for updates" }));
  expect(check).toHaveBeenCalledWith({ asked: true });
});

it("answers the way the update window does, and says when it last checked", () => {
  useUpdates.setState({ checkedAt: now() - 5 * 60_000 });
  render(<AboutMoss now={now} />);
  expect(screen.getByText("Moss · Free and open source")).toBeInTheDocument();
  expect(screen.getByText("Last checked 5 min ago")).toBeInTheDocument();
  show({ state: "checking" });
  expect(status()).toHaveTextContent("Checking for updates…");
  expect(screen.getByRole("button", { name: "Check for updates" })).toBeDisabled();
  show({ state: "current" });
  expect(status()).toHaveTextContent("Moss is up to date.");
  expect(screen.getByRole("button", { name: "Check for updates" })).toBeEnabled();
  show({ state: "failed", during: "check", message: "The network connection was lost." });
  expect(status()).toHaveTextContent("Couldn’t check for updates: The network connection was lost.");
  show({ state: "downloading", version: "0.2.0", done: 0, total: 0 });
  expect(status()).toHaveTextContent("Updating to Moss 0.2.0…");
  show({ state: "downloading", version: "0.2.0", done: 1, total: 4 });
  expect(status()).toHaveTextContent("Updating to Moss 0.2.0 · 25%");
});

it("installs a new Moss, restarts into it, and tries a failed download again", async () => {
  const install = vi.fn(() => Promise.resolve());
  const restart = vi.fn(() => Promise.resolve());
  useUpdates.setState({ install, restart });
  const user = userEvent.setup();
  render(<AboutMoss now={now} />);
  show({ state: "available", version: "0.2.0" });
  expect(status()).toHaveTextContent("Moss 0.2.0 is available.");
  await user.click(screen.getByRole("button", { name: "Update now" }));
  show({ state: "failed", during: "download", message: "cut off", version: "0.2.0" });
  expect(status()).toHaveTextContent("Couldn’t download Moss 0.2.0: cut off");
  await user.click(screen.getByRole("button", { name: "Try again" }));
  expect(install).toHaveBeenCalledTimes(2);
  show({ state: "ready", version: "0.2.0" });
  expect(status()).toHaveTextContent("Moss 0.2.0 is ready.");
  await user.click(screen.getByRole("button", { name: "Restart now" }));
  expect(restart).toHaveBeenCalledOnce();
});

it("in Moss Dev, says updates are off and offers no check", () => {
  useAppIdentity.setState(mossDev);
  useUpdates.setState({ version: "0.1.0" });
  render(<AboutMoss />);
  expect(screen.getByText("Moss Dev 0.1.0 · Free and open source")).toBeInTheDocument();
  expect(screen.getByText("Updates are off in Moss Dev.")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
