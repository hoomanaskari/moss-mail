import { afterEach, expect, it, vi } from "vitest";
import { pristineUpdates, useUpdates } from "../state/updates";

const listeners = new Map<string, (e: { payload: unknown }) => void>();
const unlisten = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return unlisten;
  },
}));
const { checkForUpdates, watchMenu } = await import("./menu");

afterEach(() => useUpdates.setState(pristineUpdates));

it("listens for the names the menu sends (src-tauri/src/menu.rs)", () => {
  expect(checkForUpdates).toBe("check-for-updates");
});

it("checks for updates in the update window when the menu says so, and nothing else", async () => {
  const check = vi.fn(() => Promise.resolve());
  useUpdates.setState({ check });
  const stop = watchMenu();
  await Promise.resolve();
  listeners.get("moss://menu")!({ payload: "something-else" });
  expect(check).not.toHaveBeenCalled();
  listeners.get("moss://menu")!({ payload: checkForUpdates });
  expect(check).toHaveBeenCalledWith({ asked: true, open: true });
  stop();
  expect(unlisten).toHaveBeenCalledOnce();
});
