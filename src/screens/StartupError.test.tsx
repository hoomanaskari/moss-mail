import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { start, useStartup } from "../state/startup";
import { StartupError } from "./StartupError";

const stopWatching = vi.fn();
vi.mock("../services/quit", async (actual) => ({
  ...(await actual<typeof import("../services/quit")>()),
  watchQuit: vi.fn(() => stopWatching),
}));

const pristine = useStartup.getState();

/** Loading fails once with `problem`, then works when tried again. */
async function failOnce(problem: string) {
  let release!: () => void;
  const connect = vi
    .fn<() => Promise<void>>()
    .mockRejectedValueOnce(new Error(problem))
    .mockImplementation(() => new Promise<void>((resolve) => (release = resolve)));
  await start(connect);
  return { connect, release: () => act(async () => release()) };
}

beforeEach(() => {
  useStartup.setState(pristine, true);
  vi.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => vi.restoreAllMocks());

describe("StartupError", () => {
  it("says plainly that Moss couldn't open your mail, and why", async () => {
    await failOnce("database is locked");
    render(<StartupError />);
    expect(screen.getByText("Moss couldn’t open your mail")).toBeInTheDocument();
    expect(screen.getByText("database is locked")).toBeInTheDocument();
  });

  it("Try again loads the mail again, and says it is trying until it has", async () => {
    const { connect, release } = await failOnce("database is locked");
    render(<StartupError />);
    await userEvent.setup().click(screen.getByRole("button", { name: /Try again/ }));
    expect(connect).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("button", { name: /Trying again…/ })).toBeInTheDocument();
    await release();
    expect(useStartup.getState()).toEqual({ failed: undefined, trying: false });
  });

  it("↵ tries again too, once, however often it is pressed while trying", async () => {
    const { connect, release } = await failOnce("database is locked");
    render(<StartupError />);
    const user = userEvent.setup();
    await user.keyboard("{Enter}");
    await user.keyboard("{Enter}");
    await user.keyboard("a");
    expect(connect).toHaveBeenCalledTimes(2);
    await release();
  });

  it("listens for ⌘Q, so Moss can still quit, and stops when it goes", async () => {
    const { watchQuit } = await import("../services/quit");
    await failOnce("database is locked");
    const { unmount } = render(<StartupError />);
    expect(watchQuit).toHaveBeenCalled();
    unmount();
    expect(stopWatching).toHaveBeenCalled();
  });
});
