import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { useUpdates, type UpdateState } from "../state/updates";
import { resetStore } from "../test/fixtures";
import { UpdateDialog } from "./UpdateDialog";

beforeEach(() => resetStore([]));

const open = (update: UpdateState, version: string | null = "0.1.0") => act(() => useUpdates.setState({ update, open: true, version: version ?? undefined }));
const dialog = () => screen.getByRole("dialog");

it("stays shut until a check you asked for opens it", () => {
  const { container } = render(<UpdateDialog />);
  act(() => useUpdates.setState({ update: { state: "available", version: "0.2.0" } }));
  expect(container).toBeEmptyDOMElement();
  // Open with nothing to say yet.
  act(() => useUpdates.setState({ update: { state: "idle" }, open: true }));
  expect(container).toBeEmptyDOMElement();
});

it("opens as a modal and says it's checking", () => {
  const modal = vi.spyOn(HTMLDialogElement.prototype, "showModal");
  render(<UpdateDialog />);
  open({ state: "checking" });
  expect(modal).toHaveBeenCalledOnce();
  expect(dialog()).toHaveAccessibleName("Checking for updates…");
  modal.mockRestore();
});

it("moves the focus to each answer's main button as the answer comes in", () => {
  const modal = vi.spyOn(HTMLDialogElement.prototype, "showModal");
  render(<UpdateDialog />);
  open({ state: "checking" });
  expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
  open({ state: "available", version: "0.2.0" });
  expect(screen.getByRole("button", { name: "Update now" })).toHaveFocus();
  expect(modal).toHaveBeenCalledOnce();
  modal.mockRestore();
});

it("says Moss is up to date, with the version", async () => {
  render(<UpdateDialog />);
  open({ state: "current" });
  expect(dialog()).toHaveAccessibleName("Moss is up to date");
  expect(dialog()).toHaveTextContent("You have Moss 0.1.0, the newest version.");
  await userEvent.setup().click(screen.getByRole("button", { name: "OK" }));
  expect(useUpdates.getState().open).toBe(false);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("says up to date even before the version is known", () => {
  render(<UpdateDialog />);
  open({ state: "current" }, null);
  expect(dialog()).toHaveTextContent("You have the newest version.");
});

it("offers the new Moss with what's new, and updates now or later", async () => {
  const install = vi.fn(() => Promise.resolve());
  useUpdates.setState({ install });
  const user = userEvent.setup();
  render(<UpdateDialog />);
  open({ state: "available", version: "0.2.0", notes: "**Faster** search" });
  expect(dialog()).toHaveAccessibleName("Moss 0.2.0 is available");
  expect(dialog()).toHaveTextContent("You have Moss 0.1.0. Moss downloads the new version, installs it and restarts.");
  expect(screen.getByRole("region", { name: "What’s new" })).toHaveTextContent("Faster search");
  expect(screen.getByRole("button", { name: "Update now" })).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "Update now" }));
  expect(install).toHaveBeenCalledOnce();
  await user.click(screen.getByRole("button", { name: "Later" }));
  expect(useUpdates.getState().open).toBe(false);
});

it("offers it without notes, or a known version, too", () => {
  render(<UpdateDialog />);
  open({ state: "available", version: "0.2.0" }, null);
  expect(dialog()).toHaveTextContent(/^Moss 0\.2\.0 is availableMoss downloads/);
  expect(screen.queryByRole("region", { name: "What’s new" })).not.toBeInTheDocument();
});

it("shows the download, and can be hidden while it goes on", async () => {
  render(<UpdateDialog />);
  open({ state: "downloading", version: "0.2.0", done: 0, total: 0 });
  expect(dialog()).toHaveAccessibleName("Updating to Moss 0.2.0");
  expect(dialog()).toHaveTextContent("Downloading… Moss restarts when it’s done.");
  open({ state: "downloading", version: "0.2.0", done: 4, total: 10 });
  expect(screen.getByRole("progressbar", { name: "Downloading Moss 0.2.0" })).toHaveAttribute("aria-valuenow", "40");
  expect(dialog()).toHaveTextContent("Downloaded 40%.");
  await userEvent.setup().click(screen.getByRole("button", { name: "Hide" }));
  expect(useUpdates.getState()).toMatchObject({ open: false, update: { state: "downloading" } });
});

it("offers Restart now once the new Moss is in", async () => {
  const restart = vi.fn(() => Promise.resolve());
  useUpdates.setState({ restart });
  render(<UpdateDialog />);
  open({ state: "ready", version: "0.2.0" });
  expect(dialog()).toHaveAccessibleName("Moss 0.2.0 is ready");
  await userEvent.setup().click(screen.getByRole("button", { name: "Restart now" }));
  expect(restart).toHaveBeenCalledOnce();
});

it("says why a check failed, and checks again", async () => {
  const check = vi.fn(() => Promise.resolve());
  useUpdates.setState({ check });
  render(<UpdateDialog />);
  open({ state: "failed", during: "check", message: "The network connection was lost." });
  expect(dialog()).toHaveAccessibleName("Couldn’t check for updates");
  expect(screen.getByRole("alert")).toHaveTextContent("The network connection was lost.");
  await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
  expect(check).toHaveBeenCalledWith({ asked: true });
});

it("says why a download failed, and downloads again", async () => {
  const install = vi.fn(() => Promise.resolve());
  useUpdates.setState({ install });
  render(<UpdateDialog />);
  open({ state: "failed", during: "download", message: "cut off", version: "0.2.0" });
  expect(dialog()).toHaveAccessibleName("Couldn’t download Moss 0.2.0");
  await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
  expect(install).toHaveBeenCalledOnce();
});

it("closes with Esc, and keeps its keys from the screen behind", async () => {
  const behind = vi.fn();
  window.addEventListener("keydown", behind);
  render(<UpdateDialog />);
  open({ state: "current" });
  expect(screen.getByRole("button", { name: "OK" })).toHaveFocus();
  await userEvent.setup().keyboard("e");
  expect(behind).not.toHaveBeenCalled();
  act(() => {
    dialog().dispatchEvent(new Event("cancel", { cancelable: true }));
  });
  expect(useUpdates.getState().open).toBe(false);
  window.removeEventListener("keydown", behind);
});
