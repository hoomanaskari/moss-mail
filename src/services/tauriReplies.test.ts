import { beforeEach, expect, it, vi } from "vitest";

const invoke = vi.fn();
const listeners = new Map<string, (e: { payload: unknown }) => void>();
const stopped: string[] = [];

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return () => stopped.push(name);
  },
}));

const { tauriReplies } = await import("./tauriReplies");
const { onAiText } = await import("./tauriText");

beforeEach(() => {
  invoke.mockReset();
  listeners.clear();
  stopped.length = 0;
});

it("stopping is one Rust command", async () => {
  invoke.mockResolvedValue(true);
  expect(await tauriReplies.stop("r1")).toBe(true);
  expect(invoke.mock.calls).toEqual([["ai_stop", { id: "r1" }]]);
});

it("quick replies stream in as JSON from moss://ai-text for their own id", async () => {
  const seen: unknown[] = [];
  let finish: (options: unknown) => void = () => {};
  invoke.mockReturnValue(new Promise((resolve) => (finish = resolve)));
  const request = { id: "q1", threadId: "7", accountId: "work", signature: ["Best", "Maya"] };
  const done = tauriReplies.options(request, (o) => seen.push(o));
  await Promise.resolve();
  const send = listeners.get("moss://ai-text")!;
  send({ payload: { id: "q1", text: '[{"label":"Yes","draft":"Su"}]' } });
  send({ payload: { id: "other", text: "Not this one" } });
  finish([{ label: "Yes", draft: "Sure." }]);
  expect(await done).toEqual([{ label: "Yes", draft: "Sure." }]);
  expect(seen).toEqual([[{ label: "Yes", draft: "Su" }]]);
  expect(invoke).toHaveBeenCalledWith("ai_reply_options", request);
  expect(stopped).toEqual(["moss://ai-text"]);
});

it("a reply streams in from moss://ai-text for its own id, and stops listening when it's done", async () => {
  const seen: string[] = [];
  let finish: (text: string) => void = () => {};
  invoke.mockReturnValue(new Promise((resolve) => (finish = resolve)));
  const request = { id: "r1", threadId: "7", accountId: "work", job: { kind: "accept" as const }, text: "" };
  const done = tauriReplies.write(request, (t) => seen.push(t));
  await Promise.resolve();
  const send = listeners.get("moss://ai-text")!;
  send({ payload: { id: "r1", text: "Hi" } });
  send({ payload: { id: "other", text: "Not this one" } });
  send({ payload: { id: "r1", text: "Hi Dana" } });
  finish("Hi Dana.");
  expect(await done).toBe("Hi Dana.");
  expect(seen).toEqual(["Hi", "Hi Dana"]);
  expect(invoke).toHaveBeenCalledWith("ai_reply", request);
  expect(stopped).toEqual(["moss://ai-text"]);
});

it("a failed reply stops listening too", async () => {
  invoke.mockRejectedValue({ kind: "notReady", message: "the AI model isn't downloaded yet" });
  const request = { id: "r2", accountId: "work", job: { kind: "fix" as const }, text: "Hi" };
  await expect(tauriReplies.write(request, () => {})).rejects.toMatchObject({ kind: "notReady" });
  await Promise.resolve();
  expect(stopped).toEqual(["moss://ai-text"]);
});

it("any stream can be heard by its id", async () => {
  const seen: string[] = [];
  const stop = onAiText("a1", (t) => seen.push(t));
  await Promise.resolve();
  listeners.get("moss://ai-text")!({ payload: { id: "a1", text: "It's" } });
  stop();
  expect(seen).toEqual(["It's"]);
  expect(stopped).toEqual(["moss://ai-text"]);
});
