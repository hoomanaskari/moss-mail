import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

type Payload = { type: "enter" | "over" | "drop" | "leave"; paths: string[] };
let handler: ((e: { payload: Payload }) => void) | undefined;
const unlisten = vi.fn();
let listening: Promise<void> = Promise.resolve();

vi.mock("../lib/platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (cb: (e: { payload: Payload }) => void) => {
      handler = cb;
      return listening.then(() => unlisten);
    },
  }),
}));

const { useFileDrop } = await import("./useFileDrop");

beforeEach(() => {
  handler = undefined;
  unlisten.mockClear();
  listening = Promise.resolve();
});

const send = (payload: Payload) => act(() => handler!({ payload }));

it("files from Finder say when they're over the window, and their paths when dropped", async () => {
  const drop = vi.fn();
  const over = vi.fn();
  const { unmount } = renderHook(() => useFileDrop(true, drop, over));
  await vi.waitFor(() => expect(handler).toBeDefined());
  send({ type: "enter", paths: ["/a.pdf"] });
  send({ type: "over", paths: [] });
  expect(over.mock.calls).toEqual([[true], [true]]);
  send({ type: "leave", paths: [] });
  expect(over).toHaveBeenLastCalledWith(false);
  send({ type: "drop", paths: ["/a.pdf", "/b.txt"] });
  expect(drop).toHaveBeenCalledWith(["/a.pdf", "/b.txt"]);
  // A drop with nothing in it (dragged from another app) adds nothing.
  send({ type: "drop", paths: [] });
  expect(drop).toHaveBeenCalledTimes(1);
  await act(async () => {});
  unmount();
  expect(unlisten).toHaveBeenCalledOnce();
});

it("doesn't listen without an email being written", async () => {
  renderHook(() => useFileDrop(false, vi.fn(), vi.fn()));
  await act(async () => {});
  expect(handler).toBeUndefined();
});

it("stops listening once it starts, if the composer closed first", async () => {
  let ready = () => {};
  listening = new Promise((resolve) => (ready = resolve));
  const { unmount } = renderHook(() => useFileDrop(true, vi.fn(), vi.fn()));
  await vi.waitFor(() => expect(handler).toBeDefined());
  unmount();
  expect(unlisten).not.toHaveBeenCalled();
  await act(async () => ready());
  expect(unlisten).toHaveBeenCalledOnce();
});

it("ignores native drops while quit makes the composer inert", async () => {
  const { useQuit } = await import("./quitState");
  const drop = vi.fn();const over = vi.fn();
  renderHook(() => useFileDrop(true, drop, over));
  await vi.waitFor(() => expect(handler).toBeDefined());
  useQuit.setState({ token: "quit" });
  send({ type: "enter", paths: ["/a.pdf"] });
  send({ type: "drop", paths: ["/a.pdf"] });
  expect(drop).not.toHaveBeenCalled();expect(over).not.toHaveBeenCalledWith(true);
});
