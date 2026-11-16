import type { ProviderId } from "./types";

export interface SetupStepCopy {
  title: string;
  sub?: string;
  /** Page the step's Open button goes to. */
  url?: string;
}

export interface Provider {
  id: ProviderId;
  name: string;
  /** Letter on the provider tile; Other (IMAP) shows an @ icon instead. */
  letter?: string;
  /** What to expect, on the Choose provider list. */
  how: string;
}

/** In the order of the Choose provider list, so 1–7 pick them. */
export const providers: Provider[] = [
  { id: "gmail", name: "Gmail", letter: "G", how: "Google accounts · App password, about 2 minutes" },
  { id: "outlook", name: "Outlook / Microsoft 365", letter: "O", how: "Outlook, Hotmail, work or school · One click in your browser" },
  { id: "icloud", name: "iCloud", letter: "i", how: "@icloud.com, @me.com and custom domains · App password" },
  { id: "yahoo", name: "Yahoo", letter: "Y", how: "Yahoo Mail · App password" },
  { id: "fastmail", name: "Fastmail", letter: "F", how: "Fastmail · App password" },
  { id: "resend", name: "Resend", letter: "R", how: "Mail sent to your own domains · API key" },
  { id: "imap", name: "Other (IMAP)", how: "Any other email · Email and password" },
];

export function providerById(id: ProviderId) {
  return providers.find((p) => p.id === id)!;
}

const icloudDomains = ["icloud.com", "me.com", "mac.com"];

/**
 * An iCloud address on a custom domain. iCloud only lets the account's own @icloud.com
 * address log in, so Moss asks for that too. Matches `MailService::for_address` in Rust.
 */
export function needsICloudLogin(email: string) {
  const domain = /^[^@\s]+@([^@\s]+\.[^@\s]+)$/.exec(email.trim())?.[1].toLowerCase();
  return !!domain && !icloudDomains.includes(domain);
}

/** Copy for the providers that sign in with an app password: Gmail, iCloud, Yahoo and Fastmail. */
export interface AppPasswordCopy {
  title: string;
  sub: string;
  steps: SetupStepCopy[];
  emailLabel: string;
  passwordLabel: string;
  placeholder: string;
  /** iCloud: the field for the @icloud.com address a custom domain logs in with. */
  login?: { label: string; help: string; placeholder: string };
  /** Footer label for ⌘O / the first Open button, e.g. "Open Apple Account". */
  openLabel: string;
  footnote: string;
  error: {
    title: string;
    sub: string;
    note: string;
    fix: SetupStepCopy;
  };
}

export const appPasswordCopy: Record<"gmail" | "icloud" | "yahoo" | "fastmail", AppPasswordCopy> = {
  gmail: {
    title: "Connect Gmail with an app password",
    sub: "Google gives Moss its own password, separate from yours. It takes about 2 minutes.",
    steps: [
      { title: "Turn on 2-Step Verification", sub: "Already on? Then this step is done.", url: "https://myaccount.google.com/signinoptions/twosv" },
      {
        title: "Create an app password",
        sub: "On myaccount.google.com/apppasswords, type Moss as the name and press Create.",
        url: "https://myaccount.google.com/apppasswords",
      },
      { title: "Paste it below", sub: "It’s 16 letters. Spaces don’t matter." },
    ],
    emailLabel: "Gmail address",
    passwordLabel: "App password",
    placeholder: "abcd efgh ijkl mnop",
    openLabel: "Open Google’s page",
    footnote: "Moss saves the app password in the macOS Keychain. You can revoke it at Google any time.",
    error: {
      title: "Google didn’t accept that app password",
      sub: "This usually means a typo, your normal Google password, or a work account where app passwords are off.",
      note: "Use the 16-letter app password from Google, not your normal password.",
      fix: {
        title: "Create a new app password",
        sub: "myaccount.google.com/apppasswords → type Moss → Create",
        url: "https://myaccount.google.com/apppasswords",
      },
    },
  },
  icloud: {
    title: "Connect iCloud Mail",
    sub: "iCloud needs an app-specific password. Your normal Apple Account password won’t work here.",
    steps: [
      {
        title: "Create an app-specific password",
        sub: "account.apple.com → Sign-In and Security → App-Specific Passwords",
        url: "https://account.apple.com/account/manage",
      },
      { title: "Name it Moss, then paste it below", sub: "It looks like this: abcd-efgh-ijkl-mnop" },
    ],
    emailLabel: "iCloud email",
    passwordLabel: "App-specific password",
    placeholder: "abcd-efgh-ijkl-mnop",
    login: { label: "Your @icloud.com address", help: "iCloud signs custom domains in with it", placeholder: "name@icloud.com" },
    openLabel: "Open Apple Account",
    footnote: "Moss saves the password in the macOS Keychain. You can revoke it at account.apple.com any time.",
    error: {
      title: "That password didn’t work",
      sub: "iCloud said no. This usually means it was your Apple Account password, not an app-specific one.",
      note: "Use an app-specific password. It looks like abcd-efgh-ijkl-mnop.",
      fix: {
        title: "Create an app-specific password",
        sub: "account.apple.com → Sign-In and Security → App-Specific Passwords",
        url: "https://account.apple.com/account/manage",
      },
    },
  },
  yahoo: {
    title: "Connect Yahoo Mail",
    sub: "Yahoo needs an app password. Your normal Yahoo password won’t work here.",
    steps: [
      {
        title: "Create an app password",
        sub: "login.yahoo.com → Account security → Generate app password",
        url: "https://login.yahoo.com/account/security",
      },
      { title: "Name it Moss, then paste it below", sub: "It’s 16 letters. Spaces don’t matter." },
    ],
    emailLabel: "Yahoo email",
    passwordLabel: "App password",
    placeholder: "abcd efgh ijkl mnop",
    openLabel: "Open Yahoo Account",
    footnote: "Moss saves the password in the macOS Keychain. You can revoke it at login.yahoo.com any time.",
    error: {
      title: "That password didn’t work",
      sub: "Yahoo said no. This usually means it was your normal Yahoo password, not an app password.",
      note: "Use an app password. It’s 16 letters, like abcd efgh ijkl mnop.",
      fix: {
        title: "Create an app password",
        sub: "login.yahoo.com → Account security → Generate app password",
        url: "https://login.yahoo.com/account/security",
      },
    },
  },
  fastmail: {
    title: "Connect Fastmail",
    sub: "Fastmail needs an app password. Your normal Fastmail password won’t work here.",
    steps: [
      {
        title: "Create an app password",
        sub: "app.fastmail.com → Settings → Privacy & Security → App passwords",
        url: "https://app.fastmail.com/settings/security/apppasswords",
      },
      { title: "Name it Moss, choose “Mail (IMAP/POP/SMTP)”, then paste it below", sub: "It’s 16 characters. Spaces don’t matter." },
    ],
    emailLabel: "Fastmail email",
    passwordLabel: "App password",
    placeholder: "abcd efgh ijkl mnop",
    openLabel: "Open Fastmail settings",
    footnote: "Moss saves the password in the macOS Keychain. You can revoke it in Fastmail’s settings any time.",
    error: {
      title: "That password didn’t work",
      sub: "Fastmail said no. This usually means it was your normal Fastmail password, not an app password.",
      note: "Use an app password. It’s 16 characters, like abcd efgh ijkl mnop.",
      fix: {
        title: "Create an app password",
        sub: "app.fastmail.com → Settings → Privacy & Security → App passwords",
        url: "https://app.fastmail.com/settings/security/apppasswords",
      },
    },
  },
};

/** Gmail's advanced sign-in through your own Google Cloud project. */
export const googleOAuthSteps: SetupStepCopy[] = [
  { title: "Create a project", sub: "console.cloud.google.com/projectcreate", url: "https://console.cloud.google.com/projectcreate" },
  {
    title: "Turn on the Gmail API",
    sub: "APIs & Services → Library → Gmail API → Enable",
    url: "https://console.cloud.google.com/apis/library/gmail.googleapis.com",
  },
  {
    title: "Set up the consent screen, then publish it",
    sub: "Choose External and name it Moss. Then open Audience and press Publish app.",
    url: "https://console.cloud.google.com/auth/branding",
  },
  {
    title: "Create an OAuth client",
    sub: "Credentials → Create credentials → OAuth client ID → Desktop app",
    url: "https://console.cloud.google.com/apis/credentials",
  },
  { title: "Paste the Client ID and secret below" },
];

/** Outlook's advanced sign-in with your own Microsoft Entra app. */
export const microsoftClientSteps: SetupStepCopy[] = [
  {
    title: "Register a new app",
    sub: "entra.microsoft.com → App registrations → New registration",
    url: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/CreateApplicationBlade",
  },
  { title: "Allow personal and work accounts", sub: "Supported account types → Any organizational directory and personal Microsoft accounts" },
  {
    title: "Add a redirect address",
    sub: "Authentication → Add a platform → Mobile and desktop → http://localhost",
    url: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
  },
  {
    title: "Copy the Application (client) ID",
    sub: "It’s on the app’s Overview page. You don’t need a client secret.",
    url: "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
  },
  { title: "Paste the client ID below" },
];
