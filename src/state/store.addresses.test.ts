/** The store's side of your addresses: replying from the right one, choosing From, the address row, blocking. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Account, AccountAddress } from "../data/types";
import { setMail, demoMail } from "../services/mail";
import { fakeMail } from "../test/fakeMail";
import { jonas, lena, message, person, resetStore, thread, work } from "../test/fixtures";
import { useSignatures } from "./signatures";
import { contactsFrom, splitCountsFor, unreadCountsFor, useStore, visibleThreads } from "./store";

const s = () => useStore.getState();

const address = (email: string, extra: Partial<AccountAddress> = {}): AccountAddress => ({
  email,
  state: "added",
  source: "you",
  seen: 0,
  blocked: false,
  ...extra,
});

const me = person("Maya Ross", "maya@icloud.com");
const studioMe = person("Maya Ross", "hi@rossstudio.art");

const icloud: Account = {
  id: "icloud",
  name: "iCloud",
  letter: "I",
  address: "maya@icloud.com",
  senderName: "Maya Ross",
  color: "sky",
  addresses: [
    address("hi@rossstudio.art", { name: "Ross Studio" }),
    address("shop@mayaross.me", { state: "suggested", source: "mail", seen: 2 }),
  ],
};

const toStudio = thread("studio", {
  accountId: "icloud",
  toAddress: "hi@rossstudio.art",
  messages: [message({ to: [studioMe, jonas], cc: [me, lena] })],
});
const toShop = thread("shop", { accountId: "icloud", toAddress: "shop@mayaross.me", unread: true });
const toMe = thread("me", { accountId: "icloud", toAddress: "maya@icloud.com" });

beforeEach(() => resetStore([toStudio, toShop, toMe], [work, icloud]));
afterEach(() => setMail(demoMail));

const account = () => s().accounts.find((a) => a.id === "icloud")!;

describe("replying from the address it came to", () => {
  it("a reply goes from the address the email came to, and says so", () => {
    s().compose("reply", "studio");
    expect(s().draft).toMatchObject({ fromAccountId: "icloud", fromAddress: "hi@rossstudio.art", fromReason: "It came to hi@rossstudio.art" });
  });

  it("reply all leaves out every address of yours", () => {
    s().compose("reply-all", "studio");
    expect(s().draft).toMatchObject({ to: [toStudio.from, jonas], cc: [lena] });
  });

  it("mail to an address you don't send from yet is answered from the account's own, with why", () => {
    s().compose("reply", "shop");
    expect(s().draft?.fromAddress).toBeUndefined();
    expect(s().draft?.fromReason).toBe("It came to shop@mayaross.me. Add it in Settings to reply from it");
  });

  it("a new email goes from the address you are looking at", () => {
    s().setAccountFilter("icloud");
    s().setAddressFilter("hi@rossstudio.art");
    s().compose("new");
    expect(s().draft).toMatchObject({ fromAccountId: "icloud", fromAddress: "hi@rossstudio.art" });
  });

  it("a new email goes from the account's own address when you look at one you can't send from", () => {
    s().setAccountFilter("icloud");
    s().setAddressFilter("shop@mayaross.me");
    s().compose("new");
    expect(s().draft?.fromAddress).toBeUndefined();
  });
});

describe("choosing From", () => {
  beforeEach(() => {
    useSignatures.setState({
      signatures: [
        { id: "studio", name: "Studio", lines: ["Maya Ross", "Ross Studio"] },
        { id: "plain", name: "Plain", lines: ["Maya"] },
      ],
      byAccount: { icloud: { newEmails: "plain", replies: "plain" }, work: { newEmails: null, replies: null } },
      byAddress: { "hi@rossstudio.art": { newEmails: "studio", replies: "studio" } },
    });
  });

  it("the picker opens and closes", () => {
    s().compose("new");
    s().toggleFromPicker();
    expect(s().draft?.fromOpen).toBe(true);
    s().toggleFromPicker();
    expect(s().draft?.fromOpen).toBe(false);
  });

  it("picking an address in another account moves the email there, with that address's signature", () => {
    s().compose("new");
    expect(s().draft).toMatchObject({ fromAccountId: "work", signatureId: null });
    s().toggleFromPicker();
    s().chooseFrom("icloud", "hi@rossstudio.art");
    expect(s().draft).toMatchObject({ fromAccountId: "icloud", fromAddress: "hi@rossstudio.art", fromOpen: false, signatureId: "studio" });
    expect(s().draft?.fromReason).toBeUndefined();
  });

  it("picking the account's own address clears the other one", () => {
    s().compose("reply", "studio");
    expect(s().draft?.signatureId).toBe("studio");
    s().chooseFrom("icloud", "maya@icloud.com");
    expect(s().draft).toMatchObject({ fromAccountId: "icloud", signatureId: "plain" });
    expect(s().draft?.fromAddress).toBeUndefined();
  });

  it("a signature you picked stays when From changes", () => {
    s().compose("new");
    s().chooseSignature("plain");
    s().chooseFrom("icloud", "hi@rossstudio.art");
    expect(s().draft?.signatureId).toBe("plain");
  });

  it("does nothing without a draft", () => {
    s().chooseFrom("icloud", "hi@rossstudio.art");
    s().toggleFromPicker();
    expect(s().draft).toBeUndefined();
  });
});

describe("the address row", () => {
  it("narrows the inbox and its counts to one address, and picking an account clears it", () => {
    s().setAccountFilter("icloud");
    s().setAddressFilter("shop@mayaross.me");
    expect(visibleThreads(s()).map((t) => t.id)).toEqual(["shop"]);
    expect(s().cursorId).toBe("shop");
    expect(splitCountsFor(s().threads, "icloud", "shop@mayaross.me").important).toBe(1);
    expect(unreadCountsFor(s().threads, "icloud", "hi@rossstudio.art").important).toBe(0);
    expect(unreadCountsFor(s().threads, "icloud", "shop@mayaross.me").important).toBe(1);
    s().setAddressFilter(undefined);
    expect(visibleThreads(s())).toHaveLength(3);
    s().setAddressFilter("hi@rossstudio.art");
    s().setAccountFilter("work");
    expect(s().addressFilter).toBeUndefined();
  });

  it("only applies inside an account", () => {
    s().setAddressFilter("hi@rossstudio.art");
    expect(s().addressFilter).toBeUndefined();
  });
});

describe("your addresses", () => {
  it("adding one makes it an address you send from, and saves it", async () => {
    const { backend } = fakeMail({ addAddress: vi.fn(async () => undefined) });
    s().addAddress("icloud", " Maya@MayaRoss.me ");
    expect(account().addresses?.at(-1)).toEqual(address("maya@mayaross.me"));
    expect(backend.addAddress).toHaveBeenCalledWith("icloud", "maya@mayaross.me", undefined);
  });

  it("adding a suggested one turns it into one you send from", () => {
    s().addAddress("icloud", "shop@mayaross.me");
    expect(account().addresses?.find((a) => a.email === "shop@mayaross.me")).toMatchObject({ state: "added", source: "mail", seen: 2 });
  });

  it("an address can come with its own name, and an account's first address starts its list", () => {
    s().addAddress("work", "design@northwind.co", "  Northwind Design ");
    expect(s().accounts[0].addresses).toEqual([address("design@northwind.co", { name: "Northwind Design" })]);
  });

  it("removing a blocked address keeps it blocked, as one you said no to", () => {
    s().blockAddress("icloud", "shop@mayaross.me", true);
    s().removeAddress("icloud", "shop@mayaross.me");
    expect(account().addresses?.find((a) => a.email === "shop@mayaross.me")).toMatchObject({ state: "dismissed", blocked: true });
  });

  it("the account's own address, or one that isn't an address, isn't added", () => {
    s().addAddress("icloud", "maya@icloud.com");
    s().addAddress("icloud", "not an address");
    expect(account().addresses).toHaveLength(2);
  });

  it("the engine's answer replaces the guess", async () => {
    const saved = { ...icloud, addresses: [address("maya@mayaross.me", { name: "Maya" })] };
    fakeMail({ addAddress: vi.fn(async () => saved) });
    s().addAddress("icloud", "maya@mayaross.me");
    await vi.waitFor(() => expect(account()).toEqual(saved));
  });

  it("when saving fails, the account goes back and a note says so", async () => {
    fakeMail({ addAddress: vi.fn(async () => Promise.reject(new Error("no"))) });
    vi.spyOn(console, "warn").mockImplementation(() => {});
    s().addAddress("icloud", "maya@mayaross.me");
    await vi.waitFor(() => expect(s().toast).toMatchObject({ text: "Couldn’t save that, so Moss put it back", problem: true }));
    expect(account()).toEqual(icloud);
  });

  it("renaming one changes the name mail from it shows; an empty name means the account's", () => {
    const { backend } = fakeMail({ updateAddress: vi.fn(async () => undefined) });
    s().renameAddress("icloud", "hi@rossstudio.art", "Studio");
    expect(account().addresses?.[0].name).toBe("Studio");
    s().renameAddress("icloud", "hi@rossstudio.art", "  ");
    expect(account().addresses?.[0].name).toBeUndefined();
    expect(backend.updateAddress).toHaveBeenLastCalledWith("icloud", "hi@rossstudio.art", undefined);
  });

  it("removing one, or saying no to a suggestion, takes it off the list and out of the address row", () => {
    const { backend } = fakeMail({ removeAddress: vi.fn(async () => undefined) });
    s().setAccountFilter("icloud");
    s().setAddressFilter("hi@rossstudio.art");
    s().removeAddress("icloud", "hi@rossstudio.art");
    s().removeAddress("icloud", "shop@mayaross.me");
    expect(account().addresses).toEqual([]);
    expect(s().addressFilter).toBeUndefined();
    expect(backend.removeAddress).toHaveBeenCalledTimes(2);
  });

  it("looking for addresses asks the engine and shows what it found", async () => {
    const found = { ...icloud, addresses: [...icloud.addresses!, address("new@mayaross.me", { state: "suggested", source: "mail", seen: 1 })] };
    fakeMail({ discoverAddresses: vi.fn(async () => found) });
    s().discoverAddresses("icloud");
    await vi.waitFor(() => expect(account().addresses).toHaveLength(3));
  });

  it("an unknown account changes nothing", () => {
    s().addAddress("nope", "a@b.co");
    s().removeAddress("nope", "a@b.co");
    expect(s().accounts).toEqual([work, icloud]);
  });

  it("you are never suggested as a recipient, from any of your addresses", () => {
    const threads = [thread("x", { accountId: "icloud", messages: [message({ from: studioMe, to: [me, jonas] })] })];
    expect(contactsFrom(threads, [icloud]).map((p) => p.email)).toEqual([jonas.email]);
  });
});

describe("blocking an address", () => {
  it("sends the mail that came to it to Blocked, and new mail will follow", () => {
    const { backend } = fakeMail({ blockAddress: vi.fn(async () => undefined) });
    s().blockAddress("icloud", "shop@mayaross.me", true);
    expect(s().threads.map((t) => t.id)).toEqual(["studio", "me"]);
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["shop"]);
    expect(s().blockedThreads[0].sender).toBe("blocked");
    expect(account().addresses?.[1].blocked).toBe(true);
    expect(s().toast?.text).toBe("Mail to shop@mayaross.me goes to Blocked now");
    expect(backend.blockAddress).toHaveBeenCalledWith("icloud", "shop@mayaross.me", true);
  });

  it("the selection moves on when the email under it goes to Blocked, and stays when it doesn't", () => {
    s().setCursor("shop");
    s().blockAddress("icloud", "shop@mayaross.me", true);
    expect(s().cursorId).toBe("me");
    s().setCursor("studio");
    s().blockAddress("icloud", "nothing@mayaross.me", true);
    expect(s().cursorId).toBe("studio");
  });

  it("done mail to it goes to Blocked too", () => {
    useStore.setState({ doneThreads: [thread("old-shop", { accountId: "icloud", toAddress: "shop@mayaross.me", doneAt: 1 }), thread("kept", { doneAt: 1 })] });
    s().blockAddress("icloud", "shop@mayaross.me", true);
    expect(s().doneThreads.map((t) => t.id)).toEqual(["kept"]);
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["shop", "old-shop"]);
    expect(s().blockedThreads[1]).not.toHaveProperty("doneAt");
  });

  it("an address with no entry yet gets one, so it can be unblocked", () => {
    resetStore([thread("x", { accountId: "icloud", toAddress: "leak@mayaross.me" })], [icloud]);
    s().blockAddress("icloud", "leak@mayaross.me", true);
    expect(account().addresses?.at(-1)).toEqual(address("leak@mayaross.me", { state: "dismissed", source: "mail", blocked: true }));
  });

  it("unblocking brings its mail back to the inbox", () => {
    s().blockAddress("icloud", "shop@mayaross.me", true);
    s().blockAddress("icloud", "shop@mayaross.me", false);
    expect(s().threads.map((t) => t.id)).toContain("shop");
    expect(s().blockedThreads).toEqual([]);
    expect(account().addresses?.[1].blocked).toBe(false);
    expect(s().toast?.text).toBe("Mail to shop@mayaross.me is back in your inbox");
  });

  it("unblocking an email blocked for its address lifts that block too, in its account only", () => {
    s().blockAddress("icloud", "shop@mayaross.me", true);
    const elsewhere = thread("work-shop", { accountId: "work", toAddress: "shop@mayaross.me", from: jonas, sender: "blocked" });
    const bySender = thread("by-sender", { accountId: "icloud", toAddress: "maya@icloud.com", from: jonas, sender: "blocked" });
    useStore.setState((x) => ({ blockedThreads: [...x.blockedThreads, elsewhere, bySender] }));
    s().unblockSender("shop");
    expect(account().addresses?.[1].blocked).toBe(false);
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["work-shop", "by-sender"]);
  });

  it("can't block an address of an account that isn't there", () => {
    s().blockAddress("nope", "shop@mayaross.me", true);
    expect(s().blockedThreads).toEqual([]);
  });
});
