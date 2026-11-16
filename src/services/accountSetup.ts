/**
 * What the Add account screens need from the mail engine. The demo version below runs
 * in the browser; the Tauri version will call the Rust commands with the same shapes.
 */
import type { Account, ProviderId } from "../data/types";

export type Security = "ssl" | "starttls" | "none";

export interface Server {
  host: string;
  port: string;
  security: Security;
}

export interface ServerSettings {
  incoming: Server;
  outgoing: Server;
}

export interface ResendDomain {
  name: string;
  /** e.g. "Verified · 3 addresses seen: hello@, prints@, maya@" */
  detail: string;
  /** Addresses seen on the domain, offered as "Only x@domain". */
  addresses: string[];
  /** The domain has had mail lately. Every domain still starts unticked: you choose. */
  active: boolean;
}

export interface SyncSnapshot {
  done: number;
  total: number;
  folders: number;
  /** 0 to 1, for the "Recent mail, last 30 days" step. */
  recent: number;
  oldestYear: number;
}

/** "failed": anything but a wrong password, such as no connection; `message` says what to do. */
export type SetupFailure = { ok: false; reason: "rejected" } | { ok: false; reason: "failed"; message: string };
/** `account` is the account the mail engine saved, when there is one. */
export type SignInResult = { ok: true; account?: Account } | SetupFailure;
/** `account` is the account the mail engine saved, when there is one. */
export type BrowserResult =
  | { ok: true; address: string; account?: Account }
  | { ok: false; reason: "admin-approval" }
  | { ok: false; reason: "failed"; message: string };

export interface AccountSetup {
  /** App password or IMAP password sign-in. Nothing is saved unless it works. */
  /** `login`: who to log in as when it isn't `email`, e.g. the @icloud.com address behind an iCloud custom domain. */
  connectWithPassword(req: { provider: ProviderId; email: string; password: string; login?: string; servers?: ServerSettings }): Promise<SignInResult>;
  /** OAuth in the system browser (Outlook, or Gmail with your own client). Resolves when the browser hands back. */
  signInWithBrowser(req: { provider: "gmail" | "outlook"; clientId?: string; clientSecret?: string }): Promise<BrowserResult>;
  /** Looks the email's domain up in Mozilla's list of providers (ISPDB). */
  findServerSettings(email: string): Promise<ServerSettings | null>;
  /** Checks a Resend API key and lists its domains, or says what's wrong with it. */
  checkResendKey(key: string): Promise<ResendDomain[] | string>;
  /** Saves a Resend account that sends and receives as `address`. */
  connectResend(req: { key: string; address: string }): Promise<SignInResult>;
  syncProgress(address: string): SyncSnapshot;
  /** The sign-in page for Outlook, to open again or copy. */
  signInLink(): string;
  /** The page an IT admin opens to approve Moss for their organisation. */
  adminApprovalLink(): string;
}

const isAppPassword = (pw: string) => /^[a-z]{16}$/i.test(pw.replace(/[\s-]/g, ""));

const knownServers: Record<string, ServerSettings> = {
  "posteo.de": {
    incoming: { host: "posteo.de", port: "993", security: "ssl" },
    outgoing: { host: "posteo.de", port: "465", security: "ssl" },
  },
};

export const demoDomains: ResendDomain[] = [
  { name: "rossstudio.art", detail: "Verified · 3 addresses seen: hello@, prints@, maya@", addresses: ["hello", "prints", "maya"], active: true },
  { name: "mayaross.dev", detail: "Verified · 1 address seen: me@", addresses: ["me"], active: true },
  { name: "old-portfolio.net", detail: "Verified · no mail in the last 90 days", addresses: [], active: false },
];

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const badResendKey = "That key didn’t work. Check it at resend.com/api-keys.";

/** Runs the flow on sample answers until the Rust commands are wired in. */
export const demoSetup: AccountSetup = {
  async connectWithPassword({ provider, password }) {
    await wait(400);
    if (provider === "imap") return password ? { ok: true } : { ok: false, reason: "rejected" };
    return isAppPassword(password) ? { ok: true } : { ok: false, reason: "rejected" };
  },
  async signInWithBrowser({ provider }) {
    await wait(2500);
    return { ok: true, address: provider === "gmail" ? "maya.ross@gmail.com" : "maya.ross@outlook.com" };
  },
  async findServerSettings(email) {
    await wait(300);
    return knownServers[email.split("@")[1]?.toLowerCase() ?? ""] ?? null;
  },
  async checkResendKey(key) {
    await wait(300);
    return /^re_\w{6,}/.test(key) ? demoDomains : badResendKey;
  },
  connectResend: async () => ({ ok: true }),
  syncProgress: () => ({ done: 1240, total: 1930, folders: 14, recent: 0.64, oldestYear: 2016 }),
  signInLink: () => "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=moss",
  adminApprovalLink: () => "https://login.microsoftonline.com/common/adminconsent?client_id=moss",
};

let current: AccountSetup = demoSetup;

/** The setup backend in use. */
export const accountSetup = () => current;

/** Swap the backend, e.g. for the Tauri one or a test double. */
export function setAccountSetup(setup: AccountSetup) {
  current = setup;
}
