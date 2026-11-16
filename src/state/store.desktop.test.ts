import { describe, expect, it, vi } from "vitest";

vi.mock("../lib/platform", async (actual) => ({ ...(await actual<typeof import("../lib/platform")>()), isTauri: true }));

describe("store in the desktop app", () => {
  it("starts with no mail and no accounts: the demo mailbox is never shown there, even if loading fails", async () => {
    // The test setup has loaded the store already, as the browser has it.
    vi.resetModules();
    const { useStore } = await import("./store");
    const s = useStore.getState();
    expect(s.accounts).toEqual([]);
    expect(s.threads).toEqual([]);
    expect(s.doneThreads).toEqual([]);
    expect(s.blockedThreads).toEqual([]);
    expect(s.cursorId).toBeUndefined();
  });
});
