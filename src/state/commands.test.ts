import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { lena, resetStore, thread, work } from "../test/fixtures";
import { contextThread, paletteGroups } from "./commands";
import { SEP } from "../lib/text";
import { useDrafts, type SavedDraft } from "./drafts";
import { useScheduled, type ScheduledEmail } from "./scheduled";
import { resetAskPanel, turnsOf, useAskPanel } from "./askPanel";
import { useSearch } from "./search";
import { useStore } from "./store";
import { useTrash } from "./trash";
import { pristineUpdates, useUpdates } from "./updates";
import { installedMoss, mossDev, useAppIdentity } from "../services/appIdentity";

const s = () => useStore.getState();
const items = (q: string, title: string) => paletteGroups(q).find((g) => g.title === title)!.items;
const group = (q: string, title: string) => items(q, title).map((c) => c.id);
const run = (q: string, title: string, id: string) => items(q, title).find((c) => c.id === id)!.run();

// Tuesday 3 November 2026, 1:20 PM: Thursday's deadline is still two days off.
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 10, 3, 13, 20));
  resetStore([thread("a", { subject: "Q4 budget", askDue: "Thursday", from: lena }), thread("b")]);
});
afterEach(() => vi.useRealTimers());

describe("contextThread", () => {
  it("is the email under the cursor, the open one, or none at inbox zero", () => {
    s().setCursor("b");
    expect(contextThread()?.id).toBe("b");
    s().open("a");
    expect(contextThread()?.id).toBe("a");
    useStore.setState({ view: "zero" });
    expect(contextThread()).toBeUndefined();
  });

  it("is the selected email in the Blocked list", () => {
    useStore.setState({ blockedThreads: [thread("x", { sender: "blocked" })] });
    s().openBlocked();
    expect(contextThread()?.id).toBe("x");
  });

  it("is the selected email in the Done list", () => {
    useStore.setState({ doneThreads: [thread("x", { doneAt: 1 })] });
    s().openDone();
    expect(contextThread()?.id).toBe("x");
  });

  it("is the selected result in Search, when a list holds its email", () => {
    s().openSearch();
    const doc = { id: "m-b", threadId: "b", accountId: "work", from: lena, subject: "B", summary: "", when: "", date: "2026-11-01", hasAttachment: false, relevance: 1 };
    useSearch.setState({ hits: [{ doc, subject: [], summary: [], best: true }], cursorId: "m-b" });
    expect(contextThread()?.id).toBe("b");
    // Search deletes its own way (⌘⌫), taking the result out too.
    expect(group("delete", "Commands")).not.toContain("delete");
    useSearch.setState({ hits: [{ doc: { ...doc, threadId: undefined }, subject: [], summary: [], best: true }] });
    expect(contextThread()).toBeUndefined();
  });

  it("is the done email you are reading", () => {
    useStore.setState({ doneThreads: [thread("x", { doneAt: 1 })] });
    s().openDone();
    s().open("x");
    expect(contextThread()?.id).toBe("x");
  });
});

describe("paletteGroups", () => {
  it("asks AI only once you type", () => {
    expect(group("", "Ask AI")).toEqual([]);
    expect(group("did dana send it", "Ask AI")).toEqual(["ask-moss", "ask-email"]);
    expect(group("remind before the deadline", "Ask AI")).toEqual(["ask-moss", "ask-email", "ai-remind"]);
  });

  it("Ask Moss asks in the Ask panel (↵), or opens the email under the cursor and asks about it (⇧↵)", async () => {
    resetAskPanel();
    const [all, email] = items("did dana send it", "Ask AI");
    expect(all).toMatchObject({ label: "Ask Moss: “did dana send it”", sub: "About the email you’re reading, or any other", shortcut: "↵", ai: true });
    expect(email).toMatchObject({ label: "Ask about Q4 budget", sub: "Opens it, then asks Moss", shortcut: "⇧↵" });
    all.run();
    expect(useAskPanel.getState().open).toBe(true);
    expect(useStore.getState().view, "it stays on the list").toBe("inbox");
    expect(turnsOf(useAskPanel.getState())[0].question).toBe("did dana send it");
    resetAskPanel();
    const cursor = useStore.getState().cursorId;
    email.run();
    // From the list, it opens the email first, so the question is about it.
    expect(useStore.getState()).toMatchObject({ view: "thread", openThreadId: cursor });
    expect(useAskPanel.getState().open).toBe(true);
    expect(turnsOf(useAskPanel.getState())[0].question).toBe("did dana send it");
    // Already reading it, ↵ asks about it: there's nothing more to offer.
    expect(group("is it signed", "Ask AI")).toEqual(["ask-moss"]);
    useStore.setState({ view: "zero" });
    expect(group("anything", "Ask AI")).toEqual(["ask-moss"]);
  });

  it("keeps actions on the email visible whatever you type", () => {
    const ids = group("theme", "Commands");
    expect(ids).toEqual(["remind", "remind-no-reply", "move-out", "theme"]);
  });

  it("moves the email to Important, or out of it, with M's menu", () => {
    useStore.setState({ threads: [thread("r", { split: "receipts", section: "info" }), ...s().threads] });
    s().setSplit("receipts");
    expect(items("", "Commands").find((c) => c.id === "move-important")).toMatchObject({ label: "Move to Important", shortcut: "M" });
    run("", "Commands", "move-important");
    expect(s().threads.find((t) => t.id === "r")).toMatchObject({ split: "important", movedByYou: true });
    s().setSplit("important");
    s().setCursor("r");
    expect(items("", "Commands").find((c) => c.id === "move-out")).toMatchObject({ label: "Move out of Important…", shortcut: "M" });
    run("", "Commands", "move-out");
    expect(s().movingId).toBe("r");
  });

  it("offers each other tab, and Moss's sorting back, once you type", () => {
    expect(group("", "Commands")).not.toContain("move-other");
    expect(group("move", "Commands")).toEqual(expect.arrayContaining(["move-other", "move-calendar", "move-newsletters", "move-receipts"]));
    expect(group("move", "Commands")).not.toContain("move-important");
    expect(group("newsletters", "Commands")).toContain("move-newsletters");
    run("newsletters", "Commands", "move-newsletters");
    expect(s().threads[0]).toMatchObject({ split: "newsletters", movedByYou: true });
    s().setCursor("a");
    expect(group("move", "Commands")).not.toContain("move-newsletters");
    expect(group("move", "Commands")).toContain("move-moss");
    run("move", "Commands", "move-moss");
    expect(s().threads[0].movedByYou).toBeUndefined();
  });

  it("offers marking the email read or unread once you type it", () => {
    expect(group("", "Commands")).not.toContain("read");
    const [unread] = items("unread", "Commands").filter((c) => c.id === "read");
    expect(unread).toMatchObject({ label: "Mark as unread", shortcut: "U" });
    unread.run();
    expect(s().threads[0].unread).toBe(true);
    const read = items("read", "Commands").find((c) => c.id === "read")!;
    expect(read.label).toBe("Mark as read");
    read.run();
    expect(s().threads[0].unread).toBe(false);
  });

  it("has no email actions when there is no email in context", () => {
    resetStore([]);
    expect(group("", "Commands")).toEqual(["new", "search", "done", "zero", "add-account", "settings", "signatures", "addresses", "agents", "shortcuts", "updates", "theme"]);
    expect(group("delete", "Commands")).not.toContain("delete");
  });

  it("checks for updates in the update window, installs one that's offered, and restarts into one that's ready", () => {
    useUpdates.setState(pristineUpdates);
    const check = vi.fn(() => Promise.resolve());
    const install = vi.fn(() => Promise.resolve());
    const restart = vi.fn(() => Promise.resolve());
    useUpdates.setState({ check, install, restart });
    const label = () => items("update", "Commands").find((c) => c.id === "updates")?.label;
    expect(label()).toBe("Check for updates");
    run("update", "Commands", "updates");
    expect(check).toHaveBeenCalledWith({ asked: true, open: true });
    for (const update of [{ state: "available", version: "0.2.0" }, { state: "failed", during: "download", message: "cut off", version: "0.2.0" }] as const) {
      useUpdates.setState({ update });
      expect(label()).toBe("Update to Moss 0.2.0");
      run("update", "Commands", "updates");
    }
    expect(install).toHaveBeenCalledTimes(2);
    useUpdates.setState({ update: { state: "failed", during: "check", message: "offline" } });
    expect(label()).toBe("Check for updates");
    useUpdates.setState({ update: { state: "ready", version: "0.2.0" } });
    expect(label()).toBe("Restart to update to Moss 0.2.0");
    run("update", "Commands", "updates");
    expect(restart).toHaveBeenCalledOnce();
    useUpdates.setState(pristineUpdates);
  });

  it("has no update command in Moss Dev, which never updates", () => {
    useAppIdentity.setState(mossDev);
    expect(items("update", "Commands").find((c) => c.id === "updates")).toBeUndefined();
    useAppIdentity.setState(installedMoss);
  });

  it("offers “Match the system theme” only once a theme is pinned", () => {
    expect(group("theme", "Commands")).not.toContain("theme-system");
    s().setTheme("light");
    expect(group("theme", "Commands")).toContain("theme-system");
  });

  it("filters Go to, but shows every account when nothing matches", () => {
    expect(group("personal", "Go to")).toEqual(["go-personal"]);
    expect(group("zzzz", "Go to")).toEqual(["go-all", "go-work", "go-personal", "go-drafts", "go-scheduled", "go-trash", "go-blocked", "go-senders"]);
    expect(group("blocked", "Go to")).toEqual(["go-blocked", "go-senders"]);
    expect(group("senders", "Go to")).toEqual(["go-senders"]);
  });

  it("numbers the first nine accounts ⌘1–⌘9", () => {
    const go = paletteGroups("").find((g) => g.title === "Go to")!.items;
    expect(go.map((c) => c.shortcut)).toEqual(["⌘0", "⌘1", "⌘2", undefined, undefined, undefined, "⌘⇧B", undefined]);
  });


  it("runs the command it shows", () => {
    run("", "Go to", "go-personal");
    expect(s().accountFilter).toBe("personal");
    run("", "Go to", "go-all");
    expect(s().accountFilter).toBe("all");
  });

  it("offers moving the email to Trash once you type it", () => {
    expect(group("", "Commands")).not.toContain("delete");
    for (const q of ["delete", "trash", "remove this"]) expect(group(q, "Commands")).toContain("delete");
    expect(items("delete", "Commands").find((c) => c.id === "delete")).toMatchObject({ label: "Delete (move to Trash)", shortcut: "⌫" });
    s().setCursor("b");
    run("delete", "Commands", "delete");
    expect(s().threads.map((t) => t.id)).not.toContain("b");
  });

  it("offers inbox zero on Z", () => {
    expect(items("zero", "Commands").find((c) => c.id === "zero")).toMatchObject({ label: "Show inbox zero", shortcut: "Z" });
    run("zero", "Commands", "zero");
    expect(s().toast).toMatchObject({ text: expect.stringMatching(/still waiting/) });
  });

  it("Remind me… opens the Remind menu for the email in context", () => {
    s().setCursor("b");
    run("", "Commands", "remind");
    expect(s().remindingId).toBe("b");
    expect(s().threads.find((t) => t.id === "b")?.section).not.toBe("later");
  });

  it("the AI suggestion snoozes until 4 PM the day before the deadline, and says when", () => {
    s().setCursor("a");
    expect(items("remind before the deadline", "Ask AI").find((c) => c.id === "ai-remind")!.label).toBe("Remind me Wed 4 PM");
    run("remind before the deadline", "Ask AI", "ai-remind");
    expect(s().threads.find((t) => t.id === "a")?.section).toBe("later");
    expect(s().toast).toMatchObject({ text: "Coming back Wed 4 PM" });
  });

  it("a deadline Moss found counts from its real day", () => {
    resetStore([thread("a", { askDue: "Due Thu", dueDate: "2026-11-05", from: lena })]);
    expect(items("remind before the deadline", "Ask AI").find((c) => c.id === "ai-remind")!.label).toBe("Remind me Wed 4 PM");
    resetStore([thread("a", { askDue: "Overdue", dueDate: "2026-11-01", from: lena })]);
    expect(group("remind before the deadline", "Ask AI")).not.toContain("ai-remind");
  });

  it("suggests no reminder when the day before the deadline has gone, or it names no day", () => {
    vi.setSystemTime(new Date(2026, 10, 4, 17, 0)); // Wednesday 5 PM
    expect(group("remind before the deadline", "Ask AI")).not.toContain("ai-remind");
    resetStore([thread("a", { askDue: "Soon", from: lena })]);
    expect(group("remind before the deadline", "Ask AI")).not.toContain("ai-remind");
  });

  it("starts a new email or a reply", () => {
    run("", "Commands", "reply");
    expect(s().draft).toMatchObject({ mode: "reply", threadId: "a" });
    run("", "Commands", "new");
    expect(s().draft?.mode).toBe("new");
  });

  it("switches theme both ways, and offers no font switch", () => {
    run("theme", "Commands", "theme");
    expect(s().theme).toBe("light");
    expect(items("theme", "Commands").find((c) => c.id === "theme")?.label).toBe("Switch to dark theme");
    run("theme", "Commands", "theme");
    expect(s().theme).toBe("dark");
    run("theme", "Commands", "theme-system");
    expect(s().themeChoice).toBe("system");
    expect(group("font", "Commands")).not.toContain("font");
  });

  it("opens Search, Add account and each part of Settings", () => {
    run("search", "Commands", "search");
    expect(s().view).toBe("search");
    run("done", "Commands", "done");
    expect(s().view).toBe("done");
    run("account", "Commands", "add-account");
    expect(s().view).toBe("add-account");
    run("settings", "Commands", "settings");
    expect(s()).toMatchObject({ view: "settings", settingsSection: "accounts" });
    run("signatures", "Commands", "signatures");
    expect(s().settingsSection).toBe("signatures");
    run("shortcuts", "Commands", "shortcuts");
    expect(s().settingsSection).toBe("keyboard");
    run("mcp", "Commands", "agents");
    expect(s()).toMatchObject({ view: "settings", settingsSection: "ai" });
  });

  it("real mail offers Remind me if no reply only on an email you wrote last", () => {
    resetStore([thread("r", { from: lena, youWroteLast: false })]);
    expect(group("", "Commands")).not.toContain("remind-no-reply");
  });

  it("Remind me if no reply waits on your latest email", () => {
    resetStore([thread("w", { from: lena, youWroteLast: true, lastMessageAt: Date.now() })]);
    run("", "Commands", "remind-no-reply");
    expect(s().threads[0]).toMatchObject({ section: "later", waiting: { why: { by: "you" } } });
  });


  it("has no shortcut past the ninth account", () => {
    const many = Array.from({ length: 10 }, (_, i) => ({ ...work, id: `acct${i}`, name: `A${i}` }));
    resetStore([], many);
    expect(items("", "Go to").find((c) => c.id === "go-acct9")?.shortcut).toBeUndefined();
  });
});

describe("senders in the palette", () => {
  const jordan = { name: "Jordan Blake", email: "jordan@growthleads.example", initials: "JB" };

  it("a new sender's email offers Block and Accept, for them or everyone at the domain, first", () => {
    resetStore([thread("n", { from: jordan, sender: "new" }), thread("b")]);
    s().open("n");
    expect(items("block", "Commands").slice(0, 4).map((c) => [c.label, c.shortcut])).toEqual([
      ["Block Jordan Blake", "B"],
      ["Block everyone at growthleads.example", "⇧B"],
      ["Accept Jordan Blake", "A"],
      ["Accept everyone at growthleads.example", "⇧A"],
    ]);
    run("", "Commands", "accept-domain");
    expect(s().toast?.text).toBe("Accepted everyone at growthleads.example");
    useStore.setState({ threads: [thread("n", { from: jordan, sender: "new" }), thread("b")] });
    run("", "Commands", "accept");
    expect(s().threads[0].sender).toBe("known");
    useStore.setState({ threads: [thread("n", { from: jordan, sender: "new" }), thread("b")] });
    run("", "Commands", "block-domain");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n"]);
  });

  it("someone you know can be blocked when you ask for it, without keys", () => {
    resetStore([thread("n", { from: jordan, sender: "known" })]);
    expect(group("", "Commands")).not.toContain("block");
    expect(items("block", "Commands").filter((c) => c.id.startsWith("block")).map((c) => [c.id, c.shortcut])).toEqual([
      ["block", undefined],
      ["block-domain", undefined],
    ]);
    run("block", "Commands", "block");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n"]);
  });

  it("never offers a public domain", () => {
    resetStore([thread("n", { from: { name: "Pat", email: "pat@gmail.com", initials: "P" }, sender: "new" })]);
    expect(group("block", "Commands")).not.toContain("block-domain");
    expect(group("", "Commands")).not.toContain("accept-domain");
  });

  it("Go to opens Trash, where Back to inbox puts the email in context back", () => {
    expect(items("trash", "Go to")[0]).toMatchObject({ id: "go-trash", label: "Trash (deleted emails)" });
    expect(group("deleted", "Go to")).toEqual(["go-trash"]);
    expect(group("", "Commands")).not.toContain("restore");
    useTrash.setState({ threads: [thread("x", { subject: "Old invoice" })] });
    run("", "Go to", "go-trash");
    expect(s().view).toBe("trash");
    expect(items("", "Commands")[0]).toMatchObject({ id: "restore", label: "Back to inbox", shortcut: "E" });
    run("", "Commands", "restore");
    expect(s().threads.map((t) => t.id)).toContain("x");
    expect(group("", "Commands")).not.toContain("restore");
  });

  it("blocked mail offers Unblock, and Go to opens the Blocked list", () => {
    useStore.setState({ blockedThreads: [thread("x", { from: jordan, sender: "blocked" })] });
    s().openBlocked();
    s().open("x");
    expect(items("", "Commands")[0]).toMatchObject({ id: "unblock", label: "Unblock Jordan Blake", shortcut: "U" });
    expect(group("block", "Commands")).not.toContain("block");
    run("", "Commands", "unblock");
    expect(s().threads.map((t) => t.id)).toContain("x");
    expect(items("blocked", "Go to")[0]).toMatchObject({ label: "Blocked mail", shortcut: "⌘⇧B" });
    run("", "Go to", "go-blocked");
    expect(s().view).toBe("blocked");
    expect(items("senders", "Go to")[0]).toMatchObject({ label: "Blocked and accepted senders" });
    run("senders", "Go to", "go-senders");
    expect(s().view).toBe("senders");
    s().leave();
    expect(s().view).toBe("blocked");
  });
});

describe("drafts", () => {
  const kept = (id: string, fromAccountId = "work") => ({ ...({} as SavedDraft), id, fromAccountId, updatedAt: 1 });

  it("Drafts opens the list, and counts the drafts of the account you are looking at", () => {
    const label = () => items("draft", "Go to").find((c) => c.id === "go-drafts")!.label;
    expect(label()).toBe("Drafts");
    useDrafts.setState({ drafts: [kept("a"), kept("b", "personal")] });
    expect(label()).toBe(`Drafts${SEP}2`);
    s().setAccountFilter("personal");
    expect(label()).toBe(`Drafts${SEP}1`);
    run("drafts", "Go to", "go-drafts");
    expect(s().view).toBe("drafts");
  });
});

describe("send later", () => {
  const waiting = (id: string, fromAccountId = "work") => ({ draft: { id, fromAccountId }, sendAt: 1, nextTry: 1, tries: 0 }) as ScheduledEmail;

  it("Scheduled opens the list, and counts what waits in the account you are looking at", () => {
    useScheduled.setState({ emails: [] });
    const label = () => items("sched", "Go to").find((c) => c.id === "go-scheduled")!.label;
    expect(label()).toBe("Scheduled");
    useScheduled.setState({ emails: [waiting("a"), waiting("b", "personal")] });
    expect(label()).toBe(`Scheduled${SEP}2`);
    s().setAccountFilter("personal");
    expect(label()).toBe(`Scheduled${SEP}1`);
    run("sched", "Go to", "go-scheduled");
    expect(s().view).toBe("scheduled");
    useScheduled.setState({ emails: [] });
  });

  it("Send later is a command while you write", () => {
    expect(items("send later", "Commands").map((c) => c.id)).not.toContain("send-later");
    s().compose("reply", "a");
    s().updateDraft({ to: [lena] });
    const command = items("send later", "Commands").find((c) => c.id === "send-later")!;
    expect(command).toMatchObject({ label: "Send later…", shortcut: "⌘⇧↵" });
    command.run();
    expect(useScheduled.getState().picking).toEqual({ kind: "compose" });
    useScheduled.setState({ picking: undefined });
  });
});

describe("fallbacks", () => {
  it("uses the full name when the sender has no short name, and has no AI without an email", () => {
    resetStore([thread("a", { askDue: "Thursday", from: { ...lena, short: undefined } })]);
    expect(items("remind before the deadline", "Ask AI").find((c) => c.id === "ai-remind")!.sub).toBe("One day before Lena Vogt’s Thursday deadline");
    resetStore([]);
    expect(group("summarize", "Ask AI")).toEqual(["ask-moss"]);
  });
});

describe("your addresses", () => {
  const icloud = {
    ...work,
    id: "icloud",
    name: "iCloud",
    address: "maya@icloud.com",
    addresses: [
      { email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 1, blocked: false },
      { email: "old@mayaross.me", state: "dismissed" as const, source: "mail" as const, seen: 1, blocked: true },
    ],
  };

  it("an email to an address you don't send from offers sending from it or blocking it, once you type", () => {
    resetStore([thread("x", { accountId: "icloud", toAddress: "shop@mayaross.me" })], [icloud]);
    expect(group("", "Commands")).not.toContain("address-block");
    expect(group("block", "Commands")).toContain("address-block");
    expect(items("shop", "Commands").map((c) => c.label)).toEqual(expect.arrayContaining(["Send from shop@mayaross.me", "Block mail to shop@mayaross.me"]));
    run("send from", "Commands", "address-add");
    expect(s().accounts[0].addresses?.at(-1)).toMatchObject({ email: "shop@mayaross.me", state: "added" });
    expect(group("block", "Commands")).not.toContain("address-block");
  });

  it("blocking an address from the palette sends its mail to Blocked", () => {
    resetStore([thread("x", { accountId: "icloud", toAddress: "shop@mayaross.me" })], [icloud]);
    run("block", "Commands", "address-block");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["x"]);
  });

  it("mail blocked for its address offers unblocking the address", () => {
    resetStore([], [icloud]);
    useStore.setState({ blockedThreads: [thread("x", { accountId: "icloud", toAddress: "old@mayaross.me", sender: "blocked" })] });
    s().openBlocked();
    expect(items("", "Commands").map((c) => c.label)).toContain("Unblock mail to old@mayaross.me");
    run("", "Commands", "address-unblock");
    expect(s().accounts[0].addresses?.some((a) => a.blocked)).toBe(false);
    expect(s().threads.map((t) => t.id)).toEqual(["x"]);
  });

  it("mail to your own address, or one you send from, offers nothing about it", () => {
    resetStore([thread("x", { accountId: "icloud", toAddress: "hi@rossstudio.art" }), thread("y", { accountId: "icloud" })], [icloud]);
    expect(group("address", "Commands").filter((id) => id.startsWith("address-"))).toEqual([]);
    s().setCursor("y");
    expect(group("address", "Commands").filter((id) => id.startsWith("address-"))).toEqual([]);
  });

  it("Go to offers each address of the account you are looking at, with its key", () => {
    resetStore([thread("x", { accountId: "icloud", toAddress: "hi@rossstudio.art" })], [icloud]);
    expect(group("", "Go to")).not.toContain("go-address-hi@rossstudio.art");
    s().setAccountFilter("icloud");
    const go = items("rossstudio", "Go to").find((c) => c.id === "go-address-hi@rossstudio.art")!;
    expect(go).toMatchObject({ label: "Only hi@rossstudio.art", shortcut: "⌥2" });
    go.run();
    expect(s().addressFilter).toBe("hi@rossstudio.art");
    const all = items("addresses", "Go to").find((c) => c.id === "go-address-all")!;
    expect(all.label).toBe("All of iCloud’s addresses");
    all.run();
    expect(s().addressFilter).toBeUndefined();
  });

  it("has no key past the ninth address", () => {
    const many = { ...icloud, addresses: Array.from({ length: 10 }, (_, i) => ({ email: `a${i}@x.io`, state: "added" as const, source: "you" as const, seen: 0, blocked: false })) };
    resetStore([], [many]);
    s().setAccountFilter("icloud");
    expect(items("a9@x.io", "Go to").find((c) => c.id === "go-address-a9@x.io")?.shortcut).toBeUndefined();
  });

  it("goes to the addresses in Settings", () => {
    run("addresses", "Commands", "addresses");
    expect(s()).toMatchObject({ view: "settings", settingsSection: "accounts" });
  });
});

describe("correcting what Moss decided, in the palette", () => {
  // Move out of Important… (M's menu) is always there; these are the rest.
  const corrections = (q = "") => group(q, "Commands").filter((id) => (id.startsWith("move-") && id !== "move-out") || ["no-deadline", "no-remind"].includes(id));

  it("the AI's call on needs reply comes first, to undo it at once", () => {
    resetStore([thread("a", { aiSure: 92 })]);
    expect(group("", "Commands")[0]).toBe("move-info");
    expect(items("", "Commands")[0]).toMatchObject({ label: "Move to For your information", sub: "Needs your reply · AI, 92% sure" });
    run("", "Commands", "move-info");
    expect(s().threads[0].section).toBe("info");
    resetStore([thread("a", { aiSure: 92, section: "info" })]);
    expect(items("", "Commands")[0].label).toBe("Move to Needs your reply");
  });

  it("other moves wait until you type", () => {
    resetStore([thread("a")]);
    expect(corrections()).toEqual([]);
    expect(corrections("move")).toContain("move-info");
    // Snoozed: no section to move between.
    resetStore([thread("a", { section: "later" })]);
    expect(corrections("move")).not.toContain("move-info");
  });

  it("a split the AI picked says so on the way back: Move to Important, or out of it with M's menu", () => {
    resetStore([thread("a", { split: "receipts", section: "info", splitWhy: { by: "ai", sure: 88 } })]);
    expect(corrections()).toEqual(["move-important"]);
    expect(items("", "Commands").find((c) => c.id === "move-important")).toMatchObject({ sub: "Moved to Receipts · AI, 88% sure" });
    run("", "Commands", "move-important");
    expect(s().threads[0]).toMatchObject({ split: "important", movedByYou: true });
    expect(s().threads[0].splitWhy).toBeUndefined();
    resetStore([thread("a", { splitWhy: { by: "ai", sure: 88 } })]);
    expect(items("", "Commands").find((c) => c.id === "move-out")).toMatchObject({ sub: "Moved to Important · AI, 88% sure" });
  });

  it("a deadline Moss found can go", () => {
    resetStore([thread("a", { due: "Due Fri", dueDate: "2026-11-06", dueWhy: { by: "rule", evidence: "by Friday" } })]);
    expect(corrections()).toEqual(["no-deadline"]);
    expect(items("", "Commands")[0]).toMatchObject({ label: "No deadline", sub: "Due Fri · says “by Friday”" });
    run("", "Commands", "no-deadline");
    expect(s().threads[0].dueDate).toBeUndefined();
  });

  it("an email Moss waits on can stop waiting", () => {
    resetStore([thread("a", { section: "later", youWroteLast: true, waiting: { until: Date.now() + 86_400_000, why: { by: "ai", sure: 90 } } })]);
    expect(corrections()).toEqual(["no-remind"]);
    expect(group("", "Commands")).not.toContain("remind-no-reply");
    expect(items("", "Commands")[0]).toMatchObject({ label: "Don’t remind me" });
    run("", "Commands", "no-remind");
    expect(s().threads[0].waiting).toBeUndefined();
  });
});
