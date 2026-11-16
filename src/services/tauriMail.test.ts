import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as draftWrites from "../state/drafts";
import { noFilters } from "../lib/search";
import { useSignatures } from "../state/signatures";
import type { Draft } from "../state/store";
import { person } from "../test/fixtures";
import type { RustSummary, RustThread } from "./convert";

const invoke = vi.fn();
const unlisten = vi.fn();
let listener: ((e: { payload: unknown }) => void) | undefined;
let resolveListen: (() => void) | undefined;

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: (e: { payload: unknown }) => void) => {
    if (_name === "moss://send-prepared") return Promise.resolve(unlisten);
    listener = cb;
    return new Promise((resolve) => (resolveListen = () => resolve(unlisten)));
  },
}));

const openPath = vi.fn();
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: (...args: unknown[]) => openPath(...args) }));
const open = vi.fn();
const save = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...args: unknown[]) => open(...args), save: (...args: unknown[]) => save(...args) }));

const { errorText, tauriMail } = await import("./tauriMail");

const rp = { name: "Daniel Okafor", email: "daniel@northwind.co", initials: "DO", short: "Daniel" };
const summary: RustSummary = {
  id: "7",
  accountId: "gmail",
  split: "important",
  section: "reply",
  from: rp,
  subject: "Q4 budget",
  summary: "Sign off?",
  lastMessageAt: Date.now(),
  snoozedUntil: null,
  unread: true,
  done: false,
  labels: [],
  messageCount: 1,
  hasAttachments: false,
  sender: "known",
};

beforeEach(() => {
  invoke.mockReset();
  unlisten.mockReset();
});

describe("the desktop mail backend", () => {
  it("is live", () => expect(tauriMail.live).toBe(true));

  it("loads accounts, threads and a whole thread", async () => {
    invoke.mockResolvedValueOnce([{ id: "gmail", name: "Gmail", letter: "G", address: "a@gmail.com", senderName: "A", color: "work", provider: "gmail", connection: null }]);
    expect((await tauriMail.accounts())[0]).toMatchObject({ id: "gmail", name: "Gmail" });
    expect(invoke).toHaveBeenLastCalledWith("accounts");

    invoke.mockResolvedValueOnce([{ ...summary, sender: "new" }]);
    const threads = await tauriMail.threads();
    // The row already says the sender is new, before the thread is opened.
    expect(threads[0]).toMatchObject({ id: "7", partial: true, sender: "new" });
    expect(invoke).toHaveBeenLastCalledWith("threads", { account: null });

    const full: RustThread = { ...summary, messages: [], sender: "new" };
    invoke.mockResolvedValueOnce(full);
    expect(await tauriMail.thread("7")).toMatchObject({ id: "7", partial: false, sender: "new" });
    expect(invoke).toHaveBeenLastCalledWith("thread", { id: "7" });
  });

  it("changes an account's addresses, and answers with the account as the engine has it", async () => {
    const rust = {
      id: "icloud",
      name: "iCloud",
      letter: "I",
      address: "maya@icloud.com",
      senderName: "Maya Ross",
      color: "sky",
      provider: "icloud",
      connection: null,
      addresses: [{ email: "hi@rossstudio.art", state: "added", source: "you", seen: 0, blocked: false }],
    };
    invoke.mockResolvedValue(rust);
    expect((await tauriMail.addAddress("icloud", "hi@rossstudio.art", undefined))?.addresses).toEqual(rust.addresses);
    expect(invoke).toHaveBeenLastCalledWith("add_address", { account: "icloud", email: "hi@rossstudio.art", name: null });
    await tauriMail.updateAddress("icloud", "hi@rossstudio.art", "Studio");
    expect(invoke).toHaveBeenLastCalledWith("update_address", { account: "icloud", email: "hi@rossstudio.art", name: "Studio" });
    await tauriMail.updateAddress("icloud", "hi@rossstudio.art", undefined);
    expect(invoke).toHaveBeenLastCalledWith("update_address", { account: "icloud", email: "hi@rossstudio.art", name: null });
    await tauriMail.removeAddress("icloud", "hi@rossstudio.art");
    expect(invoke).toHaveBeenLastCalledWith("remove_address", { account: "icloud", email: "hi@rossstudio.art" });
    await tauriMail.blockAddress("icloud", "shop@mayaross.me", true);
    expect(invoke).toHaveBeenLastCalledWith("block_address", { account: "icloud", email: "shop@mayaross.me", blocked: true });
    await tauriMail.discoverAddresses("icloud");
    expect(invoke).toHaveBeenLastCalledWith("discover_addresses", { account: "icloud" });
    await tauriMail.setAddressSignatures("icloud", "hi@rossstudio.art", { newEmails: "s", replies: null });
    expect(invoke).toHaveBeenLastCalledWith("set_address_signatures", {
      account: "icloud",
      email: "hi@rossstudio.art",
      choice: { newEmails: "s", replies: null },
    });
    // An engine from before addresses sends none.
    invoke.mockResolvedValue({ ...rust, addresses: undefined });
    expect(await tauriMail.discoverAddresses("icloud")).not.toHaveProperty("addresses");
  });

  it("loads the Done list, newest first, with when each was done", async () => {
    invoke.mockResolvedValueOnce([{ ...summary, done: true, doneAt: 1234 }]);
    expect((await tauriMail.doneThreads())[0]).toMatchObject({ id: "7", doneAt: 1234 });
    expect(invoke).toHaveBeenLastCalledWith("done_threads", { account: null, limit: 200 });
  });

  it("loads the Blocked list, newest first", async () => {
    invoke.mockResolvedValueOnce([{ ...summary, sender: "blocked" }]);
    expect((await tauriMail.blockedThreads())[0]).toMatchObject({ id: "7", sender: "blocked" });
    expect(invoke).toHaveBeenLastCalledWith("blocked_threads", { account: null, limit: 200 });
  });

  it("blocks, accepts and unblocks a thread's sender", async () => {
    invoke.mockResolvedValue(null);
    await tauriMail.blockSender("7", false);
    expect(invoke).toHaveBeenLastCalledWith("block_sender", { id: "7", wholeDomain: false });
    await tauriMail.blockSender("7", true);
    expect(invoke).toHaveBeenLastCalledWith("block_sender", { id: "7", wholeDomain: true });
    await tauriMail.acceptSender("7", false);
    expect(invoke).toHaveBeenLastCalledWith("accept_sender", { id: "7", wholeDomain: false });
    await tauriMail.acceptSender("7", true);
    expect(invoke).toHaveBeenLastCalledWith("accept_sender", { id: "7", wholeDomain: true });
    await tauriMail.unblockSender("7");
    expect(invoke).toHaveBeenLastCalledWith("unblock_sender", { id: "7" });
    await tauriMail.senderRules();
    expect(invoke).toHaveBeenLastCalledWith("sender_rules");
    await tauriMail.forgetSenderRule("work", "growthleads.example");
    expect(invoke).toHaveBeenLastCalledWith("forget_sender_rule", { account: "work", sender: "growthleads.example" });
  });

  it("sends each action to its command", async () => {
    invoke.mockResolvedValue(null);
    await tauriMail.markDone("7");
    expect(invoke).toHaveBeenLastCalledWith("mark_done", { id: "7" });
    await tauriMail.markNotDone("7");
    expect(invoke).toHaveBeenLastCalledWith("mark_not_done", { id: "7" });
    await tauriMail.deleteThread("7");
    expect(invoke).toHaveBeenLastCalledWith("delete_thread", { id: "7" });
    await tauriMail.restoreThread("7");
    expect(invoke).toHaveBeenLastCalledWith("restore_thread", { id: "7" });
    await tauriMail.markRead("7", true);
    expect(invoke).toHaveBeenLastCalledWith("mark_read", { id: "7", read: true });
    await tauriMail.markRead("7", false);
    expect(invoke).toHaveBeenLastCalledWith("mark_read", { id: "7", read: false });
    await tauriMail.snooze("7", 123);
    expect(invoke).toHaveBeenLastCalledWith("snooze", { id: "7", until: 123 });
    await tauriMail.moveThread("7", "important");
    expect(invoke).toHaveBeenLastCalledWith("move_thread", { id: "7", to: "important" });
    await tauriMail.moveThread("7", null);
    expect(invoke).toHaveBeenLastCalledWith("move_thread", { id: "7", to: null });
    invoke.mockResolvedValueOnce(true);
    expect(await tauriMail.waitForReply("7", 123)).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith("wait_for_reply", { id: "7", until: 123 });
    await tauriMail.setSection("7", "info");
    expect(invoke).toHaveBeenLastCalledWith("correct", { id: "7", correction: { kind: "section", section: "info" } });
    await tauriMail.setSplit("7", "receipts");
    expect(invoke).toHaveBeenLastCalledWith("correct", { id: "7", correction: { kind: "split", split: "receipts" } });
    await tauriMail.clearDeadline("7");
    expect(invoke).toHaveBeenLastCalledWith("correct", { id: "7", correction: { kind: "noDeadline" } });
    invoke.mockResolvedValueOnce(false);
    expect(await tauriMail.remindIfNoReply("7", true)).toBe(false);
    expect(invoke).toHaveBeenLastCalledWith("correct", { id: "7", correction: { kind: "remindIfNoReply", on: true } });
    await tauriMail.allowImages("gmail", "alex@studio-build.example.test");
    expect(invoke).toHaveBeenLastCalledWith("allow_images", { account: "gmail", email: "alex@studio-build.example.test" });
    const saved = { signatures: [{ id: "s", name: "Work", lines: ["Maya"] }], byAccount: {}, letMossChoose: true, recent: ["s"] };
    invoke.mockResolvedValueOnce(saved);
    expect(await tauriMail.signatures()).toEqual(saved);
    expect(invoke).toHaveBeenLastCalledWith("signatures");
    await tauriMail.saveSignature(saved.signatures[0]);
    expect(invoke).toHaveBeenLastCalledWith("save_signature", { signature: saved.signatures[0] });
    await tauriMail.deleteSignature("s");
    expect(invoke).toHaveBeenLastCalledWith("delete_signature", { id: "s" });
    await tauriMail.setSignatureChoice("gmail", "newEmails", "s");
    expect(invoke).toHaveBeenLastCalledWith("set_signature_choice", { account: "gmail", kind: "newEmails", signature: "s" });
    await tauriMail.setLetMossChoose(false);
    expect(invoke).toHaveBeenLastCalledWith("set_let_moss_choose", { on: false });
    await tauriMail.markSignatureUsed("s");
    expect(invoke).toHaveBeenLastCalledWith("mark_signature_used", { id: "s" });
    await tauriMail.updateAccount("gmail", { color: "sky" });
    expect(invoke).toHaveBeenLastCalledWith("update_account", { id: "gmail", patch: { color: "sky" } });
    await tauriMail.reorderAccounts(["b", "a"]);
    expect(invoke).toHaveBeenLastCalledWith("reorder_accounts", { ids: ["b", "a"] });
    await tauriMail.credentialCleanupStatus();
    expect(invoke).toHaveBeenLastCalledWith("credential_cleanup_status");
    await tauriMail.retryCredentialCleanup();
    expect(invoke).toHaveBeenLastCalledWith("retry_credential_cleanup");
    await tauriMail.removeAccount("gmail");
    expect(invoke).toHaveBeenLastCalledWith("remove_account", { id: "gmail" });
    await tauriMail.removeAccount("gmail", 3);
    expect(invoke).toHaveBeenLastCalledWith("remove_account", { id: "gmail", discardServerChanges: 3 });
    await tauriMail.retryServerChange("gmail", "ab", 2);
    expect(invoke).toHaveBeenLastCalledWith("retry_server_change", { account: "gmail", change: "ab", revision: 2 });
    await tauriMail.discardServerChange("gmail", "ab", 2);
    expect(invoke).toHaveBeenLastCalledWith("discard_server_change", { account: "gmail", change: "ab", revision: 2 });
    await tauriMail.interruptedSends();
    expect(invoke).toHaveBeenLastCalledWith("interrupted_sends");
    await tauriMail.dismissInterruptedSends(["op"]);
    expect(invoke).toHaveBeenLastCalledWith("dismiss_interrupted_sends", { ids: ["op"] });
    await tauriMail.syncNow();
    expect(invoke).toHaveBeenLastCalledWith("sync_now", { account: null });
    await tauriMail.syncNow("gmail");
    expect(invoke).toHaveBeenLastCalledWith("sync_now", { account: "gmail" });
  });

  it("opens an attachment with the default app, once it's saved as a file", async () => {
    invoke.mockResolvedValueOnce("/cache/attachments/12/1/Q4.pdf");
    await tauriMail.openAttachment("12", 1);
    expect(invoke).toHaveBeenLastCalledWith("attachment_file", { messageId: "12", index: 1 });
    expect(openPath).toHaveBeenCalledWith("/cache/attachments/12/1/Q4.pdf");
    invoke.mockRejectedValueOnce({ kind: "offline", message: "can't reach the server" });
    await expect(tauriMail.openAttachment("12", 1)).rejects.toThrow("can't reach the server");
  });

  it("asks for an attachment's thumbnail, which may not exist", async () => {
    invoke.mockResolvedValueOnce("data:image/jpeg;base64,AAAA");
    expect(await tauriMail.attachmentPreview("12", 1)).toBe("data:image/jpeg;base64,AAAA");
    expect(invoke).toHaveBeenLastCalledWith("attachment_preview", { messageId: "12", index: 1 });
    invoke.mockResolvedValueOnce(null);
    expect(await tauriMail.attachmentPreview("12", 2)).toBeUndefined();
  });

  it("loads the body of a message stored with headers only", async () => {
    invoke.mockResolvedValueOnce(true);
    expect(await tauriMail.loadMessage("12")).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith("load_message", { messageId: "12" });
  });

  it("storage and preloading are one command each", async () => {
    invoke.mockResolvedValue(undefined);
    await tauriMail.preload(["1", "2"]);
    expect(invoke).toHaveBeenLastCalledWith("preload", { ids: ["1", "2"] });
    await tauriMail.storage();
    expect(invoke).toHaveBeenLastCalledWith("storage");
    await tauriMail.setOffline("recent");
    expect(invoke).toHaveBeenLastCalledWith("set_offline", { offline: "recent" });
    await tauriMail.setCacheLimit(250_000_000);
    expect(invoke).toHaveBeenLastCalledWith("set_cache_limit", { bytes: 250_000_000 });
    await tauriMail.setAttachmentText(false);
    expect(invoke).toHaveBeenLastCalledWith("set_attachment_text", { on: false });
    await tauriMail.clearCache();
    expect(invoke).toHaveBeenLastCalledWith("clear_cache");
    await tauriMail.rebuildIndex();
    expect(invoke).toHaveBeenLastCalledWith("rebuild_index");
  });

  it("reads sync progress for the Syncing screen", async () => {
    invoke.mockResolvedValueOnce(null);
    expect(await tauriMail.syncProgress("gmail")).toBeUndefined();
    invoke.mockResolvedValueOnce({ accountId: "gmail", done: 10, folders: 3, recent: 1, oldestYear: 2020, more: false });
    expect(await tauriMail.syncProgress("gmail")).toEqual({ done: 10, total: 10, folders: 3, recent: 1, oldestYear: 2020 });
    expect(invoke).toHaveBeenLastCalledWith("sync_progress", { account: "gmail" });
  });

  it("reads why accounts aren't syncing, for a window that opened after it was said", async () => {
    const problems = [{ accountId: "gmail", problem: "signedOut", message: "the keychain is not available" }];
    invoke.mockResolvedValueOnce(problems);
    expect(await tauriMail.syncProblems()).toEqual(problems);
    expect(invoke).toHaveBeenLastCalledWith("sync_problems");
  });

  const draft: Draft = {
    id: "d1",
    mode: "reply",
    threadId: "7",
    fromAccountId: "gmail",
    to: [person("Daniel Okafor", "daniel@northwind.co")],
    cc: [],
    bcc: [],
    subject: "Re: Q4 budget",
    body: "Yes",
    bodyHtml: "<p>Yes</p>",
    fieldsOpen: true,
    showCc: false,
    showBcc: false,
    remindIfNoReply: false,
    signatureId: null,
  };

  it("sends only what the engine needs, and turns its errors into sentences", async () => {
    invoke.mockResolvedValueOnce("7");
    await tauriMail.send(draft);
    expect(invoke).toHaveBeenLastCalledWith("send", {
      recovery: expect.objectContaining({ id: draft.id, body: draft.body, updatedAt: expect.any(Number) }),
      draft: { mode: "reply", threadId: "7", fromAccountId: "gmail", fromAddress: null, to: draft.to, cc: [], bcc: [], subject: "Re: Q4 budget", body: "Yes", bodyHtml: "<p>Yes</p>", signature: [], remindIfNoReply: false, attachments: [] },
    });
    await tauriMail.send({ ...draft, remindIfNoReply: true });
    expect(invoke.mock.lastCall?.[1].draft.remindIfNoReply).toBe(true);
    invoke.mockResolvedValueOnce("8");
    await tauriMail.send({ ...draft, mode: "new", threadId: undefined });
    expect(invoke.mock.lastCall?.[1].draft.threadId).toBeNull();
    invoke.mockRejectedValueOnce({ kind: "offline", message: "can't reach the server" });
    await expect(tauriMail.send(draft)).rejects.toThrow("can't reach the server");
  });

  it("sends the lines of the signature on the draft, which the engine adds under the body", async () => {
    useSignatures.setState({ signatures: [{ id: "s", name: "Work", lines: ["Maya Ross", "northwind.co"] }] });
    invoke.mockResolvedValue("7");
    await tauriMail.send({ ...draft, signatureId: "s" });
    expect(invoke.mock.lastCall?.[1].draft.signature).toEqual(["Maya Ross", "northwind.co"]);
    // Deleted meanwhile: nothing to add.
    await tauriMail.send({ ...draft, signatureId: "gone" });
    expect(invoke.mock.lastCall?.[1].draft.signature).toEqual([]);
  });

  it("asks the engine who you write to, ready to show as people", async () => {
    invoke.mockResolvedValueOnce([{ name: "Priya Shah", email: "priya@acme.co" }]);
    expect(await tauriMail.contacts()).toEqual([{ name: "Priya Shah", email: "priya@acme.co", initials: "PS" }]);
    expect(invoke).toHaveBeenLastCalledWith("contacts");
  });

  it("asks the engine to remember who you chose from the suggestions, and tells them back", async () => {
    await tauriMail.pickRecipient("pri", "priya@acme.co");
    expect(invoke).toHaveBeenLastCalledWith("pick_recipient", { typed: "pri", email: "priya@acme.co" });
    invoke.mockResolvedValueOnce([{ typed: "pri", email: "priya@acme.co" }]);
    expect(await tauriMail.recipientPicks()).toEqual([{ typed: "pri", email: "priya@acme.co" }]);
    expect(invoke).toHaveBeenLastCalledWith("recipient_picks");
  });

  it("keeps drafts on this Mac, with what the composer needs to show them again", async () => {
    const kept = {
      id: "d1",
      mode: "reply",
      threadId: "7",
      fromAccountId: "gmail",
      fromAddress: "hi@rossstudio.art",
      to: [{ name: "Daniel Okafor", email: "daniel@northwind.co" }],
      cc: [],
      bcc: [{ name: "lena@x.io", email: "lena@x.io" }],
      subject: "Re: Q4 budget",
      body: "Yes",
      bodyHtml: "<p>Yes</p>",
      remindIfNoReply: true,
      updatedAt: 5,
      signatureId: "s1",
      signatureChosen: true,
      attachments: [{ id: "3", name: "Plan.pdf", bytes: 8, contentType: "application/pdf" }],
    };
    invoke.mockResolvedValueOnce([kept, { ...kept, id: "d2", mode: "new", threadId: null, fromAddress: null, signatureId: null, signatureChosen: false, bcc: [] }]);
    const [reply, fresh] = await tauriMail.drafts();
    expect(invoke).toHaveBeenLastCalledWith("drafts");
    expect(reply).toEqual({
      ...kept,
      to: [{ name: "Daniel Okafor", email: "daniel@northwind.co", initials: "DO" }],
      bcc: [{ name: "lena@x.io", email: "lena@x.io", initials: "L" }],
      fieldsOpen: true,
      showCc: false,
      showBcc: true,
    });
    expect(fresh).toMatchObject({ signatureId: null, signatureChosen: false, showBcc: false });
    expect(fresh).not.toHaveProperty("threadId");
    expect(fresh).not.toHaveProperty("fromAddress");

    invoke.mockResolvedValueOnce({ ...kept, revision: "saved-reply" });
    await tauriMail.saveDraft({ ...reply, to: [person("Daniel Okafor", "daniel@northwind.co", { short: "Daniel" })], signatureOpen: true });
    expect(invoke).toHaveBeenLastCalledWith("save_draft", { draft: kept });
    invoke.mockResolvedValueOnce({ ...kept, revision: "saved-fresh" });
    await tauriMail.saveDraft({ ...fresh, signatureChosen: undefined });
    expect(invoke.mock.lastCall?.[1].draft).toMatchObject({ threadId: null, fromAddress: null, signatureChosen: false });

    invoke.mockResolvedValueOnce(true);
    await tauriMail.deleteDraft("d1", false);
    expect(invoke).toHaveBeenLastCalledWith("delete_draft", { id: "d1", keepFiles: false });
  });

  it("recovers a lost save acknowledgement by reading the exact attempted content", async () => {
    const raw = { id: "receipt", mode: "new", threadId: null, fromAccountId: "gmail", to: [], cc: [], bcc: [], subject: "Work", body: "Kept text", bodyHtml: "<p>Kept text</p>", remindIfNoReply: false, updatedAt: 1, signatureId: null, signatureChosen: false, fromAddress: null, attachments: [] };
    invoke.mockResolvedValueOnce([raw]);
    const [draft] = await tauriMail.drafts();
    invoke.mockRejectedValueOnce(new Error("reply lost")).mockResolvedValueOnce({ ...raw, revision: "acknowledged" });
    await expect(tauriMail.saveDraft(draft)).resolves.toMatchObject({ body: "Kept text", revision: "acknowledged" });
    expect(invoke.mock.calls.slice(1).map(([command]) => command)).toEqual(["save_draft", "draft_save_receipt"]);
    expect(invoke.mock.calls[2][1]).toEqual(invoke.mock.calls[1][1]);
  });

  it.each(["different content", "receipt unavailable"])("preserves the original save failure when %s", async (reason) => {
    const raw = { id: "receipt", mode: "new", threadId: null, fromAccountId: "gmail", to: [], cc: [], bcc: [], subject: "", body: "Local text", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, signatureChosen: false };
    invoke.mockResolvedValueOnce([raw]);
    const [draft] = await tauriMail.drafts();
    const failure = { kind: "draftConflict", message: "Changed elsewhere" };
    invoke.mockRejectedValueOnce(failure);
    if (reason === "different content") invoke.mockResolvedValueOnce(null);
    else invoke.mockRejectedValueOnce(new Error("database unavailable"));
    await expect(tauriMail.saveDraft(draft)).rejects.toBe(failure);
    expect(invoke.mock.calls.slice(1).map(([command]) => command)).toEqual(["save_draft", "draft_save_receipt"]);
  });

  it("rechecks a saved snapshot without writing and preserves read errors", async () => {
    const raw = { id: "receipt", mode: "new", threadId: null, fromAccountId: "gmail", to: [], cc: [], bcc: [], subject: "", body: "Text", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, signatureChosen: false };
    invoke.mockResolvedValueOnce([raw]);
    const [draft] = await tauriMail.drafts();
    invoke.mockResolvedValueOnce({ ...raw, revision: "confirmed" });
    await expect(tauriMail.draftSaveReceipt(draft)).resolves.toMatchObject({ revision: "confirmed", body: "Text" });
    invoke.mockResolvedValueOnce(null);
    await expect(tauriMail.draftSaveReceipt(draft)).resolves.toBeUndefined();
    const error = new Error("read unavailable");
    invoke.mockRejectedValueOnce(error);
    await expect(tauriMail.draftSaveReceipt(draft)).rejects.toBe(error);
    expect(invoke.mock.calls.slice(1).map(([command]) => command)).toEqual(["draft_save_receipt", "draft_save_receipt", "draft_save_receipt"]);
  });

  it("a draft from before files has none", async () => {
    const old = { id: "d1", mode: "new", threadId: null, fromAccountId: "gmail", to: [], cc: [], bcc: [], subject: "", body: "", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, signatureChosen: false };
    invoke.mockResolvedValueOnce([old]);
    const [draft] = await tauriMail.drafts();
    expect(draft.attachments).toEqual([]);
    invoke.mockResolvedValueOnce({ ...old, revision: "saved-old" });
    await tauriMail.saveDraft({ ...draft, attachments: undefined });
    expect(invoke.mock.lastCall?.[1].draft.attachments).toEqual([]);
  });

  it("requests owner-scoped staged-file cleanup and propagates failures", async () => {
    invoke.mockResolvedValueOnce(true);
    await expect(tauriMail.discardDraftFile("draft-1", "file-2")).resolves.toBeUndefined();
    expect(invoke).toHaveBeenLastCalledWith("discard_draft_file", { draftId: "draft-1", id: "file-2" });
    invoke.mockRejectedValueOnce(new Error("database unavailable"));
    await expect(tauriMail.discardDraftFile("draft-1", "file-2")).rejects.toThrow("database unavailable");
  });

  it("picks files with the system picker and keeps copies of them in the engine", async () => {
    open.mockResolvedValueOnce(["/Users/maya/Plan.pdf", "/Users/maya/notes.txt"]);
    expect(await tauriMail.pickFiles()).toEqual(["/Users/maya/Plan.pdf", "/Users/maya/notes.txt"]);
    expect(open).toHaveBeenLastCalledWith({ multiple: true, directory: false, title: "Attach files" });
    // Cancelled.
    open.mockResolvedValueOnce(null);
    expect(await tauriMail.pickFiles()).toEqual([]);

    const plan = { id: "3", name: "Plan.pdf", bytes: 8, contentType: "application/pdf" };
    invoke.mockResolvedValueOnce([plan]);
    expect(await tauriMail.attachFiles("d1", ["/Users/maya/Plan.pdf"])).toEqual([plan]);
    expect(invoke).toHaveBeenLastCalledWith("attach_files", { draftId: "d1", paths: ["/Users/maya/Plan.pdf"] });
    // Files dropped from Finder come as paths; anything else has none to read.
    invoke.mockClear();
    expect(await tauriMail.attachFiles("d1", [new File(["x"], "x.txt")])).toEqual([]);
    expect(invoke).not.toHaveBeenCalled();
    invoke.mockRejectedValueOnce({ kind: "unavailable", message: "can't read gone.pdf" });
    await expect(tauriMail.attachFiles("d1", ["/gone.pdf"])).rejects.toThrow("can't read gone.pdf");

    invoke.mockResolvedValueOnce("data:image/jpeg;base64,AA==");
    expect(await tauriMail.draftPreview("d1", "3")).toBe("data:image/jpeg;base64,AA==");
    expect(invoke).toHaveBeenLastCalledWith("draft_preview", { draftId: "d1", id: "3" });
    invoke.mockResolvedValueOnce(null);
    expect(await tauriMail.draftPreview("d1", "4")).toBeUndefined();
  });

  it.each([
    ["connection lost", "connection lost"],
    [null, "Something went wrong. Try again."],
    [new Error("bridge disconnected"), "bridge disconnected"],
  ])("preserves untyped send failures: %s", async (failure, message) => {
    invoke.mockRejectedValueOnce(failure);
    await expect(tauriMail.send(draft)).rejects.toMatchObject({ message, kind: undefined });
    invoke.mockRejectedValueOnce(failure);
    await expect(tauriMail.sendAfter("untyped", draft, 6000)).rejects.toMatchObject({ message, kind: undefined });
  });

  it.each(["deliveryUnknown", "deliveryAccepted"])("preserves %s on direct sends", async (kind) => {
    invoke.mockRejectedValueOnce({ kind, message: "Do not resend" });
    await expect(tauriMail.send(draft)).rejects.toMatchObject({ kind, message: "Do not resend" });
    expect(invoke.mock.lastCall?.[1].recovery).toMatchObject({ id: draft.id, body: draft.body });
  });

  it("reopens the authoritative draft returned by Undo with its fresh revision", async () => {
    const raw = { id: "restored", mode: "new", threadId: null, fromAccountId: "gmail", to: [], cc: [], bcc: [], subject: "Kept", body: "Latest", bodyHtml: "<p>Latest</p>", remindIfNoReply: false, updatedAt: 10, signatureId: null, signatureChosen: false, revision: "fresh" };
    invoke.mockResolvedValueOnce({ cancelled: true, early: false, restored: raw });
    expect(await tauriMail.cancelSend("held")).toEqual({ cancelled: true, early: false, restored: {
      ...raw, threadId: undefined, attachments: [], fieldsOpen: true, showCc: false, showBcc: false,
    } });
  });

  it("holds a sent email in the engine so Undo can take it back", async () => {
    invoke.mockResolvedValueOnce(true);
    expect(await tauriMail.sendAfter("h1", draft, 6000)).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith("send_after", {
      hold: "h1",
      recovery: expect.objectContaining({ id: draft.id, body: draft.body, signatureId: draft.signatureId, updatedAt: expect.any(Number) }),
      delayMs: 6000,
      draft: { mode: "reply", threadId: "7", fromAccountId: "gmail", fromAddress: null, to: draft.to, cc: [], bcc: [], subject: "Re: Q4 budget", body: "Yes", bodyHtml: "<p>Yes</p>", signature: [], remindIfNoReply: false, attachments: [] },
    });
    await tauriMail.send({ ...draft, remindIfNoReply: true });
    expect(invoke.mock.lastCall?.[1].draft.remindIfNoReply).toBe(true);
    invoke.mockRejectedValueOnce({ kind: "offline", message: "can't reach the server" });
    await expect(tauriMail.sendAfter("h2", draft, 6000)).rejects.toThrow("can't reach the server");
    invoke.mockRejectedValueOnce({ kind: "deliveryUnknown", message: "Check Sent before retrying" });
    await expect(tauriMail.sendAfter("uncertain", draft, 6000)).rejects.toMatchObject({ kind: "deliveryUnknown", message: "Check Sent before retrying" });
    invoke.mockResolvedValueOnce({ cancelled: true, early: true, restored: null });
    expect(await tauriMail.cancelSend("h1")).toEqual({ cancelled: true, early: true });
    expect(invoke).toHaveBeenLastCalledWith("cancel_send", { hold: "h1" });
  });

  it.each(["schedule", "send", "direct"] as const)("freezes %s content before waiting for earlier draft writes", async (action) => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const wait = vi.spyOn(draftWrites, "draftsSaved").mockReturnValueOnce(gate);
    const previous = useSignatures.getState().signatures;
    useSignatures.setState({ signatures: [{ id: "freeze", name: "Work", lines: ["Original signature"] }] });
    invoke.mockResolvedValueOnce(action === "schedule" ? { status: "scheduled" } : true);
    const writing = { ...draft, revision: "base", signatureId: "freeze", to: draft.to.map((p) => ({ ...p })) };
    const acknowledge = vi.spyOn(draftWrites, "acknowledgedDraftRevision").mockImplementation((frozen, source) => {
      expect(source).toBe(writing);
      expect(frozen.revision).toBe("base");
      return "acknowledged";
    });
    const pending = action === "schedule" ? tauriMail.scheduleSend(writing, 9000, "frozen") : action === "direct" ? tauriMail.send(writing) : tauriMail.sendAfter("frozen", writing, 6000);
    try {
      expect(invoke).not.toHaveBeenCalled();
      useSignatures.setState({ signatures: [{ id: "freeze", name: "Work", lines: ["Changed signature"] }] });
      writing.to[0].email = "changed@fixture.test";
      release();
      await pending;
      const args = invoke.mock.lastCall![1];
      expect((action === "schedule" ? args.draft : args.recovery).revision).toBe("acknowledged");
      const outgoing = action === "schedule" ? args.email : args.draft;
      expect(outgoing.signature).toEqual(["Original signature"]);
      expect(outgoing.to[0].email).toBe(draft.to[0].email);
    } finally {
      release();
      await pending;
      wait.mockRestore();
      acknowledge.mockRestore();
      useSignatures.setState({ signatures: previous });
    }
  });

  it("converts the authoritative row in a revision conflict", async () => {
    invoke.mockResolvedValueOnce({ status: "changed", current: { operationId: "op1", revision: 2, draft: { ...draft, threadId: null, updatedAt: 1 }, sendAt: 9000, nextTry: 9000, tries: 0, error: null } });
    const outcome = await tauriMail.rescheduleSend(draft.id, 10000, "op1", 1);
    expect(outcome).toMatchObject({ status: "changed", current: { operationId: "op1", revision: 2, draft: { fieldsOpen: true } } });
    expect(outcome).not.toHaveProperty("current.draft.threadId");
  });

  it("sets an email to send later, and lists, moves and cancels those waiting", async () => {
    invoke.mockResolvedValueOnce({ status: "scheduled" });
    await tauriMail.scheduleSend(draft, 9000, "op1");
    const [command, args] = invoke.mock.lastCall!;
    expect(command).toBe("schedule_send");
    expect(args.operation).toBe("op1");
    expect(args.sendAt).toBe(9000);
    expect(args.email).toEqual({ mode: "reply", threadId: "7", fromAccountId: "gmail", fromAddress: null, to: draft.to, cc: [], bcc: [], subject: "Re: Q4 budget", body: "Yes", bodyHtml: "<p>Yes</p>", signature: [], remindIfNoReply: false, attachments: [] });
    expect(args.draft).toMatchObject({ id: draft.id, threadId: "7", to: [{ name: draft.to[0].name, email: draft.to[0].email }], signatureChosen: false });
    expect(args.draft.updatedAt).toEqual(expect.any(Number));
    invoke.mockRejectedValueOnce({ kind: "noRecipients", message: "add someone to send it to" });
    await expect(tauriMail.scheduleSend(draft, 9000, "op1")).rejects.toThrow("add someone to send it to");

    const kept = { ...args.draft, cc: [], bcc: [] };
    invoke.mockResolvedValueOnce([{ draft: kept, sendAt: 9000, nextTry: 9600, tries: 1, error: "offline" }, { draft: kept, sendAt: 9500, nextTry: 9500, tries: 0, error: null }]);
    const [failed, waiting] = await tauriMail.scheduled();
    expect(invoke).toHaveBeenLastCalledWith("scheduled");
    expect(failed).toMatchObject({ sendAt: 9000, nextTry: 9600, tries: 1, error: "offline", draft: { id: draft.id, fieldsOpen: true } });
    expect(failed.draft.to[0]).toHaveProperty("initials");
    expect(waiting).not.toHaveProperty("error");

    invoke.mockResolvedValueOnce({ status: "scheduled" });
    await tauriMail.rescheduleSend("d1", 12_000, "op1", 1);
    expect(invoke).toHaveBeenLastCalledWith("reschedule_send", { id: "d1", sendAt: 12_000, operation: "op1", expectedRevision: 1 });
    invoke.mockRejectedValueOnce({ kind: "invalid", message: "It’s sending now." });
    await expect(tauriMail.rescheduleSend("d1", 1, "op1", 1)).rejects.toThrow("It’s sending now.");

    invoke.mockResolvedValueOnce({ status: "cancelled", restored: kept });
    expect(await tauriMail.unscheduleSend("d1", "op1")).toMatchObject({ status: "cancelled", restored: { id: draft.id, fieldsOpen: true } });
    expect(invoke).toHaveBeenLastCalledWith("unschedule_send", { id: "d1", operation: "op1" });
    invoke.mockResolvedValueOnce({ status: "cancelled", early: true, restored: null });
    expect(await tauriMail.unscheduleSend("d1", "early")).toEqual({ status: "cancelled", early: true });
    invoke.mockRejectedValueOnce({ kind: "unavailable", message: "sent already" });
    await expect(tauriMail.unscheduleSend("d1", "op1")).rejects.toThrow("sent already");
  });

  it("hands on what happened to emails set to send later, ready for the screen", async () => {
    const seen: unknown[] = [];
    const stop = tauriMail.onEvent((e) => seen.push(e));
    resolveListen!();
    await Promise.resolve();
    const kept = { id: "d1", mode: "new", threadId: null, fromAccountId: "gmail", to: [{ name: "Dana", email: "dana@x.co" }], cc: [], bcc: [], subject: "Lunch?", body: "", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, signatureChosen: false };
    listener!({ payload: { type: "scheduledSent", late: true, email: { draft: kept, sendAt: 5, nextTry: 5, tries: 0, error: null } } });
    listener!({ payload: { type: "scheduledFailed", email: { draft: kept, sendAt: 5, nextTry: 65, tries: 1, error: "offline" } } });
    listener!({ payload: { type: "scheduledWaiting", email: { draft: kept, sendAt: 5, nextTry: 65, tries: 1, error: "Waiting for recovery" } } });
    expect(seen).toMatchObject([
      { type: "scheduledSent", late: true, email: { sendAt: 5, draft: { subject: "Lunch?", to: [{ initials: "D" }] } } },
      { type: "scheduledFailed", email: { nextTry: 65, error: "offline" } },
      { type: "scheduledWaiting", email: { tries: 1, nextTry: 65, error: "Waiting for recovery", draft: { to: [{ initials: "D" }] } } },
    ]);
    stop();
  });

  it("searches, then applies the filters the engine doesn't know", async () => {
    const hit = { thread: summary, messageId: "m1", snippet: [{ text: "Sign off?", matched: false }], rank: -1 };
    invoke.mockResolvedValueOnce([hit]);
    const hits = await tauriMail.search("budget", noFilters);
    expect(hits.map((h) => h.doc.id)).toEqual(["m1"]);
    expect(invoke).toHaveBeenLastCalledWith("search", { query: "budget", account: null, limit: 100 });

    invoke.mockResolvedValueOnce([hit]);
    expect(await tauriMail.search(" ", { ...noFilters, accountId: "gmail", from: person("Daniel", "daniel@northwind.co"), hasAttachment: true })).toEqual([]);
    expect(invoke).toHaveBeenLastCalledWith("search", { query: "daniel@northwind.co", account: "gmail", limit: 100 });
    invoke.mockResolvedValueOnce([]);
    await tauriMail.search("", noFilters);
    expect(invoke).toHaveBeenLastCalledWith("search", { query: "", account: null, limit: 100 });
  });

  it("passes engine events on until stopped", async () => {
    const seen: unknown[] = [];
    const stop = tauriMail.onEvent((e) => seen.push(e));
    resolveListen!();
    await Promise.resolve();
    listener!({ payload: { type: "mailChanged", accountId: "gmail" } });
    expect(seen).toEqual([{ type: "mailChanged", accountId: "gmail" }]);
    stop();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("stops listening even when stopped before listening began", async () => {
    const stop = tauriMail.onEvent(() => {});
    stop();
    resolveListen!();
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledOnce();
  });
});

describe("error text", () => {
  it("uses the engine's message, or a calm fallback", () => {
    expect(errorText("plain")).toBe("plain");
    expect(errorText({ message: "from rust" })).toBe("from rust");
    expect(errorText({ message: 5 })).toBe("Something went wrong. Try again.");
    expect(errorText(null)).toBe("Something went wrong. Try again.");
  });
});


it("reads server recovery status through the account-scoped native command", async () => {
  const status = { accountId: "work", state: "blocked", failure: "uncertain", kind: "move", completed: 1, pending: 2, discarded: 0, dropped: 0 };
  invoke.mockResolvedValue(status);
  expect(await tauriMail.serverChangeStatus("work")).toEqual(status);
  expect(invoke).toHaveBeenCalledWith("server_change_status", { account: "work" });
});

it("leaves destination selection to the native backup command", async () => {
  invoke.mockResolvedValueOnce(7);
  expect(await tauriMail.createBackup()).toBe(7);
  expect(invoke).toHaveBeenLastCalledWith("backup_start");
  expect(save).not.toHaveBeenCalled();
  invoke.mockResolvedValueOnce(null);
  expect(await tauriMail.createBackup()).toBeNull();
  invoke.mockResolvedValueOnce({ state: "running", id: 7, progress: { phase: "checking" }, cancelling: false });
  expect(await tauriMail.backupStatus()).toMatchObject({ state: "running", id: 7 });
  expect(invoke).toHaveBeenLastCalledWith("backup_status");
  invoke.mockResolvedValueOnce(true);
  expect(await tauriMail.cancelBackup(7)).toBe(true);
  expect(invoke).toHaveBeenLastCalledWith("backup_cancel", { id: 7 });
});

it("permits the user to choose an archive destination with the native save dialog", () => {
  const path = "../../src-tauri/capabilities/default.json";
  const caps = JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));
  expect(caps.permissions).not.toContain("dialog:allow-save");
});

it("keeps recovery separate from editable drafts and preserves terminal responses", async () => {
  const raw = { id: "local-source", mode: "new" as const, threadId: null, fromAccountId: "work", to: [], cc: [], bcc: [], subject: "", body: "Local work", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, signatureChosen: false, attachments: [] };
  invoke.mockResolvedValueOnce([raw]);
  const [draft] = await tauriMail.drafts();
  const recovery = { id: "recovery", version: "v1", draft: raw, missingFiles: ["unavailable"], updatedAt: 1 };
  const entry = { id: "recovery", version: "v1", originalId: raw.id, updatedAt: 1 };
  invoke.mockResolvedValueOnce([entry]);
  await expect(tauriMail.localDraftRecoveryIndex()).resolves.toEqual([entry]);
  expect(invoke).toHaveBeenLastCalledWith("local_draft_recovery_index");
  invoke.mockResolvedValueOnce({ status: "saved", recovery });
  await expect(tauriMail.keepLocalDraftRecovery("recovery", undefined, draft)).resolves.toEqual({ status: "saved", recovery: { ...recovery, draft } });
  expect(invoke).toHaveBeenLastCalledWith("keep_local_draft_recovery", { id: "recovery", previous: null, draft: expect.objectContaining({ id: raw.id, body: raw.body }) });
  invoke.mockResolvedValueOnce({ status: "resolved", adoptedId: "new-draft" });
  await expect(tauriMail.keepLocalDraftRecovery("recovery", "v1", draft)).resolves.toEqual({ status: "resolved", adoptedId: "new-draft" });
  expect(invoke.mock.lastCall?.[1].previous).toBe("v1");
  invoke.mockResolvedValueOnce(recovery).mockResolvedValueOnce(null);
  await expect(tauriMail.localDraftRecovery("recovery")).resolves.toEqual({ ...recovery, draft });
  await expect(tauriMail.localDraftRecovery("gone")).resolves.toBeUndefined();
  expect(invoke).toHaveBeenLastCalledWith("local_draft_recovery", { id: "gone" });
  invoke.mockResolvedValueOnce({ ...raw, id: "new-draft" }).mockResolvedValueOnce(null);
  await expect(tauriMail.adoptLocalDraftRecovery("recovery", "v1", "new-draft", "work", ["unavailable"])).resolves.toMatchObject({ id: "new-draft", body: raw.body });
  await expect(tauriMail.adoptLocalDraftRecovery("recovery", "v1", "new-draft", "work", ["unavailable"])).resolves.toBeUndefined();
  expect(invoke).toHaveBeenLastCalledWith("adopt_local_draft_recovery", { id: "recovery", version: "v1", newId: "new-draft", account: "work", missing: ["unavailable"] });
  invoke.mockResolvedValueOnce(null);
  await tauriMail.discardLocalDraftRecovery("recovery", "v1");
  expect(invoke).toHaveBeenLastCalledWith("discard_local_draft_recovery", { id: "recovery", version: "v1" });
  const failure = { kind: "store", message: "Storage unavailable" };
  invoke.mockRejectedValueOnce(failure);
  await expect(tauriMail.keepLocalDraftRecovery("recovery", "v1", draft)).rejects.toBe(failure);
});

it("pages delivery metadata and resolves a selected operation through protected native commands", async () => {
  const page = { entries: [], next: { id: "cursor", createdAt: 10 } };
  invoke.mockResolvedValueOnce(page).mockResolvedValueOnce({ entries: [], next: null });
  expect(await tauriMail.deliveryAttention()).toEqual(page);
  expect(invoke).toHaveBeenLastCalledWith("delivery_attention", { after: null });
  await tauriMail.deliveryAttention(page.next);
  expect(invoke).toHaveBeenLastCalledWith("delivery_attention", { after: page.next });
  const raw = { id: "source", mode: "new", threadId: null, fromAccountId: "work", to: [], cc: [], bcc: [], subject: "", body: "Local work", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, signatureChosen: false, attachments: [] };
  invoke.mockResolvedValueOnce(raw).mockResolvedValueOnce(null);
  await expect(tauriMail.deliveryAttentionDraft("selected")).resolves.toMatchObject({ id: "source", body: "Local work" });
  expect(invoke).toHaveBeenLastCalledWith("delivery_attention_draft", { id: "selected" });
  await expect(tauriMail.deliveryAttentionDraft("gone")).resolves.toBeUndefined();
  const outcome = { status: "resolved", recoveryId: "recovery" };
  invoke.mockResolvedValueOnce(outcome);
  await expect(tauriMail.resolveDelivery("selected", "copied")).resolves.toEqual(outcome);
  expect(invoke).toHaveBeenLastCalledWith("resolve_delivery", { id: "selected", action: "copied" });
});
