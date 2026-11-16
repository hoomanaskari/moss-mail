import { TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "../design-system/atoms";
import { Notice } from "../design-system/molecules";
import { plainLabel } from "../lib/displayLabel";
import type { ServerChangeStatus } from "../services/mail";
import { discardServerChange, refreshServerChanges, retryServerChange, useServerChanges } from "../services/serverChanges";
import { useStore } from "../state/store";
import styles from "./InboxScreen.module.css";
import noticeStyles from "./ServerChangesNotice.module.css";

const names: Record<ServerChangeStatus["kind"], string> = {
  legacy: "Server change", done: "Archive", undone: "Restore to Inbox", read: "Read status",
  delete: "Delete", restore: "Restore", move: "Move", forwarded: "Forwarded marker", draftCleanup: "Draft cleanup",
};
const reasons = {
  refused: "The server refused this change. Try again, or discard it so later changes can go.",
  unreadable: "This version of Moss cannot safely read a saved server change.",
  uncertain: "The server’s answer may mean this change partly worked. Check your mail on the server, then try again or discard it.",
  tooLarge: "A saved server change is too large to process safely.",
};
const retryable = new Set(["refused", "uncertain"]);

/**
 * How long a change may be on its way before the inbox mentions it. Most reach the server in
 * under a second (deleting a sent email's draft copy, archiving on Done), and a note that
 * flashes past says nothing but alarms.
 */
export const slowChangeMs = 5000;

/** When each change now on its way was first seen; the next moment one of them turns slow. */
function useOnItsWay(waiting: string[]) {
  const seen = useRef(new Map<string, number>());
  const [, recheck] = useState(0);
  const now = Date.now();
  for (const key of seen.current.keys()) if (!waiting.includes(key)) seen.current.delete(key);
  for (const key of waiting) if (!seen.current.has(key)) seen.current.set(key, now);
  const left = waiting.map((key) => seen.current.get(key)! + slowChangeMs - now).filter((ms) => ms > 0);
  const next = left.length ? Math.min(...left) : undefined;
  useEffect(() => {
    if (next === undefined) return;
    const timer = setTimeout(() => recheck((n) => n + 1), next);
    return () => clearTimeout(timer);
  }, [next]);
  return (key: string) => now - seen.current.get(key)! < slowChangeMs;
}

const onItsWay = (status: ServerChangeStatus) => `${status.accountId}:${status.change}`;

export function ServerChangesNotice() {
  const accounts = useStore((s) => s.accounts);
  const accountFilter = useStore((s) => s.accountFilter);
  const { byAccount } = useServerChanges();
  const waiting = Object.values(byAccount).flatMap((view) => (!view.unavailable && view.status?.state === "claimed" ? [onItsWay(view.status)] : []));
  const tooSoon = useOnItsWay(waiting);
  return accounts.filter((account) => accountFilter === "all" || account.id === accountFilter).map((account) => {
    const view = byAccount[account.id];
    if (!view || (!view.unavailable && (!view.status || view.status.state === "queued"))) return null;
    if (!view.unavailable && view.status?.state === "claimed" && tooSoon(onItsWay(view.status))) return null;
    const { status, unavailable, failed } = view;
    const name = plainLabel(account.name);
    const blocked = status?.state === "blocked";
    return <div key={account.id} role="status" aria-label={`${name} server change status`} aria-live="polite">
      <Notice variant="caution" icon={TriangleAlert} className={styles.syncProblem}
        action={<span className={noticeStyles.actions}>
          {blocked && retryable.has(status.failure!) && <Button variant="secondary" aria-label={`Try the server change again in ${name}`} onClick={() => void retryServerChange(account.id)}>Try again</Button>}
          {blocked && <Button variant="secondary" aria-label={`Discard the server change in ${name}`} onClick={() => void discardServerChange(account.id)}>Discard</Button>}
          <Button variant="secondary" aria-label={`Check server changes in ${name}`} onClick={() => void refreshServerChanges(account.id)}>Check status</Button>
        </span>}>
        {status ? <>
          <strong><bdi>{name}</bdi>: {names[status.kind]} is waiting.</strong>{" "}
          {status.state === "claimed" ? "Waiting for a definite server outcome. Do not repeat this change."
            : status.state === "queued" ? "This change is waiting to reach the server."
              : reasons[status.failure!]}
          <span className={noticeStyles.detail}>The server acknowledged {status.completed} of {status.completed + status.pending + status.discarded + status.dropped} parts. {status.pending} remain pending.</span>
          {(status.discarded > 0 || status.dropped > 0) && <span className={noticeStyles.detail}>Some earlier parts were stopped; they are not counted as successful.</span>}
          {blocked && <span className={noticeStyles.detail}>Discard stops Moss trying: the change stays on this Mac only, and your mail server keeps your mail as it was.</span>}
          {failed && <span className={noticeStyles.detail}>Couldn’t do that: {failed}</span>}
          {unavailable && <span className={noticeStyles.detail}>Cannot refresh recovery status. Last known status is shown.</span>}
        </> : <><strong><bdi>{name}</bdi>: Server change status unavailable.</strong> Cannot check whether server changes are waiting.</>}
      </Notice>
    </div>;
  });
}
