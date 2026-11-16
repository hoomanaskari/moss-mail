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

const { tauriSummaries } = await import("./tauriSummaries");

beforeEach(() => invoke.mockReset().mockResolvedValue("ok"));

it("each call is one Rust command", async () => {
  await tauriSummaries.kept("7");
  await tauriSummaries.discard("7");
  await tauriSummaries.summarize("7", { kind: "facts" });
  await tauriSummaries.summarize("7", { kind: "custom", instruction: "In French" }, true);
  await tauriSummaries.rules();
  await tauriSummaries.saveRule({ sender: "acme.co", kind: "actions" });
  await tauriSummaries.saveRule({ id: 3, sender: "dana@acme.co", kind: "custom", instruction: "Prices" });
  await tauriSummaries.removeRule(3);
  await tauriSummaries.automatic();
  await tauriSummaries.setAutomatic(false);
  expect(invoke.mock.calls).toEqual([
    ["ai_summaries", { id: "7" }],
    ["discard_summary", { id: "7" }],
    ["ai_summarize", { id: "7", kind: "facts", instruction: undefined, again: false, stream: "summary-1" }],
    ["ai_summarize", { id: "7", kind: "custom", instruction: "In French", again: true, stream: "summary-2" }],
    ["summary_rules"],
    ["save_summary_rule", { id: undefined, sender: "acme.co", kind: "actions", instruction: undefined }],
    ["save_summary_rule", { id: 3, sender: "dana@acme.co", kind: "custom", instruction: "Prices" }],
    ["remove_summary_rule", { id: 3 }],
    ["automatic_summaries"],
    ["set_automatic_summaries", { on: false }],
  ]);
});

it("a summary streams in as JSON from moss://ai-text for its own stream", async () => {
  const seen: unknown[] = [];
  let finish: (view: unknown) => void = () => {};
  invoke.mockReturnValue(new Promise((resolve) => (finish = resolve)));
  const done = tauriSummaries.summarize("7", { kind: "short" }, false, (c) => seen.push(c));
  await Promise.resolve();
  const [, args] = invoke.mock.calls[0] as [string, { stream: string }];
  listener!({ payload: { id: args.stream, text: '{"format":"text","text":"Dana"}' } });
  listener!({ payload: { id: "other", text: "{}" } });
  finish("view");
  expect(await done).toBe("view");
  expect(seen).toEqual([{ format: "text", text: "Dana" }]);
});

it("hears the summaries rules write in the background, and nothing else", async () => {
  const seen: string[] = [];
  tauriSummaries.onWritten((id) => seen.push(id));
  await Promise.resolve();
  expect(heard).toBe("moss://event");
  listener!({ payload: { type: "mailChanged", accountId: "gmail" } });
  listener!({ payload: { type: "summaryWritten" } });
  listener!({ payload: { type: "summaryWritten", threadId: "7" } });
  expect(seen).toEqual(["7"]);
});
