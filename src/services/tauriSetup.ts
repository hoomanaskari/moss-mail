/** The Add account flow in the desktop app: signs in through the Rust engine, which saves the secret in the Keychain. */
import { invoke } from "@tauri-apps/api/core";
import { badResendKey, type AccountSetup, type BrowserResult, type SetupFailure, type SignInResult } from "./accountSetup";
import { toAccount, toServerSettings, toUiServers, type RustAccount, type RustServerSettings } from "./convert";
import { openLink } from "../lib/links";

/** A sign-in error from the engine (`moss_accounts::SignInError`). */
export interface SignInError {
  kind: string;
  host?: string;
  domain?: string;
  detail?: string;
  message?: string;
}

/** What went wrong, as the flow shows it: the wrong-password screen, or a sentence. */
export function setupFailure(error: unknown): SetupFailure {
  const e = (error ?? {}) as SignInError;
  const failed = (message: string): SetupFailure => ({ ok: false, reason: "failed", message });
  switch (e.kind) {
    case "wrongPassword":
    case "gmailAppPassword":
      return { ok: false, reason: "rejected" };
    case "invalidEmail":
      return failed("That doesn’t look like an email address. Check it and try again.");
    case "unreachable":
    case "timeout":
      return failed(`Moss couldn’t reach ${e.host || "the mail server"}. Check your internet connection and try again.`);
    case "settingsNotFound":
      return failed(`Moss couldn’t find the server settings for ${e.domain}. Type them in below.`);
    case "iCloudAddressNeeded":
      return failed("iCloud signs custom domains in with your Apple Account’s @icloud.com address. Add it above and try again.");
    case "keychain":
      return failed("Moss couldn’t save the password in the Keychain. Unlock your Mac’s Keychain and try again.");
    case "invalidResendKey":
    case "resendKeyRestricted":
      return failed("Resend didn’t accept this key. Make a Full access key and paste it again.");
    case "resendDomainMissing":
      return failed(`${e.domain} isn’t in this Resend account. Add it in Resend first.`);
    default:
      return failed(e.message || "Moss couldn’t connect this account. Try again in a moment.");
  }
}

/** A browser sign-in that went wrong, as the Outlook and own-client screens show it. */
function browserFailure(error: unknown): BrowserResult {
  const failed = (message: string): BrowserResult => ({ ok: false, reason: "failed", message });
  switch ((error as SignInError | undefined)?.kind) {
    case "adminApprovalRequired":
      return { ok: false, reason: "admin-approval" };
    case "accessDenied":
      return failed("The sign-in was cancelled, so nothing was added. Try again when you’re ready.");
    case "timeout":
      return failed("The sign-in page wasn’t finished in time. Try again.");
    case "invalidOAuthClient":
      return failed("The sign-in page didn’t accept this client. Check the client ID and try again.");
    default:
      return setupFailure(error) as BrowserResult;
  }
}

/** The sign-in page in use, for "Open again" and "Copy link", and the client ID it signs in with. */
let browserSignIn = { url: "", clientId: "" };

async function signIn(run: () => Promise<RustAccount>): Promise<SignInResult> {
  try {
    return { ok: true, account: toAccount(await run()) };
  } catch (error) {
    return setupFailure(error);
  }
}

const noProgress = () => ({ done: 0, total: 0, folders: 0, recent: 0, oldestYear: new Date().getFullYear() });

export const tauriSetup: AccountSetup = {
  connectWithPassword: ({ provider, email, password, login, servers }) =>
    signIn(() =>
      invoke<RustAccount>("add_password_account", {
        request: {
          service: provider,
          email: email.trim(),
          password,
          servers: servers ? toServerSettings(servers, email.trim()) : null,
          ...(login?.trim() ? { login: login.trim() } : {}),
        },
      }),
    ),
  // Outlook, or Gmail with your own client: the engine waits for the browser to come back.
  signInWithBrowser: async ({ provider, clientId, clientSecret }) => {
    try {
      const request = { provider, clientId: clientId?.trim() || null, clientSecret: clientSecret || null };
      browserSignIn = await invoke<{ url: string; clientId: string }>("start_browser_sign_in", { request });
      openLink(browserSignIn.url);
      const account = toAccount(await invoke<RustAccount>("finish_browser_sign_in"));
      return { ok: true, address: account.address, account };
    } catch (error) {
      return browserFailure(error);
    }
  },
  findServerSettings: async (email) => {
    try {
      const found = await invoke<{ servers: RustServerSettings }>("discover", { email });
      return toUiServers(found.servers);
    } catch {
      return null;
    }
  },
  checkResendKey: async (key) => {
    try {
      const domains = await invoke<{ name: string; status: string }[]>("check_resend_key", { key });
      return domains.map((d) => ({
        name: d.name,
        detail: d.status === "verified" ? "Verified" : `Not verified yet (${d.status})`,
        addresses: [],
        active: d.status === "verified",
      }));
    } catch (error) {
      switch ((error as SignInError | undefined)?.kind) {
        case "invalidResendKey":
          return badResendKey;
        case "resendKeyRestricted":
          return "This key only sends, or only covers some domains. Moss needs a Full access key: make one at resend.com/api-keys.";
        default:
          return (setupFailure(error) as { message: string }).message;
      }
    }
  },
  connectResend: ({ key, address }) => signIn(() => invoke<RustAccount>("add_resend_account", { key, email: address })),
  syncProgress: noProgress,
  signInLink: () => browserSignIn.url,
  adminApprovalLink: () => `https://login.microsoftonline.com/common/adminconsent?client_id=${browserSignIn.clientId}`,
};
