import { useRemoteImages } from "./remoteImages";
import { create } from "zustand";
import { accounts as demoAccounts, blockedThreads as demoBlocked, doneThreads as demoDone, splits, threads as demoThreads } from "../data/demo";
import type { Account, AccountAddress, DraftFile, Message, Person, RecipientPick, Section, SplitId, Thread } from "../data/types";
import { fileSize, tooBig } from "../lib/attachments";
import { doneToday } from "../lib/done";
import { canPreview } from "../lib/fileKind";
import { stripRe } from "../lib/subject";
import { domainOf, isPublicDomain } from "../lib/senders";
import { listTime, remindTime, tomorrowMorning } from "../lib/time";
import { addressOf, inView, isMine, isValidAddress, replyFrom, sendingAddress } from "../lib/addresses";
import { ai } from "../services/ai";
import { mail, quietly, type Sent } from "../services/mail";
import { showCredentialCleanup, refreshCredentialCleanup } from "../services/credentialCleanup";
import { errorText } from "../services/errorText";
import { preserveCancelledDraft, keepUncertainDraft, retainVolatileDraft, reloadDrafts, releaseResolvedLocalDraft, localRecoveryFor, adoptDraft, attachmentImportPending, beginAttachmentImport, attachDraftFiles, discardDraftFile, forgetDraft, handoffDraft, keepDraft, stopDraftSave, threadDraft, useDrafts, visibleDrafts } from "./drafts";
import { useSearch, type PutBack } from "./search";
import { pickFor, signedBefore, useSignatures, type SignaturePick } from "./signatures";
import { initialPreferences, systemTheme } from "./usePreferences";
import { isTauri } from "../lib/platform";
import { textToHtml } from "../lib/textHtml";
import { plainText } from "../lib/markdown";

export type View = "inbox" | "thread" | "compose" | "zero" | "add-account" | "settings" | "search" | "done" | "blocked" | "drafts" | "senders" | "scheduled";
export type SettingsSection = "accounts" | "signatures" | "appearance" | "ai" | "storage" | "keyboard";
export type ComposeMode = "reply" | "reply-all" | "forward" | "new";
export type Theme = "dark" | "light";
/** "system" follows the macOS / OS light-dark setting. */
export type ThemeChoice = "system" | Theme;
/** Text size of the inbox list, the reading view and emails. */
export type ReadingSize = "smaller" | "default" | "larger";

export interface Draft {
  /** Store-issued identity of the content this edit started from. */
  revision?: string;
  /** Stays the same as you edit, so saving replaces the kept copy. */
  id: string;
  mode: ComposeMode;
  threadId?: string;
  fromAccountId: string;
  /** Which of the account's addresses it goes from; the account's own when not set. */
  fromAddress?: string;
  to: Person[];
  cc: Person[];
  bcc: Person[];
  subject: string;
  /** Plain text, used for checks like "mentions an attachment". */
  body: string;
  /** Email-ready HTML from the editor. */
  bodyHtml: string;
  /** Collapsed shows one summary line instead of the From / To / CC / BCC rows. */
  fieldsOpen: boolean;
  showCc: boolean;
  showBcc: boolean;
  remindIfNoReply: boolean;
  /** Why this From account was picked, e.g. "The account you use with Priya". */
  fromReason?: string;
  /** The signature added under the body; null for none. */
  signatureId: string | null;
  /** You picked the signature yourself, so changing From no longer swaps it. */
  signatureChosen?: boolean;
  /** The signature picker (⌥S) is open. */
  signatureOpen?: boolean;
  /** The From picker (⌥F) is open. */
  fromOpen?: boolean;
  /** The files that go with it, in the order you added them; none yet when not set. */
  attachments?: DraftFile[];
}

/** Something ⌘Z can take back: the email as it was, and where it sat in its list. */
export interface MoveStep {
  /** `deleted` from the list, `deletedDone` from the Done list; `moved` to another tab (M). */
  kind: "done" | "notDone" | "later" | "deleted" | "deletedDone" | "moved";
  thread: Thread;
  index: number;
  /** The screen you were on. */
  view: View;
  /** Deleted from Search: the result to put back. */
  hit?: PutBack;
}

/** An email deleted that no list here holds (found by Search: only you sent it, or it's long
 * done). ⌘Z brings it back in the engine, which knows where it was; the lists reload. */
export interface FoundStep {
  kind: "deletedFound";
  thread: Pick<Thread, "id">;
  view: View;
  hit?: PutBack;
}

/** A sent email held for a few seconds, or one that couldn't go: ⌘Z opens it again in the composer. */
export interface SendStep {
  kind: "send";
  /** What the mail engine knows this held email by. */
  hold: string;
  draft: Draft;
  /** When it goes; ⌘Z can't take it back after that. */
  until: number;
  /** Why it couldn't go. */
  failed?: string;
}

/** An email set to send later: ⌘Z takes it back into the composer. */
export interface ScheduleStep {
  kind: "schedule";
  draft: Draft;
  takeBack: () => void;
}

export type UndoStep = MoveStep | FoundStep | SendStep | ScheduleStep;

/** The note at the bottom after an action, with Undo. A new `id` restarts its timer. */
export interface Toast {
  id: number;
  text: string;
  /** Something didn't work: no Undo, and an alert instead of a tick. */
  problem?: boolean;
  /** Nothing for ⌘Z to take back, so no Undo (blocking, accepting). */
  undo?: false;
  /** What the Undo button says instead, e.g. "Open". */
  undoLabel?: string;
  /** In place of Undo: a button to where the change went, with its key. */
  show?: { label: string; keys?: string; run: () => void };
  /** Stays up until another note replaces it. */
  stays?: boolean;
  /** A held email: counts down to when it goes, then says it is sending. It stays up until then. */
  until?: number;
  /** The held email this note is about. */
  hold?: string;
}

interface State {
  themeChoice: ThemeChoice;
  /** The theme on screen: the choice, or the system theme when following it. */
  theme: Theme;
  readingSize: ReadingSize;
  view: View;
  accounts: Account[];
  /** `unsentChanges`: the refusal said this many server changes would be lost. */
  accountRemovals: Record<string, { token: symbol; pending: boolean; error?: string; unsentChanges?: number }>;
  accountFilter: string;
  /** Inside one account: only the mail that came to this address of it. */
  addressFilter?: string;
  split: SplitId;
  threads: Thread[];
  cursorId?: string;
  openThreadId?: string;
  /** Real mail: people to suggest as recipients, from all your mail, the ones you write to most first. */
  knownPeople: Person[];
  /** Who you chose from the recipient suggestions for what you typed, newest first. */
  recipientPicks: RecipientPick[];
  /** Remembers that you chose `email` from the recipient suggestions after typing `typed`. */
  pickRecipient(typed: string, email: string): void;
  /** Emails you marked done, most recently done first. */
  doneThreads: Thread[];
  doneCursorId?: string;
  /** Mail from senders you blocked, newest first. It lives in each account's Blocked folder. */
  blockedThreads: Thread[];
  blockedCursorId?: string;
  /**
   * Emails opened that no list holds: ones only you sent, or done longer ago than the Done
   * list reaches. Read from the engine when opened (from Search, Ask or a forward), and kept
   * apart so reloading the lists can't take an open one away.
   */
  foundThreads: Thread[];
  /** What ⌘Z can take back, newest last. */
  undoSteps: UndoStep[];
  toast?: Toast;
  paletteOpen: boolean;
  /** The email the Remind menu is open for, while it is. */
  remindingId?: string;
  /** The Remind menu is for "if no one replies": your email waits for an answer until then. */
  remindNoReply?: boolean;
  /** The email the Move menu (M) is open for, while it is. */
  movingId?: string;
  /** Text the palette opens with. */
  paletteQuery: string;
  recipientsOpen: boolean;
  /** Messages whose remote images you chose to load. Hidden by default, for privacy. */
  imagesShown: string[];
  /** Thumbnails of attachments by "messageId:index", and of files you attached by "draft:id": an image URL, or null while asking or when there is none. */
  previews: Record<string, string | null>;
  replyChoice: number;
  /** The Quick reply as you changed it; none until you type in it. */
  replyDraft?: string;
  draft?: Draft;
  /** Where you were when the composer opened: after Send, an email that answers none goes back there. */
  composedFrom?: Pick<State, "view" | "openThreadId">;
  /** Send was pressed: the composer shows it for a moment, then gives way to the list. */
  sending?: boolean;
  /** Why the email in the composer couldn't go. */
  sendError?: string;
  /** Send was pressed on a new email with no subject: the composer asks, and Send again sends it. */
  noSubjectAsked?: boolean;
  /** Why the files you chose or dropped couldn't be attached. */
  attachProblem?: string;
  handled: number;
  settingsSection: SettingsSection;
  /** Where Esc goes back to from Add account, Settings and Search: the screens they were opened from. */
  history: View[];

  setTheme(choice: ThemeChoice): void;
  setReadingSize(size: ReadingSize): void;
  setAccountFilter(id: string): void;
  /** Narrows the account you are looking at to the mail that came to one of its addresses; none for all of them. */
  setAddressFilter(email: string | undefined): void;
  /** Adds a newly connected account at the end of the account row. */
  addAccount(account: Account): void;
  /** Rename or recolour an account. */
  updateAccount(id: string, patch: Partial<Pick<Account, "name" | "letter" | "color">>): void;
  /** Move an account one place earlier (-1) or later (1) in the account row. */
  moveAccount(id: string, step: 1 | -1): void;
  /** Forget an account and its mail on this device. Nothing changes on the server. */
  /** With `discardServerChanges`, removes it although that many server changes will be lost. */
  removeAccount(id: string, discardServerChanges?: number): Promise<boolean>;
  /** You send from this address too (a suggested one becomes yours). Nothing changes on the server. */
  addAddress(accountId: string, email: string, name?: string): void;
  /** The name mail from this address shows; blank for the account's sender name. */
  renameAddress(accountId: string, email: string, name: string): void;
  /** Takes an address off the account, or says no to a suggested one. Moss won't suggest it again. */
  removeAddress(accountId: string, email: string): void;
  /** Mail to this address goes to Blocked (or comes back), here and on the server. */
  blockAddress(accountId: string, email: string, blocked: boolean): void;
  /** Asks the engine to look for addresses in your mail, and Gmail for its "Send mail as" list. Resolves once it answered. */
  discoverAddresses(accountId: string): Promise<void>;
  setSplit(split: SplitId): void;
  /** Move to the next (1) or previous (-1) split tab, wrapping around. */
  cycleSplit(step: 1 | -1): void;
  setCursor(id: string): void;
  moveCursor(step: 1 | -1): void;
  open(id?: string): void;
  /** Opens any email, reading it from the engine first when no list has it. False when it's gone. */
  openFound(threadId: string): Promise<boolean>;
  back(): void;
  /** Done, or not done again for an email that already is. */
  markDone(id?: string): void;
  /** Back from the Done list to the inbox. */
  markNotDone(id?: string): void;
  /** Moves an email to Trash, from the list, the Done list or Search (`hit`, the result it
   * was, so ⌘Z puts that back too). ⌘Z brings it back. */
  deleteThread(id?: string, hit?: PutBack): void;
  /** Hides an email until `until` (ms; 8 AM tomorrow when left out), then it comes back to the top. */
  remindLater(id?: string, until?: number): void;
  /** Opens the Remind menu (H) for an email: the one given, the open one, or the one under the cursor.
   * `noReply` (⇧H) asks when to bring your own email back if no one answers; only when the latest is yours. */
  openRemind(id?: string, noReply?: boolean): void;
  closeRemind(): void;
  /** Your latest email waits for an answer: out of sight until `until` (ms), back sooner if someone replies. */
  waitForReply(until: number, id?: string): void;
  /** Opens the Move menu (M) for an email: the one given, the open one, or the one under the cursor. */
  openMove(id?: string): void;
  closeMove(): void;
  /** Moves an email to another tab for good (null: Moss sorts it again). To Important it needs you;
   * for your own email, Moss then asks when to bring it back if no one replies. */
  moveTo(split: SplitId | null, id?: string): void;
  /** Corrections to what Moss decided, on the email given, the open one, or the one under the cursor. Each wins for good. */
  moveToSection(section: "reply" | "info", id?: string): void;
  /** Takes the due chip away. */
  clearDeadline(id?: string): void;
  /** ⇧H: your latest email comes back if no one replies (out of sight until then); `false` stops waiting. */
  remindIfNoReply(id?: string, on?: boolean): void;
  /** Inbox zero on demand: the screen when nothing waits, or how much still does. */
  showInboxZero(): void;
  /** Marks an email unread, or read again: the one given, the open one, or the one under the cursor. */
  toggleRead(id?: string): void;
  /** Takes back the last done, not done, delete or snooze, or a send while it waits. */
  undo(): void;
  dismissToast(): void;
  openDone(): void;
  setDoneCursor(id: string): void;
  moveDoneCursor(step: 1 | -1): void;
  /** Blocks this email's sender (with `wholeDomain`, everyone at their domain) in its account. */
  blockSender(wholeDomain?: boolean, id?: string): void;
  /** Never asks "Block this sender?" about this email's sender (with `wholeDomain`, anyone at their domain) again. */
  acceptSender(wholeDomain?: boolean, id?: string): void;
  /** Unblocks this blocked email's sender: their mail comes back to the inbox. */
  unblockSender(id?: string): void;
  openBlocked(): void;
  /** Everyone you blocked or accepted, so either can be undone. */
  openSenders(): void;
  setBlockedCursor(id: string): void;
  moveBlockedCursor(step: 1 | -1): void;
  setPaletteOpen(open: boolean): void;
  toggleRecipients(): void;
  /** Just this once: one message's remote images, until Moss closes. */
  showImages(messageId: string): void;
  /** Images from this message's sender, from now on, in its account. */
  allowImages(messageId: string): void;
  /** Opens one of a message's files with the system's default app. */
  openAttachment(messageId: string, index: number): void;
  /** Asks the mail engine for thumbnails of a message's files that can have one. */
  loadPreviews(message: Message): void;
  setReplyChoice(index: number): void;
  setReplyDraft(text: string): void;
  /** A new email, reply or forward. A reply opens the one you have half written there, unless `fresh`. */
  compose(mode: ComposeMode, threadId?: string, fresh?: boolean): void;
  /** The Quick reply in the composer; `signatureId`, when given, is the signature you picked on it (null: none). */
  editInstantReply(signatureId?: string | null): void;
  updateDraft(patch: Partial<Draft>): void;
  toggleSignaturePicker(): void;
  toggleFromPicker(): void;
  /** Send the draft from this address (of this account) and close the picker. */
  chooseFrom(accountId: string, email: string): void;
  /** Use this signature (or none) on the draft and close the picker. */
  chooseSignature(id: string | null): void;
  /** Attaches files to the email being written: these (dropped), or the ones you choose in the file picker. */
  attach(files?: string[] | File[]): void;
  /** Takes a file off the email being written. */
  removeAttachment(id: string): void;
  send(): void;
  /** Sends a draft written elsewhere (the Ask panel), with Undo, without leaving this screen. */
  sendDraft(draft: Draft): void;
  /** Sends the Quick reply; `signatureId`, when given, is the signature you picked on it (null: none). */
  sendInstantReply(signatureId?: string | null): void;
  openDrafts(): void;
  /** Back to a saved draft, in the composer. */
  openDraft(id: string): void;
  /** Deletes the draft being written and closes the composer. */
  discardDraft(): void;
  /** Deletes a saved draft: the one given, or the one under the cursor in the Drafts list. */
  deleteDraft(id?: string): void;
  openAddAccount(): void;
  openSettings(section?: SettingsSection): void;
  openSearch(): void;
  /** Close Add account, Settings or Search and go back to the screen underneath. */
  leave(): void;
}

const sectionOrder: Record<Section, number> = { reply: 0, info: 1, later: 2 };

/** In Important, what you moved there comes first in its section. */
const yoursFirst = (t: Thread) => (t.split === "important" && t.movedByYou ? 0 : 1);

/** Threads shown in the list for the current account and split, in display order. Like
 * `list_threads` in moss-store. */
export function visibleThreads(s: Pick<State, "threads" | "accountFilter" | "split" | "addressFilter">): Thread[] {
  return s.threads
    .filter((t) => t.split === s.split && inView(t, s.accountFilter, s.addressFilter))
    .sort((a, b) => sectionOrder[a.section] - sectionOrder[b.section] || yoursFirst(a) - yoursFirst(b));
}

/**
 * Tab counts for an account filter: how many emails wait in each tab, For your info
 * included, leaving out those reminded for later. Like `split_counts` in moss-store.
 */
export function splitCountsFor(threads: Thread[], accountFilter: string, addressFilter?: string): Record<SplitId, number> {
  const counts = { important: 0, other: 0, calendar: 0, newsletters: 0, receipts: 0 };
  for (const t of threads) {
    // Reminded mail waits nowhere until its time comes.
    if (t.section === "later" || !inView(t, accountFilter, addressFilter)) continue;
    counts[t.split] += 1;
  }
  return counts;
}

/** Unread emails in each split, not counting snoozed ones: they aren't back yet. */
export function unreadCountsFor(threads: Thread[], accountFilter: string, addressFilter?: string): Record<SplitId, number> {
  const counts = { important: 0, other: 0, calendar: 0, newsletters: 0, receipts: 0 };
  for (const t of threads) {
    if (!t.unread || t.section === "later" || !inView(t, accountFilter, addressFilter)) continue;
    counts[t.split] += 1;
  }
  return counts;
}

/** Each account's badge, by the same rule as the split tabs, and `all` of them together. */
export function accountCountsFor(threads: Thread[], accountIds: string[]): Record<string, number> {
  const total = (filter: string) => Object.values(splitCountsFor(threads, filter)).reduce((a, b) => a + b, 0);
  return Object.fromEntries(["all", ...accountIds].map((id) => [id, total(id)]));
}

/** Today's progress toward inbox zero: emails done today, out of those plus what still waits. */
export function progressFor(threads: Thread[], doneThreads: Thread[], accountFilter: string, now: number) {
  const left = accountCountsFor(threads, [accountFilter])[accountFilter];
  const done = visibleDone({ doneThreads, accountFilter }).filter((t) => t.doneAt !== undefined && doneToday(t.doneAt, now)).length;
  return { done, total: done + left, eta: left ? `${left} left` : "Inbox zero" };
}

const noReply = /^no-?reply@/i;

/** Everyone in your mail but you and no-reply senders, most often seen first: for recipient autocomplete. */
export function contactsFrom(threads: Thread[], accounts: Account[], known: Person[] = []): Person[] {
  const seen = new Map<string, { person: Person; times: number }>();
  // Already ranked by the engine from all your mail, so they come first.
  for (const p of known) seen.set(p.email.toLowerCase(), { person: p, times: Infinity });
  for (const m of threads.flatMap((t) => t.messages)) {
    for (const p of [m.from, ...m.to, ...m.cc]) {
      const key = p.email.toLowerCase();
      if (isMine(key, accounts) || noReply.test(key)) continue;
      const known = seen.get(key);
      if (known) known.times++;
      else seen.set(key, { person: p, times: 1 });
    }
  }
  return [...seen.values()].sort((a, b) => b.times - a.times).map((c) => c.person);
}

/** The Done list for the account you are looking at. */
export function visibleDone(s: Pick<State, "doneThreads" | "accountFilter">): Thread[] {
  return s.doneThreads.filter((t) => s.accountFilter === "all" || t.accountId === s.accountFilter);
}

/** The Blocked list for the account you are looking at. */
export function visibleBlocked(s: Pick<State, "blockedThreads" | "accountFilter">): Thread[] {
  return s.blockedThreads.filter((t) => s.accountFilter === "all" || t.accountId === s.accountFilter);
}

/** Threads still waiting on you (not snoozed). When none are left you reach inbox zero. */
function waiting(s: Pick<State, "threads" | "accountFilter" | "split" | "addressFilter">) {
  return visibleThreads(s).filter((t) => t.section !== "later");
}

/**
 * The screen that can actually be shown. A thread view without its thread, or a composer
 * without a draft, has nothing to draw; fall back to the inbox rather than a blank window.
 */
export function shownView(s: Pick<State, "view" | Lists | "openThreadId" | "draft">): View {
  if (s.view === "thread" && !openThread(s)) return "inbox";
  if (s.view === "compose" && !s.draft) return "inbox";
  return s.view;
}

/** Everywhere an email on screen can be. */
type Lists = "threads" | "doneThreads" | "blockedThreads" | "foundThreads";

/** An email in the list, the Done list, the Blocked list, or one opened that no list holds. */
export function findThread(s: Pick<State, Lists>, id: string | undefined): Thread | undefined {
  // Runs in selectors on every store change: no copy of the lists, and no search without an id.
  if (id === undefined) return undefined;
  return s.threads.find((t) => t.id === id) ?? s.doneThreads.find((t) => t.id === id) ?? s.blockedThreads.find((t) => t.id === id) ?? s.foundThreads.find((t) => t.id === id);
}

/** The email being read. */
export function openThread(s: Pick<State, Lists | "openThreadId">): Thread | undefined {
  return findThread(s, s.openThreadId);
}

/** Why the draft's files are too many for the account it goes from, or nothing when they fit. */
export function filesTooBig(d: Draft): string | undefined {
  const files = d.attachments ?? [];
  const total = files.reduce((sum, f) => sum + f.bytes, 0);
  const over = tooBig(total, accountFor(d.fromAccountId).connection?.provider);
  if (!over || files.length < 2) return over;
  // Which one to take off first.
  const biggest = files.reduce((a, b) => (b.bytes > a.bytes ? b : a));
  return `${over} The biggest is ${biggest.name} (${fileSize(biggest.bytes)}).`;
}

/** The forwarded email's files, which go with the forward unless you take them off. */
function forwardedFiles(message: Message): DraftFile[] {
  return message.attachments.map((a, i) => {
    // Demo mail's files have no index: their place in the list stands in.
    const index = a.index ?? i;
    const forwarded = { messageId: message.id, index };
    return { id: `${message.id}:${index}`, name: a.name, bytes: a.bytes ?? 0, contentType: a.contentType ?? "", forwarded };
  });
}

export function accountFor(id: string) {
  const { accounts } = useStore.getState();
  return accounts.find((a) => a.id === id) ?? accounts[0];
}

/** How long a sent email waits for Undo: Gmail and Spark wait 5 s; one more gives time to read the note. */
export const undoSendMs = 6000;
/** How long the pressed Send shows before the composer gives way to the list. */
export const sendBeatMs = 250;

/** How many server changes removing an account would lose, when that is all that stops it. */
function unsentServerChanges(error: unknown): number | undefined {
  if (!error || typeof error !== "object" || !("kind" in error) || error.kind !== "unsentServerChanges" || !("count" in error)) return undefined;
  const { count } = error;
  return typeof count === "number" && Number.isSafeInteger(count) && count > 0 ? count : undefined;
}

let toastId = 0;
export const toast = (text: string): Toast => ({ id: ++toastId, text });

const without = (list: Thread[], id: string) => list.filter((t) => t.id !== id);

/** `list` with `change` made to thread `id`. */
const changed = (list: Thread[], id: string, change: Partial<Thread>) => list.map((t) => (t.id === id ? { ...t, ...change } : t));

/** The email an action is about: the one given, the open one, or the one under the cursor. */
const targetOf = (s: State, id?: string) => id ?? (s.view === "thread" ? s.openThreadId : s.cursorId);

/** Whether an address is this thread's sender (with `wholeDomain`, anyone at their domain). */
function isSender(thread: Thread, wholeDomain = false) {
  const email = thread.from.email.toLowerCase();
  const domain = domainOf(email);
  return (address: string) => (wholeDomain ? domainOf(address) === domain : address.toLowerCase() === email);
}

/** The same sender in the same account: whose mail a block or an accept is about. */
function sameSender(thread: Thread, wholeDomain = false) {
  const is = isSender(thread, wholeDomain);
  return (t: Thread) => t.accountId === thread.accountId && is(t.from.email);
}

/** Marks the messages in that account from whoever `from` picks as showing their images. */
function withImagesFrom(accountId: string, from: (address: string) => boolean) {
  const mine = (m: Message) => from(m.from.email);
  return (t: Thread): Thread =>
    t.accountId !== accountId || !t.messages.some(mine)
      ? t
      : { ...t, messages: t.messages.map((m) => (mine(m) ? { ...m, imageTrust: "you-allowed" as const } : m)) };
}

/** A thread with its state set, and no longer done. */
const as = (sender: Thread["sender"]) => ({ doneAt: _, ...t }: Thread): Thread => ({ ...t, sender });

function insertAt(list: Thread[], index: number, thread: Thread) {
  return [...list.slice(0, index), thread, ...list.slice(index)];
}

/** The email an action acts on: the open one, the one under the cursor, or in the Done list its cursor's. */
function actionTarget(s: State) {
  return s.view === "thread" ? s.openThreadId : s.view === "done" ? s.doneCursorId : s.cursorId;
}

/** Whether the latest email in a thread is yours: the engine says for a row whose messages haven't loaded. */
export function wroteLast(t: Thread, accounts: Account[]) {
  const last = t.messages.at(-1);
  return t.youWroteLast ?? (!t.partial && !!last && isMine(last.from.email.toLowerCase(), accounts));
}

/** A thread in the tab you moved it to: in Important it needs you; elsewhere it's for your
 * information. Snoozed mail stays snoozed. `null` gives it back to Moss, which places it once the
 * lists reload. Like `summary_sql` in moss-store. */
function placedIn(t: Thread, to: SplitId | null): Thread {
  if (!to) {
    const { movedByYou: _, ...back } = t;
    return back;
  }
  const section: Section = t.section === "later" ? "later" : to === "important" ? "reply" : "info";
  // Why the AI put it elsewhere no longer counts.
  const { splitWhy: _, ...rest } = t;
  return { ...rest, split: to, section, movedByYou: true };
}

/**
 * Hides an email until `at`, then it comes back to the top. Keeps your place: the email below
 * takes its spot, or the one above when it was last. `said` words the note from when it's back.
 */
function hideUntil(target: string, at: number, said: (when: string) => string, patch: Partial<Thread> = {}) {
  const s = useStore.getState();
  const now = Date.now();
  const index = s.threads.findIndex((t) => t.id === target);
  const threads = s.threads.map((t) =>
    t.id === target ? { ...t, section: "later" as const, time: listTime(at, now), due: undefined, ...patch } : t,
  );
  const i = waiting(s).findIndex((t) => t.id === target);
  const list = waiting({ ...s, threads });
  const next = list[Math.min(i, list.length - 1)];
  useStore.setState({
    threads,
    remindingId: undefined,
    remindNoReply: false,
    ...landOn(s, threads, next),
    cursorId: next?.id ?? target,
    ...(index >= 0 && {
      undoSteps: remember(s.undoSteps, { kind: "later", thread: s.threads[index], index, view: s.view }),
      toast: toast(said(remindTime(at, now))),
    }),
  });
  movedOn(s);
}

/** The list once an email leaves it (done or deleted), and where you land: Search stays put;
 * reading an email moves you on to the next one; sending from the composer (or finishing the
 * last email) takes you back to the list. */
function leaveList(s: State, target: string) {
  const i = visibleThreads(s).findIndex((t) => t.id === target);
  const threads = without(s.threads, target);
  const after = visibleThreads({ ...s, threads });
  return { threads, handled: s.handled + 1, draft: undefined, ...landOn(s, threads, after[Math.min(i, after.length - 1)]) };
}

/** What still waits for you in Important, in every account. */
export const stillWaiting = (threads: Thread[]) => waiting({ threads, accountFilter: "all", split: "important" }).length;

/** Inbox zero is the moment the last email waiting in Important, in any account, is handled.
 * Other splits, or one account cleared while another still waits, just show the list's empty
 * state; so does handling what's left after you went back to the list yourself. */
function justCleared(s: State, threads: Thread[]) {
  return s.split === "important" && stillWaiting(s.threads) > 0 && stillWaiting(threads) === 0;
}

/** Where you land once an email leaves what you were looking at: Search stays put; reading moves
 * you on to `next`; otherwise the list, or inbox zero when that was the last one waiting. */
function landOn(s: State, threads: Thread[], next: Thread | undefined) {
  const view: View = s.view === "search" ? "search" : justCleared(s, threads) ? "zero" : s.view === "thread" && next ? "thread" : "inbox";
  return {
    cursorId: next?.id,
    openThreadId: view === "thread" ? next?.id : undefined,
    view,
    recipientsOpen: false,
    replyChoice: 0,
    replyDraft: undefined,
    ...(view !== "thread" && view !== "search" && { history: [] }),
  };
}

/** Keeps the last 20 steps. */
export const remember = (undo: UndoStep[], step: UndoStep) => [...undo, step].slice(-20);

/** Screens where the email list is what you see, so undo can take you back to the email. */
const onTheList: View[] = ["inbox", "zero", "thread", "compose"];

/**
 * The demo mailbox, for the browser and dev scenes. The desktop app starts empty and shows mail
 * only once it has loaded the real thing, so demo mail can never sit on top of a real engine.
 */
const startingMail = isTauri
  ? { accounts: [], threads: [], doneThreads: [], blockedThreads: [] }
  : { accounts: demoAccounts, threads: demoThreads, doneThreads: demoDone, blockedThreads: demoBlocked };

const initial = visibleThreads({ threads: startingMail.threads, accountFilter: "all", split: "important" });

export const useStore = create<State>()((set, get) => ({
  ...initialPreferences(),
  accountRemovals: {},
  view: "inbox",
  accountFilter: "all",
  split: "important",
  ...startingMail,
  cursorId: initial[0]?.id,
  knownPeople: [],
  recipientPicks: [],
  pickRecipient: (typed, email) => {
    const pick = { typed: typed.trim().toLowerCase(), email: email.toLowerCase() };
    set((s) => ({ recipientPicks: [pick, ...s.recipientPicks.filter((p) => p.typed !== pick.typed || p.email !== pick.email)] }));
    quietly(mail().pickRecipient(pick.typed, pick.email));
  },
  foundThreads: [],
  undoSteps: [],
  paletteOpen: false,
  paletteQuery: "",
  recipientsOpen: false,
  imagesShown: [],
  previews: {},
  replyChoice: 0,
  handled: 0,
  settingsSection: "accounts",
  history: [],

  setTheme: (themeChoice) => set({ themeChoice, theme: themeChoice === "system" ? systemTheme() : themeChoice }),
  setReadingSize: (readingSize) => set({ readingSize }),

  setAccountFilter: (accountFilter) => {
    const list = visibleThreads({ ...get(), accountFilter, addressFilter: undefined });
    set({ accountFilter, addressFilter: undefined, cursorId: list[0]?.id, view: "inbox", history: [] });
  },

  setAddressFilter: (email) => {
    const s = get();
    if (email && s.accountFilter === "all") return;
    const addressFilter = email?.toLowerCase();
    set({ addressFilter, cursorId: visibleThreads({ ...s, addressFilter })[0]?.id });
  },

  addAccount: (account) => {
    const { accounts } = get();
    if (!accounts.some((a) => a.id === account.id)) {
      const accountRemovals = { ...get().accountRemovals };
      delete accountRemovals[account.id];
      set({ accounts: [...accounts, account], accountRemovals });
    }
  },

  updateAccount: (id, patch) => {
    set((s) => ({ accounts: s.accounts.map((a) => (a.id === id ? { ...a, ...patch } : a)) }));
    quietly(mail().updateAccount(id, patch));
  },

  moveAccount: (id, step) => {
    const accounts = [...get().accounts];
    const i = accounts.findIndex((a) => a.id === id);
    const j = i + step;
    if (i < 0 || j < 0 || j >= accounts.length) return;
    [accounts[i], accounts[j]] = [accounts[j], accounts[i]];
    set({ accounts });
    quietly(mail().reorderAccounts(accounts.map((a) => a.id)));
  },

  removeAccount: async (id, discardServerChanges) => {
    const current = get();
    if (!current.accounts.some(a => a.id === id) || current.accountRemovals[id]?.pending) return false;
    const token = Symbol(id);
    set({ accountRemovals: { ...current.accountRemovals, [id]: { token, pending: true } } });
    try {
      const cleanup = await (discardServerChanges === undefined ? mail().removeAccount(id) : mail().removeAccount(id, discardServerChanges));
      if (get().accountRemovals[id]?.token !== token) return false;
      if (cleanup?.pending === false) void refreshCredentialCleanup();
      else showCredentialCleanup(cleanup);
      set((s) => {
        const accounts = s.accounts.filter((a) => a.id !== id);
        const accountRemovals = { ...s.accountRemovals };
        delete accountRemovals[id];
        return {
          accounts, accountRemovals,
          threads: s.threads.filter((t) => t.accountId !== id),
          doneThreads: s.doneThreads.filter((t) => t.accountId !== id),
          blockedThreads: s.blockedThreads.filter((t) => t.accountId !== id),
          foundThreads: s.foundThreads.filter((t) => t.accountId !== id),
          accountFilter: s.accountFilter === id ? "all" : s.accountFilter,
          ...(!accounts.length && { view: "add-account" as const, history: [] }),
        };
      });
      useDrafts.setState(s => ({ drafts: s.drafts.filter(d => d.fromAccountId !== id) }));
      return true;
    } catch (error) {
      if (get().accountRemovals[id]?.token !== token) return false;
      const unsentChanges = unsentServerChanges(error);
      set(s => ({ accountRemovals: { ...s.accountRemovals, [id]: { token, pending: false, error: errorText(error), ...(unsentChanges && { unsentChanges }) } } }));
      return false;
    }
  },

  setSplit: (split) => {
    const list = visibleThreads({ ...get(), split });
    set({ split, cursorId: list[0]?.id });
  },

  cycleSplit: (step) => {
    const order = splits.map((x) => x.id);
    const i = order.indexOf(get().split);
    get().setSplit(order[(i + step + order.length) % order.length]);
  },

  setCursor: (cursorId) => set({ cursorId }),

  moveCursor: (step) => {
    const s = get();
    // Reading a done or blocked email moves through its own list.
    const reading = s.view === "thread" && !s.threads.some((t) => t.id === s.openThreadId);
    const inBlocked = reading && s.blockedThreads.some((t) => t.id === s.openThreadId);
    const list = inBlocked ? visibleBlocked(s) : reading ? visibleDone(s) : visibleThreads(s);
    if (!list.length) return;
    const current = s.view === "thread" ? s.openThreadId : s.cursorId;
    const i = Math.max(0, list.findIndex((t) => t.id === current));
    const next = list[Math.min(list.length - 1, Math.max(0, i + step))];
    if (s.view === "thread") {
      const cursor = inBlocked ? "blockedCursorId" : reading ? "doneCursorId" : "cursorId";
      set({ openThreadId: next.id, [cursor]: next.id, recipientsOpen: false, replyChoice: 0, replyDraft: undefined });
      opened(next.id);
    } else set({ cursorId: next.id });
  },

  open: (id) => {
    const s = get();
    const target = id ?? s.cursorId;
    if (!target) return;
    const listed = s.threads.some((t) => t.id === target);
    const done = !listed && s.doneThreads.some((t) => t.id === target);
    const blocked = !listed && s.blockedThreads.some((t) => t.id === target);
    set({
      // Opened from Search, the Done list, Blocked or Drafts: Esc goes back there.
      ...(s.view === "search" || s.view === "done" || s.view === "blocked" || s.view === "drafts" ? goTo(s, "thread") : { view: "thread" as const }),
      openThreadId: target,
      // One no list holds leaves every list's cursor where it was.
      ...(blocked ? { blockedCursorId: target } : done ? { doneCursorId: target } : listed && { cursorId: target }),
      recipientsOpen: false,
      replyChoice: 0,
      replyDraft: undefined,
    });
    opened(target);
  },

  openFound: async (threadId) => {
    if (!findThread(get(), threadId)) {
      const found = await mail().thread(threadId);
      if (!found) return false;
      set((s) => ({ foundThreads: [...s.foundThreads, found] }));
    }
    get().open(threadId);
    return true;
  },

  back: () => {
    const { view, draft, history } = get();
    if (view === "compose" && draft?.threadId) set({ view: "thread" });
    else if (view === "thread" && history.length) get().leave();
    else set({ view: "inbox", history: [] });
  },

  markDone: (id) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.view === "done" ? s.doneCursorId : s.cursorId);
    if (!target || s.blockedThreads.some((t) => t.id === target)) return;
    const index = s.threads.findIndex((t) => t.id === target);
    if (index < 0 && s.doneThreads.some((t) => t.id === target)) return get().markNotDone(target);
    const thread = s.threads[index];
    settle(mail().markDone(target), target);
    set({
      ...leaveList(s, target),
      ...(thread && {
        doneThreads: [{ ...thread, doneAt: Date.now() }, ...s.doneThreads],
        undoSteps: remember(s.undoSteps, { kind: "done", thread, index, view: s.view }),
        toast: toast("Moved to Done"),
      }),
    });
    movedOn(s);
  },

  deleteThread: (id, hit) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.view === "done" ? s.doneCursorId : s.cursorId);
    const index = s.threads.findIndex((t) => t.id === target);
    const doneIndex = s.doneThreads.findIndex((t) => t.id === target);
    const found = s.foundThreads.some((t) => t.id === target) || !!hit;
    const blocked = s.blockedThreads.some((t) => t.id === target);
    if (!target || blocked || (index < 0 && doneIndex < 0 && !found)) return;
    settle(mail().deleteThread(target), target);
    const deleted = toast("Moved to Trash");
    if (index >= 0) {
      const thread = s.threads[index];
      set({ ...leaveList(s, target), undoSteps: remember(s.undoSteps, { kind: "deleted", thread, index, view: s.view, hit }), toast: deleted });
      return movedOn(s);
    }
    if (doneIndex < 0) {
      set({
        foundThreads: without(s.foundThreads, target),
        undoSteps: remember(s.undoSteps, { kind: "deletedFound", thread: { id: target }, view: s.view, hit }),
        toast: deleted,
      });
      // Reading it, opened from Search: back there.
      if (s.view === "thread" && s.openThreadId === target) get().leave();
      return;
    }
    // From the Done list, or a done email you're reading: on to the next done one.
    const thread = s.doneThreads[doneIndex];
    const i = visibleDone(s).findIndex((t) => t.id === target);
    const doneThreads = without(s.doneThreads, target);
    const after = visibleDone({ ...s, doneThreads });
    const next = after[Math.min(i, after.length - 1)];
    set({
      doneThreads,
      doneCursorId: next?.id,
      ...(s.view === "thread" && next && { openThreadId: next.id }),
      undoSteps: remember(s.undoSteps, { kind: "deletedDone", thread, index: doneIndex, view: s.view, hit }),
      toast: deleted,
    });
    if (s.view === "thread" && !next) get().back();
    movedOn(s);
  },

  markNotDone: (id) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.doneCursorId);
    const index = s.doneThreads.findIndex((t) => t.id === target);
    if (!target || index < 0) return;
    const thread = s.doneThreads[index];
    const { doneAt: _, ...open } = thread;
    const list = visibleDone(s);
    const i = list.findIndex((t) => t.id === target);
    const after = visibleDone({ ...s, doneThreads: without(s.doneThreads, target) });
    settle(mail().markNotDone(target), target);
    set({
      threads: [open, ...s.threads],
      doneThreads: without(s.doneThreads, target),
      doneCursorId: after[Math.min(i, after.length - 1)]?.id,
      undoSteps: remember(s.undoSteps, { kind: "notDone", thread, index, view: s.view }),
      toast: toast("Back in your inbox"),
    });
  },

  undo: () => {
    const s = get();
    const step = s.undoSteps.at(-1);
    if (!step) return;
    if (step.kind === "send") return unsend(step);
    if (step.kind === "schedule") {
      set({ undoSteps: s.undoSteps.slice(0, -1), toast: undefined });
      return step.takeBack();
    }
    const id = step.thread.id;
    if (step.hit) useSearch.getState().putBack(step.hit);
    if (step.kind === "deletedFound") {
      settle(mail().restoreThread(id), id);
      return set({ undoSteps: s.undoSteps.slice(0, -1), toast: undefined });
    }
    const reading = step.view === "thread" || step.view === "compose";
    // On the list or an email you go back to where you were; elsewhere (Settings, Search…) you stay.
    const view: View = onTheList.includes(s.view) ? (reading ? "thread" : "inbox") : s.view;
    const where = { view, ...(view === "thread" && { openThreadId: id }), ...(view === "inbox" && { history: [] }) };
    if (step.kind === "moved") {
      const before = step.thread;
      settle(mail().moveThread(id, before.movedByYou ? before.split : null), id);
      const swap = (list: Thread[]) => list.map((t) => (t.id === id ? before : t));
      return set({
        threads: swap(s.threads),
        doneThreads: swap(s.doneThreads),
        foundThreads: swap(s.foundThreads),
        ...(step.index >= 0 && { cursorId: id }),
        undoSteps: s.undoSteps.slice(0, -1),
        toast: undefined,
      });
    }
    if (step.kind === "notDone" || step.kind === "deletedDone") {
      settle(step.kind === "notDone" ? mail().markDone(id) : mail().restoreThread(id), id);
      set({
        ...where,
        threads: without(s.threads, id),
        doneThreads: insertAt(without(s.doneThreads, id), step.index, step.thread),
        doneCursorId: id,
        undoSteps: s.undoSteps.slice(0, -1),
        toast: undefined,
      });
      return movedOn(s);
    }
    const back = { done: () => mail().markNotDone(id), deleted: () => mail().restoreThread(id), later: () => mail().snooze(id, null) };
    settle(back[step.kind](), id);
    set({
      ...where,
      threads: insertAt(without(s.threads, id), step.index, step.thread),
      doneThreads: without(s.doneThreads, id),
      cursorId: id,
      handled: step.kind === "later" ? s.handled : s.handled - 1,
      undoSteps: s.undoSteps.slice(0, -1),
      toast: undefined,
    });
    movedOn(s);
  },

  dismissToast: () => set({ toast: undefined }),

  openDone: () => {
    set({ ...goTo(get(), "done"), doneCursorId: visibleDone(get())[0]?.id });
    if (mail().live) quietly(loadDone());
  },
  setDoneCursor: (doneCursorId) => set({ doneCursorId }),
  moveDoneCursor: (step) => {
    const s = get();
    const list = visibleDone(s);
    const i = Math.max(0, list.findIndex((t) => t.id === s.doneCursorId));
    set({ doneCursorId: list[Math.min(list.length - 1, Math.max(0, i + step))]?.id });
  },

  blockSender: (wholeDomain = false, id) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.cursorId);
    const thread = findThread(s, target);
    const domain = thread && domainOf(thread.from.email);
    if (!thread || (wholeDomain && (!domain || isPublicDomain(domain)))) return;
    const caught = sameSender(thread, wholeDomain);
    const list = visibleThreads(s);
    const i = list.findIndex((t) => t.id === thread.id);
    const threads = s.threads.filter((t) => !caught(t));
    const after = visibleThreads({ ...s, threads });
    const next = after[Math.min(Math.max(i, 0), after.length - 1)];
    const reading = s.view === "thread" && next !== undefined;
    const moved = [...s.threads, ...s.doneThreads].filter(caught).map(as("blocked"));
    settle(mail().blockSender(thread.id, wholeDomain), thread.id);
    set({
      threads,
      doneThreads: s.doneThreads.filter((t) => !caught(t)),
      blockedThreads: [...moved, ...s.blockedThreads],
      cursorId: next?.id,
      openThreadId: reading ? next.id : undefined,
      view: reading ? "thread" : s.view === "thread" ? "inbox" : s.view,
      ...(!reading && s.view === "thread" && { history: [] }),
      toast: {
        ...toast(wholeDomain ? `Blocked everyone at ${domain}` : `Blocked ${thread.from.name}`),
        undo: false,
        show: { label: "Show Blocked", keys: "⌘⇧B", run: () => get().openBlocked() },
      },
    });
    movedOn(s);
  },

  acceptSender: (wholeDomain = false, id) => {
    const s = get();
    const thread = findThread(s, id ?? (s.view === "thread" ? s.openThreadId : s.cursorId));
    const domain = thread && domainOf(thread.from.email);
    if (!thread || (wholeDomain && (!domain || isPublicDomain(domain)))) return;
    const same = sameSender(thread, wholeDomain);
    // Accepting someone also shows their images, as the engine does.
    const images = withImagesFrom(thread.accountId, isSender(thread, wholeDomain));
    const accept = (list: Thread[]) => list.map((t) => (same(t) ? images({ ...t, sender: "known" as const }) : t));
    settle(mail().acceptSender(thread.id, wholeDomain), thread.id);
    set({
      threads: accept(s.threads),
      doneThreads: accept(s.doneThreads),
      toast: { ...toast(wholeDomain ? `Accepted everyone at ${domain}` : `Accepted ${thread.from.name}`), undo: false },
    });
  },

  unblockSender: (id) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.blockedCursorId);
    const thread = s.blockedThreads.find((t) => t.id === target);
    if (!thread) return;
    // Blocked for the address it came to: unblocking lifts that too, as the engine does.
    const address = thread.toAddress && addressOf(accountFor(thread.accountId), thread.toAddress)?.blocked ? thread.toAddress : undefined;
    const bySender = sameSender(thread);
    const caught = (t: Thread) => bySender(t) || (!!address && t.accountId === thread.accountId && t.toAddress === address);
    if (address) set({ accounts: s.accounts.map((a) => (a.id === thread.accountId ? withAddress(a, address, { blocked: false }) : a)) });
    const list = visibleBlocked(s);
    const i = list.findIndex((t) => t.id === thread.id);
    const blockedThreads = s.blockedThreads.filter((t) => !caught(t));
    const after = visibleBlocked({ ...s, blockedThreads });
    settle(mail().unblockSender(thread.id), thread.id);
    set({
      threads: [...s.blockedThreads.filter(caught).map(as("known")).map(withImagesFrom(thread.accountId, isSender(thread))), ...s.threads],
      blockedThreads,
      blockedCursorId: after[Math.min(i, after.length - 1)]?.id,
      toast: { ...toast(`${thread.from.name}’s mail is back in your inbox`), undo: false },
    });
  },

  addAddress: (accountId, email, name) => {
    const key = email.trim().toLowerCase();
    const account = get().accounts.find((a) => a.id === accountId);
    const trimmed = name?.trim() || undefined;
    if (!account || !isValidAddress(key) || key === account.address.toLowerCase()) return;
    changeAccount(
      accountId,
      (a) => withAddress(a, key, { state: "added", ...(trimmed && { name: trimmed }) }, "you"),
      () => mail().addAddress(accountId, key, trimmed),
    );
  },

  renameAddress: (accountId, email, name) => {
    const trimmed = name.trim() || undefined;
    changeAccount(
      accountId,
      (a) => ({
        ...a,
        addresses: a.addresses?.map((x) => {
          if (x.email !== email) return x;
          const { name: _, ...rest } = x;
          return trimmed ? { ...rest, name: trimmed } : rest;
        }),
      }),
      () => mail().updateAddress(accountId, email, trimmed),
    );
  },

  removeAddress: (accountId, email) => {
    const changed = changeAccount(
      accountId,
      (a) => ({ ...a, addresses: a.addresses?.flatMap((x) => (x.email !== email ? [x] : x.blocked ? [{ ...x, state: "dismissed" as const }] : [])) }),
      () => mail().removeAddress(accountId, email),
    );
    if (changed && get().addressFilter === email) get().setAddressFilter(undefined);
  },

  blockAddress: (accountId, email, blocked) => {
    const s = get();
    const key = email.toLowerCase();
    const caught = (t: Thread) => t.accountId === accountId && t.toAddress === key;
    const changed = changeAccount(
      accountId,
      (a) => {
        const next = withAddress(a, key, { blocked }, "mail", "dismissed");
        // Unblocked, and you had said no to it: it leaves the list.
        return { ...next, addresses: next.addresses?.filter((x) => x.blocked || x.state !== "dismissed") };
      },
      () => mail().blockAddress(accountId, key, blocked),
      true,
    );
    if (!changed) return;
    if (!blocked) {
      const back = s.blockedThreads.filter(caught).map(({ sender: _, ...t }) => t);
      set({
        threads: [...back, ...s.threads],
        blockedThreads: s.blockedThreads.filter((t) => !caught(t)),
        toast: { ...toast(`Mail to ${key} is back in your inbox`), undo: false },
      });
      return;
    }
    const list = visibleThreads(s);
    const i = list.findIndex((t) => t.id === s.cursorId);
    const threads = s.threads.filter((t) => !caught(t));
    const after = visibleThreads({ ...s, threads });
    const reading = openThread(s);
    const leaves = !!reading && caught(reading);
    set({
      threads,
      doneThreads: s.doneThreads.filter((t) => !caught(t)),
      blockedThreads: [...[...s.threads, ...s.doneThreads].filter(caught).map(as("blocked")), ...s.blockedThreads],
      cursorId: after.some((t) => t.id === s.cursorId) ? s.cursorId : after[Math.min(Math.max(i, 0), after.length - 1)]?.id,
      ...(leaves && { view: "inbox" as const, openThreadId: undefined, history: [] }),
      toast: { ...toast(`Mail to ${key} goes to Blocked now`), undo: false },
    });
  },

  discoverAddresses: async (accountId) => {
    await changeAccount(accountId, (a) => a, () => mail().discoverAddresses(accountId));
  },

  openBlocked: () => {
    set({ ...goTo(get(), "blocked"), blockedCursorId: visibleBlocked(get())[0]?.id });
    if (mail().live) quietly(loadBlocked());
  },
  openSenders: () => set(goTo(get(), "senders")),
  setBlockedCursor: (blockedCursorId) => set({ blockedCursorId }),
  moveBlockedCursor: (step) => {
    const s = get();
    const list = visibleBlocked(s);
    const i = Math.max(0, list.findIndex((t) => t.id === s.blockedCursorId));
    set({ blockedCursorId: list[Math.min(list.length - 1, Math.max(0, i + step))]?.id });
  },

  toggleRead: (id) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.cursorId);
    const thread = findThread(s, target);
    if (!thread) return;
    const unread = !thread.unread;
    saveUnread(thread.id, unread);
    // Like Esc: an email you mean to come back to leaves the screen.
    if (unread && s.view === "thread") get().back();
  },
  remindLater: (id, until) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.cursorId);
    if (!target) return;
    const at = until ?? tomorrowMorning(Date.now());
    settle(mail().snooze(target, at), target);
    hideUntil(target, at, (when) => `Coming back ${when}`);
  },
  openRemind: (id, noReply = false) => {
    const s = get();
    const target = id ?? (s.view === "thread" ? s.openThreadId : s.cursorId);
    if (!target) return;
    const thread = findThread(s, target);
    if (noReply && !(thread && wroteLast(thread, s.accounts)))
      return set({ toast: { ...toast("The latest email isn’t yours, so there’s no answer to wait for"), undo: false } });
    set({ remindingId: target, remindNoReply: noReply });
  },
  closeRemind: () => set({ remindingId: undefined, remindNoReply: false }),
  waitForReply: (until, id) => {
    const s = get();
    const target = id ?? s.remindingId;
    if (!target) return;
    settle(mail().waitForReply(target, until), target);
    // Your email, done once sent, comes back to the list to wait there.
    const thread = findThread(s, target);
    if (thread && !s.threads.some((t) => t.id === target)) {
      const { doneAt: _, ...open } = thread;
      set({ threads: [{ ...open, split: "important" }, ...s.threads], doneThreads: without(s.doneThreads, target), foundThreads: without(s.foundThreads, target) });
    }
    hideUntil(target, until, (when) => `Back ${when} if no one replies`, { waiting: { until, why: { by: "you" } } });
  },

  openMove: (id) => {
    const target = id ?? actionTarget(get());
    if (target) set({ movingId: target });
  },
  closeMove: () => set({ movingId: undefined }),
  moveTo: (to, id) => {
    const s = get();
    const target = id ?? s.movingId ?? actionTarget(s);
    if (!target) return;
    settle(mail().moveThread(target, to), target);
    const text = to ? `Moved to ${splits.find((x) => x.id === to)?.label}` : "Moss sorts it again";
    const thread = findThread(s, target);
    if (!thread) return set({ movingId: undefined, toast: toast(text) });
    const moved = placedIn(thread, to);
    const swap = (list: Thread[]) => list.map((t) => (t.id === target ? moved : t));
    const index = s.threads.findIndex((t) => t.id === target);
    const threads = swap(s.threads);
    // Keep your place in the list it left: the email below takes its spot.
    const i = visibleThreads(s).findIndex((t) => t.id === target);
    const after = visibleThreads({ ...s, threads });
    const left = s.view === "inbox" && i >= 0 && !after.some((t) => t.id === target);
    set({
      threads,
      doneThreads: swap(s.doneThreads),
      foundThreads: swap(s.foundThreads),
      movingId: undefined,
      ...(left && { cursorId: after[Math.min(i, after.length - 1)]?.id }),
      undoSteps: remember(s.undoSteps, { kind: "moved", thread, index, view: s.view }),
      toast: toast(text),
      // Your own email: when should it come back if no one answers?
      ...(to === "important" && wroteLast(thread, s.accounts) && { remindingId: target, remindNoReply: true }),
    });
  },

  moveToSection: (section, id) => {
    const s = get();
    const target = targetOf(s, id);
    if (!target) return;
    settle(mail().setSection(target, section), target);
    set({
      threads: changed(s.threads, target, { section, aiSure: undefined }),
      toast: { ...toast(`Moved to ${section === "reply" ? "Needs your reply" : "For your information"}`), undo: false },
    });
  },
  clearDeadline: (id) => {
    const s = get();
    const target = targetOf(s, id);
    if (!target) return;
    settle(mail().clearDeadline(target), target);
    set({
      threads: changed(s.threads, target, { due: undefined, askDue: undefined, dueDate: undefined, dueWhy: undefined }),
      toast: { ...toast("No deadline"), undo: false },
    });
  },
  remindIfNoReply: (id, on = true) => {
    const s = get();
    const thread = findThread(s, targetOf(s, id));
    if (!thread) return;
    if (!thread.youWroteLast) {
      set({ toast: { ...toast("Reply first, then Moss can wait for an answer"), undo: false } });
      return;
    }
    settle(mail().remindIfNoReply(thread.id, on), thread.id);
    if (!on) {
      set({ threads: changed(s.threads, thread.id, { section: "info", waiting: undefined }), toast: { ...toast("Moss won’t remind you"), undo: false } });
      return;
    }
    // Like the engine: three days after you wrote, or from now when that has passed.
    const now = Date.now();
    const wait = 3 * 86_400_000;
    const sent = thread.lastMessageAt ?? now;
    const until = sent + wait > now ? sent + wait : now + wait;
    const threads = changed(s.threads, thread.id, { section: "later", time: listTime(until, now), waiting: { until, why: { by: "you" } } });
    const i = waiting(s).findIndex((t) => t.id === thread.id);
    const list = waiting({ ...s, threads });
    set({
      threads,
      cursorId: list[Math.min(i, list.length - 1)]?.id ?? thread.id,
      toast: { ...toast(`Back ${remindTime(until, now)} if ${thread.from.short ?? thread.from.name} doesn’t reply`), undo: false },
    });
  },

  showInboxZero: () => {
    const n = stillWaiting(get().threads);
    if (!n) set({ view: "zero", history: [] });
    else set({ toast: { ...toast(`${n} ${n === 1 ? "email" : "emails"} still waiting`), undo: false } });
  },

  setPaletteOpen: (paletteOpen) => set({ paletteOpen, paletteQuery: "" }),
  toggleRecipients: () => set((s) => ({ recipientsOpen: !s.recipientsOpen })),
  showImages: (id) => set((s) => (s.imagesShown.includes(id) ? s : { imagesShown: [...s.imagesShown, id] })),
  allowImages: (id) => {
    if (useRemoteImages.getState().policy === "ask") {
      set({ toast: { ...toast("Strict image privacy is on. Use Just this once to show this message’s images."), undo: false } });
      return;
    }
    const s = get();
    const thread = [...s.threads, ...s.doneThreads].find((t) => t.messages.some((m) => m.id === id));
    if (!thread) return;
    const email = thread.messages.find((m) => m.id === id)!.from.email;
    const allow = withImagesFrom(thread.accountId, (address) => address.toLowerCase() === email.toLowerCase());
    set({ threads: s.threads.map(allow), doneThreads: s.doneThreads.map(allow) });
    quietly(mail().allowImages(thread.accountId, email));
  },
  openAttachment: (messageId, index) => quietly(mail().openAttachment(messageId, index)),
  loadPreviews: (message) => {
    for (const { index, name, contentType, preview } of message.attachments) {
      const key = `${message.id}:${index}`;
      if (index === undefined || preview || key in get().previews || !canPreview(name, contentType)) continue;
      const found = (url: string | null) => set((s) => ({ previews: { ...s.previews, [key]: url } }));
      found(null);
      quietly(mail().attachmentPreview(message.id, index).then((url) => url && found(url)));
    }
  },
  setReplyChoice: (replyChoice) => set({ replyChoice, replyDraft: undefined }),
  setReplyDraft: (replyDraft) => set({ replyDraft }),

  compose: (mode, threadId, fresh) => {
    const s = get();
    const thread = findThread(s, threadId ?? s.openThreadId ?? s.cursorId);
    if (mode === "new" || !thread) {
      const fromAccountId = s.accountFilter === "all" ? s.accounts[0].id : s.accountFilter;
      const chosen = s.addressFilter ? sendingAddress(accountFor(fromAccountId), s.addressFilter) : undefined;
      const fromAddress = chosen && !chosen.primary ? chosen.email : undefined;
      set({
        view: "compose",
        composedFrom: startedFrom(s),
        sendError: undefined,
        noSubjectAsked: false,
        attachProblem: undefined,
        draft: {
          id: crypto.randomUUID(),
          mode: "new",
          fromAccountId,
          ...(fromAddress && { fromAddress }),
          to: [],
          cc: [],
          bcc: [],
          subject: "",
          body: "",
          bodyHtml: "",
          fieldsOpen: true,
          showCc: false,
          showBcc: false,
          remindIfNoReply: false,
          signatureId: signaturePick({ fromAccountId, fromAddress, mode: "new" }).id,
        },
      });
      return;
    }
    // A reply where you have one half written opens it, wherever you wrote it.
    const mine = threadDraft(useDrafts.getState().drafts, thread.id);
    if (!fresh && mode !== "forward" && mine && mine.mode !== "forward" && mine.mode !== "new") return get().openDraft(mine.id);
    const message = thread.messages[thread.messages.length - 1];
    const account = accountFor(thread.accountId);
    const others = (list: Person[]) => list.filter((x) => !isMine(x.email, [account]));
    // Replying goes from the address the email came to, when you can send from it.
    const { address: fromAddress, reason: fromReason } = replyFrom(thread, account);
    const subject = stripRe(thread.fullSubject ?? thread.subject);
    set({
      view: "compose",
      composedFrom: startedFrom(s),
      sendError: undefined,
      noSubjectAsked: false,
      attachProblem: undefined,
      openThreadId: thread.id,
      draft: {
        id: crypto.randomUUID(),
        mode,
        threadId: thread.id,
        fromAccountId: thread.accountId,
        ...(fromAddress && { fromAddress }),
        ...(fromReason && { fromReason }),
        to: mode === "forward" ? [] : mode === "reply-all" && message ? [thread.from, ...others(message.to)] : [thread.from],
        cc: mode === "reply-all" && message ? others(message.cc) : [],
        bcc: [],
        subject: `${mode === "forward" ? "Fwd" : "Re"}: ${subject}`,
        body: "",
        bodyHtml: "",
        fieldsOpen: true,
        showCc: mode === "reply-all",
        showBcc: false,
        remindIfNoReply: false,
        signatureId: signaturePick({ fromAccountId: thread.accountId, fromAddress, mode, threadId: thread.id }).id,
        ...(mode === "forward" && message?.attachments.length && { attachments: forwardedFiles(message) }),
      },
    });
    previewFiles(get().draft!.id, get().draft!.attachments ?? []);
  },

  editInstantReply: (signatureId) => {
    const s = get();
    const thread = openThread(s);
    const text = s.replyDraft ?? thread?.replyOptions?.[s.replyChoice]?.draft;
    // Quick reply's words are a reply of their own, not a change to a draft you have there.
    get().compose("reply", thread?.id, true);
    if (text !== undefined) get().updateDraft({ body: text, bodyHtml: textToHtml(text), fieldsOpen: false });
    if (signatureId !== undefined) get().updateDraft({ signatureId, signatureChosen: true });
  },

  updateDraft: (patch) =>
    set((s) => {
      if (!s.draft) return {};
      const draft = { ...s.draft, ...patch };
      // A new From brings its own signature, unless you picked one or set it in the same change.
      if ((patch.fromAccountId || "fromAddress" in patch) && !draft.signatureChosen && !("signatureId" in patch))
        draft.signatureId = signaturePick(draft).id;
      return { draft, ...("subject" in patch && { noSubjectAsked: false }) };
    }),

  toggleSignaturePicker: () => get().updateDraft({ signatureOpen: !get().draft?.signatureOpen }),
  toggleFromPicker: () => get().updateDraft({ fromOpen: !get().draft?.fromOpen }),
  chooseFrom: (accountId, email) => {
    if (!get().draft) return;
    const chosen = sendingAddress(accountFor(accountId), email);
    get().updateDraft({ fromAccountId: accountId, fromAddress: chosen.primary ? undefined : chosen.email, fromReason: undefined, fromOpen: false });
  },
  chooseSignature: (signatureId) => {
    if (!get().draft) return;
    if (signatureId) useSignatures.getState().markUsed(signatureId);
    get().updateDraft({ signatureId, signatureChosen: true, signatureOpen: false });
  },

  attach: (files) => {
    const draft = get().draft;
    if (!draft) return;
    set({ attachProblem: undefined });
    const finished = beginAttachmentImport(draft.id);
    const chosen = files ? Promise.resolve(files) : mail().pickFiles();
    chosen
      .then((files) => (files.length ? attachDraftFiles(draft.id, files) : []))
      .then(
        (added) => {
          const now = get().draft;
          if (!added.length) return;
          // A late import belongs to the old email, never the new composer.
          if (now?.id !== draft.id) {
            for (const file of added) discardDraftFile(draft.id, file.id);
            return;
          }
          get().updateDraft({ attachments: [...(now.attachments ?? []), ...added] });
          previewFiles(draft.id, added);
        },
        (error: Error) => {
          if (get().draft?.id === draft.id) set({ attachProblem: `Couldn’t attach: ${error.message}` });
        },
      ).finally(() => {
        finished();
        if (get().draft?.id === draft.id && !attachmentImportPending(draft.id) && get().sendError === attachingMessage)
          set({ sendError: undefined });
      });
  },
  removeAttachment: (id) => {
    const draft = get().draft;
    const file = draft?.attachments?.find((file) => file.id === id);
    if (!draft || !file) return;
    get().updateDraft({ attachments: draft.attachments!.filter((f) => f.id !== id) });
    if (!file.forwarded) discardDraftFile(draft.id, id);
  },

  // The email waits `undoSendMs` so ⌘Z can take it back. Sending a reply handles its thread.
  send: () => {
    const { draft, sending, noSubjectAsked } = get();
    // Too many files: the composer already says so, before anything goes.
    if (!draft || sending || filesTooBig(draft) || refusePendingAttachments(draft) || refuseLocalRecovery(draft)) return;
    if (draft.mode === "new" && !draft.subject.trim() && !noSubjectAsked) return set({ noSubjectAsked: true });
    const retained = retainVolatileDraft(draft);
    const hold = `send-${Date.now()}-${++holds}`;
    const until = Date.now() + undoSendMs;
    stopDraftSave(draft.id);
    set({ sending: true, sendError: undefined, noSubjectAsked: false });
    const shown = new Promise<void>((done) =>
      setTimeout(() => {
        heldBack(hold, draft, until);
        done();
      }, sendBeatMs),
    );
    mail()
      .sendAfter(hold, draft, undoSendMs)
      .then(
        (sent) => shown.then(() => { if (sent) wentOut(hold, sent); else if (retained.pending()) preserveCancelledDraft(draft); }),
        // Refused while Send still shows: the composer says why.
        (error: Error) => {
          if (deliveryNeedsAttention(error)) return shown.then(() => notSent(hold, error, draft, retained.pending()));
          if (get().sending) {
            keepDraft(draft);
            set({ sending: false, sendError: error.message });
          } else notSent(hold, error, draft, retained.pending());
        },
      ).finally(retained.release);
  },

  sendDraft: (draft) => {
    if (refusePendingAttachments(draft) || refuseLocalRecovery(draft)) return;
    const retained = retainVolatileDraft(draft);
    const hold = `send-${Date.now()}-${++holds}`;
    const until = Date.now() + undoSendMs;
    handoffDraft(draft.id); // The backend persists the handoff before waiting for Undo.
    rememberPeople([...draft.to, ...draft.cc, ...draft.bcc]);
    set((s) => ({
      undoSteps: remember(s.undoSteps, { kind: "send", hold, draft, until }),
      toast: { ...toast("Sending"), until, hold },
    }));
    mail()
      .sendAfter(hold, draft, undoSendMs)
      .then(
        (sent) => { if (sent) wentOut(hold, sent); else if (retained.pending()) preserveCancelledDraft(draft); },
        (error: Error) => notSent(hold, error, draft, retained.pending()),
      ).finally(retained.release);
  },

  // Sent like any reply, from behind the open email. Sending never moves an email to Done: only you do that.
  sendInstantReply: (signatureId) => {
    const s = get();
    const thread = openThread(s);
    const text = s.replyDraft ?? thread?.replyOptions?.[s.replyChoice]?.draft;
    if (!thread || !text?.trim()) return;
    const { view, history } = s;
    get().compose("reply", thread.id, true);
    get().updateDraft({ body: text, bodyHtml: textToHtml(text), ...(signatureId !== undefined && { signatureId, signatureChosen: true }) });
    set({ view, history, replyDraft: undefined });
    get().send();
  },

  openDrafts: () => {
    set(goTo(get(), "drafts"));
    // Drafts started on your phone or in webmail show up without waiting for the next sync.
    quietly(mail().syncNow());
    useDrafts.setState({ cursorId: visibleDrafts(useDrafts.getState().drafts, get().accountFilter)[0]?.id });
  },
  openDraft: (id) => {
    const draft = useDrafts.getState().drafts.find((d) => d.id === id);
    if (!draft) return;
    const s = get();
    set({ view: "compose", composedFrom: startedFrom(s), draft, sendError: undefined, noSubjectAsked: false, ...(draft.threadId && { openThreadId: draft.threadId }) });
    previewFiles(draft.id, draft.attachments ?? []);
  },
  discardDraft: () => {
    const { draft } = get();
    const released = draft && releaseResolvedLocalDraft(draft);
    if (draft && !released && refuseLocalRecovery(draft)) return;
    if (draft && !released) forgetDraft(draft.id);
    if (released) quietly(reloadDrafts());
    get().back();
    set({ draft: undefined });
  },
  deleteDraft: (id) => {
    const { drafts, cursorId } = useDrafts.getState();
    const target = id ?? cursorId;
    const list = visibleDrafts(drafts, get().accountFilter);
    const i = list.findIndex((d) => d.id === target);
    if (i < 0) return;
    const released = releaseResolvedLocalDraft(list[i]);
    if (!released && refuseLocalRecovery(list[i])) return;
    const after = list.filter((d) => d.id !== target);
    if (!released) forgetDraft(list[i].id);
    else quietly(reloadDrafts());
    useDrafts.setState({ cursorId: after[Math.min(i, after.length - 1)]?.id });
    set({ toast: { ...toast(released ? "Local edits discarded" : "Draft deleted"), undo: false } });
  },

  openAddAccount: () => set(goTo(get(), "add-account")),
  openSettings: (section = "accounts") => set({ ...goTo(get(), "settings"), settingsSection: section }),
  openSearch: () => set(goTo(get(), "search")),
  leave: () => {
    const history = get().history;
    set({ view: history.at(-1) ?? "inbox", history: history.slice(0, -1) });
  },
}));

let holds = 0;

/** Asks the engine for thumbnails of a draft's files that can have one and haven't been asked for. */
function previewFiles(draftId: string, files: DraftFile[]) {
  for (const f of files) {
    const key = f.forwarded ? `${f.forwarded.messageId}:${f.forwarded.index}` : `draft:${draftId}:${f.id}`;
    if (key in useStore.getState().previews || !canPreview(f.name, f.contentType)) continue;
    const found = (url: string | null) => useStore.setState((s) => ({ previews: { ...s.previews, [key]: url } }));
    found(null);
    const asked = f.forwarded ? mail().attachmentPreview(f.forwarded.messageId, f.forwarded.index) : mail().draftPreview(draftId, f.id);
    quietly(asked.then((url) => url && found(url)));
  }
}

/** People you just wrote to are suggested from now on, before the engine is asked again. */
function rememberPeople(people: Person[]) {
  useStore.setState((s) => {
    const fresh = new Set(people.map((p) => p.email.toLowerCase()));
    return { knownPeople: [...people, ...s.knownPeople.filter((p) => !fresh.has(p.email.toLowerCase()))] };
  });
}

/** Where a composer opening now was started from: a composer already open keeps where it was started. */
const startedFrom = (s: State): State["composedFrom"] => (s.view === "compose" ? s.composedFrom : { view: s.view, openThreadId: s.openThreadId });

const findSend = (steps: UndoStep[], hold: string) => steps.find((x): x is SendStep => x.kind === "send" && x.hold === hold);

/**
 * After the pressed Send: back to the email you answered or forwarded, or to where you started a
 * new one, with the email held and a note counting down. Sending never moves an email to Done:
 * only you do that.
 */
function heldBack(hold: string, draft: Draft, until: number) {
  const s = useStore.getState();
  if (!s.sending) return; // Refused at once: the composer says why.
  handoffDraft(draft.id); // The backend persists the handoff before waiting for Undo.
  rememberPeople([...draft.to, ...draft.cc, ...draft.bcc]);
  const answered = draft.threadId;
  const listed = findThread(s, answered) !== undefined;
  const from = s.composedFrom;
  const back = from && (from.view !== "thread" || findThread(s, from.openThreadId)) ? from : { view: "inbox" as const };
  useStore.setState((s) => ({
    sending: false,
    draft: undefined,
    ...(listed ? { view: "thread" as const, openThreadId: answered } : back),
    undoSteps: remember(s.undoSteps, { kind: "send", hold, draft, until }),
    toast: { ...toast("Sending"), until, hold },
  }));
  // A reply to an email no list holds (one you started on your phone): it opens once it loads.
  if (answered && !listed) quietly(openIfStill(answered, here()));
}

/** Where you are: an email opens on its own only if you're still there, not after you moved on. */
type Spot = Pick<State, "view" | "openThreadId" | "cursorId">;

function here(): Spot {
  const { view, openThreadId, cursorId } = useStore.getState();
  return { view, openThreadId, cursorId };
}

/** Loads conversation `id`, which no list holds, and opens it, unless you moved on meanwhile. */
async function openIfStill(id: string, left: Spot) {
  const found = await mail().thread(id);
  const now = here();
  const movedOn = (["view", "openThreadId", "cursorId"] as const).some((key) => now[key] !== left[key]);
  if (!found || movedOn) return;
  useStore.setState((s) => ({ foundThreads: [...s.foundThreads, found] }));
  useStore.getState().open(id);
}

/**
 * The held email has gone: nothing left to take back. Reading the email it went into, what you
 * sent loads in before the note says Sent, so it never blinks out. Anywhere else, Sent offers View.
 */
function wentOut(hold: string, sent: Sent) {
  const id = typeof sent === "string" ? sent : undefined;
  const { view, openThreadId } = useStore.getState();
  const reading = id !== undefined && view === "thread" && openThreadId === id;
  const gone = () =>
    useStore.setState((s) => {
      const step = findSend(s.undoSteps, hold);
      if (!step) return {};
      const undoSteps = s.undoSteps.filter((x) => x !== step);
      const show = id !== undefined && !reading && { show: { label: "View", run: () => void useStore.getState().openFound(id) } };
      return { undoSteps, ...(s.toast?.hold === hold && { toast: { ...toast("Sent"), undo: false as const, ...show } }) };
    });
  if (reading) loadThread(id).then(gone, gone);
  else gone();
}

/** The held email couldn't go: it stays a step, so ⌘Z or Open brings it back with why. */
function deliveryNeedsAttention(error: Error) {
  return "kind" in error && (error.kind === "deliveryUnknown" || error.kind === "deliveryAccepted" || error.kind === "deliveryBusy");
}

function notSent(hold: string, error: Error, draft: Draft, unconfirmed: boolean) {
  if (!deliveryNeedsAttention(error)) keepDraft(draft);
  else if (unconfirmed) keepUncertainDraft(draft);
  const reason = error.message;
  useStore.setState((s) => {
    const step = findSend(s.undoSteps, hold);
    if (!step) return {};
    if (deliveryNeedsAttention(error)) return {
      undoSteps: s.undoSteps.filter((x) => x !== step),
      toast: { ...toast(reason), problem: true, undo: false as const, stays: true },
    };
    return {
      undoSteps: s.undoSteps.map((x) => (x === step ? { ...step, failed: reason } : x)),
      toast: { ...toast("Couldn’t send your email"), problem: true, undoLabel: "Open", stays: true },
    };
  });
}

/** ⌘Z on a send: takes the held email back and opens it again. */
function unsend(step: SendStep) {
  const s = useStore.getState();
  if ((!step.failed && Date.now() >= step.until) || (s.view === "compose" && s.draft)) return;
  const retained = retainVolatileDraft(step.draft);
  const reopen = (restored: Draft = step.draft) => {
    useStore.setState((current) => ({
      undoSteps: current.undoSteps.filter((x) => x.kind !== "send" || x.hold !== step.hold),
      // Another composer may have opened while the backend decided. Keep both drafts.
      ...(current.view === "compose" && current.draft
        ? { toast: toast("Email restored to Drafts") }
        : {
            view: "compose" as const,
            draft: restored,
            sendError: step.failed,
            toast: undefined,
            ...(restored.threadId && { openThreadId: restored.threadId }),
          }),
    }));
  };
  void mail().cancelSend(step.hold).then(
    (outcome) => {
      if (!outcome.cancelled) return useStore.setState({ toast: { ...toast("Too late: it had already gone"), problem: true } });
      if (outcome.restoreFailed) {
        keepUncertainDraft(step.draft);
        useStore.setState({ toast: { ...toast("Send cancelled. The draft could not be restored. Its saved data is retained for recovery."), problem: true, stays: true } });
        return;
      }
      if (outcome.restored) { adoptDraft(outcome.restored); reopen(outcome.restored); }
      else if (outcome.early) { reopen(keepDraft(step.draft)); }
      else {
        useDrafts.setState(state => { const { [step.draft.id]: _old, ...retired } = state.retired;return { retired }; });
        quietly(reloadDrafts());
        useStore.setState({ toast: { ...toast("This draft was already removed or handed off elsewhere."), problem: true } });
      }
    },
    () => {
      if (retained.pending()) keepUncertainDraft(step.draft);
      useStore.setState({ toast: { ...toast("Couldn’t confirm Undo. Your email may still send."), problem: true } });
    },
  ).finally(retained.release);
}

/** Real mail: reads the Done list from the engine. What it says is done leaves the inbox list. */
export async function loadDone() {
  const fresh = await current(() => mail().doneThreads());
  const done = new Set(fresh.map((t) => t.id));
  const reading = openThread(useStore.getState());
  useStore.setState((s) => {
    const doneThreads = merge(fresh, s.doneThreads, s.openThreadId);
    const list = visibleDone({ ...s, doneThreads });
    return {
      doneThreads,
      threads: s.threads.filter((t) => !done.has(t.id)),
      doneCursorId: list.some((t) => t.id === s.doneCursorId) ? s.doneCursorId : list[0]?.id,
    };
  });
  loadNewMessage(reading);
}

/** Real mail: reads the Blocked list from the engine. What it says is blocked leaves the inbox list. */
export async function loadBlocked() {
  const fresh = await current(() => mail().blockedThreads());
  const blocked = new Set(fresh.map((t) => t.id));
  const reading = openThread(useStore.getState());
  useStore.setState((s) => {
    const blockedThreads = merge(fresh, s.blockedThreads, s.openThreadId);
    const list = visibleBlocked({ ...s, blockedThreads });
    return {
      blockedThreads,
      threads: s.threads.filter((t) => !blocked.has(t.id)),
      blockedCursorId: list.some((t) => t.id === s.blockedCursorId) ? s.blockedCursorId : list[0]?.id,
    };
  });
  loadNewMessage(reading);
}

/** Changes made on screen so far. A read of the engine that started before the latest one is out of date. */
let changes = 0;

/** Reads from the engine, and reads again if you changed something meanwhile, so it can't undo that on screen. */
async function current<T>(read: () => Promise<T>): Promise<T> {
  for (;;) {
    const at = changes;
    const value = await read();
    if (at === changes) return value;
  }
}

/**
 * Keeps messages already loaded when a thread hasn't changed, so an open email doesn't flicker.
 * The email you're reading (`openId`) keeps them even when it has changed, until
 * [loadNewMessage] brings the new one: the list's row has only a one-line preview of it.
 */
function merge(fresh: Thread[], old: Thread[], openId: string | undefined): Thread[] {
  const known = new Map(old.map((t) => [t.id, t]));
  return fresh.map(unsaved).map((t) => {
    const was = known.get(t.id);
    if (!was) return t;
    if (was.lastMessageAt !== t.lastMessageAt)
      return t.id === openId && !was.partial ? { ...t, messages: was.messages, fullSubject: was.fullSubject, partial: false } : t;
    // What was worked out on screen for it stays: the ask, and the replies Moss wrote.
    const kept = { ...t, ...(was.ask && { ask: was.ask }), ...(was.replyOptions && { replyOptions: was.replyOptions }) };
    return was.partial ? kept : { ...kept, messages: was.messages, fullSubject: was.fullSubject, partial: false };
  });
}

/** Real mail: reads the open threads again, keeping the cursor where it was: on its thread, or
 * on the one that took its place when it's gone. */
export async function reloadThreads() {
  const fresh = await current(() => mail().threads());
  const open = new Set(fresh.map((t) => t.id));
  const reading = openThread(useStore.getState());
  useStore.setState((s) => {
    const threads = merge(fresh, s.threads, s.openThreadId);
    const list = visibleThreads({ ...s, threads });
    const was = Math.max(0, visibleThreads(s).findIndex((t) => t.id === s.cursorId));
    const cursorId = list.some((t) => t.id === s.cursorId) ? s.cursorId : list[Math.min(was, list.length - 1)]?.id;
    return { threads, cursorId, doneThreads: s.doneThreads.filter((t) => !open.has(t.id)) };
  });
  loadNewMessage(reading);
}

/** The email you're reading, as it was before a list was read again: a new message in it loads in full. */
function loadNewMessage(reading: Thread | undefined) {
  const now = openThread(useStore.getState());
  if (reading && now?.id === reading.id && !reading.partial && now.lastMessageAt !== reading.lastMessageAt) quietly(loadThread(now.id));
}

/**
 * Real mail: the screen moves an email at once, then the engine has the last word. Both lists
 * are read again once it answers; if it refused, that puts the email back, and a note says so.
 */
function settle(change: Promise<unknown>, threadId: string) {
  changes++;
  if (!mail().live) return quietly(change);
  const reload = () => Promise.all([reloadThreads(), loadDone(), loadBlocked()]);
  change.then(
    () => quietly(reload()),
    (error: unknown) => {
      console.warn("Moss:", error);
      useStore.setState((s) => ({
        toast: { ...toast("Couldn’t save that, so Moss put it back"), problem: true },
        // A held send, or one set to send later, still waits to go.
        undoSteps: s.undoSteps.filter((step) => step.kind === "send" || step.kind === "schedule" || step.thread.id !== threadId),
      }));
      quietly(reload());
    },
  );
}

/**
 * Real mail: an opened thread loads its messages and counts as read. Mail that needs your
 * reply also gets "the ask" from the writing model; other mail never wakes it.
 */
function opened(id: string) {
  const thread = openThread({ ...useStore.getState(), openThreadId: id });
  if (thread?.partial) quietly(loadThread(id));
  else if (thread) latestPreviews(thread);
  if (!mail().live) return setUnread(id, false);
  saveUnread(id, false);
  if (thread?.section === "reply" && thread.ask === undefined) void writeAsk(id);
}

/** After an action: an email it moved you on to counts as opened, as with ↑/↓, so its body loads. */
function movedOn(before: Pick<State, "view" | "openThreadId">) {
  const { view, openThreadId } = useStore.getState();
  const was = before.view === "thread" ? before.openThreadId : undefined;
  if (view === "thread" && openThreadId && openThreadId !== was) opened(openThreadId);
}

/**
 * Read changes the engine hasn't saved yet, by thread: whether it is unread. A list or thread
 * read from the engine before then is older than the screen, so these win over it.
 */
const savingUnread = new Map<string, boolean>();

/** What the screen says until the engine has saved it. */
function unsaved(t: Thread): Thread {
  const unread = savingUnread.get(t.id);
  return unread === undefined ? t : { ...t, unread };
}

/** Marks a thread unread or read at once, then tells the engine. */
function saveUnread(id: string, unread: boolean) {
  setUnread(id, unread);
  savingUnread.set(id, unread);
  changes++;
  quietly(
    mail()
      .markRead(id, !unread)
      .finally(() => {
        if (savingUnread.get(id) === unread) savingUnread.delete(id);
      }),
  );
}

/** Sets a thread's unread state wherever it is listed. */
function setUnread(id: string, unread: boolean) {
  const mark = (list: Thread[]) => list.map((t) => (t.id === id ? { ...t, unread } : t));
  useStore.setState((s) => ({
    threads: mark(s.threads),
    doneThreads: mark(s.doneThreads),
    blockedThreads: mark(s.blockedThreads),
    foundThreads: mark(s.foundThreads),
  }));
}

/** Puts the writing model's one line on what the email wants of you on the thread. */
async function writeAsk(id: string) {
  try {
    const ask = await ai().ask(id);
    const line = ask && plainText(ask);
    if (line) useStore.setState((s) => ({ threads: s.threads.map((t) => (t.id === id ? { ...t, ask: line } : t)) }));
  } catch {
    // Not downloaded yet, or the model failed: the email shows without an ask.
  }
}

/** Replaces a thread's preview with all its messages, fetching bodies and images that are still on the server. */
export async function loadThread(id: string) {
  const first = await mail().thread(id);
  const shallow = first?.messages.filter((m) => m.headersOnly || m.imagesMissing) ?? [];
  let filled = false;
  for (const m of shallow) filled = (await mail().loadMessage(m.id)) || filled;
  const loaded = filled ? await mail().thread(id) : first;
  if (!loaded) return;
  const full = unsaved(loaded);
  useStore.setState((s) => ({
    threads: s.threads.map((t) => (t.id === id ? { ...full, ...(t.ask && { ask: t.ask }) } : t)),
    doneThreads: s.doneThreads.map((t) => (t.id === id ? { ...full, doneAt: t.doneAt } : t)),
    blockedThreads: s.blockedThreads.map((t) => (t.id === id ? full : t)),
    foundThreads: s.foundThreads.map((t) => (t.id === id ? full : t)),
  }));
  latestPreviews(full);
}

/** Thumbnails for the files of the message a thread shows. */
function latestPreviews(thread: Thread) {
  const latest = thread.messages.at(-1);
  if (latest) useStore.getState().loadPreviews(latest);
}

/**
 * The signature Moss picks for a draft, or for a reply about to be written: from its From address,
 * whether it's new, a reply or a forward, and how much you already wrote in its thread.
 */
export function signaturePick(d: Pick<Draft, "fromAccountId" | "fromAddress" | "mode" | "threadId">): SignaturePick {
  const s = useStore.getState();
  const kind = d.mode === "new" ? "new" : d.mode === "forward" ? "forward" : "reply";
  return pickFor({ accountId: d.fromAccountId, address: d.fromAddress, kind, signedBefore: signedBefore(findThread(s, d.threadId), s.accounts) });
}

/**
 * The account with `email` among its addresses, changed by `patch`. One it doesn't have yet is
 * added from `source` in `state`.
 */
function withAddress(
  account: Account,
  email: string,
  patch: Partial<AccountAddress>,
  source: AccountAddress["source"] = "you",
  state: AccountAddress["state"] = "added",
): Account {
  const list = account.addresses ?? [];
  if (list.some((x) => x.email === email)) return { ...account, addresses: list.map((x) => (x.email === email ? { ...x, ...patch } : x)) };
  return { ...account, addresses: [...list, { email, state, source, seen: 0, blocked: false, ...patch }] };
}

/**
 * Changes an account on screen at once, then saves it: the engine's answer replaces the guess, and
 * a failure puts the account back with a note. `reload` fetches the lists again afterwards, for
 * changes that move mail. Resolves once saved (or put back); undefined when there is no such account.
 */
function changeAccount(id: string, change: (a: Account) => Account, save: () => Promise<Account | undefined>, reload = false) {
  const before = useStore.getState().accounts.find((a) => a.id === id);
  if (!before) return undefined;
  const replace = (account: Account) => useStore.setState((s) => ({ accounts: s.accounts.map((a) => (a.id === id ? account : a)) }));
  replace(change(before));
  const lists = () => mail().live && reload && quietly(Promise.all([reloadThreads(), loadDone(), loadBlocked()]));
  return save().then(
    (saved) => {
      if (saved) replace(saved);
      lists();
    },
    (error: unknown) => {
      console.warn("Moss:", error);
      replace(before);
      useStore.setState({ toast: { ...toast("Couldn’t save that, so Moss put it back"), problem: true } });
      lists();
    },
  );
}

/** Open `view` on top of the current screen, remembering where to go back to. */
export function goTo(s: Pick<State, "view" | "history">, view: View): Pick<State, "view" | "history"> {
  return { view, history: s.view === view ? s.history : [...s.history, s.view] };
}

const attachingMessage = "Still attaching files. Try again when they are ready.";

/** Every send surface checks before taking its immutable delivery snapshot. */
export function refusePendingAttachments(draft: Draft): boolean {
  if (!attachmentImportPending(draft.id)) return false;
  const current = useStore.getState();
  if (current.view === "compose" && current.draft?.id === draft.id)
    useStore.setState({ sendError: attachingMessage });
  else useStore.setState({ toast: { ...toast(attachingMessage), problem: true, undo: false } });
  return true;
}

/** Refuse before optimistic handoff changes can hide a conflicting editor. */
export function refuseLocalRecovery(draft: Draft): boolean {
  if (!localRecoveryFor(draft)) return false;
  const message = "Review the local recovery in Drafts before sending, scheduling or discarding this email.";
  const current = useStore.getState();
  if (current.view === "compose" && current.draft?.id === draft.id) useStore.setState({ sendError: message });
  else useStore.setState({ toast: { ...toast(message), problem: true, undo: false } });
  return true;
}
