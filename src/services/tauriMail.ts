import { protectQuit } from "./quit";
import type { BackupStatus } from "./backups";
/** The mail backend in the desktop app: each call is one Tauri command into the Rust engine. */
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { filterHits } from "../lib/search";
import { useSignatures } from "../state/signatures";
import { initialsOf } from "../lib/people";
import type { DraftFile, Person } from "../data/types";
import { beginDraftHandoff, acknowledgeDraftHandoff, acknowledgedDraftRevision, confirmDraftSaved, draftsSaved, type SavedDraft } from "../state/drafts";
import type { ScheduledEmail } from "../state/scheduled";
import type { Draft } from "../state/store";
import { toAccount, toProgress, toSearchHits, toThread, type RustAccount, type RustSearchHit, type RustSummary, type RustSyncProgress, type RustThread } from "./convert";
import type { LocalDraftRecovery, LocalDraftRecoveryEntry, LocalDraftRecoveryWrite, MailBackend, MailEvent, ScheduleOutcome, Sent, StorageInfo } from "./mail";
import { subscribe } from "./tauriEvents";

const today = () => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

import { errorText } from "./errorText";

export { errorText };

/** The fields of a draft the engine sends; the rest is layout. */
function outgoing(d: Draft) {
  return {
    mode: d.mode,
    threadId: d.threadId ?? null,
    fromAccountId: d.fromAccountId,
    fromAddress: d.fromAddress ?? null,
    to: d.to,
    cc: d.cc,
    bcc: d.bcc,
    subject: d.subject,
    body: d.body,
    bodyHtml: d.bodyHtml,
    // The engine adds it under the body, above any quote.
    signature: useSignatures.getState().signatures.find((x) => x.id === d.signatureId)?.lines ?? [],
    remindIfNoReply: d.remindIfNoReply,
    attachments: d.attachments ?? [],
  };
}

/** A draft as the engine keeps it: no layout, and people by name and address only. */
export interface RustDraft {
  revision?: string;
  id: string;
  mode: Draft["mode"];
  threadId: string | null;
  fromAccountId: string;
  fromAddress?: string | null;
  to: Pick<Person, "name" | "email">[];
  cc: Pick<Person, "name" | "email">[];
  bcc: Pick<Person, "name" | "email">[];
  subject: string;
  body: string;
  bodyHtml: string;
  remindIfNoReply: boolean;
  updatedAt: number;
  signatureId: string | null;
  signatureChosen: boolean;
  /** Missing on drafts kept before files could be attached. */
  attachments?: DraftFile[];
}

const bare = (people: Person[]) => people.map(({ name, email }) => ({ name, email }));
const named = (people: RustDraft["to"]): Person[] => people.map(({ name, email }) => ({ name, email, initials: initialsOf(name) }));

function kept(d: SavedDraft): RustDraft {
  return {
    id: d.id,
    revision: d.revision,
    mode: d.mode,
    threadId: d.threadId ?? null,
    fromAccountId: d.fromAccountId,
    fromAddress: d.fromAddress ?? null,
    to: bare(d.to),
    cc: bare(d.cc),
    bcc: bare(d.bcc),
    subject: d.subject,
    body: d.body,
    bodyHtml: d.bodyHtml,
    remindIfNoReply: d.remindIfNoReply,
    updatedAt: d.updatedAt,
    signatureId: d.signatureId,
    signatureChosen: !!d.signatureChosen,
    attachments: d.attachments ?? [],
  };
}

/** A kept draft, ready for the composer: its fields open, CC and BCC shown when they have people. */
export function reopened({ threadId, fromAddress, attachments = [], ...d }: RustDraft): SavedDraft {
  return {
    ...d,
    attachments,
    ...(threadId && { threadId }),
    ...(fromAddress && { fromAddress }),
    to: named(d.to),
    cc: named(d.cc),
    bcc: named(d.bcc),
    fieldsOpen: true,
    showCc: d.cc.length > 0,
    showBcc: d.bcc.length > 0,
  };
}

/** An email set to send later, as the engine keeps it. */
export interface RustScheduled {
  revision: number;
  operationId: string;
  draft: RustDraft;
  sendAt: number;
  nextTry: number;
  tries: number;
  error: string | null;
}

const toScheduled = ({ draft, error, ...e }: RustScheduled): ScheduledEmail => ({
  ...e,
  draft: reopened(draft),
  ...(error && { error }),
});

type RustScheduleOutcome = Exclude<ScheduleOutcome, { status: "cancelled" | "changed" }> | { status: "cancelled"; restored: RustDraft | null; early?: boolean } | { status: "changed"; current: RustScheduled };
const scheduleOutcome = (value: RustScheduleOutcome): ScheduleOutcome => value.status === "cancelled"
  ? { status: "cancelled", ...(value.early && { early: true }), ...(value.restored && { restored: reopened(value.restored) }) }
  : value.status === "changed" ? { status: "changed", current: toScheduled(value.current) } : value;

/** Engine events, with the emails in them ready for the screen. */
type RustEvent =
  | Exclude<MailEvent, { type: "scheduledSent" | "scheduledFailed" | "scheduledWaiting" }>
  | { type: "scheduledSent"; email: RustScheduled; late: boolean }
  | { type: "scheduledFailed"; email: RustScheduled }
  | { type: "scheduledWaiting"; email: RustScheduled };

function toEvent(e: RustEvent): MailEvent {
  if (e.type === "scheduledSent" || e.type === "scheduledFailed" || e.type === "scheduledWaiting") return { ...e, email: toScheduled(e.email) };
  return e;
}

/** A command whose refusal is a sentence to show. */
async function said<T>(command: Promise<T>): Promise<T> {
  try {
    return await command;
  } catch (error) {
    throw new Error(errorText(error));
  }
}

export const tauriMail: MailBackend = {
  live: true,
  accounts: async () => (await invoke<RustAccount[]>("accounts")).map(toAccount),
  threads: async () => {
    const now = Date.now();
    return (await invoke<RustSummary[]>("threads", { account: null })).map((t) => toThread(t, now));
  },
  thread: async (id) => toThread(await invoke<RustThread>("thread", { id }), Date.now()),
  doneThreads: async () => {
    const now = Date.now();
    return (await invoke<RustSummary[]>("done_threads", { account: null, limit: 200 })).map((t) => toThread(t, now));
  },
  markDone: (id) => invoke("mark_done", { id }),
  markNotDone: (id) => invoke("mark_not_done", { id }),
  deleteThread: (id) => invoke("delete_thread", { id }),
  restoreThread: (id) => invoke("restore_thread", { id }),
  markRead: (id, read) => invoke("mark_read", { id, read }),
  blockedThreads: async () => {
    const now = Date.now();
    return (await invoke<RustSummary[]>("blocked_threads", { account: null, limit: 200 })).map((t) => toThread(t, now));
  },
  blockSender: (id, wholeDomain) => invoke("block_sender", { id, wholeDomain }),
  acceptSender: (id, wholeDomain) => invoke("accept_sender", { id, wholeDomain }),
  unblockSender: (id) => invoke("unblock_sender", { id }),
  senderRules: () => invoke("sender_rules"),
  forgetSenderRule: (account, sender) => invoke("forget_sender_rule", { account, sender }),
  snooze: (id, until) => invoke("snooze", { id, until }),
  moveThread: (id, to) => invoke("move_thread", { id, to }),
  waitForReply: (id, until) => invoke("wait_for_reply", { id, until }),
  setSection: async (id, section) => void (await invoke("correct", { id, correction: { kind: "section", section } })),
  setSplit: async (id, split) => void (await invoke("correct", { id, correction: { kind: "split", split } })),
  clearDeadline: async (id) => void (await invoke("correct", { id, correction: { kind: "noDeadline" } })),
  remindIfNoReply: (id, on) => invoke<boolean>("correct", { id, correction: { kind: "remindIfNoReply", on } }),
  allowImages: (account, email) => invoke("allow_images", { account, email }),
  signatures: () => invoke("signatures"),
  saveSignature: (signature) => invoke("save_signature", { signature }),
  deleteSignature: (id) => invoke("delete_signature", { id }),
  setSignatureChoice: (account, kind, signature) => invoke("set_signature_choice", { account, kind, signature }),
  setLetMossChoose: (on) => invoke("set_let_moss_choose", { on }),
  markSignatureUsed: (id) => invoke("mark_signature_used", { id }),
  contacts: async () => named(await invoke<RustDraft["to"]>("contacts")),
  pickRecipient: (typed, email) => invoke("pick_recipient", { typed, email }),
  recipientPicks: () => invoke("recipient_picks"),
  drafts: async () => (await invoke<RustDraft[]>("drafts")).map(reopened),
  deliveryAttention: (after) => invoke("delivery_attention", { after: after ?? null }),
  deliveryAttentionDraft: async (id) => {
    const draft = await invoke<RustDraft | null>("delivery_attention_draft", { id });
    return draft ? reopened(draft) : undefined;
  },
  resolveDelivery: (id, action) => protectQuit("A delivery review is still being confirmed.", () => invoke("resolve_delivery", { id, action })),
  localDraftRecoveryIndex: () => invoke<LocalDraftRecoveryEntry[]>("local_draft_recovery_index"),
  localDraftRecovery: async (id) => {
    const recovery = await invoke<(Omit<LocalDraftRecovery, "draft"> & { draft: RustDraft }) | null>("local_draft_recovery", { id });
    return recovery ? { ...recovery, draft: reopened(recovery.draft) } : undefined;
  },
  keepLocalDraftRecovery: async (id, previous, draft) => {
    const result = await invoke<
      { status: "saved"; recovery: Omit<LocalDraftRecovery, "draft"> & { draft: RustDraft } }
      | Extract<LocalDraftRecoveryWrite, { status: "resolved" }>
    >("keep_local_draft_recovery", { id, previous: previous ?? null, draft: kept(draft) });
    return result.status === "saved" ? { ...result, recovery: { ...result.recovery, draft: reopened(result.recovery.draft) } } : result;
  },
  discardLocalDraftRecovery: (id, version) => invoke<void>("discard_local_draft_recovery", { id, version }),
  adoptLocalDraftRecovery: (id, version, newId, account, missing) => protectQuit("A recovery is still being opened.", async () => {
    const draft = await invoke<RustDraft | null>("adopt_local_draft_recovery", { id, version, newId, account, missing });
    return draft ? reopened(draft) : undefined;
  }),
  draftSaveReceipt: async (draft) => {
    const receipt = await invoke<RustDraft | null>("draft_save_receipt", { draft: kept(draft) });
    return receipt ? reopened(receipt) : undefined;
  },
  saveDraft: async (draft) => {
    const args = structuredClone({ draft: kept(draft) });
    try {
      return reopened(await invoke<RustDraft>("save_draft", args));
    } catch (error) {
      // The commit may have succeeded before its reply was lost. Read only: a
      // differing or retired row must retain the original failure and its kind.
      const receipt = await invoke<RustDraft | null>("draft_save_receipt", args).catch(() => null);
      if (receipt) return reopened(receipt);
      throw error;
    }
  },
  deleteDraft: async (id, keepFiles, revision) => void (await invoke("delete_draft", { id, keepFiles, revision })),
  pickFiles: async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    return (await open({ multiple: true, directory: false, title: "Attach files" })) ?? [];
  },
  attachFiles: (draftId, files) => protectQuit("An attachment import is still being confirmed.", async () => {
    // Files dropped from Finder arrive as paths (see useFileDrop); a File has none to read.
    const paths = files.filter((f): f is string => typeof f === "string");
    if (!paths.length) return [];
    try {
      return await invoke<DraftFile[]>("attach_files", { draftId, paths });
    } catch (error) {
      throw new Error(errorText(error));
    }
  }),
  discardDraftFile: async (draftId, id) => { await invoke("discard_draft_file", { draftId, id }); },
  draftPreview: async (draftId, id) => (await invoke<string | null>("draft_preview", { draftId, id })) ?? undefined,
  send: (draft) => protectQuit("A send is still being confirmed.", async () => {
    try {
      const args = structuredClone({ draft: outgoing(draft), recovery: kept({ ...draft, updatedAt: Date.now() }) });
      await draftsSaved();
      await confirmDraftSaved(draft);
      args.recovery.revision = acknowledgedDraftRevision(args.recovery, draft);
      await invoke("send", args);
    } catch (error) {
      const kind = typeof error === "object" && error !== null && "kind" in error ? error.kind : undefined;
      throw Object.assign(new Error(errorText(error)), { kind });
    }
  }),
  sendAfter: (hold, draft, delayMs) => protectQuit("A send is still being confirmed.", async (accepted) => {
    let stop: (() => void) | undefined;
    const finished = beginDraftHandoff(draft);
    try {
      const args = structuredClone({ hold, draft: outgoing(draft), recovery: kept({ ...draft, updatedAt: Date.now() }), delayMs });
      stop = await listen<string>("moss://send-prepared", ({ payload }) => { if (payload === hold) { acknowledgeDraftHandoff(draft);finished();accepted(); } }).catch(() => undefined);
      await draftsSaved();
      await confirmDraftSaved(draft);
      args.recovery.revision = acknowledgedDraftRevision(args.recovery, draft);
      return await invoke<Sent>("send_after", args);
    } catch (error) {
      const kind = typeof error === "object" && error !== null && "kind" in error ? error.kind : undefined;
      throw Object.assign(new Error(errorText(error)), { kind });
    } finally { stop?.();finished(); }
  }),
  cancelSend: (hold) => protectQuit("A send cancellation is still being confirmed.", async () => {
    const { restored, ...outcome } = await invoke<{ cancelled: boolean; early: boolean; restored: RustDraft | null }>("cancel_send", { hold });
    return { ...outcome, ...(restored && { restored: reopened(restored) }) };
  }),
  scheduleSend: (draft, sendAt, operation) => protectQuit("A schedule is still being confirmed.", async () => {
    const args = structuredClone({ operation, draft: kept({ ...draft, updatedAt: Date.now() }), email: outgoing(draft), sendAt });
    await draftsSaved();
    await confirmDraftSaved(draft);
    args.draft.revision = acknowledgedDraftRevision(args.draft, draft);
    return scheduleOutcome(await said(invoke<RustScheduleOutcome>("schedule_send", args)));
  }),
  scheduled: async () => (await invoke<RustScheduled[]>("scheduled")).map(toScheduled),
  rescheduleSend: (id, sendAt, operation, expectedRevision) => protectQuit("A schedule change is still being confirmed.", async () => scheduleOutcome(await said(invoke<RustScheduleOutcome>("reschedule_send", { id, sendAt, operation, expectedRevision })))),
  unscheduleSend: (id, operation) => protectQuit("A schedule cancellation is still being confirmed.", async () => scheduleOutcome(await said(invoke<RustScheduleOutcome>("unschedule_send", { id, operation })))),
  updateAccount: (id, patch) => invoke("update_account", { id, patch }),
  addAddress: async (account, email, name) => toAccount(await invoke<RustAccount>("add_address", { account, email, name: name ?? null })),
  updateAddress: async (account, email, name) => toAccount(await invoke<RustAccount>("update_address", { account, email, name: name ?? null })),
  removeAddress: async (account, email) => toAccount(await invoke<RustAccount>("remove_address", { account, email })),
  blockAddress: async (account, email, blocked) => toAccount(await invoke<RustAccount>("block_address", { account, email, blocked })),
  discoverAddresses: async (account) => toAccount(await invoke<RustAccount>("discover_addresses", { account })),
  setAddressSignatures: (account, email, choice) => invoke("set_address_signatures", { account, email, choice }),
  reorderAccounts: (ids) => invoke("reorder_accounts", { ids }),
  removeAccount: (id, discardServerChanges) => invoke("remove_account", discardServerChanges === undefined ? { id } : { id, discardServerChanges }),
  credentialCleanupStatus: () => invoke("credential_cleanup_status"),
  retryCredentialCleanup: () => invoke("retry_credential_cleanup"),
  syncNow: (accountId) => invoke("sync_now", { account: accountId ?? null }),
  syncProgress: async (accountId) => {
    const p = await invoke<RustSyncProgress | null>("sync_progress", { account: accountId });
    return p ? toProgress(p) : undefined;
  },
  syncProblems: () => invoke("sync_problems"),
  serverChangeStatus: (accountId) => invoke("server_change_status", { account: accountId }),
  retryServerChange: (account, change, revision) => invoke("retry_server_change", { account, change, revision }),
  discardServerChange: (account, change, revision) => invoke("discard_server_change", { account, change, revision }),
  interruptedSends: () => invoke("interrupted_sends"),
  dismissInterruptedSends: (ids) => invoke("dismiss_interrupted_sends", { ids }),
  search: async (query, filters) => {
    // With only a sender chosen, search for their address.
    const text = query.trim() || filters.from?.email || "";
    const account = filters.accountId === "all" ? null : filters.accountId;
    const hits = await invoke<RustSearchHit[]>("search", { query: text, account, limit: 100 });
    return filterHits(toSearchHits(hits, query, Date.now()), filters, today());
  },
  openAttachment: async (messageId, index) => {
    let path: string;
    try {
      path = await invoke<string>("attachment_file", { messageId, index });
    } catch (error) {
      throw new Error(errorText(error));
    }
    const opener = await import("@tauri-apps/plugin-opener");
    await opener.openPath(path);
  },
  attachmentPreview: async (messageId, index) =>
    (await invoke<string | null>("attachment_preview", { messageId, index })) ?? undefined,
  loadMessage: (messageId) => invoke<boolean>("load_message", { messageId }),
  preload: (ids) => invoke<void>("preload", { ids }),
  backupStatus: () => invoke<BackupStatus>("backup_status"),
  createBackup: () => invoke<number | null>("backup_start"),
  cancelBackup: (id) => invoke<boolean>("backup_cancel", { id }),
  storage: () => invoke<StorageInfo>("storage"),
  setOffline: (offline) => invoke<void>("set_offline", { offline }),
  setCacheLimit: (bytes) => invoke<void>("set_cache_limit", { bytes }),
  setAttachmentText: (on) => invoke<void>("set_attachment_text", { on }),
  clearCache: () => invoke<number>("clear_cache"),
  rebuildIndex: () => invoke<void>("rebuild_index"),
  onEvent: (handler) => subscribe<RustEvent>("moss://event", (e) => handler(toEvent(e))),
};
