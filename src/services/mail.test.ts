import { describe, expect, it, vi } from "vitest";
import type { Draft } from "../state/store";
import { demoMail, mail, quietly, setMail } from "./mail";

describe("the demo mailbox", () => {
  it("picks files with the browser's picker, and only shows them", async () => {
    const file = new File(["hi"], "notes.txt", { type: "text/plain" });
    let chosen: File[] | null = [file];
    const click = vi.spyOn(HTMLInputElement.prototype, "click").mockImplementation(function (this: HTMLInputElement) {
      expect(this).toMatchObject({ type: "file", multiple: true });
      Object.defineProperty(this, "files", { value: chosen });
      this.onchange!(new Event("change"));
    });
    expect(await demoMail.pickFiles()).toEqual([file]);
    chosen = null;
    expect(await demoMail.pickFiles()).toEqual([]);
    click.mockImplementation(function (this: HTMLInputElement) {
      this.oncancel!(new Event("cancel"));
    });
    expect(await demoMail.pickFiles()).toEqual([]);
    click.mockRestore();

    expect(await demoMail.attachFiles("d1", [file])).toEqual([{ id: expect.any(String), name: "notes.txt", bytes: 2, contentType: "text/plain" }]);
    expect(await demoMail.attachFiles("d1", ["/Users/maya/Plan.pdf"])).toEqual([{ id: expect.any(String), name: "Plan.pdf", bytes: 0, contentType: "" }]);
    expect(await demoMail.draftPreview("d1", "x")).toBeUndefined();
    await expect(demoMail.discardDraftFile("d1", "x")).resolves.toBeUndefined();
    await demoMail.deleteDraft("x", true);
  });

  it("is the default, and its actions do nothing beyond the screen", async () => {
    expect(mail()).toBe(demoMail);
    expect(demoMail.live).toBe(false);
    expect(await demoMail.accounts()).toEqual([]);
    expect(await demoMail.threads()).toEqual([]);
    expect(await demoMail.drafts()).toEqual([]);
    expect(await demoMail.scheduled()).toEqual([]);
    expect(await demoMail.rescheduleSend("x", 1, "op-x", 1)).toEqual({ status: "rescheduled", revision: 2 });
    expect(await demoMail.unscheduleSend("x", "op-x")).toEqual({ status: "cancelled", early: true });
    expect(await demoMail.contacts()).toEqual([]);
    expect(await demoMail.recipientPicks()).toEqual([]);
    await demoMail.pickRecipient("pri", "priya@acme.co");
    expect(await demoMail.thread("x")).toBeUndefined();
    expect(await demoMail.doneThreads()).toEqual([]);
    expect(await demoMail.blockedThreads()).toEqual([]);
    expect(await demoMail.trashThreads()).toEqual([]);
    await demoMail.blockSender("x", true);
    await demoMail.acceptSender("x", true);
    await demoMail.unblockSender("x");
    expect((await demoMail.senderRules()).map((r) => [r.sender, r.blocked])).toContainEqual(["accounts.google.com", false]);
    await demoMail.forgetSenderRule("work", "x@y.z");
    expect(await demoMail.syncProgress("x")).toBeUndefined();
    expect(await demoMail.attachmentPreview("x", 0)).toBeUndefined();
    expect(await demoMail.syncProblems()).toEqual([]);
    expect(await demoMail.serverChangeStatus("work")).toBeNull();
    await demoMail.retryServerChange("work", "ab", 1);
    await demoMail.discardServerChange("work", "ab", 1);
    expect(await demoMail.interruptedSends()).toEqual([]);
    await demoMail.dismissInterruptedSends(["op"]);
    expect(await demoMail.search("x", { accountId: "all", hasAttachment: false, date: "any" })).toEqual([]);
    for (const action of [demoMail.markDone, demoMail.markNotDone, demoMail.syncNow]) await expect(action("x")).resolves.toBeUndefined();
    await demoMail.markRead("x", false);
    await demoMail.snooze("x", null);
    await demoMail.moveThread("x", "other");
    expect(await demoMail.waitForReply("x", 1)).toBe(true);
    await demoMail.setSection("x", "info");
    await demoMail.setSplit("x", "other");
    await demoMail.clearDeadline("x");
    expect(await demoMail.remindIfNoReply("x", true)).toBe(true);
    await demoMail.allowImages("work", "a@b.co");
    expect(await demoMail.signatures()).toEqual({ signatures: [], byAccount: {}, byAddress: {}, letMossChoose: true, recent: [] });
    await demoMail.saveSignature({ id: "s", name: "Work", lines: [] });
    await demoMail.deleteSignature("s");
    await demoMail.setSignatureChoice("work", "replies", null);
    await demoMail.setLetMossChoose(false);
    await demoMail.markSignatureUsed("s");
    await demoMail.updateAccount("x", {});
    expect(await demoMail.removeAccount("x")).toEqual({ pending: false });
    expect(await demoMail.credentialCleanupStatus()).toEqual({ pending: false });
    expect(await demoMail.retryCredentialCleanup()).toEqual({ pending: false });
    await demoMail.reorderAccounts([]);
    await demoMail.openAttachment("x", 0);
    expect(await demoMail.loadMessage("x")).toBe(false);
    demoMail.onEvent(() => {})();
  });

  it("holds a sent email for its delay, unless Undo takes it back first", async () => {
    vi.useFakeTimers();
    const draft = { subject: "Hi" } as Draft;
    const sent = demoMail.sendAfter("a", draft, 6000);
    const undone = demoMail.sendAfter("b", draft, 6000);
    await vi.advanceTimersByTimeAsync(5000);
    expect(await demoMail.cancelSend("b")).toEqual({ cancelled: true, early: true });
    await vi.advanceTimersByTimeAsync(1000);
    expect(await sent).toBe(true);
    expect(await undone).toBe(false);
    expect(await demoMail.cancelSend("a")).toEqual({ cancelled: false, early: false });
    vi.useRealTimers();
  });

  it("quietly logs a failed action instead of breaking the screen", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    quietly(Promise.reject(new Error("offline")));
    await new Promise((r) => setTimeout(r));
    expect(warn).toHaveBeenCalledWith("Moss:", new Error("offline"));
    warn.mockRestore();
  });

  it("can be swapped for another backend", () => {
    const other = { ...demoMail, live: true };
    setMail(other);
    expect(mail()).toBe(other);
    setMail(demoMail);
    quietly(Promise.resolve());
  });
});

it("the demo has the storage drawn in the design, and frees its cache", async () => {
  const { demoMail, demoStorage } = await import("./mail");
  expect(await demoMail.storage()).toEqual(demoStorage);
  expect(await demoMail.clearCache()).toBe(142_000_000);
  await demoMail.preload(["a"]);
  await demoMail.setOffline("recent");
  await demoMail.setCacheLimit(1);
  await demoMail.setAttachmentText(false);
  await demoMail.rebuildIndex();
});

it("the demo does not pretend to create recovery files", async () => {
  expect(await demoMail.backupStatus()).toEqual({ state: "idle" });
  expect(await demoMail.createBackup()).toBeNull();
  expect(await demoMail.cancelBackup(7)).toBe(false);
});

it("the demo never claims a durable save receipt", async () => {
  const draft = { id: "demo", mode: "new" as const, fromAccountId: "work", to: [], cc: [], bcc: [], subject: "Text", body: "Text", bodyHtml: "", fieldsOpen: true, showCc: false, showBcc: false, remindIfNoReply: false, signatureId: null, updatedAt: 1 };
  expect(await demoMail.draftSaveReceipt(draft)).toBeUndefined();
});

it("does not claim durable recovery in the browser demo", async () => {
  await expect(demoMail.localDraftRecoveryIndex()).resolves.toEqual([]);
  await expect(demoMail.localDraftRecovery("missing")).resolves.toBeUndefined();
  await expect(demoMail.keepLocalDraftRecovery("recovery", undefined, {} as Parameters<typeof demoMail.keepLocalDraftRecovery>[2])).rejects.toThrow("requires the desktop app");
  await expect(demoMail.adoptLocalDraftRecovery("recovery", "v1", "new", "work", [])).rejects.toThrow("requires the desktop app");
  await expect(demoMail.discardLocalDraftRecovery("recovery", "v1")).rejects.toThrow("requires the desktop app");
});

it("has no delivery recovery receipts in the demo", async () => {
  await expect(demoMail.deliveryAttention()).resolves.toEqual({ entries: [], next: null });
  await expect(demoMail.deliveryAttentionDraft("missing")).resolves.toBeUndefined();
  await expect(demoMail.resolveDelivery("missing", "reviewed")).resolves.toEqual({ status: "alreadyFinished" });
});
