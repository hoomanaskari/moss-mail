import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
const openLink = vi.fn();
vi.mock("../lib/links", () => ({ openLink: (url: string) => openLink(url) }));

const { setupFailure, tauriSetup } = await import("./tauriSetup");

const account = { id: "gmail", name: "Gmail", letter: "G", address: "maya@gmail.com", senderName: "Maya", color: "work", provider: "gmail", connection: { provider: "gmail", summary: "Gmail · App password" } };

beforeEach(() => {
  invoke.mockReset();
  openLink.mockReset();
});

describe("signing in from the desktop app", () => {
  it("an app password signs in through the engine and returns the saved account", async () => {
    invoke.mockResolvedValueOnce(account);
    const result = await tauriSetup.connectWithPassword({ provider: "gmail", email: " maya@gmail.com ", password: "abcd efgh ijkl mnop" });
    expect(result).toMatchObject({ ok: true, account: { id: "gmail", connection: account.connection } });
    expect(invoke).toHaveBeenCalledWith("add_password_account", {
      request: { service: "gmail", email: "maya@gmail.com", password: "abcd efgh ijkl mnop", servers: null },
    });
  });

  it("an iCloud custom domain sends the @icloud.com address to log in with", async () => {
    invoke.mockResolvedValue(account);
    await tauriSetup.connectWithPassword({ provider: "icloud", email: "info@fernway.co", password: "pw", login: " maya@icloud.com " });
    expect(invoke.mock.lastCall?.[1].request).toMatchObject({ email: "info@fernway.co", login: "maya@icloud.com" });
    await tauriSetup.connectWithPassword({ provider: "icloud", email: "maya@icloud.com", password: "pw", login: " " });
    expect(invoke.mock.lastCall?.[1].request).not.toHaveProperty("login");
  });

  it("other IMAP sends the server settings", async () => {
    invoke.mockResolvedValueOnce(account);
    await tauriSetup.connectWithPassword({
      provider: "imap",
      email: "maya@posteo.de",
      password: "pw",
      servers: { incoming: { host: "posteo.de", port: "993", security: "ssl" }, outgoing: { host: "posteo.de", port: "465", security: "ssl" } },
    });
    expect(invoke.mock.lastCall?.[1].request.servers.imap).toEqual({ host: "posteo.de", port: 993, security: "tls", username: "maya@posteo.de" });
  });

  it("a refused password shows the wrong-password screen; other problems say what happened", async () => {
    invoke.mockRejectedValueOnce({ kind: "wrongPassword", service: "gmail" });
    expect(await tauriSetup.connectWithPassword({ provider: "gmail", email: "a@gmail.com", password: "x" })).toEqual({ ok: false, reason: "rejected" });
    invoke.mockRejectedValueOnce({ kind: "unreachable", host: "imap.gmail.com", detail: "dns" });
    expect(await tauriSetup.connectWithPassword({ provider: "gmail", email: "a@gmail.com", password: "x" })).toEqual({
      ok: false,
      reason: "failed",
      message: "Moss couldn’t reach imap.gmail.com. Check your internet connection and try again.",
    });
  });

  describe("in the browser", () => {
    const page = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=c1";
    const outlook = { ...account, id: "outlook", address: "maya@outlook.com", connection: { provider: "outlook", summary: "Microsoft 365 · Signed in with browser" } };

    it("opens the sign-in page, waits, and returns the account that signed in", async () => {
      invoke.mockResolvedValueOnce({ url: page, clientId: "c1" }).mockResolvedValueOnce(outlook);
      const result = await tauriSetup.signInWithBrowser({ provider: "outlook" });
      expect(invoke).toHaveBeenNthCalledWith(1, "start_browser_sign_in", { request: { provider: "outlook", clientId: null, clientSecret: null } });
      expect(openLink).toHaveBeenCalledWith(page);
      expect(invoke).toHaveBeenNthCalledWith(2, "finish_browser_sign_in");
      expect(result).toMatchObject({ ok: true, address: "maya@outlook.com", account: { id: "outlook", address: "maya@outlook.com" } });
      // "Open again", "Copy link" and the page an IT admin approves Moss on.
      expect(tauriSetup.signInLink()).toBe(page);
      expect(tauriSetup.adminApprovalLink()).toBe("https://login.microsoftonline.com/common/adminconsent?client_id=c1");
    });

    it("sends your own client along", async () => {
      invoke.mockResolvedValueOnce({ url: page, clientId: "g-id" }).mockResolvedValueOnce(account);
      await tauriSetup.signInWithBrowser({ provider: "gmail", clientId: " g-id ", clientSecret: "secret" });
      expect(invoke).toHaveBeenNthCalledWith(1, "start_browser_sign_in", { request: { provider: "gmail", clientId: "g-id", clientSecret: "secret" } });
    });

    it("says what went wrong, and opens nothing when it couldn't start", async () => {
      invoke.mockRejectedValueOnce({ kind: "invalidOAuthClient", detail: "the client ID is empty" });
      expect(await tauriSetup.signInWithBrowser({ provider: "gmail", clientId: " " })).toEqual({
        ok: false,
        reason: "failed",
        message: "The sign-in page didn’t accept this client. Check the client ID and try again.",
      });
      expect(openLink).not.toHaveBeenCalled();

      const fails = async (error: unknown) => {
        invoke.mockResolvedValueOnce({ url: page, clientId: "c1" }).mockRejectedValueOnce(error);
        return tauriSetup.signInWithBrowser({ provider: "outlook" });
      };
      expect(await fails({ kind: "adminApprovalRequired", service: "outlook", detail: "AADSTS65001" })).toEqual({ ok: false, reason: "admin-approval" });
      expect(await fails({ kind: "accessDenied", detail: "user said no" })).toMatchObject({
        message: "The sign-in was cancelled, so nothing was added. Try again when you’re ready.",
      });
      expect(await fails({ kind: "timeout" })).toMatchObject({ message: "The sign-in page wasn’t finished in time. Try again." });
      expect(await fails({ kind: "unreachable", host: "login.microsoftonline.com", detail: "dns" })).toMatchObject({
        message: "Moss couldn’t reach login.microsoftonline.com. Check your internet connection and try again.",
      });
    });
  });

  it("finds server settings, or nothing", async () => {
    invoke.mockResolvedValueOnce({
      servers: { imap: { host: "posteo.de", port: 993, security: "tls", username: "x" }, smtp: { host: "posteo.de", port: 587, security: "startTls", username: "x" } },
      service: "imap",
      source: "ispdb",
    });
    expect(await tauriSetup.findServerSettings("maya@posteo.de")).toEqual({
      incoming: { host: "posteo.de", port: "993", security: "ssl" },
      outgoing: { host: "posteo.de", port: "587", security: "starttls" },
    });
    expect(invoke).toHaveBeenLastCalledWith("discover", { email: "maya@posteo.de" });
    invoke.mockRejectedValueOnce({ kind: "settingsNotFound", domain: "x.io" });
    expect(await tauriSetup.findServerSettings("a@x.io")).toBeNull();
  });

  it("checks a Resend key and connects one of its domains", async () => {
    invoke.mockResolvedValueOnce([
      { name: "rossstudio.art", status: "verified" },
      { name: "old.net", status: "pending" },
    ]);
    expect(await tauriSetup.checkResendKey("re_1")).toEqual([
      { name: "rossstudio.art", detail: "Verified", addresses: [], active: true },
      { name: "old.net", detail: "Not verified yet (pending)", addresses: [], active: false },
    ]);
    invoke.mockRejectedValueOnce({ kind: "invalidResendKey" });
    expect(await tauriSetup.checkResendKey("bad")).toBe("That key didn’t work. Check it at resend.com/api-keys.");
    invoke.mockRejectedValueOnce({ kind: "resendKeyRestricted" });
    expect(await tauriSetup.checkResendKey("re_send_only")).toBe(
      "This key only sends, or only covers some domains. Moss needs a Full access key: make one at resend.com/api-keys.",
    );
    invoke.mockRejectedValueOnce({ kind: "unreachable", host: "api.resend.com", detail: "dns" });
    expect(await tauriSetup.checkResendKey("re_1")).toBe("Moss couldn’t reach api.resend.com. Check your internet connection and try again.");
    invoke.mockResolvedValueOnce(account);
    expect(await tauriSetup.connectResend({ key: "re_1", address: "hello@rossstudio.art" })).toMatchObject({ ok: true });
    expect(invoke).toHaveBeenLastCalledWith("add_resend_account", { key: "re_1", email: "hello@rossstudio.art" });
  });

  it("progress starts at nothing until sync reports", () => {
    expect(tauriSetup.syncProgress("a@b.co")).toMatchObject({ done: 0, total: 0 });
  });
});

describe("sign-in problems in words", () => {
  const message = (e: unknown) => {
    const f = setupFailure(e);
    return f.reason === "failed" ? f.message : f.reason;
  };

  it("has a sentence for each kind", () => {
    expect(message({ kind: "gmailAppPassword", problem: "notAnAppPassword" })).toBe("rejected");
    expect(message({ kind: "invalidEmail" })).toMatch(/email address/);
    expect(message({ kind: "timeout" })).toMatch(/couldn’t reach the mail server/);
    expect(message({ kind: "settingsNotFound", domain: "x.io" })).toMatch(/x\.io/);
    expect(message({ kind: "keychain", detail: "locked" })).toMatch(/Keychain/);
    expect(message({ kind: "iCloudAddressNeeded" })).toMatch(/@icloud\.com address/);
    expect(message({ kind: "resendKeyRestricted" })).toMatch(/Full access/);
    expect(message({ kind: "invalidResendKey" })).toMatch(/Full access/);
    expect(message({ kind: "resendDomainMissing", domain: "x.io" })).toMatch(/x\.io isn’t in this Resend account/);
    expect(message({ kind: "server", message: "the server refused: NO" })).toBe("the server refused: NO");
    expect(message(undefined)).toMatch(/Try again in a moment/);
  });
});
