import { beforeEach, expect, it, vi } from "vitest";

const invoke = vi.fn();
let listener: ((e: { payload: unknown }) => void) | undefined;
let heard: string | undefined;

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    heard = name;
    listener = cb;
    return () => {};
  },
}));

const { tauriAi } = await import("./tauriAi");

beforeEach(() => invoke.mockReset().mockResolvedValue("ok"));

it("each call is one Rust command", async () => {
  await tauriAi.status();
  await tauriAi.choose("e2b");
  await tauriAi.download();
  await tauriAi.cancel();
  await tauriAi.ask("7");
  await tauriAi.plan("essentials");
  await tauriAi.remove();
  await tauriAi.removeModel("e2b");
  await tauriAi.downloadModels(["e2b"]);
  await tauriAi.stopModel("e2b");
  expect(invoke.mock.calls).toEqual([
    ["ai_status"],
    ["ai_choose", { id: "e2b" }],
    ["ai_download"],
    ["ai_cancel"],
    ["ai_ask", { id: "7" }],
    ["ai_plan", { plan: "essentials" }],
    ["ai_remove"],
    ["ai_remove_model", { id: "e2b" }],
    ["ai_download_models", { ids: ["e2b"] }],
    ["ai_stop_model", { id: "e2b" }],
  ]);
});

it("status changes come from the moss://ai event", async () => {
  const seen: unknown[] = [];
  tauriAi.onStatus((s) => seen.push(s));
  await Promise.resolve();
  listener!({ payload: { download: { state: "ready" } } });
  expect(heard).toBe("moss://ai");
  expect(seen).toEqual([{ download: { state: "ready" } }]);
});
