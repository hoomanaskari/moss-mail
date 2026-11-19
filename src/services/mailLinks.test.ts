import { beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../state/store";
import { resetStore, thread } from "../test/fixtures";

const listeners = new Map<string, () => void>();
const unlisten = vi.fn();
const waiting: string[][] = [];
const invoke = vi.fn(async (command: string) => (command === "take_mail_links" ? (waiting.shift() ?? []) : undefined));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: () => void) => {
    listeners.set(name, handler);
    return unlisten;
  },
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const { mailLinkEvent, watchMailLinks } = await import("./mailLinks");

const s = () => useStore.getState();

beforeEach(() => {
  resetStore([thread("t1")]);
  invoke.mockClear();
});

it("listens for the name Rust sends (src-tauri/src/mail_links.rs)", () => {
  expect(mailLinkEvent).toBe("moss://mail-link");
});

it("opens a link that started Moss, then each one that arrives later", async () => {
  waiting.push(["mailto:lena@northwind.co?subject=First"]);
  const stop = await watchMailLinks();
  expect(s().draft?.subject).toBe("First");

  waiting.push(["mailto:jonas@northwind.co?subject=Second"]);
  listeners.get(mailLinkEvent)!();
  await vi.waitFor(() => expect(s().draft?.subject).toBe("Second"));
  expect(s().draft!.to.map((p) => p.email)).toEqual(["jonas@northwind.co"]);

  stop();
  expect(unlisten).toHaveBeenCalledOnce();
});

it("listens before it looks, so a link that arrives in between isn't missed", async () => {
  invoke.mockImplementationOnce(async () => {
    expect(listeners.has(mailLinkEvent)).toBe(true);
    return [];
  });
  listeners.clear();
  await watchMailLinks();
  expect(invoke).toHaveBeenCalledWith("take_mail_links");
});
