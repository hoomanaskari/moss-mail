/**
 * Dev scenes for your addresses, on an iCloud account with custom domains: replying from the one
 * it came to, the From menu, one address's inbox, a catch-all address, and Settings. Not in
 * design.pen yet; they are drawn with the design system's parts.
 */
import type { Account, Person, Thread } from "../data/types";
import { useSignatures } from "../state/signatures";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

const person = (name: string, email: string): Person => ({
  name,
  email,
  initials: name
    .split(" ")
    .map((w) => w[0])
    .join(""),
});

export const icloudDemoAccount: Account = {
  id: "icloud",
  name: "iCloud",
  letter: "I",
  address: "maya.ross@icloud.com",
  senderName: "Maya Ross",
  color: "sky",
  connection: { provider: "icloud", summary: "iCloud · App password" },
  addresses: [
    { email: "hi@mayaross.me", state: "added", source: "you", seen: 12, blocked: false },
    { email: "photos@rossprints.shop", name: "Ross Prints", state: "added", source: "you", seen: 4, blocked: false },
    { email: "shop@mayaross.me", state: "suggested", source: "mail", seen: 3, blocked: false },
  ],
};

const me = (email: string) => person("Maya Ross", email);

const mail = (
  id: string,
  toAddress: string,
  from: Person,
  subject: string,
  summary: string,
  time: string,
  body: string[],
  extra: Partial<Thread> = {},
): Thread => ({
  id,
  accountId: "icloud",
  split: "important",
  section: "reply",
  from,
  subject,
  summary,
  time,
  toAddress,
  messages: [{ id: `${id}-1`, from, to: [me(toAddress)], cc: [], bcc: [], time, sentFull: `Sunday, 1 November, ${time}`, paragraphs: body, attachments: [] }],
  ...extra,
});

const icloudThreads: Thread[] = [
  mail("print-order", "photos@rossprints.shop", person("Ana Costa", "ana@costa.pt"), "A2 print of the harbour", "Can you ship to Porto by the 20th?", "9:20 AM", [
    "Hi, I love the harbour photo. Could I order an A2 print, shipped to Porto by the 20th?",
  ]),
  mail("guest-post", "hi@mayaross.me", person("Sam Patel", "sam@slowweekend.example"), "Guest post for The Slow Weekend", "Would you write 800 words on Lisbon?", "8:10 AM", [
    "Hi Maya, would you write about 800 words on a slow weekend in Lisbon for our December letter?",
  ]),
  mail("shop-sale", "shop@mayaross.me", person("Bairro Books", "news@bairrobooks.example"), "20% off this weekend", "Your favourite authors, on sale", "7:45 AM", [
    "This weekend only: 20% off everything in store and online.",
  ], { section: "info", split: "other" }),
  mail("mum-icloud", "maya.ross@icloud.com", person("Helen Ross", "helen.ross@icloud.com"), "Photos from Sunday", "Here are the ones from the garden", "Yesterday", [
    "Here are the photos from the garden. The one with the dog is my favourite.",
  ], { section: "info" }),
];

/** Puts the iCloud account first, with its mail and a signature for the print shop. Safe to run twice. */
function withIcloud() {
  useStore.setState((s) => ({
    accounts: [icloudDemoAccount, ...s.accounts.filter((a) => a.id !== "icloud")],
    threads: [...icloudThreads, ...s.threads.filter((t) => t.accountId !== "icloud")],
  }));
  useSignatures.setState((s) => ({
    signatures: [
      ...s.signatures.filter((x) => x.id !== "prints"),
      { id: "prints", name: "Ross Prints", lines: ["Maya Ross", "Ross Prints · Fine art photography", "rossprints.shop"] },
    ],
    byAccount: { ...s.byAccount, icloud: { newEmails: "personal", replies: null } },
    byAddress: { ...s.byAddress, "photos@rossprints.shop": { newEmails: "prints", replies: "prints" } },
  }));
}

export const addressScenes: Record<string, Scene> = {
  "address-reply": {
    shows: /It came to photos@rossprints\.shop/,
    apply: () => {
      withIcloud();
      useStore.getState().compose("reply", "print-order");
    },
  },
  "address-from-menu": {
    shows: /Send from/,
    apply: () => {
      withIcloud();
      useStore.getState().compose("reply", "print-order");
      useStore.getState().toggleFromPicker();
    },
  },
  "address-inbox": {
    shows: /All addresses/,
    apply: () => {
      withIcloud();
      useStore.getState().setAccountFilter("icloud");
      useStore.getState().setAddressFilter("photos@rossprints.shop");
    },
  },
  "address-catch-all": {
    shows: /you don’t send from yet/,
    apply: () => {
      withIcloud();
      useStore.getState().setSplit("other");
      useStore.getState().open("shop-sale");
    },
  },
  "settings-addresses": {
    shows: /Found in your mail|Look in my mail/,
    apply: () => {
      withIcloud();
      useStore.getState().openSettings("accounts");
    },
  },
};
