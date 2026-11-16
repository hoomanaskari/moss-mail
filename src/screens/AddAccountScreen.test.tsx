import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoDomains, demoSetup, setAccountSetup, type AccountSetup } from "../services/accountSetup";
import { demoAi, setAi } from "../services/ai";
import { useSync } from "../services/live";
import { resetAddAccount, useAddAccount } from "../state/addAccount";
import { useAiSettings } from "../state/aiSettings";
import { useStore } from "../state/store";
import { resetDemoStore } from "../test/fixtures";
import { digitFroms, pressDigit } from "../test/keys";
import { AddAccountScreen } from "./AddAccountScreen";

const f = () => useAddAccount.getState();
const s = () => useStore.getState();
const footer = () => screen.getByRole("contentinfo");

let setup: AccountSetup;
let open: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  resetDemoStore();
  resetAddAccount();
  s().openAddAccount();
  setup = {
    ...demoSetup,
    connectWithPassword: vi.fn(async () => ({ ok: true }) as const),
    signInWithBrowser: vi.fn(() => new Promise<never>(() => {})),
    findServerSettings: vi.fn(async () => null),
    checkResendKey: vi.fn(async (key: string) => (/^re_\w{6,}/.test(key) ? demoDomains : "That key didn’t work. Check it at resend.com/api-keys.")),
  };
  setAccountSetup(setup);
  open = vi.spyOn(window, "open").mockImplementation(() => null);
});
afterEach(() => {
  setAccountSetup(demoSetup);
  open.mockRestore();
});

/** Puts the flow in a state, as the dev scenes do. */
const at = (patch: Partial<ReturnType<typeof f>>) => act(() => useAddAccount.setState({ stage: "sign-in", ...patch }));

describe("choosing a provider", () => {
  it("lists every provider and picks one by number", async () => {
    render(<AddAccountScreen />);
    expect(screen.getByRole("heading", { name: "Which email do you want to add?" })).toBeInTheDocument();
    expect(screen.getAllByRole("option")).toHaveLength(7);
    expect(footer()).toHaveTextContent("1 – 7Pick");
    await userEvent.setup().keyboard("3");
    expect(screen.getByRole("heading", { name: "Connect iCloud Mail" })).toBeInTheDocument();
  });

  it.each(digitFroms)("picks by number from the %s keys", (from) => {
    render(<AddAccountScreen />);
    expect(pressDigit(3, from)).toBe(false);
    expect(screen.getByRole("heading", { name: "Connect iCloud Mail" })).toBeInTheDocument();
  });

  it("moves with the arrows (J and K do nothing), continues with ↵, and ignores other keys", async () => {
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    await user.keyboard("jJ");
    expect(f()).toMatchObject({ stage: "provider", cursor: 0 });
    await user.keyboard("{ArrowDown}{ArrowDown}kK{ArrowUp}x{Meta>}2{/Meta}");
    expect(f()).toMatchObject({ stage: "provider", cursor: 1 });
    expect(footer()).not.toHaveTextContent(/Move|J K/);
    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "Sign in with Microsoft" })).toBeInTheDocument();
  });

  it("hovering a row leaves the cursor alone, a click picks one, and Esc or the top bar cancels", async () => {
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    await user.hover(screen.getAllByRole("option")[4]);
    expect(f().cursor).toBe(0);
    expect(screen.getAllByRole("option")[0]).toHaveAttribute("aria-selected", "true");
    await user.click(screen.getAllByRole("option")[5]);
    expect(f().provider).toBe("resend");
    await user.click(screen.getByRole("button", { name: /Back/ }));
    expect(f().stage).toBe("provider");
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
  });
});

describe("app passwords", () => {
  it("follows the steps, fills the fields and connects", async () => {
    at({ provider: "gmail" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    expect(footer()).toHaveTextContent(/Ctrl\+1Open Google’s page/);
    await user.keyboard("{Meta>}1{/Meta}");
    expect(open).toHaveBeenCalledWith(expect.stringContaining("twosv"), "_blank", "noopener,noreferrer");
    expect(footer()).toHaveTextContent(/Ctrl\+2Open Google’s page/);
    await user.click(screen.getByRole("button", { name: /Open.*2/ }));
    expect(f().stepsDone).toBe(2);
    await user.type(screen.getByLabelText("Gmail address"), "maya.ross@gmail.com");
    await user.type(screen.getByLabelText("App password"), "abcdefghijklmnop");
    await user.click(screen.getByRole("button", { name: "Show" }));
    expect(screen.getByLabelText("App password")).toHaveAttribute("type", "text");
    await user.click(screen.getByRole("button", { name: "Hide" }));
    await user.click(screen.getByLabelText("App password"));
    await user.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByRole("heading", { name: "Gmail is connected" })).toBeInTheDocument());
  });

  it.each(digitFroms)("⌘1 and ⌘2 open each step's page from the %s keys", (from) => {
    at({ provider: "gmail" });
    render(<AddAccountScreen />);
    expect(pressDigit(1, from, { meta: true })).toBe(false);
    expect(open).toHaveBeenCalledWith(expect.stringContaining("twosv"), "_blank", "noopener,noreferrer");
    pressDigit(2, from, { ctrl: true });
    expect(open).toHaveBeenCalledTimes(2);
  });

  it("explains a rejected password and how to fix it", async () => {
    at({ provider: "icloud", error: "rejected" });
    render(<AddAccountScreen />);
    expect(screen.getByRole("heading", { name: "That password didn’t work" })).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Use an app-specific password");
    expect(screen.getByLabelText("App-specific password")).toHaveAttribute("aria-invalid", "true");
    expect(footer()).toHaveTextContent(/Ctrl\+OOpen Apple Account.*Try again/);
    const user = userEvent.setup();
    await user.keyboard("{Meta>}o{/Meta}");
    expect(open).toHaveBeenCalledWith(expect.stringContaining("account.apple.com"), "_blank", "noopener,noreferrer");
    await user.click(screen.getByRole("button", { name: /Open/ }));
    expect(open).toHaveBeenCalledTimes(2);
    // ⌘1 only means something when there are several pages.
    await user.keyboard("{Meta>}1{/Meta}");
    expect(open).toHaveBeenCalledTimes(2);
  });

  it("Gmail’s error points to the OAuth client when app passwords are off, and tries again", async () => {
    at({ provider: "gmail", error: "rejected", email: "maya.ross@gmail.com", password: "x" });
    render(<AddAccountScreen />);
    expect(screen.getByText(/Your admin turned them off/)).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again↵" }));
    expect(setup.connectWithPassword).toHaveBeenCalled();
  });

  it.each([
    ["yahoo", "Connect Yahoo Mail", "Open Yahoo Account"],
    ["fastmail", "Connect Fastmail", "Open Fastmail settings"],
  ] as const)("%s has its own page and steps", (provider, title, open) => {
    at({ provider });
    render(<AddAccountScreen />);
    expect(screen.getByRole("heading", { name: title })).toBeInTheDocument();
    expect(footer()).toHaveTextContent(open);
    expect(footer()).not.toHaveTextContent("Advanced");
  });

  it("asks a custom domain on iCloud for the @icloud.com address", async () => {
    at({ provider: "icloud" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    const email = screen.getByLabelText("iCloud email");
    await user.type(email, "maya@icloud.com");
    expect(screen.queryByLabelText("Your @icloud.com address")).not.toBeInTheDocument();
    await user.clear(email);
    await user.type(email, "info@fernway.co");
    const login = screen.getByLabelText("Your @icloud.com address");
    expect(screen.getByText("iCloud signs custom domains in with it")).toBeInTheDocument();
    await user.type(login, "maya.ross@icloud.com");
    expect(f().login).toBe("maya.ross@icloud.com");
  });

  it("⌘V pastes into the password when you are not in a field", async () => {
    at({ provider: "icloud" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    await navigator.clipboard.writeText("pasted-text");
    await user.keyboard("{Meta>}v{/Meta}");
    await waitFor(() => expect(f().password).toBe("pasted-text"));
    expect(screen.getByLabelText("App-specific password")).toHaveFocus();
  });

  it("↵ on a focused button presses that button, not Connect", async () => {
    at({ provider: "icloud" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    screen.getByRole("button", { name: /Open/ }).focus();
    await user.keyboard("{Enter}");
    expect(open).toHaveBeenCalled();
    expect(setup.connectWithPassword).not.toHaveBeenCalled();
  });
});

describe("own client (advanced)", () => {
  it("⌥A switches Gmail to your own OAuth client and back", async () => {
    at({ provider: "gmail" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    await user.keyboard("{Alt>}a{/Alt}");
    expect(screen.getByRole("heading", { name: "Use your own Google OAuth client" })).toBeInTheDocument();
    expect(footer()).toHaveTextContent("App password");
    await user.keyboard("{Meta>}3{/Meta}{Meta>}5{/Meta}");
    expect(open).toHaveBeenCalledTimes(1);
    await user.type(screen.getByLabelText("Client ID"), "1.apps");
    await user.type(screen.getByLabelText("Client secret"), "GOCSPX");
    await user.click(screen.getByRole("button", { name: /Sign in with Google/ }));
    expect(setup.signInWithBrowser).toHaveBeenCalledWith({ provider: "gmail", clientId: "1.apps", clientSecret: "GOCSPX" });
    await user.click(screen.getByRole("button", { name: /Use an app password instead/ }));
    expect(screen.getByRole("heading", { name: "Connect Gmail with an app password" })).toBeInTheDocument();
  });

  it("Outlook’s own client ID has one field, and ⌘V fills it", async () => {
    at({ provider: "outlook", advanced: true });
    render(<AddAccountScreen />);
    expect(screen.getByRole("heading", { name: "Use your own Microsoft client ID" })).toBeInTheDocument();
    expect(footer()).toHaveTextContent("Built-in sign-in");
    const user = userEvent.setup();
    await navigator.clipboard.writeText("pasted-text");
    await user.keyboard("{Meta>}v{/Meta}");
    await waitFor(() => expect(f().clientId).toBe("pasted-text"));
    await user.type(screen.getByLabelText("Application (client) ID"), "!");
    expect(f().clientId).toBe("pasted-text!");
    // No page for step 2: ⌘2 does nothing.
    await user.keyboard("{Meta>}2{/Meta}");
    expect(open).not.toHaveBeenCalled();
  });
});

describe("Outlook", () => {
  it("waits for the browser, opens it again and copies the link", async () => {
    at({ provider: "outlook", waiting: true });
    render(<AddAccountScreen />);
    expect(screen.getByRole("status")).toHaveTextContent("Waiting for you in the browser");
    const user = userEvent.setup();
    await user.keyboard("{Meta>}o{/Meta}");
    await user.click(screen.getByRole("button", { name: /Open again/ }));
    expect(open).toHaveBeenCalledTimes(2);
    await user.keyboard("{Meta>}c{/Meta}");
    expect(await navigator.clipboard.readText()).toBe(demoSetup.signInLink());
    await user.click(screen.getByRole("button", { name: /Use your own Microsoft client ID/ }));
    expect(f().advanced).toBe(true);
  });

  it("asks for the admin’s approval and lets you copy the link and try again", async () => {
    at({ provider: "outlook", error: "admin-approval" });
    render(<AddAccountScreen />);
    expect(screen.getByRole("heading", { name: "Your organisation needs to approve Moss" })).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Copy link/ }));
    expect(await navigator.clipboard.readText()).toBe(demoSetup.adminApprovalLink());
    await navigator.clipboard.writeText("");
    await user.keyboard("{Meta>}c{/Meta}");
    expect(await navigator.clipboard.readText()).toBe(demoSetup.adminApprovalLink());
    await user.click(screen.getByRole("button", { name: "Try again↵" }));
    expect(setup.signInWithBrowser).toHaveBeenCalledWith({ provider: "outlook" });
  });
});

describe("Resend", () => {
  it("checks a pasted key, then picks domains with the keyboard", async () => {
    at({ provider: "resend" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    expect(screen.getByRole("button", { name: "Connect↵" })).toBeDisabled();
    const key = screen.getByLabelText("API key");
    await user.type(key, "re_abc123xyz");
    await waitFor(() => expect(screen.getByText("Key works")).toBeInTheDocument());
    expect(screen.getByText("Found 3 domains on this key.")).toBeInTheDocument();
    await user.tab();
    expect(key).toHaveValue("re_••••••xyz");
    (document.activeElement as HTMLElement).blur();
    expect(footer()).not.toHaveTextContent(/Move|J K/);
    await user.keyboard("jJ{ArrowDown} {ArrowDown}kK{ArrowUp}{ArrowUp} ");
    expect(f().resend.domains.map((d) => d.chosen)).toEqual([true, true, false]);
    expect(f().resend.cursor).toBe(0);
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(screen.getByRole("combobox", { name: "Addresses for rossstudio.art" })).toHaveFocus();
    (document.activeElement as HTMLElement).blur();
    await user.keyboard("x");
    await user.click(screen.getByRole("checkbox", { name: "old-portfolio.net" }));
    await user.hover(screen.getByText("mayaross.dev"));
    expect(f().resend.cursor).toBe(2);
    await user.click(screen.getByText("mayaross.dev"));
    expect(f().resend.cursor).toBe(1);
    await user.selectOptions(screen.getByRole("combobox", { name: "Addresses for rossstudio.art" }), "maya");
    expect(screen.getByText("3 of 3 chosen")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Connect 3 domains/ }));
    expect(f().connected?.address).toBe("maya@rossstudio.art");
  });

  it("⌘V pastes a key and checks it", async () => {
    at({ provider: "resend" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    await navigator.clipboard.writeText("re_abc123xyz");
    await user.keyboard("{Meta>}v{/Meta}");
    await waitFor(() => expect(f().resend.status).toBe("works"));
  });

  it("says which key to make before you paste one", () => {
    at({ provider: "resend" });
    render(<AddAccountScreen />);
    expect(screen.getByText("Paste an API key with Full access, then pick which domains Moss should receive mail for.")).toBeInTheDocument();
    expect(screen.getByText("Full access, all domains · resend.com/api-keys")).toBeInTheDocument();
  });

  it("says when a key doesn’t work, and short keys show as typed", async () => {
    at({ provider: "resend" });
    render(<AddAccountScreen />);
    await userEvent.setup().type(screen.getByLabelText("API key"), "nope");
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("That key didn’t work"));
    (document.activeElement as HTMLElement).blur();
    expect(screen.getByLabelText("API key")).toHaveValue("nope");
  });

  it("names a single domain in the singular", async () => {
    at({ provider: "resend" });
    render(<AddAccountScreen />);
    await act(() => f().checkResendKey("re_abc123xyz"));
    act(() => f().toggleDomain(1));
    expect(screen.getByRole("button", { name: /Connect 1 domain↵/ })).toBeInTheDocument();
    act(() => useAddAccount.setState((x) => ({ resend: { ...x.resend, domains: x.resend.domains.slice(0, 1) } })));
    expect(screen.getByText("Found 1 domain on this key.")).toBeInTheDocument();
  });
});

describe("other IMAP", () => {
  it("looks up the servers when you leave the email field, and ⌘E jumps to them", async () => {
    vi.mocked(setup.findServerSettings).mockResolvedValue({
      incoming: { host: "posteo.de", port: "993", security: "ssl" },
      outgoing: { host: "posteo.de", port: "465", security: "ssl" },
    });
    at({ provider: "imap" });
    render(<AddAccountScreen />);
    const user = userEvent.setup();
    expect(footer()).toHaveTextContent("Edit server settings");
    await user.type(screen.getByLabelText("Email"), "maya@posteo.de");
    await user.tab();
    expect(await screen.findByText("Server settings found")).toBeInTheDocument();
    await user.keyboard("{Meta>}e{/Meta}");
    const incoming = screen.getByRole("textbox", { name: "Incoming · IMAP server" });
    expect(incoming).toHaveFocus();
    await user.type(incoming, "x");
    expect(f().imap.servers.incoming.host).toBe("posteo.dex");
    await user.type(screen.getByLabelText("Password"), "pw{Enter}");
    await waitFor(() => expect(f().stage).toBe("sync"));
  });

  it.each([
    ["imap", { email: "maya@posteo.de", password: "pw" }],
    ["icloud", { email: "maya@icloud.com", password: "pw" }],
    ["resend", {}],
  ] as const)("%s: Connect shows at once that it is connecting, and takes no second press", (provider, fields) => {
    at({ provider, ...fields, busy: true });
    render(<AddAccountScreen />);
    const connect = screen.getByRole("button", { name: /^Connecting…/ });
    expect(connect).toHaveAttribute("aria-busy", "true");
    expect(connect).toBeDisabled();
  });

  it("Connect sends the email and password", async () => {
    at({ provider: "imap", email: "maya@posteo.de", password: "pw" });
    render(<AddAccountScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Connect↵" }));
    expect(setup.connectWithPassword).toHaveBeenCalled();
  });

  it("asks you to type the servers when none are known", () => {
    at({
      provider: "imap",
      email: "maya@ross-studio.co",
      imap: { status: "not-found", servers: f().imap.servers },
    });
    render(<AddAccountScreen />);
    expect(screen.getByRole("heading", { name: "We couldn’t find your server settings" })).toBeInTheDocument();
    expect(screen.getByText(/Moss looked for ross-studio.co/)).toBeInTheDocument();
  });

  it("other keys do nothing on a sign-in page", async () => {
    at({ provider: "imap" });
    render(<AddAccountScreen />);
    await userEvent.setup().keyboard("j{Alt>}a{/Alt}{Meta>}o{/Meta}{Meta>}c{/Meta}");
    expect(f()).toMatchObject({ stage: "sign-in", advanced: false });
    expect(open).not.toHaveBeenCalled();
  });
});

describe("problems other than the password", () => {
  it("say what happened under the form", async () => {
    at({ provider: "gmail", error: "failed", errorMessage: "Moss couldn’t reach imap.gmail.com." });
    render(<AddAccountScreen />);
    expect(screen.getByText("Moss couldn’t reach imap.gmail.com.")).toBeInTheDocument();
    // Not the wrong-password screen.
    expect(screen.getByRole("button", { name: /^Connect/ })).toBeInTheDocument();
  });
});

describe("syncing", () => {
  const connected = { id: "gmail", name: "Gmail", letter: "G", address: "maya.ross@gmail.com", color: "green" as const };
  // AI on this Mac was settled before: closing goes straight to the inbox.
  beforeEach(() => setAi(demoAi({ plan: "recommended" })));

  it("shows progress, changes colour with ← → and goes to the inbox with ↵", async () => {
    act(() => useAddAccount.setState({ stage: "sync", provider: "gmail", connected }));
    render(<AddAccountScreen />);
    expect(screen.getByRole("heading", { name: "Gmail is connected" })).toBeInTheDocument();
    expect(screen.getByText("Signed in. Password saved in the macOS Keychain")).toBeInTheDocument();
    expect(screen.getByText("Folders and labels")).toBeInTheDocument();
    const user = userEvent.setup();
    await user.keyboard("{ArrowRight}{ArrowRight}{ArrowLeft}x{Meta>}a{/Meta}");
    expect(f().connected?.color).toBe("violet");
    await user.click(screen.getByRole("radio", { name: "Sky" }));
    expect(f().connected?.color).toBe("sky");
    await user.keyboard("{Enter}");
    expect(s()).toMatchObject({ view: "inbox", accountFilter: "gmail" });
  });

  it("A adds another account, Esc and the buttons close", async () => {
    const user = userEvent.setup();
    act(() => useAddAccount.setState({ stage: "sync", provider: "outlook", connected }));
    const { unmount } = render(<AddAccountScreen />);
    expect(screen.getByText("Signed in. Sign-in token saved in the macOS Keychain")).toBeInTheDocument();
    expect(screen.getByText("Folders")).toBeInTheDocument();
    await user.keyboard("a");
    expect(f().stage).toBe("provider");
    for (const close of [() => user.keyboard("{Escape}"), () => user.click(screen.getByRole("button", { name: /Close/ }))]) {
      act(() => useAddAccount.setState({ stage: "sync", provider: "resend", connected: { ...connected, id: `x${Math.random()}` } }));
      expect(screen.getByText("Connected. API key saved in the macOS Keychain")).toBeInTheDocument();
      await close();
      expect(f().stage).toBe("provider");
    }
    act(() => useAddAccount.setState({ stage: "sync", provider: "resend", connected: { ...connected, id: "y" } }));
    await user.click(screen.getByRole("button", { name: /Add another account/ }));
    await user.click(screen.getByRole("option", { name: /iCloud/ }));
    expect(f().provider).toBe("icloud");
    unmount();
  });

  it("shows real progress once sync reports it", () => {
    useSync.setState({ progress: { gmail: { done: 300, total: 300, folders: 9, recent: 1, oldestYear: 2019 } } });
    act(() => useAddAccount.setState({ stage: "sync", provider: "gmail", connected }));
    render(<AddAccountScreen />);
    expect(screen.getByText("300 of 300 emails")).toBeInTheDocument();
    expect(screen.getByText("9 found")).toBeInTheDocument();
    expect(screen.getByText("Older mail, back to 2019")).toBeInTheDocument();
    expect(screen.getByText("In the background")).toBeInTheDocument();
    useSync.setState({ progress: {} });
  });

  it("shows nothing for Sync without a connected account", () => {
    act(() => useAddAccount.setState({ stage: "sync" }));
    render(<AddAccountScreen />);
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    fireEvent.keyDown(window, { key: "Escape" });
  });
});

describe("AI on this Mac, after the first account", () => {
  const connected = { id: "gmail", name: "Gmail", letter: "G", address: "maya.ross@gmail.com", color: "green" as const };

  async function toAiStep(start: Parameters<typeof demoAi>[0] = {}) {
    const demo = demoAi(start);
    setAi(demo);
    act(() => useAddAccount.setState({ stage: "sync", provider: "gmail", connected }));
    render(<AddAccountScreen />);
    await waitFor(() => expect(useAiSettings.getState().status?.plan).toBeNull());
    expect(screen.getByText("AI")).toBeInTheDocument();
    await userEvent.setup().keyboard("{Enter}");
    expect(await screen.findByRole("heading", { name: "AI on this Mac" })).toBeInTheDocument();
    return demo;
  }

  it("comes after Sync the first time, with Recommended picked and real sizes", async () => {
    await toAiStep();
    expect(screen.getByText("This Mac: M3, 16 GB memory")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Recommended/ })).toBeChecked();
    expect(screen.getByText("About 6.4 GB")).toBeInTheDocument();
    expect(screen.getByText("About 1.4 GB")).toBeInTheDocument();
    expect(screen.getByText("Nothing to download")).toBeInTheDocument();
    expect(screen.getByText("d3-lite to sort (1.1 GB) · EmbeddingGemma to search (334 MB) · Gemma 4 E4B to write (5 GB)")).toBeInTheDocument();
    expect(screen.getByText("d3-lite to sort (1.1 GB) · EmbeddingGemma to search (334 MB)")).toBeInTheDocument();
    expect(screen.getByText("Runs only on this Mac. Your mail never leaves it.")).toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).toHaveTextContent("Skip");
    // The account is added already.
    expect(s().accounts.some((a) => a.id === "gmail")).toBe(true);
  });

  it("↵ downloads the one picked and opens the inbox", async () => {
    const demo = await toAiStep();
    const plan = vi.spyOn(demo, "plan");
    const user = userEvent.setup();
    await user.keyboard("jJ");
    expect(screen.getByRole("radio", { name: /Recommended/ })).toBeChecked();
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}kK{ArrowUp}{Meta>}{ArrowDown}{/Meta}x");
    expect(screen.getByRole("radio", { name: /Essentials only/ })).toBeChecked();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(s()).toMatchObject({ view: "inbox", accountFilter: "gmail" }));
    expect(plan).toHaveBeenCalledWith("essentials");
    expect(f().stage).toBe("provider");
  });

  it("a click picks, hovering doesn't, and the mouse downloads too", async () => {
    const demo = await toAiStep();
    const plan = vi.spyOn(demo, "plan");
    const user = userEvent.setup();
    await user.hover(screen.getByText("Skip for now"));
    expect(screen.getByRole("radio", { name: /Recommended/ })).toBeChecked();
    await user.click(screen.getByRole("radio", { name: /Essentials only/ }));
    expect(screen.getByRole("radio", { name: /Essentials only/ })).toBeChecked();
    await user.click(screen.getByRole("radio", { name: /Recommended/ }));
    await user.click(screen.getByRole("button", { name: /^Download/ }));
    await waitFor(() => expect(plan).toHaveBeenCalledWith("recommended"));
  });

  it("S, Esc, Skip and the top bar all skip, and it never asks again", async () => {
    for (const skip of [
      () => userEvent.setup().keyboard("s"),
      () => userEvent.setup().keyboard("{Escape}"),
      () => userEvent.setup().click(screen.getByRole("button", { name: /^SkipS/ })),
      () => userEvent.setup().click(screen.getByRole("button", { name: /^EscSkip/ })),
    ]) {
      const demo = await toAiStep();
      const plan = vi.spyOn(demo, "plan");
      await skip();
      await waitFor(() => expect(plan).toHaveBeenCalledWith("skip"));
      await waitFor(() => expect(s().view).toBe("inbox"));
      cleanup();
    }
    // Chosen once: the next account goes straight to its inbox.
    act(() => useAddAccount.setState({ stage: "sync", provider: "gmail", connected: { ...connected, id: "g2" } }));
    render(<AddAccountScreen />);
    await userEvent.setup().keyboard("{Enter}");
    expect(f().stage).toBe("provider");
  });

  it("shows nothing until the AI's status is in", () => {
    act(() => useAddAccount.setState({ stage: "ai", provider: "gmail", connected }));
    render(<AddAccountScreen />);
    expect(screen.queryByRole("heading", { name: "AI on this Mac" })).toBeNull();
  });
});

describe("AI on this Mac, on any Mac", () => {
  it("a click on a choice picks it, and a writing model it doesn't know still reads", async () => {
    const { AiSetup } = await import("./addAccount/AiSetup");
    const status = { ...(await demoAi().status()), writer: "retired", plans: [] };
    act(() => useAddAccount.setState({ stage: "ai", provider: "gmail", aiChoice: 0 }));
    render(<AiSetup account={{ id: "g", name: "Gmail", letter: "G", address: "m@gmail.com", color: "green" }} status={status} />);
    expect(screen.getByText("d3-lite to sort (1.1 GB) · EmbeddingGemma to search (334 MB) · Gemma to write")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: /Skip for now/ }));
    expect(f().aiChoice).toBe(2);
  });
});
