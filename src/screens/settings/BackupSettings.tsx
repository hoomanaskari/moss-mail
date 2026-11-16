import { useEffect, useRef, useState } from "react";
import { Button } from "../../design-system/atoms";
import { ListCard, SectionHeader } from "../../design-system/molecules";
import { size } from "../../lib/bytes";
import { isBackupStatus, type BackupStatus } from "../../services/backups";
import { mail } from "../../services/mail";
import shared from "./settings.module.css";
import styles from "./BackupSettings.module.css";

const phases = { preparing: "Preparing archive…", copying: "Copying local mail…", checking: "Checking archive…", hashing: "Computing checksum…", publishing: "Saving archive…" };

/** Native ownership continues after this screen unmounts; polling never owns the job. */
export function BackupSettings() {
  const backend = mail();
  const [status, setStatus] = useState<BackupStatus>();
  const [checking, setChecking] = useState(true);
  const [acting, setActing] = useState(false);
  const [requestProblem, setRequestProblem] = useState<string>();
  const [actionProblem, setActionProblem] = useState<string>();
  const [refresh, setRefresh] = useState(0);
  const epoch = useRef(0);
  const action = useRef(false);

  useEffect(() => {
    action.current = false;
    setActing(false);
    setStatus(undefined);
    setActionProblem(undefined);
    setRequestProblem(undefined);
  }, [backend]);

  useEffect(() => {
    const revision = ++epoch.current;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (revision !== epoch.current) return;
      setChecking(true);
      try {
        const result = await backend.backupStatus();
        if (revision !== epoch.current) return;
        if (!isBackupStatus(result)) throw new Error("Invalid backup status");
        setStatus(result);
        setRequestProblem(undefined);
        if (result.state === "running") timer = setTimeout(() => void poll(), 1000);
      } catch {
        if (revision === epoch.current) setRequestProblem("Cannot check backup status. Check again before starting another archive.");
      } finally {
        if (revision === epoch.current) setChecking(false);
      }
    };
    void poll();
    return () => { ++epoch.current; clearTimeout(timer); };
  }, [backend, refresh]);

  const run = async (work: () => Promise<unknown>, problem: string) => {
    if (action.current) return;
    action.current = true;
    const revision = ++epoch.current;
    setActing(true);
    setChecking(true);
    setActionProblem(undefined);
    try { await work(); }
    catch { if (revision === epoch.current) setActionProblem(problem); }
    finally {
      if (revision === epoch.current) {
        action.current = false;
        setActing(false);
        setRefresh((n) => n + 1);
      }
    }
  };
  const running = status?.state === "running";
  const problem = actionProblem ?? requestProblem;
  return <section className={shared.group} aria-label="Recovery archive">
    <SectionHeader label="Recovery archive" />
    <ListCard>
      <div className={styles.content}>
        <p>Save a private copy of the mail, drafts and files currently on this Mac. Mail kept only on the server and saved sign-ins are not included.</p>
        <p>Choose a new filename; existing files are kept. Restore is not available yet.</p>
        <div role="status" aria-live="polite">
          {checking && !status && <p>Checking backup status…</p>}
          {status?.state === "running" && <>
            <p>{status.cancelling ? "Cancelling…" : phases[status.progress.phase]}</p>
            {status.progress.phase === "copying" && <progress aria-label="Archive copy progress" value={status.progress.copiedPages} max={Math.max(1, status.progress.totalPages)} />}
          </>}
          {status?.state === "complete" && <>
            <p>Archive saved ({size(status.bytes)}). Keep it private.</p>
            <p>Keep this checksum separately to verify this copy later: <code className={styles.checksum}>{status.sha256}</code></p>
          </>}
          {status?.state === "cancelled" && <p>Backup cancelled. No archive was published.</p>}
          {status?.state === "failed" && <p>{status.message.trim() || "Archive creation reported a problem. Check the destination before trying again."}</p>}
        </div>
        {problem && <p role="alert">{problem}</p>}
        <div className={styles.actions}>
        <Button variant="secondary" disabled={!status || running || checking || acting || !!requestProblem} onClick={() => void run(() => backend.createBackup(), "Could not confirm the archive request. Check its status before trying again.")}>Create recovery archive</Button>
        {status?.state === "running" && <Button variant="secondary" disabled={acting || status.cancelling} onClick={() => void run(() => backend.cancelBackup(status.id), "Could not request cancellation. The archive may still be running.")}>Cancel backup</Button>}
        {problem && <Button variant="secondary" disabled={acting} onClick={() => { setActionProblem(undefined); setRefresh((n) => n + 1); }}>Check backup status</Button>}
        </div>
      </div>
    </ListCard>
  </section>;
}
