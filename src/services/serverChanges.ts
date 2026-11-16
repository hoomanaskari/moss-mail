import { create } from "zustand";
import { useStore } from "../state/store";
import { errorText } from "./errorText";
import { mail, type ServerChangeStatus } from "./mail";

export interface ServerChangeView {
  status: ServerChangeStatus | null;
  unavailable: boolean;
  /** Why Try again or Discard didn't work, until the next status. */
  failed?: string;
}
export const useServerChanges = create<{ byAccount: Record<string, ServerChangeView> }>(() => ({ byAccount: {} }));
const requests = new Map<string, symbol>();

useStore.subscribe((now, before) => {
  if (now.accounts === before.accounts) return;
  const ids = new Set(now.accounts.map((account) => account.id));
  for (const id of requests.keys()) if (!ids.has(id)) requests.delete(id);
  useServerChanges.setState(({ byAccount }) => ({ byAccount: Object.fromEntries(Object.entries(byAccount).filter(([id]) => ids.has(id))) }));
});

export function resetServerChanges() {
  requests.clear();
  useServerChanges.setState({ byAccount: {} });
}

function checked(value: unknown, accountId: string): ServerChangeStatus | null {
  if (value === null) return null;
  if (!value || typeof value !== "object") throw new Error("Recovery status is unavailable.");
  const status = value as ServerChangeStatus;
  const counts = [status.completed, status.pending, status.discarded, status.dropped];
  if (status.accountId !== accountId || !["queued", "claimed", "blocked"].includes(status.state)
    || typeof status.change !== "string" || !/^[0-9a-f]{32}$/.test(status.change)
    || !Number.isSafeInteger(status.revision) || status.revision < 0
    || !["legacy", "done", "undone", "read", "delete", "restore", "move", "forwarded", "draftCleanup"].includes(status.kind)
    || !counts.every((count) => Number.isInteger(count) && count >= 0 && count <= 64)
    || status.pending < 1 || counts.reduce((sum, count) => sum + count, 0) > 64
    || (status.state === "blocked" ? !["refused", "unreadable", "uncertain", "tooLarge"].includes(status.failure as string) : status.failure !== null)) {
    throw new Error("Recovery status is unavailable.");
  }
  return {
    accountId: status.accountId, change: status.change, revision: status.revision, state: status.state, failure: status.failure, kind: status.kind,
    completed: status.completed, pending: status.pending, discarded: status.discarded, dropped: status.dropped,
  };
}

export async function refreshServerChanges(accountId: string): Promise<void> {
  if (!useStore.getState().accounts.some((account) => account.id === accountId)) return;
  const ticket = Symbol();
  requests.set(accountId, ticket);
  try {
    const status = checked(await mail().serverChangeStatus(accountId), accountId);
    if (requests.get(accountId) !== ticket) return;
    useServerChanges.setState(({ byAccount }) => ({ byAccount: { ...byAccount, [accountId]: { status, unavailable: false } } }));
  } catch {
    if (requests.get(accountId) !== ticket) return;
    useServerChanges.setState(({ byAccount }) => ({ byAccount: { ...byAccount, [accountId]: { status: byAccount[accountId]?.status ?? null, unavailable: true } } }));
  }
}

/** Asks the engine about a blocked change as you saw it, then shows where it stands now. */
async function decide(accountId: string, run: (status: ServerChangeStatus) => Promise<void>): Promise<void> {
  const status = useServerChanges.getState().byAccount[accountId]?.status;
  if (status?.state !== "blocked") return;
  let failed: string | undefined;
  try {
    await run(status);
  } catch (error) {
    failed = errorText(error);
  }
  await refreshServerChanges(accountId);
  if (failed) useServerChanges.setState(({ byAccount }) => ({ byAccount: { ...byAccount, [accountId]: { ...byAccount[accountId], failed } } }));
}

/** Try again: the blocked change goes back in its place and is sent now. */
export const retryServerChange = (accountId: string) =>
  decide(accountId, (status) => mail().retryServerChange(accountId, status.change, status.revision));

/** Discard: Moss stops trying. The change stays on this Mac only. */
export const discardServerChange = (accountId: string) =>
  decide(accountId, (status) => mail().discardServerChange(accountId, status.change, status.revision));
