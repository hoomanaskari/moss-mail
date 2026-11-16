import { beforeEach, expect, it, vi } from "vitest";

type Event = { event: "Started"; data: { contentLength?: number } } | { event: "Progress"; data: { chunkLength: number } } | { event: "Finished" };

const check = vi.fn();
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.1.0" }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: () => check() }));

const { tauriUpdates } = await import("./tauriUpdates");

/** An update whose download sends these events. */
function update(version: string, events: Event[], body?: string) {
  return {
    version,
    body,
    downloadAndInstall: vi.fn(async (on: (e: Event) => void) => events.forEach(on)),
  };
}

beforeEach(() => {
  check.mockReset();
  invoke.mockReset();
});

it("says Moss's version", async () => {
  expect(await tauriUpdates.version()).toBe("0.1.0");
});

it("finds nothing when Moss is up to date", async () => {
  check.mockResolvedValue(null);
  expect(await tauriUpdates.check()).toBeNull();
});

it("finds a newer Moss with its notes, and installs it counting the bytes", async () => {
  const found = update(
    "0.2.0",
    [{ event: "Started", data: { contentLength: 10 } }, { event: "Progress", data: { chunkLength: 4 } }, { event: "Progress", data: { chunkLength: 6 } }, { event: "Finished" }],
    "Faster search",
  );
  check.mockResolvedValue(found);
  expect(await tauriUpdates.check()).toEqual({ version: "0.2.0", notes: "Faster search" });
  const seen: [number, number][] = [];
  await tauriUpdates.install((done, total) => seen.push([done, total]));
  expect(seen).toEqual([
    [4, 10],
    [10, 10],
  ]);
  expect(found.downloadAndInstall).toHaveBeenCalledOnce();
});

it("counts bytes even when the server doesn't say the size", async () => {
  check.mockResolvedValue(update("0.2.0", [{ event: "Started", data: {} }, { event: "Progress", data: { chunkLength: 3 } }]));
  await tauriUpdates.check();
  const seen: [number, number][] = [];
  await tauriUpdates.install((done, total) => seen.push([done, total]));
  expect(seen).toEqual([[3, 0]]);
});

it("won't install before a check found something", async () => {
  check.mockResolvedValue(null);
  await tauriUpdates.check();
  await expect(tauriUpdates.install(() => {})).rejects.toThrow("No update to install");
});

it("restarts into the new Moss", async () => {
  await tauriUpdates.restart();
  expect(invoke).toHaveBeenCalledWith("request_restart");
});
