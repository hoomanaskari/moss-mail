import { afterEach, describe, expect, it, vi } from "vitest";
import { accountSetup, demoSetup, setAccountSetup } from "./accountSetup";

afterEach(() => vi.useRealTimers());

async function settle<T>(p: Promise<T>) {
  await vi.runAllTimersAsync();
  return p;
}

describe("demo account setup", () => {
  it("accepts a 16-letter app password, spaces and dashes allowed, and any IMAP password", async () => {
    vi.useFakeTimers();
    const ok = (provider: "icloud" | "imap", password: string) => settle(demoSetup.connectWithPassword({ provider, email: "a@b.c", password }));
    expect(await ok("icloud", "abcd-efgh-ijkl-mnop")).toEqual({ ok: true });
    expect(await ok("icloud", "hunter2")).toEqual({ ok: false, reason: "rejected" });
    expect(await ok("imap", "anything")).toEqual({ ok: true });
    expect(await ok("imap", "")).toEqual({ ok: false, reason: "rejected" });
  });

  it("signs in through the browser", async () => {
    vi.useFakeTimers();
    expect(await settle(demoSetup.signInWithBrowser({ provider: "outlook" }))).toEqual({ ok: true, address: "maya.ross@outlook.com" });
    expect(await settle(demoSetup.signInWithBrowser({ provider: "gmail" }))).toEqual({ ok: true, address: "maya.ross@gmail.com" });
  });

  it("finds server settings for known domains only", async () => {
    vi.useFakeTimers();
    expect((await settle(demoSetup.findServerSettings("maya@posteo.de")))?.incoming.port).toBe("993");
    expect(await settle(demoSetup.findServerSettings("maya@ross-studio.co"))).toBeNull();
    expect(await settle(demoSetup.findServerSettings("no-at-sign"))).toBeNull();
  });

  it("lists a Resend key’s domains, or nothing for a bad key", async () => {
    vi.useFakeTimers();
    expect(await settle(demoSetup.checkResendKey("re_abc123xyz"))).toHaveLength(3);
    expect(await settle(demoSetup.checkResendKey("nope"))).toBe("That key didn’t work. Check it at resend.com/api-keys.");
  });

  it("reports sync progress and links", () => {
    expect(demoSetup.syncProgress("x").total).toBe(1930);
    expect(demoSetup.signInLink()).toMatch(/^https:/);
    expect(demoSetup.adminApprovalLink()).toMatch(/adminconsent/);
  });

  it("can be swapped for another backend", () => {
    const other = { ...demoSetup };
    setAccountSetup(other);
    expect(accountSetup()).toBe(other);
    setAccountSetup(demoSetup);
    expect(accountSetup()).toBe(demoSetup);
  });
});
