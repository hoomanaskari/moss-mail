import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoSetup, setAccountSetup, type AccountSetup } from "../services/accountSetup";
import { fakeMail } from "../test/fakeMail";
import { resetStore, work } from "../test/fixtures";
import { resetAddAccount, useAddAccount } from "./addAccount";
import { useStore } from "./store";

const f = () => useAddAccount.getState();
const s = () => useStore.getState();

function fakeSetup(overrides: Partial<AccountSetup> = {}): AccountSetup {
  return {
    ...demoSetup,
    connectWithPassword: vi.fn(async () => ({ ok: true }) as const),
    signInWithBrowser: vi.fn(async () => ({ ok: true, address: "maya@outlook.com" }) as const),
    findServerSettings: vi.fn(async () => null),
    checkResendKey: vi.fn(async () => "This key can only send. Moss needs a Full access key."),
    ...overrides,
  };
}

beforeEach(() => {
  resetStore([], [work]);
  resetAddAccount();
  s().openAddAccount();
});
afterEach(() => setAccountSetup(demoSetup));

describe("choosing a provider", () => {
  it("starts on the list with the cursor on Gmail, and moves within it", () => {
    expect(f()).toMatchObject({ stage: "provider", cursor: 0 });
    f().moveCursor(-1);
    expect(f().cursor).toBe(0);
    for (let i = 0; i < 10; i++) f().moveCursor(1);
    expect(f().cursor).toBe(6);
  });

  it("continues with the provider under the cursor, or one picked by number", () => {
    f().moveCursor(1);
    f().pick();
    expect(f()).toMatchObject({ stage: "sign-in", provider: "outlook" });
    f().back();
    f().pick(5);
    expect(f().provider).toBe("resend");
  });

  it("Outlook starts the browser sign-in straight away", async () => {
    const setup = fakeSetup();
    setAccountSetup(setup);
    f().pick(1);
    expect(f().waiting).toBe(true);
    await vi.waitFor(() => expect(f().stage).toBe("sync"));
    expect(f().connected?.address).toBe("maya@outlook.com");
  });
});

describe("going back", () => {
  it("leaves the advanced way first, then goes back to the list, then closes", () => {
    f().pick(0);
    f().toggleAdvanced();
    expect(f().advanced).toBe(true);
    f().back();
    expect(f()).toMatchObject({ stage: "sign-in", advanced: false });
    f().back();
    expect(f().stage).toBe("provider");
    f().back();
    expect(s().view).toBe("inbox");
  });

  it("an error is cleared by going back", () => {
    useAddAccount.setState({ stage: "sign-in", provider: "icloud", error: "rejected" });
    f().back();
    expect(f()).toMatchObject({ stage: "provider", error: undefined });
  });
});

describe("setup steps", () => {
  it("opening a step’s page marks it done and moves on", () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    f().pick(0);
    f().openStep(0, "https://myaccount.google.com/signinoptions/twosv");
    expect(open).toHaveBeenCalledWith("https://myaccount.google.com/signinoptions/twosv", "_blank", "noopener,noreferrer");
    expect(f().stepsDone).toBe(1);
    f().openStep(0, "https://example.com");
    expect(f().stepsDone).toBe(1);
    open.mockRestore();
  });
});

describe("app passwords", () => {
  it("connects, makes the account and moves to Sync", async () => {
    const setup = fakeSetup();
    setAccountSetup(setup);
    f().pick(2);
    f().setField("email", "maya.ross@icloud.com");
    f().setField("password", "abcd-efgh-ijkl-mnop");
    await f().connect();
    expect(setup.connectWithPassword).toHaveBeenCalledWith({
      provider: "icloud",
      email: "maya.ross@icloud.com",
      password: "abcd-efgh-ijkl-mnop",
      servers: undefined,
    });
    expect(f()).toMatchObject({ stage: "sync", connected: { name: "iCloud", address: "maya.ross@icloud.com", color: "personal" } });
  });

  it("signs an iCloud custom domain in with the @icloud.com address, and waits for it", async () => {
    const setup = fakeSetup();
    setAccountSetup(setup);
    f().pick(2);
    f().setField("email", "info@fernway.co");
    f().setField("password", "abcd-efgh-ijkl-mnop");
    await f().connect();
    expect(setup.connectWithPassword).not.toHaveBeenCalled();
    f().setField("login", "maya.ross@icloud.com");
    await f().connect();
    expect(setup.connectWithPassword).toHaveBeenCalledWith({
      provider: "icloud",
      email: "info@fernway.co",
      password: "abcd-efgh-ijkl-mnop",
      login: "maya.ross@icloud.com",
      servers: undefined,
    });
    expect(f().connected).toMatchObject({ name: "iCloud", address: "info@fernway.co" });
  });

  it("an @icloud.com address is its own login, whatever was typed before", async () => {
    const setup = fakeSetup();
    setAccountSetup(setup);
    f().pick(2);
    f().setField("login", "old@icloud.com");
    f().setField("email", "maya.ross@icloud.com");
    f().setField("password", "abcd-efgh-ijkl-mnop");
    await f().connect();
    expect(vi.mocked(setup.connectWithPassword).mock.calls[0][0].login).toBeUndefined();
  });

  it("shows the error when the provider says no, and clears it once you type", async () => {
    setAccountSetup(fakeSetup({ connectWithPassword: async () => ({ ok: false, reason: "rejected" }) }));
    f().pick(0);
    f().setField("email", "maya.ross@gmail.com");
    f().setField("password", "hunter2");
    await f().connect();
    expect(f()).toMatchObject({ stage: "sign-in", error: "rejected", busy: false });
    f().setField("password", "hunter22");
    expect(f().error).toBeUndefined();
  });

  it("does nothing without an email and password, or while already connecting", async () => {
    const setup = fakeSetup();
    setAccountSetup(setup);
    f().pick(0);
    await f().connect();
    expect(setup.connectWithPassword).not.toHaveBeenCalled();
    f().setField("email", "a@b.c");
    f().setField("password", "x");
    useAddAccount.setState({ busy: true });
    await f().connect();
    expect(setup.connectWithPassword).not.toHaveBeenCalled();
  });
});

describe("browser sign-in", () => {
  it("Gmail’s own OAuth client signs in with the client ID and secret", async () => {
    const setup = fakeSetup({ signInWithBrowser: vi.fn(async () => ({ ok: true, address: "maya.ross@gmail.com" }) as const) });
    setAccountSetup(setup);
    f().pick(0);
    f().toggleAdvanced();
    f().setField("clientId", "123.apps.googleusercontent.com");
    f().setField("clientSecret", "GOCSPX-x");
    await f().connect();
    expect(setup.signInWithBrowser).toHaveBeenCalledWith({
      provider: "gmail",
      clientId: "123.apps.googleusercontent.com",
      clientSecret: "GOCSPX-x",
    });
    expect(f().connected).toMatchObject({ name: "Gmail", letter: "G" });
  });

  it("Outlook may need the admin’s approval", async () => {
    setAccountSetup(fakeSetup({ signInWithBrowser: async () => ({ ok: false, reason: "admin-approval" }) }));
    f().pick(1);
    await vi.waitFor(() => expect(f().error).toBe("admin-approval"));
    expect(f().waiting).toBe(false);
  });

  it("a sign-in that comes back after you left is ignored", async () => {
    let finish: (v: { ok: true; address: string }) => void = () => {};
    setAccountSetup(fakeSetup({ signInWithBrowser: () => new Promise((r) => (finish = r)) }));
    f().pick(1);
    f().back();
    finish({ ok: true, address: "late@outlook.com" });
    await Promise.resolve();
    expect(f().stage).toBe("provider");
  });
});

describe("other IMAP", () => {
  it("looks up the server settings once the email is typed", async () => {
    const settings = {
      incoming: { host: "posteo.de", port: "993", security: "ssl" as const },
      outgoing: { host: "posteo.de", port: "465", security: "ssl" as const },
    };
    setAccountSetup(fakeSetup({ findServerSettings: async (e) => (e.endsWith("posteo.de") ? settings : null) }));
    f().pick(6);
    await f().lookUpServers("maya@posteo.de");
    expect(f().imap).toEqual({ status: "found", servers: settings });
    await f().lookUpServers("maya@ross-studio.co");
    expect(f().imap.status).toBe("not-found");
    expect(f().imap.servers.incoming.host).toBe("");
    await f().lookUpServers("not an email");
    expect(f().imap.status).toBe("idle");
  });

  it("edits a server field and sends the servers with the password", async () => {
    const setup = fakeSetup();
    setAccountSetup(setup);
    f().pick(6);
    f().setServer("incoming", { host: "imap.ross-studio.co" });
    f().setField("email", "maya@ross-studio.co");
    f().setField("password", "pw");
    await f().connect();
    expect(vi.mocked(setup.connectWithPassword).mock.calls[0][0].servers?.incoming.host).toBe("imap.ross-studio.co");
    expect(f().connected?.name).toBe("Ross-studio");
  });
});

describe("Resend", () => {
  it("checks the key, starts with no domain chosen, and connects the ones you choose", async () => {
    setAccountSetup(fakeSetup({ checkResendKey: demoSetup.checkResendKey }));
    vi.useFakeTimers();
    f().pick(5);
    const checking = f().checkResendKey("re_abc123xyz");
    await vi.runAllTimersAsync();
    await checking;
    vi.useRealTimers();
    expect(f().resend.status).toBe("works");
    expect(f().resend.domains.map((d) => d.chosen)).toEqual([false, false, false]);
    f().toggleDomain(2);
    f().toggleDomain(1);
    expect(f().resend.domains.map((d) => d.chosen)).toEqual([false, true, true]);
    f().setDomainAddresses(1, "me");
    expect(f().resend.domains[1].only).toBe("me");
    f().moveDomainCursor(1);
    f().moveDomainCursor(1);
    f().moveDomainCursor(1);
    expect(f().resend.cursor).toBe(2);
    await f().connect();
    expect(f().connected).toMatchObject({ name: "Resend", address: "me@mayaross.dev" });
  });

  it("says when a key doesn’t work, and won’t connect without a domain", async () => {
    setAccountSetup(fakeSetup());
    f().pick(5);
    await f().checkResendKey("bad");
    expect(f().resend).toMatchObject({ status: "bad", problem: "This key can only send. Moss needs a Full access key." });
    await f().connect();
    expect(f().stage).toBe("sign-in");
  });
});

describe("after connecting", () => {
  beforeEach(() => {
    useAddAccount.setState({
      stage: "sync",
      provider: "gmail",
      connected: { id: "gmail", name: "Gmail", letter: "G", address: "maya.ross@gmail.com", color: "green" },
    });
  });

  it("changes the colour with ← →, wrapping around", () => {
    f().cycleColor(1);
    expect(f().connected?.color).toBe("violet");
    useAddAccount.setState({ connected: { ...f().connected!, color: "work" } });
    f().cycleColor(-1);
    expect(f().connected?.color).toBe("olive");
  });

  it("goes to the new account’s inbox", () => {
    f().finish();
    expect(s().accounts.at(-1)).toMatchObject({ id: "gmail", color: "green", connection: { provider: "gmail" } });
    expect(s()).toMatchObject({ view: "inbox", accountFilter: "gmail" });
    expect(f().stage).toBe("provider");
  });

  it("starts again for another account, keeping this one", () => {
    f().addAnother();
    expect(s().accounts.some((a) => a.id === "gmail")).toBe(true);
    expect(f()).toMatchObject({ stage: "provider", connected: undefined });
    expect(s().view).toBe("add-account");
  });

  it("does nothing to finish when nothing was connected", () => {
    useAddAccount.setState({ connected: undefined });
    f().cycleColor(1);
    f().finish();
    expect(s().accounts).toHaveLength(1);
  });
});

describe("how Settings describes the new account", () => {
  const connected = { id: "x", name: "X", letter: "X", address: "x@y.z", color: "green" as const };
  const domain = { name: "y.z", detail: "", addresses: [], active: true, only: null };
  it.each<[Partial<ReturnType<typeof f>>, string]>([
    [{ provider: "gmail", advanced: false }, "Gmail · App password"],
    [{ provider: "gmail", advanced: true }, "Gmail · Own OAuth client"],
    [{ provider: "outlook", advanced: false }, "Microsoft 365"],
    [{ provider: "outlook", advanced: true }, "Microsoft 365 · Own client ID"],
    [{ provider: "fastmail" }, "Fastmail · App password"],
    [{ provider: "resend", resend: { key: "", status: "works", cursor: 0, domains: [{ ...domain, chosen: true }] } }, "Resend · 1 domain"],
    [
      { provider: "resend", resend: { key: "", status: "works", cursor: 0, domains: [{ ...domain, chosen: true }, { ...domain, chosen: true }] } },
      "Resend · 2 domains",
    ],
    [
      {
        provider: "imap",
        imap: { status: "found", servers: { incoming: { host: "posteo.de", port: "993", security: "ssl" }, outgoing: { host: "posteo.de", port: "465", security: "ssl" } } },
      },
      "IMAP · posteo.de",
    ],
  ])("%o → %s", (state, text) => {
    useAddAccount.setState({ ...state, stage: "sync", connected });
    f().finish();
    expect(s().accounts.at(-1)?.connection?.summary).toBe(text);
  });

  it("numbers a second account with the same name, and reuses colours once all eight are taken", () => {
    const colors = ["work", "personal", "studio", "green", "violet", "sky", "sand", "olive"] as const;
    resetStore([], colors.map((color, i) => ({ ...work, id: i ? `a${i}` : "gmail", color })));
    s().openAddAccount();
    useAddAccount.setState({ provider: "gmail", password: "abcdefghijklmnop", email: "b@gmail.com" });
    return f()
      .connect()
      .then(() => expect(f().connected).toMatchObject({ id: "gmail-2", color: "work" }));
  });
});

describe("first launch", () => {
  it("with no account yet, Esc on the provider list keeps the flow open", () => {
    resetStore([], []);
    s().openAddAccount();
    f().back();
    expect(s().view).toBe("add-account");
  });
});

describe("with the mail engine", () => {
  const saved = { ...work, id: "gmail", name: "Gmail", letter: "G", address: "maya@gmail.com", senderName: "Maya R.", color: "personal" as const, connection: { provider: "gmail" as const, summary: "Gmail · App password" } };

  it("uses the account the engine saved, and saves a new colour back", async () => {
    setAccountSetup(fakeSetup({ connectWithPassword: vi.fn(async () => ({ ok: true, account: saved }) as const) }));
    const { backend } = fakeMail();
    f().pick(0);
    f().setField("email", "maya@gmail.com");
    f().setField("password", "abcdefghijklmnop");
    await f().connect();
    expect(f().connected).toEqual({ id: "gmail", name: "Gmail", letter: "G", address: "maya@gmail.com", color: "personal" });
    f().cycleColor(1);
    f().finish();
    expect(s().accounts.at(-1)).toMatchObject({ id: "gmail", senderName: "Maya R.", color: "studio", connection: saved.connection });
    expect(backend.updateAccount).toHaveBeenCalledWith("gmail", { color: "studio" });
  });

  it("keeps the engine's colour when it wasn't changed", async () => {
    setAccountSetup(fakeSetup({ connectWithPassword: vi.fn(async () => ({ ok: true, account: saved }) as const) }));
    const { backend } = fakeMail();
    f().pick(0);
    f().setField("email", "maya@gmail.com");
    f().setField("password", "abcdefghijklmnop");
    await f().connect();
    f().addAnother();
    expect(backend.updateAccount).not.toHaveBeenCalled();
  });

  it("says why when it couldn't connect for another reason than the password", async () => {
    const message = "Moss couldn’t reach imap.gmail.com. Check your internet connection.";
    setAccountSetup(fakeSetup({ connectWithPassword: vi.fn(async () => ({ ok: false, reason: "failed", message }) as const) }));
    f().pick(0);
    f().setField("email", "maya@gmail.com");
    f().setField("password", "abcdefghijklmnop");
    await f().connect();
    expect(f()).toMatchObject({ stage: "sign-in", error: "failed", errorMessage: message, busy: false });
    f().setField("password", "x");
    expect(f().errorMessage).toBeUndefined();
  });

  it("a browser sign-in that went wrong says why", async () => {
    const message = "The sign-in page wasn’t finished in time. Try again.";
    setAccountSetup(fakeSetup({ signInWithBrowser: vi.fn(async () => ({ ok: false, reason: "failed", message }) as const) }));
    f().pick(1);
    await vi.waitFor(() => expect(f().error).toBe("failed"));
    expect(f().errorMessage).toBe(message);
  });

  it("Outlook in the browser keeps the account the engine saved", async () => {
    const outlook = { ...saved, id: "outlook", name: "Outlook", letter: "O", address: "maya@outlook.com", connection: { provider: "outlook" as const, summary: "Microsoft 365 · Signed in with browser" } };
    setAccountSetup(fakeSetup({ signInWithBrowser: vi.fn(async () => ({ ok: true, address: outlook.address, account: outlook }) as const) }));
    fakeMail();
    f().pick(1);
    await vi.waitFor(() => expect(f().stage).toBe("sync"));
    expect(f().connected).toMatchObject({ id: "outlook", address: "maya@outlook.com" });
    f().finish();
    expect(s().accounts.at(-1)).toMatchObject({ id: "outlook", connection: outlook.connection });
  });

  it("connects a Resend account through the engine, or says why not", async () => {
    const connectResend = vi.fn(async () => ({ ok: false, reason: "failed", message: "Domain missing" }) as const);
    setAccountSetup(fakeSetup({ checkResendKey: demoSetup.checkResendKey, connectResend }));
    vi.useFakeTimers();
    f().pick(5);
    const checking = f().checkResendKey("re_abc123xyz");
    await vi.runAllTimersAsync();
    await checking;
    vi.useRealTimers();
    f().toggleDomain(0);
    await f().connect();
    expect(connectResend).toHaveBeenCalledWith({ key: "re_abc123xyz", address: "hello@rossstudio.art" });
    expect(f()).toMatchObject({ stage: "sign-in", error: "failed", errorMessage: "Domain missing" });
    connectResend.mockResolvedValueOnce({ ok: true, account: { ...saved, id: "rossstudio", address: "hello@rossstudio.art" } } as never);
    await f().connect();
    expect(f().connected?.id).toBe("rossstudio");
  });

  it("a Resend answer that comes back after you left is ignored", async () => {
    let answer: (r: { ok: true }) => void = () => {};
    const connectResend = vi.fn(() => new Promise<{ ok: true }>((r) => (answer = r)));
    setAccountSetup(fakeSetup({ checkResendKey: demoSetup.checkResendKey, connectResend }));
    vi.useFakeTimers();
    f().pick(5);
    const checking = f().checkResendKey("re_abc123xyz");
    await vi.runAllTimersAsync();
    await checking;
    vi.useRealTimers();
    f().toggleDomain(0);
    const connecting = f().connect();
    f().back();
    answer({ ok: true });
    await connecting;
    expect(f().stage).toBe("provider");
  });
});

describe("edge cases", () => {
  it("an own client without a secret sends none, and nothing happens without a client ID", async () => {
    const signInWithBrowser = vi.fn(async () => ({ ok: true, address: "maya@gmail.com" }) as const);
    setAccountSetup(fakeSetup({ signInWithBrowser }));
    f().pick(0);
    f().toggleAdvanced();
    await f().connect();
    expect(signInWithBrowser).not.toHaveBeenCalled();
    f().setField("clientId", "123.apps.googleusercontent.com");
    await f().connect();
    expect(signInWithBrowser).toHaveBeenCalledWith({ provider: "gmail", clientId: "123.apps.googleusercontent.com", clientSecret: undefined });
  });

  it("a password answer that comes back after you left is ignored", async () => {
    let answer: (r: { ok: true }) => void = () => {};
    setAccountSetup(fakeSetup({ connectWithPassword: vi.fn(() => new Promise<{ ok: true }>((r) => (answer = r))) }));
    f().pick(0);
    f().setField("email", "maya@gmail.com");
    f().setField("password", "abcdefghijklmnop");
    const connecting = f().connect();
    f().back();
    answer({ ok: true });
    await connecting;
    expect(f().stage).toBe("provider");
  });

  it("a Resend domain with no addresses seen sends as hello@, and a first account has no sender name yet", async () => {
    resetStore([], []);
    s().openAddAccount();
    setAccountSetup(fakeSetup({ checkResendKey: vi.fn(async () => [{ name: "x.io", detail: "Verified", addresses: [], active: true }]) }));
    f().pick(5);
    await f().checkResendKey("re_1234567");
    f().toggleDomain(0);
    await f().connect();
    expect(f().connected?.address).toBe("hello@x.io");
    f().finish();
    expect(s().accounts[0].senderName).toBe("");
  });

  it("an address without a domain still makes an account", async () => {
    setAccountSetup(fakeSetup());
    f().pick(6);
    f().setField("email", "maya");
    f().setField("password", "pw");
    await f().connect();
    expect(f().connected?.name).toBe("");
  });
});
