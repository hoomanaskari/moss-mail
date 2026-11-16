import { create } from "zustand";
import { remindTime } from "../lib/time";
import { mail, type MailEvent, type ScheduleOutcome } from "../services/mail";
import { keepUncertainDraft, retainVolatileDraft, adoptDraft, handoffDraft, keepDraft, reloadDrafts, snapshotDraft, useDrafts, type SavedDraft } from "./drafts";
import { refuseLocalRecovery, refusePendingAttachments, filesTooBig, goTo, remember, toast, useStore, type Draft, type Toast } from "./store";

/** An email set to send later. It waits on this Mac and the engine sends it at its time. */
export interface ScheduledEmail {
  revision: number;
  operationId: string;
  /** The email as written, to open again. */
  draft: SavedDraft;
  /** When it goes (ms). */
  sendAt: number;
  /** When it is tried next (ms): `sendAt`, or later after a failed try. */
  nextTry: number;
  /** Tries that failed. */
  tries: number;
  /** Why the last try failed. */
  error?: string;
}

/** What the time picker is open for: the email in the composer, or a scheduled one to move. */
export type Picking = { kind: "compose" } | { kind: "move"; id: string; operationId: string; revision: number };

interface ScheduledState {
  /** Retained during this session so late replies cannot revive older operations. */
  latestOperation: Record<string, string>;
  finishedOperations: Record<string, true>;
  restoredOperations: Record<string, true>;
  pendingOperations: Record<string, number>;
  /** Soonest first. */
  emails: ScheduledEmail[];
  /** The selected row in the Scheduled list. */
  cursorId?: string;
  picking?: Picking;
  /** The email in the composer was set for this time until you opened it to edit. */
  editing?: { id: string; sendAt: number };
  setCursor(id: string): void;
  /** Moves through `among` (the list on screen), by default every scheduled email. */
  moveCursor(step: 1 | -1, among?: ScheduledEmail[]): void;
  /** H in the list: asks for a new time for this one. */
  openMove(id: string): void;
}

/**
 * Send later: emails waiting on this Mac for their time, and the picker that asks when.
 * Kept apart from the mail store, like Drafts.
 */
export const useScheduled = create<ScheduledState>()((set, get) => ({
  emails: [],
  latestOperation: {},
  finishedOperations: {},
  restoredOperations: {},
  pendingOperations: {},
  setCursor: (cursorId) => set({ cursorId }),
  moveCursor: (step, among) => {
    const { cursorId } = get();
    const emails = among ?? get().emails;
    const i = Math.max(0, emails.findIndex((e) => e.draft.id === cursorId));
    set({ cursorId: emails[Math.min(emails.length - 1, Math.max(0, i + step))]?.draft.id });
  },
  openMove: (id) => {
    const email = get().emails.find((e) => e.draft.id === id);
    if (email) set({ picking: { kind: "move", id, operationId: email.operationId, revision: email.revision } });
  },
}));

/** The Scheduled list for the account you are looking at. */
export function visibleScheduled(emails: ScheduledEmail[], accountFilter: string): ScheduledEmail[] {
  return emails.filter((e) => accountFilter === "all" || e.draft.fromAccountId === accountFilter);
}

const soonestFirst = (emails: ScheduledEmail[]) => [...emails].sort((a, b) => a.sendAt - b.sendAt);
const find = (id: string) => useScheduled.getState().emails.find((e) => e.draft.id === id);
const isCurrent = (id: string, operation: string) => (find(id)?.operationId ?? useScheduled.getState().latestOperation[id]) === operation;
const subjectOf = (e: ScheduledEmail) => e.draft.subject.trim() || "No subject";
const when = (at: number) => remindTime(at, Date.now());

/** Puts an email on the list, or replaces it there. */
function put(email: ScheduledEmail) {
  useScheduled.setState((s) => ({
    latestOperation: { ...s.latestOperation, [email.draft.id]: email.operationId },
    emails: soonestFirst([...s.emails.filter((e) => e.draft.id !== email.draft.id), email]),
  }));
}

function changePending(operation: string, delta: 1 | -1) {
  useScheduled.setState((s) => {
    const pendingOperations = { ...s.pendingOperations };
    const count = (pendingOperations[operation] ?? 0) + delta;
    if (count) pendingOperations[operation] = count;
    else delete pendingOperations[operation];
    return { pendingOperations };
  });
  if (eventRefreshRunning && delta === 1) eventRefreshAfterPending.add(operation);
  if (!useScheduled.getState().pendingOperations[operation] && eventRefreshAfterPending.delete(operation)) requestScheduledRefresh();
  else scheduleEventRefresh();
}

function markRestored(operation: string) {
  useScheduled.setState((s) => ({ restoredOperations: { ...s.restoredOperations, [operation]: true } }));
}

function finishOperation(id: string, operation: string) {
  useScheduled.setState((s) => ({ finishedOperations: { ...s.finishedOperations, [operation]: true } }));
  if (find(id)?.operationId === operation) drop(id);
}

function observeOutcome(id: string, operation: string, outcome: ScheduleOutcome) {
  if (outcome.status === "cancelled" || outcome.status === "completed") finishOperation(id, operation);
  if (outcome.status === "changed") {
    const current = find(id);
    if (!current || current.revision <= outcome.current.revision) put(outcome.current);
  }
  reportOutcome(outcome);
}

/** Takes an email off the list; the cursor moves to the one after it. */
function drop(id: string) {
  useScheduled.setState((s) => {
    const i = s.emails.findIndex((e) => e.draft.id === id);
    const emails = s.emails.filter((e) => e.draft.id !== id);
    if (s.cursorId !== id) return { emails };
    return { emails, cursorId: emails[Math.min(i, emails.length - 1)]?.draft.id };
  });
}

function say(text: string, extra: Partial<Toast> = {}) {
  useStore.setState({ toast: { ...toast(text), undo: false, ...extra } });
}

const problem = (error: Error) => say(error.message, { problem: true });

/** ⌘⇧↵ or Send later in the composer: asks when. Someone has to be on it first. */
export function openSendLater() {
  const { draft, sending } = useStore.getState();
  // Too many files: the composer already says so, before anything goes.
  if (!draft || sending || filesTooBig(draft) || refusePendingAttachments(draft) || refuseLocalRecovery(draft)) return;
  if (!draft.to.length && !draft.cc.length && !draft.bcc.length) {
    useStore.setState({ sendError: "add someone to send it to" });
    return;
  }
  useScheduled.setState({ picking: { kind: "compose" } });
}

export function closePicker() {
  useScheduled.setState({ picking: undefined });
}

/** A time was picked: the email in the composer goes then, or the one being moved does. */
export function pickTime(at: number) {
  const { picking } = useScheduled.getState();
  closePicker();
  if (picking?.kind === "move") moveTo(picking.id, at, picking.operationId, picking.revision);
  else if (picking?.kind === "compose") sendLater(at);
}

/**
 * The email in the composer goes at `at`: out of Drafts, onto the Scheduled list, and you are
 * back where you wrote it from. ⌘Z opens it again. Sending never moves an email to Done.
 */
function sendLater(at: number) {
  const { draft } = useStore.getState();
  if (!draft || refusePendingAttachments(draft) || refuseLocalRecovery(draft)) return;
  useScheduled.setState({ editing: undefined });
  useStore.setState({
    draft: undefined,
    sendError: undefined,
    noSubjectAsked: false,
    ...(draft.threadId ? { view: "thread" as const, openThreadId: draft.threadId } : { view: "inbox" as const }),
  });
  scheduleDraft(draft, at);
}

/**
 * `draft` goes at `at`: out of Drafts and onto the Scheduled list, from the composer or from
 * Ask Moss. ⌘Z opens it again. Sending never moves an email to Done.
 */
export function scheduleDraft(draft: Draft, at: number) {
  if (refusePendingAttachments(draft) || refuseLocalRecovery(draft)) return;
  const retained = retainVolatileDraft(draft);
  handoffDraft(draft.id); // The backend consumes the saved row in its scheduling transaction.
  const operationId = crypto.randomUUID();
  changePending(operationId, 1);
  put({ operationId, revision: 1, draft: snapshotDraft(draft), sendAt: at, nextTry: at, tries: 0 });
  useStore.setState((s) => ({
    undoSteps: remember(s.undoSteps, { kind: "schedule", draft, takeBack: () => takeBack(draft, operationId) }),
    toast: toast(`Scheduled for ${when(at)}`),
  }));
  // Capture outgoing content now; the adapter waits for earlier writes before IPC.
  const creation = mail().scheduleSend(draft, at, operationId);
  creations.set(operationId, creation);
  creation
    .then((outcome) => {
      if (!isCurrent(draft.id, operationId)) return;
      observeOutcome(draft.id, operationId, outcome);
      const intent = cancellationIntents.get(operationId);
      if (intent && outcome.status === "cancelled" && !useScheduled.getState().restoredOperations[operationId]) {
        const restored = restoreCancelled(draft, outcome);
        if (restored) {
          markRestored(operationId);
          if (intent === "cancel") say("Back in Drafts. It won’t send.");
          else if (showRestored(restored) && intent === "edit") useScheduled.setState({ editing: { id: draft.id, sendAt: at } });
        }
      }
    })
    .catch(async (error: Error) => {
      if (!isCurrent(draft.id, operationId) || useScheduled.getState().finishedOperations[operationId]) return;
      try {
        const outcome = await mail().unscheduleSend(draft.id, operationId);
        if (!isCurrent(draft.id, operationId) || useScheduled.getState().finishedOperations[operationId]) return;
        observeOutcome(draft.id, operationId, outcome);
        if (outcome.status === "cancelled") {
          const restored = restoreCancelled(draft, outcome);
          if (restored) notScheduled(restored, operationId, error.message);
        }
      } catch {
        if (isCurrent(draft.id, operationId)) {
          keepUncertainDraft(draft);
          say("Couldn’t confirm scheduling or cancellation. Check Scheduled before sending again.", { problem: true });
        }
      }
    })
    .finally(() => {
      retained.release();
      creations.delete(operationId);
      cancellationIntents.delete(operationId);
      changePending(operationId, -1);
    });
}

/** The engine won't take it: back in Drafts, and ⌘Z or Open shows it again with why. */
function notScheduled(draft: SavedDraft, operationId: string, reason: string) {
  markRestored(operationId);
  drop(draft.id);
  const reopen = () => useStore.setState({ view: "compose", draft, sendError: reason, ...(draft.threadId && { openThreadId: draft.threadId }) });
  useStore.setState((s) => ({
    undoSteps: remember(
      s.undoSteps.filter((x) => !(x.kind === "schedule" && x.draft.id === draft.id)),
      { kind: "schedule", draft, takeBack: reopen },
    ),
    toast: { ...toast(`Not scheduled: ${reason}`), problem: true, undoLabel: "Open", stays: true },
  }));
}

/** Only early cancellation licenses saving the caller's unsaved snapshot. */
function restoreCancelled(draft: Draft, outcome: Extract<ScheduleOutcome, { status: "cancelled" }>): SavedDraft | undefined {
  drop(draft.id);
  if (outcome.restored) {
    adoptDraft(outcome.restored);
    return outcome.restored;
  }
  if (outcome.early || !mail().live) return keepDraft(draft);
  useDrafts.setState((state) => {
    const { [draft.id]: retired, ...remaining } = state.retired;
    return { retired: remaining };
  });
  void reloadDrafts().catch(problem);
  say("This draft was already removed or handed off elsewhere.", { problem: true });
  return undefined;
}

function showRestored(draft: SavedDraft) {
  const current = useStore.getState();
  if (current.view === "compose" && current.draft) {
    say("Email restored to Drafts");
    return false;
  }
  useStore.setState({ view: "compose", draft, sendError: undefined, openThreadId: draft.threadId });
  return true;
}

/** ⌘Z after Send later: takes the email back off the list and opens it again. */
function takeBack(draft: Draft, operationId: string) {
  const s = useStore.getState();
  if (s.view === "compose" && s.draft) return; // You are writing another email.
  const retained = retainVolatileDraft(draft);
  if (creations.has(operationId)) cancellationIntents.set(operationId, "undo");
  mail()
    .unscheduleSend(draft.id, operationId)
    .catch(() => mail().unscheduleSend(draft.id, operationId))
    .then((outcome) => {
      if (!isCurrent(draft.id, operationId) || useScheduled.getState().restoredOperations[operationId]) return;
      observeOutcome(draft.id, operationId, outcome);
      if (outcome.status !== "cancelled") return;
      const restored = restoreCancelled(draft, outcome);
      if (!restored) return;
      markRestored(operationId);
      showRestored(restored);
    })
    .catch(() => say("Couldn’t confirm cancellation. The email may still send.", { problem: true })).finally(retained.release);
}

function reportOutcome(outcome: ScheduleOutcome) {
  if (outcome.status === "changed") { say("This schedule changed. Check the new time.", { problem: true }); return; }
  if (outcome.status === "sending") say("This email is sending. It cannot be changed now.", { problem: true });
  else if (outcome.status === "completed") say("This email has already been sent.");
  else if (outcome.status === "inDelivery") say("This email has a delivery in progress. Check its status before sending again.", { problem: true });
}

const creations = new Map<string, Promise<ScheduleOutcome>>();
const cancellationIntents = new Map<string, "undo" | "edit" | "cancel">();
const moveQueues = new Map<string, Promise<ScheduleOutcome | undefined>>();
const latestMove = new Map<string, symbol>();

/** Serialize this renderer's intent; revisions still protect against other clients. */
function moveSchedule(email: ScheduledEmail, at: number, sendingNow: boolean, expectedRevision: number) {
  const id = email.draft.id;
  const operation = email.operationId;
  const action = Symbol();
  latestMove.set(operation, action);
  changePending(operation, 1);
  put({ ...email, sendAt: at, nextTry: at, tries: 0, error: undefined });
  const previous = moveQueues.get(operation);
  const creation = creations.get(operation);
  const dependency = previous ?? creation?.catch(() => undefined);
  const request = dependency ? dependency.then((outcome) => {
    if (!isCurrent(id, operation) || useScheduled.getState().finishedOperations[operation]) return undefined;
    if (!previous && (outcome?.status === "scheduled" || outcome?.status === "alreadyScheduled")) {
      // Keep the revision the user saw. A replay moved elsewhere must conflict.
      return mail().rescheduleSend(id, at, operation, expectedRevision);
    }
    if (outcome?.status !== "rescheduled") return outcome;
    return mail().rescheduleSend(id, at, operation, outcome.revision);
  }) : mail().rescheduleSend(id, at, operation, expectedRevision);
  const ordered = request.finally(() => changePending(operation, -1));
  moveQueues.set(operation, ordered);
  void ordered.then((outcome) => {
    if (!outcome || !isCurrent(id, operation) || useScheduled.getState().finishedOperations[operation]) return;
    const latest = latestMove.get(operation) === action;
    if (outcome.status === "rescheduled") {
      const current = find(id);
      if (!current) return;
      if (current.revision > outcome.revision) {
        if (latest) say("This schedule changed. Check the new time.", { problem: true });
        return;
      }
      if (current.revision === outcome.revision && current.tries > 0) return;
      put({ ...current, ...(latest && current.revision < outcome.revision ? { sendAt: at, nextTry: at, tries: 0, error: undefined } : {}), revision: outcome.revision });
      if (latest) say(sendingNow ? `Sending “${subjectOf(email)}” now` : `Now set for ${when(at)}`);
    } else if (latest) {
      observeOutcome(id, operation, outcome);
      requestScheduledRefresh();
    }
  }, (error: Error) => {
    if (!isCurrent(id, operation) || latestMove.get(operation) !== action) return;
    problem(error);
    requestScheduledRefresh();
  }).finally(() => {
    if (moveQueues.get(operation) === ordered) moveQueues.delete(operation);
    if (latestMove.get(operation) === action) latestMove.delete(operation);
  });
}

/** H in the Scheduled list, then a time: it goes then instead, tried afresh. */
function moveTo(id: string, at: number, operationId: string, revision: number) {
  const email = find(id);
  if (!email || email.operationId !== operationId) return;
  moveSchedule(email, at, false, revision);
}

/** ⌘↵ in the Scheduled list: it goes now. The engine says when it has gone. */
export function sendNow(id: string) {
  const email = find(id);
  if (email) moveSchedule(email, Date.now(), true, email.revision);
}

/** Takes one off the list into Drafts, as the engine kept it (or as shown, for the demo). */
async function unschedule(id: string, intent: "edit" | "cancel"): Promise<SavedDraft | undefined> {
  const email = find(id);
  if (!email) return undefined;
  if (creations.has(email.operationId)) cancellationIntents.set(email.operationId, intent);
  try {
    const outcome = await mail().unscheduleSend(id, email.operationId);
    if (!isCurrent(id, email.operationId) || useScheduled.getState().restoredOperations[email.operationId]) return undefined;
    observeOutcome(id, email.operationId, outcome);
    if (outcome.status !== "cancelled") return undefined;
    const restored = restoreCancelled(email.draft, outcome);
    if (restored) markRestored(email.operationId);
    return restored;
  } catch (error) {
    if (!isCurrent(id, email.operationId)) return undefined;
    problem(error as Error);
    return undefined;
  }
}

/** ↵ in the Scheduled list: opens it to edit. It won't go until you send or schedule it again. */
export async function editScheduled(id: string) {
  const email = find(id);
  if (!email || !(await unschedule(id, "edit"))) return;
  useScheduled.setState({ editing: { id, sendAt: email.sendAt } });
  useStore.getState().openDraft(id);
}

/** ⌫ in the Scheduled list: it won't go, and waits in Drafts. */
export async function cancelScheduled(id: string) {
  if (await unschedule(id, "cancel")) say("Back in Drafts. It won’t send.");
}

/** From ⌘K or the inbox: what waits to send later. */
export function openScheduled() {
  const s = useStore.getState();
  useStore.setState(goTo(s, "scheduled"));
  useScheduled.setState({ cursorId: visibleScheduled(useScheduled.getState().emails, s.accountFilter)[0]?.draft.id });
  if (mail().live) requestScheduledRefresh();
}

let listRequest = 0;

/** Real mail: apply the newest response without undoing actions taken while it loaded. */
export async function loadScheduled() {
  const request = ++listRequest;
  const before = useScheduled.getState();
  let deadline!: ReturnType<typeof setTimeout>;
  const timeout = new Promise<never>((_, reject) => {
    deadline = setTimeout(() => reject(new Error("Loading Scheduled timed out.")), 10_000);
  });
  let incoming: ScheduledEmail[];
  try {
    incoming = await Promise.race([mail().scheduled(), timeout]);
  } finally {
    clearTimeout(deadline);
  }
  if (request !== listRequest) return false;
  useScheduled.setState((current) => {
    const beforeRows = new Map(before.emails.map((e) => [e.draft.id, e]));
    const currentRows = new Map(current.emails.map((e) => [e.draft.id, e]));
    const ids = new Set([...beforeRows.keys(), ...currentRows.keys(), ...Object.keys(before.latestOperation), ...Object.keys(current.latestOperation)]);
    const changed = new Set([...ids].filter((id) => beforeRows.get(id) !== currentRows.get(id) || before.latestOperation[id] !== current.latestOperation[id]));
    for (const e of [...before.emails, ...current.emails]) {
      if (before.pendingOperations[e.operationId] || current.pendingOperations[e.operationId]) changed.add(e.draft.id);
    }
    for (const row of incoming) {
      const existing = currentRows.get(row.draft.id);
      if (existing?.operationId === row.operationId && existing.revision > row.revision) changed.add(row.draft.id);
    }
    const emails = soonestFirst([
      ...incoming.filter((e) => !changed.has(e.draft.id) && !current.finishedOperations[e.operationId]),
      ...current.emails.filter((e) => changed.has(e.draft.id)),
    ]);
    return {
      emails,
      latestOperation: { ...current.latestOperation, ...Object.fromEntries(emails.map((e) => [e.draft.id, e.operationId])) },
      cursorId: emails.some((e) => e.draft.id === current.cursorId) ? current.cursorId : emails[0]?.draft.id,
    };
  });
  return true;
}

let eventRefreshWanted = false;
let eventRefreshRunning = false;
let eventRefreshTimer: ReturnType<typeof setTimeout> | undefined;
const eventRefreshAfterPending = new Set<string>();
const refreshBackoff = [1000, 2000, 5000];
let eventRefreshRetries = 0;
let eventRefreshDelay = 0;

/** Refresh unrelated rows now, then catch up protected rows as their actions settle. */
function scheduleEventRefresh() {
  if (!eventRefreshWanted || eventRefreshRunning || eventRefreshTimer !== undefined) return;
  eventRefreshTimer = setTimeout(() => {
    eventRefreshTimer = undefined;
    eventRefreshWanted = false;
    for (const operation of Object.keys(useScheduled.getState().pendingOperations)) eventRefreshAfterPending.add(operation);
    eventRefreshRunning = true;
    void loadScheduled().then((applied) => {
      if (applied) {
        eventRefreshRetries = 0;
        eventRefreshDelay = 0;
      } else {
        eventRefreshWanted = true;
      }
    }, () => {
      if (eventRefreshRetries < refreshBackoff.length) {
        eventRefreshDelay = refreshBackoff[eventRefreshRetries++];
        eventRefreshWanted = true;
      } else {
        say("Couldn’t refresh Scheduled. Open Scheduled to try again.", { problem: true });
      }
    }).finally(() => {
      eventRefreshRunning = false;
      scheduleEventRefresh();
    });
  }, eventRefreshDelay);
}

export function requestScheduledRefresh() {
  if (eventRefreshTimer !== undefined && eventRefreshDelay > 0) {
    clearTimeout(eventRefreshTimer);
    eventRefreshTimer = undefined;
  }
  eventRefreshWanted = true;
  eventRefreshRetries = 0;
  eventRefreshDelay = 0;
  scheduleEventRefresh();
}

/** What the engine says about an email set to send later: it went, or a try failed. */
export function scheduledEvent(e: MailEvent) {
  if (e.type === "scheduledChanged") {
    requestScheduledRefresh();
    return;
  }
  if (e.type === "scheduledSent" || e.type === "scheduledFailed" || e.type === "scheduledWaiting") {
    const state = useScheduled.getState();
    const latest = find(e.email.draft.id)?.operationId ?? state.latestOperation[e.email.draft.id];
    if (state.finishedOperations[e.email.operationId] || (latest && latest !== e.email.operationId)) return;
  }
  if (e.type === "scheduledSent") {
    finishOperation(e.email.draft.id, e.email.operationId);
    const subject = subjectOf(e.email);
    say(e.late ? `Sent “${subject}” late. Moss was closed at ${when(e.email.sendAt)}.` : `Sent “${subject}”`);
  } else if (e.type === "scheduledFailed" || e.type === "scheduledWaiting") {
    const current = find(e.email.draft.id);
    if (!current) {
      // Events may outlive a cancellation in another client. Only the backend can
      // reintroduce an absent row; never replay the event's stale snapshot.
      requestScheduledRefresh();
      return;
    }
    const pending = moveQueues.has(e.email.operationId) && useScheduled.getState().pendingOperations[e.email.operationId];
    if (current.revision > e.email.revision || (pending && current.revision === e.email.revision)) return;
    put(e.email);
    // Once is enough: the list keeps saying why, and it keeps trying.
    if (e.type === "scheduledWaiting" || e.email.tries !== 1) return;
    say(`Couldn’t send “${subjectOf(e.email)}”. Trying again at ${when(e.email.nextTry)}.`, {
      problem: true,
      stays: true,
      show: { label: "Show scheduled", run: openScheduled },
    });
  }
}
