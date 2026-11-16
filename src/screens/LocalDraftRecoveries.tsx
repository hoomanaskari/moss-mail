import { useEffect, useRef, useState } from "react";
import type { Account } from "../data/types";
import { Button } from "../design-system/atoms";
import { DraftReview } from "../design-system/molecules/DraftReview/DraftReview";
import { errorText } from "../services/errorText";
import { mail, type LocalDraftRecovery, type LocalDraftRecoveryEntry, type MailBackend } from "../services/mail";
import { useStore } from "../state/store";
import { localRecoveryHasNewerEdits, useDrafts, draftsSaved, type SavedDraft } from "../state/drafts";
import styles from "./LocalDraftRecoveries.module.css";

interface AdoptionAttempt {
  newId: string;
  account: string;
  version: string;
  missing: string[];
  copy: LocalDraftRecovery;
  running?: Promise<SavedDraft | undefined>;
  reported?: boolean;
}
// Native reply loss must not change an operation when its screen is reopened.
// Backend-scoped storage also keeps previews and independent clients isolated.
const adoptionAttempts = new WeakMap<MailBackend, Map<string, AdoptionAttempt>>();
function attemptsFor(backend: MailBackend) {
  let attempts = adoptionAttempts.get(backend);
  if (!attempts) { attempts = new Map(); adoptionAttempts.set(backend, attempts); }
  return attempts;
}

interface Props {
  refresh?: number;
  accounts: Account[];
  onAdopt: (draft: SavedDraft, recoveryId: string, version: string) => void;
  onResolve: (recoveryId: string, version: string, adoptedId: string | null) => void;
}

export function LocalDraftRecoveries({ accounts, onAdopt, onResolve, refresh }: Props) {
  useDrafts((state) => state.recoveries);
  useStore((state) => state.draft);
  const attempts = attemptsFor(mail());
  const [entries, setEntries] = useState<LocalDraftRecoveryEntry[]>([]);
  const [selected, setSelected] = useState<LocalDraftRecoveryEntry>();
  const [recovery, setRecovery] = useState<LocalDraftRecovery>();
  const [error, setError] = useState<string>();
  const [message, setMessage] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [account, setAccount] = useState("");
  const [understood, setUnderstood] = useState(false);
  const [withoutFiles, setWithoutFiles] = useState(false);
  const [discard, setDiscard] = useState(false);
  const [reload, setReload] = useState(0);
  const mounted = useRef(true);
  // Keep the operation unchanged after an ambiguous native reply.
  const attempt = useRef<AdoptionAttempt | undefined>(undefined);

  useEffect(() => {
    mounted.current = true;
    let current = true;
    void draftsSaved().then(() => mail().localDraftRecoveryIndex()).then((rows) => {
      if (current) setEntries([...rows, ...[...attempts.values()]
        .filter(({ copy }) => !rows.some((row) => row.id === copy.id))
        .map(({ copy }) => ({ id: copy.id, version: copy.version, originalId: copy.draft.id, updatedAt: copy.updatedAt }))]);
    }).catch((failure: unknown) => { if (current) setError(errorText(failure)); });
    return () => { current = false; mounted.current = false; };
  }, [reload, refresh]);

  async function review(entry: LocalDraftRecoveryEntry) {
    setSelected(entry); setRecovery(undefined); setError(undefined); setMessage(undefined);
    setAccount(""); setUnderstood(false); setWithoutFiles(false); setDiscard(false);
    attempt.current = undefined; setBusy(true);
    try {
      await draftsSaved();
      const previous = attempts.get(entry.id);
      const copy = previous?.copy ?? await mail().localDraftRecovery(entry.id);
      if (!mounted.current) return;
      if (!copy) throw new Error("This recovery is no longer available. Refresh the list.");
      attempt.current = previous;
      if (previous) { setAccount(previous.account); setUnderstood(true); setWithoutFiles(true); }
      setRecovery(copy); setSelected({ ...entry, version: copy.version });
    } catch (failure) { if (mounted.current) setError(errorText(failure)); }
    finally { if (mounted.current) setBusy(false); }
  }

  function resolved(id: string) {
    setEntries((rows) => rows.filter((row) => row.id !== id));
    setSelected(undefined); setRecovery(undefined); setDiscard(false);
  }

  async function adopt() {
    const copy = recovery!;
    const request: AdoptionAttempt = attempt.current ?? { newId: crypto.randomUUID(), account, version: copy.version, missing: copy.missingFiles, copy };
    attempt.current = request; attempts.set(copy.id, request); setBusy(true); setError(undefined);
    try {
      request.running ??= mail().adoptLocalDraftRecovery(copy.id, request.version, request.newId, request.account, request.missing);
      const draft = await request.running;
      attempts.delete(copy.id);
      if (!request.reported) {
        request.reported = true;
        if (draft) onAdopt(draft, copy.id, request.version);
        else onResolve(copy.id, request.version, request.newId);
      }
      if (!mounted.current) return;
      resolved(copy.id);
      if (!draft) {
        setMessage("This recovery was already opened. Its draft is no longer editable here.");
      }
    } catch (failure) {
      request.running = undefined;
      if (typeof failure === "object" && failure !== null && "kind" in failure && failure.kind === "localRecoveryRejected") {
        attempts.delete(copy.id);
        if (mounted.current) {
          await review({ id: copy.id, version: copy.version, originalId: copy.draft.id, updatedAt: copy.updatedAt });
          if (mounted.current) setError(errorText(failure));
        }
      } else if (mounted.current) setError(errorText(failure));
    }
    finally { if (mounted.current) setBusy(false); }
  }

  async function remove() {
    const entry = selected!;
    setBusy(true); setError(undefined);
    try {
      await mail().discardLocalDraftRecovery(entry.id, entry.version);
      attempts.delete(entry.id);
      onResolve(entry.id, entry.version, null);
      if (!mounted.current) return;
      resolved(entry.id);
    } catch (failure) { if (mounted.current) setError(errorText(failure)); }
    finally { if (mounted.current) setBusy(false); }
  }

  if (!entries.length && !error && !message) return null;
  const newer = !!recovery && localRecoveryHasNewerEdits(recovery.id, recovery.version);
  const chosen = accounts.find((candidate) => candidate.id === account);
  return <section className={styles.panel} aria-label="Local draft recoveries">
    <h2>Local draft recoveries</h2>
    <p>These copies stay on this Mac until you review them. They are separate from drafts that changed elsewhere.</p>
    {error && <p role="alert">{error}</p>}
    {message && <p role="status">{message}</p>}
    <Button variant="quiet" disabled={busy} onClick={() => { setError(undefined); setSelected(undefined); setRecovery(undefined); setEntries([]); setReload((value) => value + 1); }}>Refresh recoveries</Button>
    <ul className={styles.entries}>{entries.map((entry) => <li key={entry.id}>
      <span>Saved {new Date(entry.updatedAt).toLocaleString()}</span>
      <Button variant="outlineSm" disabled={busy} onClick={() => { void review(entry); }}>Review local recovery</Button>
    </li>)}</ul>
    {selected && <div className={styles.review}>
      {busy && <p role="status">Working…</p>}
      {recovery && <>
        {newer && <p role="alert">There are newer local edits than this copy. Save them from the editor, then review the recovery again before opening it as a new draft.</p>}
        <h3>{recovery.draft.subject || "No subject"}</h3>
        <DraftReview from={chosen?.address ?? "Choose an account"} to={recovery.draft.to} cc={recovery.draft.cc} bcc={recovery.draft.bcc} attachments={recovery.draft.attachments} />
        <pre className={styles.body}>{recovery.draft.body}</pre>
        {selected.id.startsWith("delivery-") && <p role="alert">This is a copy from delivery recovery. The original may already have been sent. Check Sent before sending this copy.</p>}
        <p>Check Sent and Scheduled before sending a recovered copy. Its original delivery may already have been accepted or scheduled.</p>
        <p>This opens a new email without the original quoted message, reply link, or signature. Review the recipients and text before sending.</p>
        <p>The selected account may sync this text to its Drafts folder.</p>
        {!accounts.length && <p>Add an account before opening this recovery as a draft.</p>}
        <label>Save recovery to account
          <select aria-label="Save recovery to account" value={account} disabled={busy || !!attempt.current} onChange={(event) => setAccount(event.target.value)}>
            <option value="">Choose an account</option>
            {accounts.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.address}</option>)}
          </select>
        </label>
        <label><input type="checkbox" checked={understood} disabled={busy || !!attempt.current} onChange={(event) => setUnderstood(event.target.checked)} />I understand this opens a standalone email</label>
        {recovery.missingFiles.length > 0 && <>
          <p>These files are unavailable in this local copy. Reattach them in the new draft if needed:</p>
          <ul>{recovery.missingFiles.map((id) => <li key={id}>{recovery.draft.attachments?.find((file) => file.id === id)?.name ?? "Unavailable attachment"}</li>)}</ul>
          <label><input type="checkbox" checked={withoutFiles} disabled={busy || !!attempt.current} onChange={(event) => setWithoutFiles(event.target.checked)} />Continue without the unavailable files</label>
        </>}
        <Button variant="primary" disabled={busy || (!attempt.current && (newer || !chosen || !understood || (recovery.missingFiles.length > 0 && !withoutFiles)))} onClick={() => { void adopt(); }}>Open as new draft</Button>
      </>}
      {discard ? <div>
        <p>Discard this local copy? Its text and files will no longer be available here.</p>
        <Button variant="subtle" disabled={busy} onClick={() => { void remove(); }}>Confirm discard</Button>
        <Button variant="quiet" disabled={busy} onClick={() => setDiscard(false)}>Keep recovery</Button>
      </div> : <Button variant="subtle" disabled={busy} onClick={() => setDiscard(true)}>Discard local recovery</Button>}
    </div>}
  </section>;
}
