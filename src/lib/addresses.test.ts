import { describe, expect, it } from "vitest";
import type { Account, AccountAddress } from "../data/types";
import { personal, thread, work } from "../test/fixtures";
import {
  addressCounts,
  addressOf,
  allSendingAddresses,
  blockedAddresses,
  inView,
  isMine,
  isValidAddress,
  replyFrom,
  sendingAddress,
  sendingAddresses,
  suggestedAddresses,
} from "./addresses";

const address = (email: string, extra: Partial<AccountAddress> = {}): AccountAddress => ({
  email,
  state: "added",
  source: "you",
  seen: 0,
  blocked: false,
  ...extra,
});

const icloud: Account = {
  id: "icloud",
  name: "iCloud",
  letter: "I",
  address: "maya@icloud.com",
  senderName: "Maya Ross",
  color: "sky",
  addresses: [
    address("hi@rossstudio.art", { name: "Ross Studio" }),
    address("maya@mayaross.me"),
    address("shop@mayaross.me", { state: "suggested", source: "mail", seen: 4 }),
    address("old@mayaross.me", { blocked: true }),
    address("spam@mayaross.me", { state: "dismissed", source: "mail", blocked: true }),
  ],
};

describe("sendingAddresses", () => {
  it("is the account's own address first, then the ones you added and didn't block", () => {
    expect(sendingAddresses(icloud)).toEqual([
      { accountId: "icloud", email: "maya@icloud.com", name: "Maya Ross", primary: true },
      { accountId: "icloud", email: "hi@rossstudio.art", name: "Ross Studio", primary: false },
      { accountId: "icloud", email: "maya@mayaross.me", name: "Maya Ross", primary: false },
    ]);
  });

  it("is just the account's address when it has no others", () => {
    expect(sendingAddresses(work)).toEqual([{ accountId: "work", email: work.address, name: "Maya Ross", primary: true }]);
  });

  it("lists every account's addresses in account order", () => {
    expect(allSendingAddresses([work, icloud]).map((a) => a.email)).toEqual([
      work.address,
      "maya@icloud.com",
      "hi@rossstudio.art",
      "maya@mayaross.me",
    ]);
  });
});

describe("sendingAddress", () => {
  it("is the address asked for when you can send from it, whatever its case", () => {
    expect(sendingAddress(icloud, "Hi@RossStudio.art").email).toBe("hi@rossstudio.art");
  });

  it("is the account's own address when none is asked for, or one you can't send from", () => {
    expect(sendingAddress(icloud).primary).toBe(true);
    expect(sendingAddress(icloud, "shop@mayaross.me").primary).toBe(true);
    expect(sendingAddress(icloud, "old@mayaross.me").primary).toBe(true);
  });
});

describe("addressOf", () => {
  it("finds one of an account's other addresses, whatever its case", () => {
    expect(addressOf(icloud, "SHOP@mayaross.me")?.seen).toBe(4);
    expect(addressOf(icloud, "nobody@mayaross.me")).toBeUndefined();
    expect(addressOf(work, "x@y.z")).toBeUndefined();
  });
});

describe("suggestedAddresses and blockedAddresses", () => {
  it("are the ones Moss asks about, and the ones whose mail goes to Blocked", () => {
    expect(suggestedAddresses(icloud).map((a) => a.email)).toEqual(["shop@mayaross.me"]);
    expect(blockedAddresses(icloud).map((a) => a.email)).toEqual(["old@mayaross.me", "spam@mayaross.me"]);
    expect(suggestedAddresses(work)).toEqual([]);
  });
});

describe("isMine", () => {
  it("is any account's own address or one you added, whatever the case", () => {
    expect(isMine("MAYA@icloud.com", [icloud])).toBe(true);
    expect(isMine("hi@rossstudio.art", [work, icloud])).toBe(true);
    expect(isMine("shop@mayaross.me", [icloud])).toBe(false);
    expect(isMine("lena@northwind.co", [work, personal])).toBe(false);
  });
});

describe("isValidAddress", () => {
  it("is a full address with a dot in its domain", () => {
    expect(isValidAddress(" hi@rossstudio.art ")).toBe(true);
    expect(isValidAddress("hi@rossstudio")).toBe(false);
    expect(isValidAddress("rossstudio.art")).toBe(false);
    expect(isValidAddress("a b@x.io")).toBe(false);
  });
});

describe("replyFrom", () => {
  it("replies from the address the email came to when you send from it", () => {
    const t = thread("t", { accountId: "icloud", toAddress: "hi@rossstudio.art" });
    expect(replyFrom(t, icloud)).toEqual({ address: "hi@rossstudio.art", reason: "It came to hi@rossstudio.art" });
  });

  it("replies from the account's own address when it came there or Moss can't tell", () => {
    expect(replyFrom(thread("t", { accountId: "icloud", toAddress: "maya@icloud.com" }), icloud)).toEqual({});
    expect(replyFrom(thread("t", { accountId: "icloud" }), icloud)).toEqual({});
  });

  it("says why when it came to an address you don't send from yet", () => {
    const t = thread("t", { accountId: "icloud", toAddress: "shop@mayaross.me" });
    expect(replyFrom(t, icloud)).toEqual({ reason: "It came to shop@mayaross.me. Add it in Settings to reply from it" });
  });
});

describe("inView", () => {
  const t = thread("t", { accountId: "icloud", toAddress: "hi@rossstudio.art" });

  it("shows everything under All accounts, and an account's mail under it", () => {
    expect(inView(t, "all")).toBe(true);
    expect(inView(t, "icloud")).toBe(true);
    expect(inView(t, "work")).toBe(false);
  });

  it("narrows an account to the mail that came to one address", () => {
    expect(inView(t, "icloud", "hi@rossstudio.art")).toBe(true);
    expect(inView(t, "icloud", "maya@icloud.com")).toBe(false);
    expect(inView(thread("u", { accountId: "icloud" }), "icloud", "hi@rossstudio.art")).toBe(false);
  });
});

describe("addressCounts", () => {
  it("counts the threads that came to each address, own address first, then the rest by most mail", () => {
    const threads = [
      thread("a", { accountId: "icloud", toAddress: "hi@rossstudio.art" }),
      thread("b", { accountId: "icloud", toAddress: "maya@icloud.com" }),
      thread("c", { accountId: "icloud", toAddress: "shop@mayaross.me" }),
      thread("d", { accountId: "icloud", toAddress: "shop@mayaross.me" }),
      thread("e", { accountId: "icloud" }),
      thread("f", { accountId: "work", toAddress: work.address }),
    ];
    expect(addressCounts(threads, icloud)).toEqual([
      { email: "maya@icloud.com", count: 1 },
      { email: "shop@mayaross.me", count: 2 },
      { email: "hi@rossstudio.art", count: 1 },
      { email: "maya@mayaross.me", count: 0 },
    ]);
  });

  it("is empty for an account with one address and no mail to others", () => {
    expect(addressCounts([thread("f", { toAddress: work.address })], work)).toEqual([]);
  });
});
