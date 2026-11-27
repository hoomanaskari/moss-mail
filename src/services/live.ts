import { resetCredentialCleanup, refreshCredentialCleanup } from "./credentialCleanup";
/**
 * Real mail in the desktop app: loads accounts and threads from the engine at launch, and
 * keeps the screen in step with what sync reports. The browser and dev scenes never run this.
 */
import { create } from "zustand";
import { loadDrafts, reloadDrafts } from "../state/drafts";
import { loadScheduled, requestScheduledRefresh, scheduledEvent, useScheduled } from "../state/scheduled";
import { useSignatures } from "../state/signatures";
import { loadBlocked, loadDone, reloadThreads, useStore, visibleThreads } from "../state/store";
import { loadTrash } from "../state/trash";

export { reloadThreads };
import type { SyncSnapshot } from "./accountSetup";
import { toAccount, toProgress } from "./convert";
import { mail, quietly, type MailEvent } from "./mail";
import { preload } from "./preload";
import { refreshServerChanges, resetServerChanges } from "./serverChanges";
import { sayInterruptedSends } from "../state/interruptedSends";

export interface SyncProblem {
  problem: "signedOut" | "offline" | "server";
  message: string;
}

interface SyncState {
  /** Latest sync progress per account id. */
  progress: Record<string, SyncSnapshot>;
  /** Why an account isn't syncing right now. */
  problems: Record<string, SyncProblem>;
}

/** What an account says about itself while it can't sync. */
export const syncProblemLabel: Record<SyncProblem["problem"], string> = {
  signedOut: "Not syncing",
  offline: "Offline",
  server: "Not syncing",
};

export const useSync = create<SyncState>()(() => ({ progress: {}, problems: {} }));

let pending: ReturnType<typeof setTimeout> | undefined;

/** New mail comes in batches; reload once they settle. The Done, Blocked or Trash list too, while it is on screen. */
function reloadSoon() {
  clearTimeout(pending);
  pending = setTimeout(() => {
    quietly(reloadThreads());
    const { view } = useStore.getState();
    if (view === "done") quietly(loadDone());
    if (view === "blocked") quietly(loadBlocked());
    if (view === "trash") quietly(loadTrash());
  }, 300);
}

function setSignedOut(accountId: string, signedOut: boolean) {
  useStore.setState((s) => ({
    accounts: s.accounts.map((a) =>
      a.id === accountId && a.connection ? { ...a, connection: { ...a.connection, signedOut } } : a,
    ),
  }));
}

export function handleEvent(e: MailEvent) {
  switch (e.type) {
    case "serverChangesChanged":
      void refreshServerChanges(e.accountId);
      break;
    case "sendsInterrupted":
      quietly(sayInterruptedSends());
      break;
    case "mailChanged":
      reloadSoon();
      break;
    case "syncProgress":
      useSync.setState((s) => ({ progress: { ...s.progress, [e.accountId]: toProgress(e) } }));
      break;
    case "syncError":
      useSync.setState((s) => ({ problems: { ...s.problems, [e.accountId]: { problem: e.problem, message: e.message } } }));
      if (e.problem === "signedOut") setSignedOut(e.accountId, true);
      break;
    case "syncOk": {
      const { [e.accountId]: _gone, ...problems } = useSync.getState().problems;
      useSync.setState({ problems });
      setSignedOut(e.accountId, false);
      break;
    }
    case "draftsChanged":
      quietly(reloadDrafts());
      break;
    case "scheduledChanged":
    case "scheduledSent":
    case "scheduledFailed":
    case "scheduledWaiting":
      scheduledEvent(e);
      break;
    case "accountUpdated": {
      const account = toAccount(e.account);
      useStore.setState((s) => ({ accounts: s.accounts.map((a) => (a.id === account.id ? account : a)) }));
      break;
    }
  }
}

/**
 * Fetches the bodies that aren't on this Mac of the threads you're about to open: the one
 * under the cursor and the next few below it, since J moves down. Keyboard-first, so this
 * follows the cursor, not the mouse.
 */
export function preloadAround(s = useStore.getState(), ahead = 3) {
  const list = visibleThreads(s);
  const at = Math.max(0, list.findIndex((t) => t.id === s.cursorId));
  preload(list.slice(at, at + ahead + 1).map((t) => t.id));
}

/**
 * First paint in the desktop app: no demo mail. With no accounts yet, Moss opens on Add
 * account; otherwise on the inbox, filled from the local database while sync carries on.
 */
export async function startLive() {
  useStore.setState({ accountRemovals: {} });
  resetCredentialCleanup();
  void refreshCredentialCleanup();
  const accounts = await mail().accounts();
  // The saved signatures replace the demo ones, which must never sign real mail.
  // An engine from before addresses had their own signatures sends none.
  useSignatures.setState({ byAddress: {}, ...(await mail().signatures()) });
  await loadDrafts();
  useScheduled.setState({ emails: [], cursorId: undefined });
  useStore.setState({
    accounts,
    threads: [],
    doneThreads: [],
    blockedThreads: [],
    cursorId: undefined,
    openThreadId: undefined,
    accountFilter: "all",
    view: accounts.length ? "inbox" : "add-account",
    history: [],
  });
  useSync.setState({ progress: {}, problems: {} });
  resetServerChanges();
  const syncUpdates = new Set<string>();
  const stop = mail().onEvent((event) => {
    if (event.type === "syncOk" || event.type === "syncError") syncUpdates.add(event.accountId);
    handleEvent(event);
  });
  try {
    void loadScheduled().catch(requestScheduledRefresh);
    if (accounts.length) {
      useStore.setState({ knownPeople: await mail().contacts(), recipientPicks: await mail().recipientPicks() });
      await reloadThreads();
      await Promise.all([loadDone(), loadBlocked()]);
    }
    for (const issue of await mail().syncProblems()) {
      if (!syncUpdates.has(issue.accountId)) handleEvent({ type: "syncError", ...issue });
    }
    await Promise.all(accounts.map((account) => refreshServerChanges(account.id)));
    // Sending started before the window was listening may have been stopped by a crash.
    quietly(sayInterruptedSends());
    preloadAround();
    const follow = useStore.subscribe((now, before) => {
      if (now.cursorId !== before.cursorId || now.threads !== before.threads) preloadAround(now);
    });
    return () => {
      follow();
      stop();
    };
  } catch (error) {
    stop();
    throw error;
  }
}
