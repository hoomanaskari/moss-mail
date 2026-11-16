import { demoDomains } from "../services/accountSetup";
import { demoAi, setAi } from "../services/ai";
import { resetAddAccount, useAddAccount } from "../state/addAccount";
import { useAiSettings } from "../state/aiSettings";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

type FlowPatch = Partial<ReturnType<typeof useAddAccount.getState>>;

/** Opens Add account in the given state, without calling the setup backend. */
const at = (shows: RegExp, patch: FlowPatch = {}): Scene => ({
  shows,
  apply: () => {
    resetAddAccount();
    useStore.getState().openAddAccount();
    useAddAccount.setState(patch);
  },
});

const signIn = { stage: "sign-in" } as const;
const twelve = "abcdefghijkl";

export const addAccountScenes: Record<string, Scene> = {
  "add-account": at(/Which email do you want to add/),
  "add-account-outlook": at(/Sign in with Microsoft/, { ...signIn, cursor: 1, provider: "outlook", busy: true, waiting: true }),
  "add-account-outlook-client-id": at(/Use your own Microsoft client ID/, {
    ...signIn,
    cursor: 1,
    provider: "outlook",
    advanced: true,
    stepsDone: 2,
  }),
  "add-account-outlook-admin": at(/needs to approve Moss/, { ...signIn, cursor: 1, provider: "outlook", error: "admin-approval" }),
  "add-account-gmail": at(/Connect Gmail with an app password/, {
    ...signIn,
    provider: "gmail",
    stepsDone: 1,
    email: "maya.ross@gmail.com",
    password: "abcdefghijklmnop",
  }),
  "add-account-connecting": at(/Connecting…/, {
    ...signIn,
    provider: "gmail",
    stepsDone: 2,
    email: "maya.ross@gmail.com",
    password: "abcdefghijklmnop",
    busy: true,
  }),
  "add-account-gmail-error": at(/didn’t accept that app password/, {
    ...signIn,
    provider: "gmail",
    email: "maya.ross@gmail.com",
    password: twelve,
    error: "rejected",
  }),
  "add-account-gmail-oauth": at(/Use your own Google OAuth client/, { ...signIn, provider: "gmail", advanced: true, stepsDone: 2 }),
  "add-account-icloud": at(/Connect iCloud Mail/, { ...signIn, cursor: 2, provider: "icloud", email: "maya.ross@icloud.com" }),
  "add-account-icloud-error": at(/That password didn’t work/, {
    ...signIn,
    cursor: 2,
    provider: "icloud",
    email: "maya.ross@icloud.com",
    password: twelve,
    error: "rejected",
  }),
  "add-account-icloud-custom-domain": at(/Your @icloud.com address/, {
    ...signIn,
    cursor: 2,
    provider: "icloud",
    email: "info@fernway.co",
    login: "maya.ross@icloud.com",
  }),
  "add-account-yahoo": at(/Connect Yahoo Mail/, { ...signIn, cursor: 3, provider: "yahoo", email: "maya.ross@yahoo.com" }),
  "add-account-fastmail": at(/Connect Fastmail/, { ...signIn, cursor: 4, provider: "fastmail", email: "maya@fastmail.com" }),
  "add-account-resend": at(/Connect Resend/, {
    ...signIn,
    cursor: 5,
    provider: "resend",
    resend: {
      key: "re_example_not_a_real_api_key",
      status: "works",
      cursor: 0,
      domains: demoDomains.map((d) => ({ ...d, chosen: d.active, only: d.name === "mayaross.dev" ? "me" : null })),
    },
  }),
  "add-account-imap": at(/Connect your email/, {
    ...signIn,
    cursor: 6,
    provider: "imap",
    email: "maya@posteo.de",
    password: "abcdefghijklmn",
    imap: {
      status: "found",
      servers: {
        incoming: { host: "posteo.de", port: "993", security: "ssl" },
        outgoing: { host: "posteo.de", port: "465", security: "ssl" },
      },
    },
  }),
  "add-account-imap-not-found": at(/couldn’t find your server settings/, {
    ...signIn,
    cursor: 6,
    provider: "imap",
    email: "maya@ross-studio.co",
    password: "abcdefghijklmn",
    imap: {
      status: "not-found",
      servers: {
        incoming: { host: "", port: "993", security: "ssl" },
        outgoing: { host: "", port: "465", security: "ssl" },
      },
    },
  }),
  "add-account-syncing": at(/Gmail is connected/, {
    stage: "sync",
    provider: "gmail",
    connected: { id: "gmail", name: "Gmail", letter: "G", address: "maya.ross@gmail.com", color: "green" },
  }),
  "add-account-ai": {
    shows: /AI on this Mac/,
    apply: () => {
      setAi(demoAi());
      void useAiSettings.getState().loadStatus();
      at(/./, {
        stage: "ai",
        provider: "gmail",
        connected: { id: "gmail", name: "Gmail", letter: "G", address: "maya.ross@gmail.com", color: "green" },
      }).apply();
    },
  },
};
