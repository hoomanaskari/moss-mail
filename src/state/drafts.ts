import { create } from "zustand";
import { errorText } from "../services/errorText";
import { LocalDraftRecoveryWriter } from "./localDraftRecovery";
import type { LocalDraftRecoveryWrite } from "../services/mail";
import { mail, quietly } from "../services/mail";
import type { Draft, View } from "./store";

/** A draft as kept on this device: the composer's draft and when it last changed. */
export type SavedDraft = Draft & { updatedAt: number };

interface DraftsState {
  /** Unsent emails, most recently changed first. */
  drafts: SavedDraft[];
  /** By draft id: its latest save worked, or failed. No entry while it was never saved. */
  status: Record<string, "saved" | "failed">;
  acknowledgements: Record<string, { base?: string; bases?: Set<string | undefined>; revision: string; chain?: symbol }>;
  unsaved: Record<string, SavedDraft>;
  unconfirmed: Map<symbol, { saved: SavedDraft; revision?: string }>;
  recoveries: Map<symbol, { writer: LocalDraftRecoveryWriter; latest?: SavedDraft; savedOrder?: number; saving?: boolean; error?: string; outcome?: LocalDraftRecoveryWrite }>;
  retired: Record<string, true>;
  volatile: Map<symbol, SavedDraft>;
  /** The selected row in the Drafts list. */
  cursorId?: string;
  setCursor(id: string): void;
  /** Moves through `among` (the list on screen), by default every draft. */
  moveCursor(step: 1 | -1, among?: SavedDraft[]): void;
}

/**
 * Drafts: every unsent email with something in it, saved as you type so nothing is lost.
 * Kept apart from the mail store, which only knows the draft that is open.
 */
export const useDrafts = create<DraftsState>()((set, get) => ({
  drafts: [],
  status: {},
  acknowledgements: {},
  unsaved: {},
  unconfirmed: new Map(),
  recoveries: new Map(),
  retired: {},
  volatile: new Map(),
  setCursor: (cursorId) => set({ cursorId }),
  moveCursor: (step, among) => {
    const { cursorId } = get();
    const drafts = among ?? get().drafts;
    const i = Math.max(0, drafts.findIndex((d) => d.id === cursorId));
    set({ cursorId: drafts[Math.min(drafts.length - 1, Math.max(0, i + step))]?.id });
  },
}));

/** Pause after the last change before a draft is saved. */
export const saveDelay = 800;

/**
 * Something worth keeping. A reply or forward starts with its people and subject filled in,
 * so those don't count there; a new email counts as soon as anything is in it.
 */
export function hasContent(d: Draft): boolean {
  if (d.body.trim() || d.attachments?.length) return true;
  const people = d.to.length + d.cc.length + d.bcc.length > 0;
  if (d.mode === "new") return people || !!d.subject.trim();
  return d.mode === "forward" && people;
}

/** Your newest unsent email in conversation `threadId`, if there is one. */
export function threadDraft(drafts: SavedDraft[], threadId: string | undefined): SavedDraft | undefined {
  return drafts.filter((d) => d.threadId !== undefined && d.threadId === threadId).sort((a, b) => b.updatedAt - a.updatedAt)[0];
}

/** The Drafts list for the account you are looking at. */
export function visibleDrafts(drafts: SavedDraft[], accountFilter: string): SavedDraft[] {
  return drafts.filter((d) => accountFilter === "all" || d.fromAccountId === accountFilter);
}

// Editing ancestry belongs to one composer session, not merely to a database id.
// Weak keys disappear with old snapshots and never enter native IPC or persistence.
const draftChains = new WeakMap<object, symbol>();
const draftOrder = new WeakMap<object, number>();
let nextDraftOrder = 0;
function copyChain<T extends Draft>(source: Draft, copy: T): T {
  // keepDraft and the composer watcher establish ancestry before copying snapshots.
  draftChains.set(copy, draftChains.get(source)!);
  draftOrder.set(copy, draftOrder.get(source)!);
  return copy;
}
const adopted = new Set<string>();
let writing: Promise<unknown> = Promise.resolve();

/** Writes one change after the ones before it, so a delete can't land before the save it follows. */
function write<T>(change: () => Promise<T>): Promise<T> {
  const next = writing.then(change, change);
  writing = next;
  return next;
}

/** Retain an in-memory snapshot through UI callbacks until native or the save queue owns it. */
export function retainVolatileDraft(draft: Draft): { release: () => void; pending: () => boolean } {
  const key = Symbol();const saved = snapshotDraft(draft);
  useDrafts.setState(state => ({ volatile: new Map(state.volatile).set(key, saved) }));
  return { pending: () => useDrafts.getState().volatile.has(key), release: () => useDrafts.setState(state => {
    const volatile = new Map(state.volatile);volatile.delete(key);return { volatile };
  }) };
}

const handoffs = new Map<symbol, Draft>();
/** Settlement must not create a competing edit while native consumes the draft revision. */
export function beginDraftHandoff(draft: Draft): () => void {
  const key = Symbol();handoffs.set(key, draft);
  return () => { handoffs.delete(key); };
}

const importing = new Map<string, number>();

/** Covers the picker, queued staging and incorporation into the composer. */
export function beginAttachmentImport(id: string): () => void {
  importing.set(id, (importing.get(id) ?? 0) + 1);
  return () => {
    const remaining = importing.get(id)! - 1;
    if (remaining) importing.set(id, remaining);
    else importing.delete(id);
  };
}

export const attachmentImportPending = (id: string): boolean => importing.has(id);

/** File staging and cleanup share the save queue, including saves already in flight. */
export function attachDraftFiles(id: string, files: string[] | File[]) {
  return write(() => mail().attachFiles(id, files));
}

export function discardDraftFile(draftId: string, id: string) {
  quietly(write(() => mail().discardDraftFile(draftId, id)));
}

/** Resolves once every save and delete so far has finished, or failed. */
export const draftsSaved = () => writing.catch(() => {});

export interface DraftQuitBlocker { id: string; subject: string; reason: string }

/** Save pending text and make one bounded retry pass. This never initiates delivery. */
export async function settleDraftsForQuit(): Promise<DraftQuitBlocker[]> {
  flush();
  await draftsSaved();
  for (const { saved } of useDrafts.getState().unconfirmed.values()) {
    await write(() => confirmUnconfirmed(saved)).catch(() => {});
  }
  const retried = new Set<symbol>();
  for (const draft of Object.values(useDrafts.getState().unsaved)) {
    if ([...handoffs.values()].some(sent => sent.id === draft.id)) continue;
    const recovery = localRecoveryFor(draft);
    if (recovery?.outcome?.status === "resolved" || localRecoveryIsCurrent(draft)) continue;
    retried.add(draftChains.get(draft)!);
    keepDraft(draft);
  }
  for (const [chain, recovery] of useDrafts.getState().recoveries) {
    if (recovery.latest && !retried.has(chain) && recovery.outcome?.status !== "resolved" && !localRecoveryIsCurrent(recovery.latest)) {
      await write(() => keepRecovery(chain, recovery.latest!)).catch(() => {});
    }
  }
  flush();
  await draftsSaved();
  return currentDraftQuitBlockers();
}

/** Recheck synchronously at the exit verdict after async continuations may have edited. */
export function currentDraftQuitBlockers(): DraftQuitBlocker[] {
  const state = useDrafts.getState();
  const blockers = new Map<string, DraftQuitBlocker>();
  const add = (draft: Draft, reason: string) => blockers.set(draft.id, { id: draft.id, subject: draft.subject.trim() || "No subject", reason });
  for (const draft of Object.values(state.unsaved)) {
    const recovery = localRecoveryFor(draft);
    if (localRecoveryIsCurrent(draft)) continue;
    add(draft, recovery?.outcome?.status === "resolved"
      ? "This recovery was resolved. Keep the remaining local edits separately or discard them."
      : recovery?.error ?? "These edits have not been confirmed saved.");
  }
  for (const recovery of state.recoveries.values()) {
    if (recovery.latest && !localRecoveryIsCurrent(recovery.latest)) add(recovery.latest,
      recovery.outcome?.status === "resolved" ? "This recovery was resolved. Keep the remaining local edits separately or discard them."
        : recovery.error ?? "These local recovery edits have not been confirmed saved.");
  }
  for (const draft of state.drafts) {
    if (state.status[draft.id] === "failed" && !state.unsaved[draft.id] && !localRecoveryFor(draft))
      add(draft, "A draft deletion has not been confirmed. Review the draft before trying again.");
  }
  for (const { saved } of state.unconfirmed.values()) add(saved, "The last save has not been confirmed.");
  for (const draft of handoffs.values()) add(draft, "A delivery handoff has not been confirmed.");
  for (const draft of state.volatile.values()) add(draft, "This draft is still being handed off or restored.");
  if (pending) add(pending.draft, "New edits are still waiting to be saved.");
  for (const id of importing.keys()) {
    const draft = state.drafts.find((row) => row.id === id);
    blockers.set(id, { id, subject: draft?.subject.trim() || "No subject", reason: "An attachment is still being selected or imported." });
  }
  return [...blockers.values()];
}

/** Advance only through saves acknowledged for this local editing chain. */
export function acknowledgedDraftRevision(draft: Pick<Draft, "id" | "revision">, source: object = draft): string | undefined {
  const ack = useDrafts.getState().acknowledgements[draft.id];
  return ack && ack.chain !== undefined && ack.chain === draftChains.get(source) && (draft.revision === ack.base || draft.revision === ack.revision || !!ack.bases?.has(draft.revision)) ? ack.revision : draft.revision;
}

function setStatus(id: string, status?: "saved" | "failed") {
  useDrafts.setState((s) => {
    const { [id]: _, ...rest } = s.status;
    return { status: status ? { ...rest, [id]: status } : rest };
  });
}

/** Freeze local content while retaining the original editing ancestry. */
export function snapshotDraft(draft: Draft): SavedDraft {
  if (!draftChains.has(draft)) draftChains.set(draft, Symbol());
  if (!draftOrder.has(draft)) draftOrder.set(draft, ++nextDraftOrder);
  return copyChain(draft, { ...draft, updatedAt: Date.now() });
}

function acknowledgeSave(saved: SavedDraft, result: SavedDraft, revision: string | undefined) {
  const composer = watched?.getState().draft;
  const chain = draftChains.get(saved);
  if (chain && composer?.id === saved.id && draftChains.get(composer) === chain && (composer.revision === saved.revision || composer.revision === revision)) {
    adoptingRevision = true;
    try { watched!.setState({ draft: { ...composer, revision: result.revision } }); }
    finally { adoptingRevision = false; }
    if (pending?.draft.id === saved.id) pending.draft = copyChain(pending.draft, { ...pending.draft, revision: result.revision });
  }
  useDrafts.setState((s) => {
    const latest = s.unsaved[saved.id] === saved;
    const previous = s.acknowledgements[saved.id];
    const bases = new Set(previous?.chain === chain ? previous?.bases : []);
    bases.add(saved.revision);
    bases.add(revision);
    const { [saved.id]: _, ...remaining } = s.unsaved;
    const unconfirmed = new Map(s.unconfirmed);
    unconfirmed.delete(chain!);
    return {
      unconfirmed,
      acknowledgements: result.revision ? { ...s.acknowledgements, [saved.id]: { base: saved.revision, bases, revision: result.revision, chain } } : s.acknowledgements,
      drafts: s.drafts.map((current) => current === saved ? copyChain(current, { ...current, revision: result.revision }) : current),
      unsaved: latest ? remaining : s.unsaved,
      status: s.drafts.includes(saved) ? { ...s.status, [saved.id]: "saved" } : s.status,
    };
  });
}

/** Runs inside the write queue, before any operation consumes an edit token. */
async function confirmUnconfirmed(source: object) {
  const chain = draftChains.get(source);
  if (chain && useDrafts.getState().recoveries.has(chain)) throw new Error("Review the local recovery before sending, scheduling or discarding this draft.");
  const uncertain = chain === undefined ? undefined : useDrafts.getState().unconfirmed.get(chain);
  if (!uncertain) return;
  const receipt = await mail().draftSaveReceipt({ ...uncertain.saved, revision: uncertain.revision });
  if (receipt) acknowledgeSave(uncertain.saved, receipt, uncertain.revision);
  useDrafts.setState((state) => {
    const unconfirmed = new Map(state.unconfirmed);
    unconfirmed.delete(chain!);
    return { unconfirmed };
  });
}

/** Resolve an ambiguous save before a send or schedule consumes its revision. */
export function confirmDraftSaved(source: object): Promise<void> {
  return write(() => confirmUnconfirmed(source));
}

/** The presence of a recovery freezes this editing chain, including failed saves. */
export function localRecoveryFor(draft: Draft) {
  const chain = draftChains.get(draft);
  return chain && useDrafts.getState().recoveries.get(chain);
}

export function localRecoveryIsCurrent(draft: Draft): boolean {
  const recovery = localRecoveryFor(draft);
  return !!recovery && recovery.outcome?.status === "saved" && draftOrder.get(draft)! <= recovery.savedOrder!;
}

/** A review must not silently adopt an older copy of this editor's text. */
export function localRecoveryHasNewerEdits(id: string, version: string): boolean {
  const state = useDrafts.getState();
  for (const [chain, record] of state.recoveries) {
    if (record.writer.id !== id) continue;
    if (record.outcome?.status !== "saved" || record.outcome.recovery.version !== version) return true;
    const candidates = [record.latest, watched?.getState().draft, pending?.draft, ...Object.values(state.unsaved)];
    return candidates.some((draft) => draft !== undefined && draftChains.get(draft) === chain && draftOrder.get(draft)! > record.savedOrder!);
  }
  return false;
}

export function localRecoveryIsPending(draft: Draft): boolean {
  return pending?.draft === draft;
}

async function keepRecovery(chain: symbol, draft: SavedDraft) {
  const recovery = useDrafts.getState().recoveries.get(chain)!;
  if (recovery.outcome?.status === "resolved") throw new Error("This recovery was resolved. Keep any new edits separately before leaving.");
  const composer = watched?.getState().draft;
  const candidates = [draft, recovery.latest, composer && draftChains.get(composer) === chain ? snapshotDraft(composer) : undefined];
  const latest = candidates.filter((candidate): candidate is SavedDraft => candidate !== undefined)
    .reduce((newest, candidate) => draftOrder.get(candidate)! > draftOrder.get(newest)! ? candidate : newest);
  useDrafts.setState((state) => ({
    recoveries: new Map(state.recoveries).set(chain, { ...recovery, latest, saving: true, error: undefined }),
    drafts: state.drafts.map((row) => draftChains.get(row) === chain ? latest : row),
    unsaved: (state.unsaved[latest.id] && draftChains.get(state.unsaved[latest.id]) !== chain)
      || state.drafts.some((row) => row.id === latest.id && draftChains.get(row) !== chain)
      ? state.unsaved : { ...state.unsaved, [latest.id]: latest },
  }));
  const outcome = await recovery.writer.keep(latest).catch((error: unknown) => {
    useDrafts.setState((state) => {
      const current = state.recoveries.get(chain);
      return current?.writer === recovery.writer ? { recoveries: new Map(state.recoveries).set(chain, { ...current, saving: false, error: errorText(error) }) } : {};
    });
    throw error;
  });
  if (useDrafts.getState().recoveries.get(chain)?.writer !== recovery.writer) return;
  useDrafts.setState((state) => {
    const current = state.recoveries.get(chain)!;
    return { recoveries: new Map(state.recoveries).set(chain, { ...current, latest: current.outcome?.status === "resolved" ? current.latest : latest, saving: false,
      outcome: current.outcome?.status === "resolved" ? current.outcome : outcome,
      savedOrder: outcome.status === "saved" ? draftOrder.get(latest) : current.savedOrder,
    }) };
  });
  const state = useDrafts.getState();
  if (state.recoveries.get(chain)?.outcome?.status !== "resolved" && state.drafts.some(row => row.id === draft.id && draftChains.get(row) === chain)) setStatus(draft.id, "failed");
}

/** Saves a draft now and puts it at the top of the list. */
export function keepDraft(draft: Draft) {
  const saved = snapshotDraft(draft);
  useDrafts.setState((s) => {
    const { [draft.id]: retired, ...remaining } = s.retired;
    const { [draft.id]: status, ...statuses } = s.status;
    return { drafts: [saved, ...s.drafts.filter((d) => d.id !== draft.id)], unsaved: { ...s.unsaved, [draft.id]: saved }, retired: remaining, status: statuses };
  });
  write(async () => {
    const chain = draftChains.get(saved)!;
    if (useDrafts.getState().recoveries.has(chain)) {
      await keepRecovery(chain, saved);
      return;
    }
    await confirmUnconfirmed(saved);
    const revision = acknowledgedDraftRevision(saved);
    try {
      const result = await mail().saveDraft({ ...saved, revision });
      acknowledgeSave(saved, result, revision);
    } catch (error: unknown) {
      if (typeof error === "object" && error !== null && "kind" in error && error.kind === "draftConflict") {
        useDrafts.setState((state) => ({ recoveries: new Map(state.recoveries).set(chain, { writer: new LocalDraftRecoveryWriter(crypto.randomUUID(), { ...saved, revision }, mail) }) }));
        const composer = watched?.getState().draft;
        const latest = composer && draftChains.get(composer) === chain ? snapshotDraft(composer) : saved;
        await keepRecovery(chain, latest);
        return;
      }
      useDrafts.setState((state) => ({ unconfirmed: new Map(state.unconfirmed).set(chain, { saved, revision }) }));
      throw error;
    }
  }).catch((error: unknown) => {
    console.warn("Moss:", error);
    if (useDrafts.getState().drafts.includes(saved)) setStatus(draft.id, "failed");
  });
  return saved;
}

/** An ambiguous delivery must never become an ordinary editable/sendable draft. */
export function keepUncertainDraft(draft: Draft) {
  const saved = snapshotDraft(draft);const chain = draftChains.get(saved)!;
  useDrafts.setState(state => {
    const unconfirmed = new Map(state.unconfirmed);unconfirmed.delete(chain);
    const recoveries = new Map(state.recoveries);
    if (!recoveries.has(chain)) recoveries.set(chain, { writer: new LocalDraftRecoveryWriter(crypto.randomUUID(), saved, mail) });
    return { recoveries, unconfirmed };
  });
  if (useDrafts.getState().drafts.some(row => row.id === draft.id && draftChains.get(row) !== chain)) {
    useDrafts.setState(state => {
      const current = state.recoveries.get(chain)!;
      const latest = current.latest && draftOrder.get(current.latest)! > draftOrder.get(saved)! ? current.latest : saved;
      return { recoveries: new Map(state.recoveries).set(chain, { ...current, latest }) };
    });
    quietly(write(() => keepRecovery(chain, saved)));
    return saved;
  }
  return keepDraft(saved);
}

/** An early cancellation may have persisted only an operation tombstone, not its text. */
export function preserveCancelledDraft(draft: Draft) {
  const saved = snapshotDraft(draft);const chain = draftChains.get(saved);
  const row = useDrafts.getState().drafts.find(row => row.id === draft.id);
  if (row && draftChains.get(row) === chain && draftOrder.get(row)! >= draftOrder.get(saved)!) return;
  if (row) keepUncertainDraft(saved); else keepDraft(saved);
}

export function keepResolvedEdits(draft: Draft) {
  const chain = draftChains.get(draft);
  if (chain && useDrafts.getState().recoveries.get(chain)?.outcome?.status === "resolved") {
    const base = snapshotDraft(draft);
    useDrafts.setState((state) => ({ recoveries: new Map(state.recoveries).set(chain, { writer: new LocalDraftRecoveryWriter(crypto.randomUUID(), base, mail) }) }));
  }
  return keepDraft(draft);
}

/** Explicitly discard volatile edits after their journal has been resolved.
 * Keep the terminal chain marker so already queued writes cannot touch the winner.
 */
export function releaseResolvedLocalDraft(draft: Draft): boolean {
  const chain = draftChains.get(draft);
  const record = chain && useDrafts.getState().recoveries.get(chain);
  if (!record || record.outcome?.status !== "resolved") return false;
  if (pending && draftChains.get(pending.draft) === chain) {
    clearTimeout(pending.timer);
    pending = undefined;
  }
  useDrafts.setState((state) => ({
    recoveries: new Map(state.recoveries).set(chain!, { ...record, latest: undefined }),
    drafts: state.drafts.filter((row) => draftChains.get(row) !== chain),
    unsaved: Object.fromEntries(Object.entries(state.unsaved).filter(([, row]) => draftChains.get(row) !== chain)),
    status: Object.fromEntries(Object.entries(state.status).filter(([id]) => id !== draft.id)),
  }));
  return true;
}

/** Release only local text covered by the reviewed journal revision. */
export function resolveLocalRecovery(id: string, version: string, adoptedId: string | null) {
  const state = useDrafts.getState();
  const recoveries = new Map(state.recoveries);
  const cleared = new Set<symbol>();
  for (const [chain, record] of recoveries) {
    if (record.writer.id !== id) continue;
    const composer = watched?.getState().draft;
    const candidates = [record.latest, composer, pending?.draft, ...Object.values(state.unsaved)]
      .filter((draft) => draft && draftChains.get(draft) === chain);
    const newest = Math.max(0, ...candidates.map((draft) => draftOrder.get(draft!)!));
    if (record.outcome?.status === "saved" && record.outcome.recovery.version === version && newest <= record.savedOrder!) cleared.add(chain);
    record.writer.resolve(adoptedId);
    recoveries.set(chain, { ...record, latest: cleared.has(chain) ? undefined : record.latest, outcome: { status: "resolved", adoptedId } });
  }
  if (pending && cleared.has(draftChains.get(pending.draft)!)) {
    clearTimeout(pending.timer);
    pending = undefined;
  }
  const drafts = state.drafts.filter((draft) => !cleared.has(draftChains.get(draft)!));
  useDrafts.setState({ recoveries, drafts,
    unsaved: Object.fromEntries(Object.entries(state.unsaved).filter(([, draft]) => !cleared.has(draftChains.get(draft)!))),
    status: Object.fromEntries(Object.entries(state.status).filter(([id]) => drafts.some((draft) => draft.id === id))),
  });
}

/** Adopt an authoritative restored row without writing an old snapshot over it. */
export function adoptDraft(draft: SavedDraft) {
  adopted.add(draft.id);
  stopDraftSave(draft.id);
  useDrafts.setState((s) => {
    const { [draft.id]: oldAck, ...acknowledgements } = s.acknowledgements;
    const { [draft.id]: oldEdit, ...unsaved } = s.unsaved;
    const unconfirmed = new Map([...s.unconfirmed].filter(([, attempt]) => attempt.saved.id !== draft.id));
    const { [draft.id]: oldRetired, ...retired } = s.retired;
    return { drafts: [draft, ...s.drafts.filter((d) => d.id !== draft.id)], acknowledgements, unsaved, unconfirmed, retired, status: { ...s.status, [draft.id]: "saved" } };
  });
}

/** The draft waiting for you to stop typing. */
let pending: { draft: Draft; timer: ReturnType<typeof setTimeout> } | undefined;

function flush() {
  if (!pending) return;
  clearTimeout(pending.timer);
  const { draft } = pending;
  pending = undefined;
  keepDraft(draft);
}

/**
 * Deletes a draft: sent, discarded or emptied. Nothing to do for one that was never saved.
 * A sent one keeps its files (`keepFiles`) until the email has gone: Undo may bring it back.
 */
export function forgetDraft(id: string, keepFiles = false) {
  const saved = useDrafts.getState().drafts.find((draft) => draft.id === id);
  handoffDraft(id);
  if (saved) quietly(write(async () => {
    try {
      await confirmUnconfirmed(saved);
      const revision = acknowledgedDraftRevision(saved);
      await (revision === undefined ? mail().deleteDraft(id, keepFiles) : mail().deleteDraft(id, keepFiles, revision));
    } catch (error) {
      useDrafts.setState((state) => {
        const { [id]: hidden, ...retired } = state.retired;
        return { retired, drafts: state.drafts.some((row) => row.id === id) ? state.drafts : [saved, ...state.drafts], status: { ...state.status, [id]: "failed" } };
      });
      throw error;
    }
  }));
}

/** Sending transfers persistence to the durable backend; the UI must not delete it. */
export function stopDraftSave(id: string) {
  if (pending?.draft.id === id) {
    clearTimeout(pending.timer);
    pending = undefined;
  }
}

/** Native has durably taken this exact snapshot. Never retire later edits or another chain. */
export function acknowledgeDraftHandoff(source: Draft) {
  const chain = draftChains.get(source);
  const order = draftOrder.get(source);
  if (!chain || order === undefined) return;
  const covered = (draft: Draft) => draftChains.get(draft) === chain && draftOrder.get(draft)! <= order;
  const state = useDrafts.getState();
  const candidates = [state.unsaved[source.id], ...state.drafts.filter(row => row.id === source.id), pending?.draft.id === source.id ? pending.draft : undefined];
  useDrafts.setState(state => {
    const unconfirmed = new Map(state.unconfirmed);
    const attempt = unconfirmed.get(chain);
    if (!attempt || covered(attempt.saved)) unconfirmed.delete(chain);
    const volatile = new Map([...state.volatile].filter(([,draft]) => !covered(draft)));
    return { unconfirmed, volatile };
  });
  if (candidates.some(draft => draft && !covered(draft))) return;
  handoffDraft(source.id);
}

export function handoffDraft(id: string) {
  stopDraftSave(id);
  useDrafts.setState((s) => {
    const { [id]: _, ...unsaved } = s.unsaved;
    return { drafts: s.drafts.filter((d) => d.id !== id), retired: { ...s.retired, [id]: true }, unsaved };
  });
  setStatus(id);
}

function edited(draft: Draft) {
  // Reopened from the list, so already saved as it is.
  if (useDrafts.getState().drafts.includes(draft as SavedDraft)) return;
  if (!hasContent(draft) && !localRecoveryFor(draft)) {
    const existed = useDrafts.getState().drafts.some((row) => row.id === draft.id);
    forgetDraft(draft.id);
    if (existed) watched!.setState({ draft: { ...draft, id: crypto.randomUUID(), revision: undefined } });
    return;
  }
  if (pending) clearTimeout(pending.timer);
  pending = { draft, timer: setTimeout(flush, saveDelay) };
}

interface Watched {
  draft?: Draft;
  view: View;
}

/**
 * Saves the open draft as it changes: after a pause in typing, and at once when you leave the
 * composer or start another email. Returns a function that stops it.
 */
interface WatchedStore {
  subscribe(listener: (now: Watched, before: Watched) => void): () => void;
  getState(): Watched;
  setState(state: Partial<Watched>): void;
}
let watched: WatchedStore | undefined;
let adoptingRevision = false;

export function watchDrafts(store: WatchedStore) {
  watched = store;
  const initial = store.getState().draft;
  if (initial) draftChains.set(initial, Symbol());
  const stop = store.subscribe((now, before) => {
    if (now.draft && now.draft !== before.draft) {
      const state = useDrafts.getState();
      const opening = state.drafts.includes(now.draft as SavedDraft) && state.unsaved[now.draft.id] !== now.draft;
      const inherited = !opening && before.draft?.id === now.draft.id ? draftChains.get(before.draft) : undefined;
      draftChains.set(now.draft, opening ? Symbol() : inherited ?? draftChains.get(now.draft) ?? Symbol());
      draftOrder.set(now.draft, ++nextDraftOrder);
    }
    if (!adoptingRevision && now.draft !== before.draft) {
      if (pending && pending.draft.id !== now.draft?.id) flush();
      if (now.draft) edited(now.draft);
    }
    if (before.view === "compose" && now.view !== "compose") flush();
  });
  return () => {
    stop();
    if (watched === store) watched = undefined;
    if (pending) clearTimeout(pending.timer);
    pending = undefined;
  };
}

/**
 * Drafts changed on the server (your phone, webmail): the list as the engine has it now. What you
 * typed here since stays until its own save is acknowledged. Wall clocks do not grant edit authority.
 */
export async function reloadDrafts() {
  return write(async () => {
    adopted.clear();
    const loaded = await mail().drafts();
    const { drafts, status, cursorId, unsaved, retired } = useDrafts.getState();
    const mine = new Map(drafts.map((d) => [d.id, d]));
    const dirty = (id: string) => adopted.has(id) || !!unsaved[id] || pending?.draft.id === id;
    const theirs = new Set(loaded.map((d) => d.id));
    const kept = [
      ...loaded.filter((d) => !retired[d.id]).map((d) => dirty(d.id) && mine.has(d.id) ? mine.get(d.id)! : d),
      ...drafts.filter((d) => !theirs.has(d.id) && dirty(d.id) && !retired[d.id]),
    ].sort((a, b) => b.updatedAt - a.updatedAt);
    const composer = watched?.getState().draft;
    const replacement = composer && !dirty(composer.id)
      ? kept.find((d) => d.id === composer.id) : undefined;
    useDrafts.setState({
      drafts: kept,
      retired: Object.fromEntries(Object.entries(retired).filter(([id]) => theirs.has(id))),
      status: Object.fromEntries(kept.flatMap((d) => {
        const state = dirty(d.id) ? status[d.id] : "saved";
        return state ? [[d.id, state]] : [];
      })),
      cursorId: kept.some((d) => d.id === cursorId) ? cursorId : cursorId && kept[0]?.id,
    });
    if (replacement && replacement.revision !== composer!.revision) watched!.setState({ draft: replacement });
  });
}

/** Real mail: the drafts kept on this Mac. */
export async function loadDrafts() {
  await reloadDrafts();
}
