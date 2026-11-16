import { beforeEach, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const { tauriAgents } = await import("./tauriAgents");

beforeEach(() => invoke.mockReset().mockResolvedValue({ on: false, bridge: "Moss" }));

it("each call is one Rust command", async () => {
  expect(await tauriAgents.status()).toEqual({ on: false, bridge: "Moss" });
  await tauriAgents.setOn(true);
  await tauriAgents.newToken();
  expect(invoke.mock.calls).toEqual([["mcp_status"], ["mcp_set_on", { on: true }], ["mcp_new_token"]]);
});
