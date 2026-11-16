import type { DeliveryAttentionCursor, DeliveryAttentionPage, DeliveryResolution, DeliveryResolutionOutcome } from "./deliveryAttention";
import type { BackupStatus } from "./backups";
/**
 * Where mail comes from and where actions go. In the desktop app that is the Rust mail
 * engine (`tauriMail`); in the browser, dev scenes and tests it is the demo mailbox, whose
 * actions only change the screen.
 */
import type { Account, AccountColor, DraftFile, Person, RecipientPick, SenderRule, SplitId, Thread } from "../data/types";
import type { SearchFilters, SearchHit } from "../lib/search";
import type { AccountSignatures, SavedSignatures, Signature } from "../state/signatures";
import type { SavedDraft } from "../state/drafts";
import type { ScheduledEmail } from "../state/scheduled";
import type { Draft } from "../state/store";
import type { SyncSnapshot } from "./accountSetup";
import type { RustAccount, RustSyncProgress } from "./convert";

export interface LocalDraftRecovery {
  id: string;
  version: string;
  draft: SavedDraft;
  missingFiles: string[];
  updatedAt: number;
}
export interface LocalDraftRecoveryEntry {
  id: string;
  version: string;
  originalId: string;
  updatedAt: number;
}
export type LocalDraftRecoveryWrite =
  | { status: "saved"; recovery: LocalDraftRecovery }
  | { status: "resolved"; adoptedId: string | null };

export type CredentialCleanupStatus = { pending: boolean };

export type ScheduleOutcome =
  | { status: "rescheduled"; revision: number }
  | { status: "changed"; current: ScheduledEmail }
  | { status: "scheduled" | "alreadyScheduled" | "sending" | "completed" }
  | { status: "cancelled"; restored?: SavedDraft; early?: boolean }
  | { status: "inDelivery"; state: "held" | "claimed" | "accepted" | "uncertain" | "complete" | "cancelled" };

/** An email you sent that Moss stopped before it went: back in Drafts. */
export interface InterruptedSend {
  id: string;
  subject: string;
  at: number;
}

export interface ServerChangeStatus {
  accountId: string;
  /** Which change, and the revision you saw, for Try again and Discard. */
  change: string;
  revision: number;
  state: "queued" | "claimed" | "blocked";
  failure: "refused" | "unreadable" | "uncertain" | "tooLarge" | null;
  kind: "legacy" | "done" | "undone" | "read" | "delete" | "restore" | "move" | "forwarded" | "draftCleanup";
  completed: number;
  pending: number;
  discarded: number;
  dropped: number;
}

/** Why an account isn't syncing right now. */
export interface SyncIssue {
  accountId: string;
  problem: "signedOut" | "offline" | "server";
  message: string;
}

export type MailEvent =
  | ({ type: "syncProgress" } & RustSyncProgress)
  | { type: "mailChanged"; accountId: string }
  | ({ type: "syncError" } & SyncIssue)
  | { type: "syncOk"; accountId: string }
  | { type: "serverChangesChanged"; accountId: string }
  | { type: "sendsInterrupted" }
  | { type: "accountUpdated"; account: RustAccount }
  /** Drafts came in from an account's Drafts folder (your phone, webmail), or left it. */
  | { type: "draftsChanged"; accountId: string }
  /** An email set to send later went: `late` when Moss was closed at its time. */
  | { type: "scheduledSent"; email: ScheduledEmail; late: boolean }
  /** An email set to send later couldn't go; it is tried again at `email.nextTry`. */
  | { type: "scheduledFailed"; email: ScheduledEmail }
  | { type: "scheduledWaiting"; email: ScheduledEmail }
  | { type: "scheduledChanged" };

/** What Moss keeps on this Mac so mail opens offline. */
export type Offline = "recent" | "text" | "everything";

/** Settings → Storage: space used, in bytes, and what's kept. */
export interface StorageInfo {
  usage: {
    /** Finds and sorts mail: details, the keyword and meaning indexes, summaries. */
    index: number;
    /** Text (and files) the offline setting keeps. */
    offline: number;
    /** Mail and files opened lately; the oldest go first. */
    cache: number;
  };
  offline: Offline;
  cacheLimit: number;
  /** The index is being rebuilt, so search finds less for now. */
  rebuilding: boolean;
  /** Moss reads the text inside attachments on this Mac, so search and Ask Moss find what's in them. */
  attachmentText: boolean;
  /** The AI models on this Mac. */
  models: number;
}

export interface AccountChange {
  name?: string;
  letter?: string;
  color?: AccountColor;
  senderName?: string;
}

/**
 * A held email: false when Undo took it back. Once it has gone, the conversation it went into,
 * or true when that isn't known (the demo mailbox keeps no conversations for what you send).
 */
export type Sent = string | boolean;

export interface CancelSendOutcome {
  restoreFailed?: boolean;
  cancelled: boolean;
  early: boolean;
  restored?: SavedDraft;
}

export interface MailBackend {
  /** Real mail from the desktop app, or the demo mailbox. */
  live: boolean;
  accounts(): Promise<Account[]>;
  /** Every open thread; their messages load when one is opened. */
  threads(): Promise<Thread[]>;
  thread(id: string): Promise<Thread | undefined>;
  /** The Done list: threads you marked done, most recently done first. */
  doneThreads(): Promise<Thread[]>;
  markDone(id: string): Promise<void>;
  /** Back in the list, and in the Inbox on the server. */
  markNotDone(id: string): Promise<void>;
  /** Out of every list, and into Trash on the server. */
  deleteThread(id: string): Promise<void>;
  /** Undoes a delete: back where it was, in the list or the Done list. */
  restoreThread(id: string): Promise<void>;
  /** Marks every message in the thread read, or unread. */
  markRead(id: string, read: boolean): Promise<void>;
  /** Blocked mail, newest first, from every account's Blocked folder. */
  blockedThreads(): Promise<Thread[]>;
  /** Blocks this thread's sender (or everyone at their domain) in its account. */
  blockSender(id: string, wholeDomain: boolean): Promise<void>;
  /** Never asks about this thread's sender (or anyone at their domain) again, and shows their images. */
  acceptSender(id: string, wholeDomain: boolean): Promise<void>;
  /** Unblocks and accepts this thread's sender; their mail comes back to the inbox. */
  unblockSender(id: string): Promise<void>;
  /** Everyone you blocked or accepted, by address or domain, in every account, newest first. */
  senderRules(): Promise<SenderRule[]>;
  /** Forgets a rule, so Moss asks about that sender again; blocked mail comes back. */
  forgetSenderRule(accountId: string, sender: string): Promise<void>;
  snooze(id: string, until: number | null): Promise<void>;
  /** Moves a thread to another split tab for good; `null` gives it back to Moss's sorting. To
   * Important stars it on the server (a star in Gmail, a flag elsewhere); out takes the star away. */
  moveThread(id: string, to: SplitId | null): Promise<void>;
  /** Your latest email waits for an answer: out of sight until `until` (ms), back sooner if
   * someone replies. False when the latest email isn't yours. */
  waitForReply(id: string, until: number): Promise<boolean>;
  /** Your corrections to what Moss decided. Each wins, now and later. */
  setSection(id: string, section: "reply" | "info"): Promise<void>;
  setSplit(id: string, split: SplitId): Promise<void>;
  clearDeadline(id: string): Promise<void>;
  /** Brings your latest email back if no one replies (or stops waiting). False when the latest email isn't yours. */
  remindIfNoReply(id: string, on: boolean): Promise<boolean>;
  /** Show images from this sender from now on, in this account. */
  allowImages(accountId: string, email: string): Promise<void>;
  /** Your signatures and which one each account adds, as saved on this device. */
  signatures(): Promise<SavedSignatures>;
  /** Saves a signature's name and lines; a new one goes to the end of the list. */
  saveSignature(signature: Signature): Promise<void>;
  /** Deletes a signature; accounts that used it get none. */
  deleteSignature(id: string): Promise<void>;
  setSignatureChoice(accountId: string, kind: keyof AccountSignatures, signatureId: string | null): Promise<void>;
  setLetMossChoose(on: boolean): Promise<void>;
  /** Moves a signature to the front of the composer's picker. */
  markSignatureUsed(id: string): Promise<void>;
  /** People to suggest as recipients, from all your mail: the ones you write to most first. */
  contacts(): Promise<Person[]>;
  /** Remembers that you chose `email` from the recipient suggestions after typing `typed`. */
  pickRecipient(typed: string, email: string): Promise<void>;
  /** Who you chose from the recipient suggestions for what you typed, newest first. */
  recipientPicks(): Promise<RecipientPick[]>;
  /** Unsent emails kept on this device, most recently changed first. */
  drafts(): Promise<SavedDraft[]>;
  /** Keeps a draft, replacing the one with the same id. */
  saveDraft(draft: SavedDraft): Promise<SavedDraft>;
  draftSaveReceipt(draft: SavedDraft): Promise<SavedDraft | undefined>;
  deliveryAttention(after?: DeliveryAttentionCursor): Promise<DeliveryAttentionPage>;
  deliveryAttentionDraft(id: string): Promise<SavedDraft | undefined>;
  resolveDelivery(id: string, action: DeliveryResolution): Promise<DeliveryResolutionOutcome>;
  localDraftRecoveryIndex(): Promise<LocalDraftRecoveryEntry[]>;
  localDraftRecovery(id: string): Promise<LocalDraftRecovery | undefined>;
  keepLocalDraftRecovery(id: string, previous: string | undefined, draft: SavedDraft): Promise<LocalDraftRecoveryWrite>;
  discardLocalDraftRecovery(id: string, version: string): Promise<void>;
  adoptLocalDraftRecovery(id: string, version: string, newId: string, account: string, missing: string[]): Promise<SavedDraft | undefined>;
  /** Deletes a draft and its files; with `keepFiles` it is being sent, and they go once it has. */
  deleteDraft(id: string, keepFiles: boolean, revision?: string): Promise<void>;
  /** Opens the system file picker: the files chosen (paths, or files in a browser), or none. */
  pickFiles(): Promise<string[] | File[]>;
  /** Keeps copies of these files (chosen or dropped) for draft `draftId`. */
  attachFiles(draftId: string, files: string[] | File[]): Promise<DraftFile[]>;
  /** Releases a staged file only when no saved draft, schedule or pending delivery owns it. */
  discardDraftFile(draftId: string, id: string): Promise<void>;
  /** A small picture of what's in a file you attached, as an image URL; none if it can't have one. */
  draftPreview(draftId: string, id: string): Promise<string | undefined>;
  /** Resolves once the email has gone; rejects with a sentence to show. */
  send(draft: Draft): Promise<void>;
  /**
   * Holds the email `delayMs` so Undo can take it back, then sends it. Resolves to where it
   * went once it has gone, false when `cancelSend` took it back first; rejects with a sentence
   * to show. Quitting Moss sends it at once.
   */
  sendAfter(hold: string, draft: Draft, delayMs: number): Promise<Sent>;
  /** Undo send: true when the email was still held and now won't go. */
  cancelSend(hold: string): Promise<CancelSendOutcome>;
  /**
   * Send later: the email waits on this Mac and goes at `sendAt` (ms), and its draft leaves
   * Drafts. Rejects with a sentence to show when it can't go (no one to send it to).
   */
  scheduleSend(draft: Draft, sendAt: number, operation: string): Promise<ScheduleOutcome>;
  /** Emails set to send later, the soonest first. */
  scheduled(): Promise<ScheduledEmail[]>;
  /** A new time; now sends it now. Rejects with a sentence once it is on its way. */
  rescheduleSend(id: string, sendAt: number, operation: string, expectedRevision: number): Promise<ScheduleOutcome>;
  /**
   * Won't send it: it goes back to Drafts, and resolves to the draft as the engine kept it
   * (the demo mailbox keeps none, so the one on screen stays). Rejects once it is on its way.
   */
  unscheduleSend(id: string, operation: string): Promise<ScheduleOutcome>;
  updateAccount(id: string, change: AccountChange): Promise<void>;
  /**
   * Your addresses. Each resolves to the account as the engine now has it (the demo mailbox has
   * none, so the screen's guess stays). Nothing changes on the server, except blocking.
   */
  addAddress(accountId: string, email: string, name: string | undefined): Promise<Account | undefined>;
  updateAddress(accountId: string, email: string, name: string | undefined): Promise<Account | undefined>;
  /** Off the account, or a suggestion said no to; Moss won't suggest it again. */
  removeAddress(accountId: string, email: string): Promise<Account | undefined>;
  /** Mail to it goes to the Blocked folder, here and on the server, or comes back. */
  blockAddress(accountId: string, email: string, blocked: boolean): Promise<Account | undefined>;
  /** Looks for addresses in your mail now, and in Gmail's "Send mail as" list. */
  discoverAddresses(accountId: string): Promise<Account | undefined>;
  /** Which signature mail from this address adds; null for the account's choice again. */
  setAddressSignatures(accountId: string, email: string, choice: AccountSignatures | null): Promise<void>;
  reorderAccounts(ids: string[]): Promise<void>;
  /** With `discardServerChanges`, removes it although that many server changes will be lost. */
  removeAccount(id: string, discardServerChanges?: number): Promise<CredentialCleanupStatus>;
  credentialCleanupStatus(): Promise<CredentialCleanupStatus>;
  retryCredentialCleanup(): Promise<CredentialCleanupStatus>;
  syncNow(accountId?: string): Promise<void>;
  syncProgress(accountId: string): Promise<SyncSnapshot | undefined>;
  /** Why accounts aren't syncing now: sync may have said so before anyone was listening. */
  syncProblems(): Promise<SyncIssue[]>;
  serverChangeStatus(accountId: string): Promise<ServerChangeStatus | null>;
  /** A blocked server change, as its status showed it, tried again now. */
  retryServerChange(accountId: string, change: string, revision: number): Promise<void>;
  /** A blocked server change, as its status showed it, stopped: it stays on this Mac only. */
  discardServerChange(accountId: string, change: string, revision: number): Promise<void>;
  /** Emails you sent that Moss stopped before they went, until the window says it showed them. */
  interruptedSends(): Promise<InterruptedSend[]>;
  dismissInterruptedSends(ids: string[]): Promise<void>;
  search(query: string, filters: SearchFilters): Promise<SearchHit[]>;
  /** Opens one of a message's files with the system's default app. */
  openAttachment(messageId: string, index: number): Promise<void>;
  /** A small picture of what's in one of a message's files, as an image URL; none if it can't have one. */
  attachmentPreview(messageId: string, index: number): Promise<string | undefined>;
  /** Fetches the body of a message stored with headers only; true if it changed. */
  loadMessage(messageId: string): Promise<boolean>;
  /** Fetches the bodies of these threads that aren't on this Mac yet, so they open at once. */
  preload(threadIds: string[]): Promise<void>;
  backupStatus(): Promise<BackupStatus>;
  createBackup(): Promise<number | null>;
  cancelBackup(id: number): Promise<boolean>;
  storage(): Promise<StorageInfo>;
  setOffline(offline: Offline): Promise<void>;
  setCacheLimit(bytes: number): Promise<void>;
  /** Turns reading the text inside attachments on or off. Off forgets the text kept. */
  setAttachmentText(on: boolean): Promise<void>;
  /** Removes the cache; resolves to the bytes freed. */
  clearCache(): Promise<number>;
  rebuildIndex(): Promise<void>;
  /** Calls `handler` for every engine event; returns a function that stops it. */
  onEvent(handler: (event: MailEvent) => void): () => void;
}

const daysAgo = (days: number) => Date.now() - days * 86_400_000;

/** The demo's blocked and accepted senders, matching its Blocked list. */
export const demoSenderRules: SenderRule[] = [
  { accountId: "studio", sender: "jordan@growthleads.example", wholeDomain: false, blocked: true, createdAt: daysAgo(0) },
  { accountId: "work", sender: "accounts.google.com", wholeDomain: true, blocked: false, createdAt: daysAgo(1) },
  { accountId: "work", sender: "promomail.example", wholeDomain: true, blocked: true, createdAt: daysAgo(3) },
  { accountId: "studio", sender: "alex@seo-boost.example", wholeDomain: false, blocked: true, createdAt: daysAgo(13) },
  { accountId: "personal", sender: "alex@studio-build.example.test", wholeDomain: false, blocked: false, createdAt: daysAgo(18) },
  { accountId: "personal", sender: "vault-bonus.example", wholeDomain: true, blocked: true, createdAt: daysAgo(20) },
];

/** The demo's storage, as drawn in design.pen. */
export const demoStorage: StorageInfo = {
  usage: { index: 38_000_000, offline: 61_000_000, cache: 142_000_000 },
  offline: "text",
  cacheLimit: 500_000_000,
  rebuilding: false,
  attachmentText: true,
  models: 6_410_000_000,
};

const done = async () => {};
const unchanged = async () => undefined;

/** The browser's file picker, for the demo mailbox: the files chosen, or none if you cancel. */
function browserFiles(): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.onchange = () => resolve([...(input.files ?? [])]);
    input.oncancel = () => resolve([]);
    input.click();
  });
}

/** The demo's held emails: what lets each one go, or not. */
const demoHeld = new Map<string, (send: boolean) => void>();

/** The demo mailbox: the store already changed the screen, so there is nothing more to do. */
const recoveryUnavailable = async (): Promise<never> => {
  throw new Error("Durable draft recovery requires the desktop app.");
};

export const demoMail: MailBackend = {
  live: false,
  accounts: async () => [],
  threads: async () => [],
  thread: async () => undefined,
  doneThreads: async () => [],
  markDone: done,
  markNotDone: done,
  deleteThread: done,
  restoreThread: done,
  markRead: done,
  blockedThreads: async () => [],
  blockSender: done,
  acceptSender: done,
  unblockSender: done,
  snooze: done,
  moveThread: done,
  waitForReply: async () => true,
  setSection: done,
  setSplit: done,
  clearDeadline: done,
  remindIfNoReply: async () => true,
  allowImages: done,
  signatures: async () => ({ signatures: [], byAccount: {}, byAddress: {}, letMossChoose: true, recent: [] }),
  saveSignature: done,
  deleteSignature: done,
  setSignatureChoice: done,
  setLetMossChoose: done,
  markSignatureUsed: done,
  contacts: async () => [],
  pickRecipient: done,
  recipientPicks: async () => [],
  drafts: async () => [],
  saveDraft: async (draft) => draft,
  draftSaveReceipt: async () => undefined,
  deliveryAttention: async () => ({ entries: [], next: null }),
  deliveryAttentionDraft: async () => undefined,
  resolveDelivery: async () => ({ status: "alreadyFinished" }),
  localDraftRecoveryIndex: async () => [],
  localDraftRecovery: async () => undefined,
  keepLocalDraftRecovery: recoveryUnavailable,
  discardLocalDraftRecovery: recoveryUnavailable,
  adoptLocalDraftRecovery: recoveryUnavailable,
  deleteDraft: done,
  pickFiles: browserFiles,
  // Nothing is kept: the demo only shows them.
  attachFiles: async (_draftId, files) =>
    files.map((f) =>
      typeof f === "string"
        ? { id: crypto.randomUUID(), name: f.split("/").pop()!, bytes: 0, contentType: "" }
        : { id: crypto.randomUUID(), name: f.name, bytes: f.size, contentType: f.type },
    ),
  discardDraftFile: done,
  draftPreview: unchanged,
  send: done,
  sendAfter: (hold, _draft, delayMs) =>
    new Promise((resolve) => {
      const release = (send: boolean) => {
        clearTimeout(timer);
        demoHeld.delete(hold);
        resolve(send);
      };
      const timer = setTimeout(() => release(true), delayMs);
      demoHeld.set(hold, release);
    }),
  cancelSend: async (hold) => {
    const release = demoHeld.get(hold);
    release?.(false);
    return { cancelled: !!release, early: !!release };
  },
  // The demo never sends: the Scheduled list on screen is all there is.
  scheduleSend: async () => ({ status: "scheduled" }),
  scheduled: async () => [],
  rescheduleSend: async (_id, _sendAt, _operation, expectedRevision) => ({ status: "rescheduled", revision: expectedRevision + 1 }),
  unscheduleSend: async () => ({ status: "cancelled", early: true }),
  updateAccount: done,
  addAddress: unchanged,
  updateAddress: unchanged,
  removeAddress: unchanged,
  blockAddress: unchanged,
  discoverAddresses: unchanged,
  setAddressSignatures: done,
  reorderAccounts: done,
  removeAccount: async () => ({ pending: false }),
  credentialCleanupStatus: async () => ({ pending: false }),
  retryCredentialCleanup: async () => ({ pending: false }),
  syncNow: done,
  syncProgress: async () => undefined,
  syncProblems: async () => [],
  serverChangeStatus: async () => null,
  retryServerChange: done,
  discardServerChange: done,
  interruptedSends: async () => [],
  dismissInterruptedSends: done,
  search: async () => [],
  openAttachment: done,
  attachmentPreview: async () => undefined,
  loadMessage: async () => false,
  preload: done,
  senderRules: async () => demoSenderRules,
  forgetSenderRule: done,
  backupStatus: async () => ({ state: "idle" }),
  createBackup: async () => null,
  cancelBackup: async () => false,
  storage: async () => demoStorage,
  setOffline: done,
  setCacheLimit: done,
  setAttachmentText: done,
  clearCache: async () => demoStorage.usage.cache,
  rebuildIndex: done,
  onEvent: () => () => {},
};

let current: MailBackend = demoMail;

export const mail = () => current;

export function setMail(backend: MailBackend) {
  current = backend;
}

/** Runs a background action; a failure is logged, since the engine reports server problems as events. */
export function quietly(action: Promise<unknown>) {
  action.catch((error: unknown) => console.warn("Moss:", error));
}
