import { useEffect, useRef, useState } from "react";
import { Button } from "../design-system/atoms";
import { DraftReview } from "../design-system/molecules/DraftReview/DraftReview";
import { useStore } from "../state/store";
import { fileSize } from "../lib/attachments";
import { plainLabel } from "../lib/displayLabel";
import { errorText } from "../services/errorText";
import { mail } from "../services/mail";
import type { DeliveryAttentionCursor, DeliveryAttentionEntry, DeliveryAttentionPage, DeliveryResolution } from "../services/deliveryAttention";
import type { SavedDraft } from "../state/drafts";
import styles from "./LocalDraftRecoveries.module.css";

export function DeliveryAttention({ onRecoveryChanged }: { onRecoveryChanged: () => void }) {
  const backend = mail();
  const accounts = useStore((state) => state.accounts);
  const accountName = (id: string) => {
    const account = accounts.find((candidate) => candidate.id === id);
    return plainLabel(account ? `${account.name} (${account.address})` : id);
  };
  const [cursor, setCursor] = useState<DeliveryAttentionCursor>();
  const [page, setPage] = useState<DeliveryAttentionPage>();
  const [refresh, setRefresh] = useState(0);
  const [problem, setProblem] = useState<string>();
  const [selected, setSelected] = useState<DeliveryAttentionEntry>();
  const [draft, setDraft] = useState<SavedDraft>();
  const [previewing, setPreviewing] = useState(false);
  const [previewProblem, setPreviewProblem] = useState<string>();
  const [confirm, setConfirm] = useState<DeliveryResolution>();
  const [acting, setActing] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);
  const busy = useRef(false);

  useEffect(() => {
    let live = true;
    setPage(undefined); setProblem(undefined);
    void backend.deliveryAttention(cursor).then((value) => { if (live) setPage(value); }, () => {
      if (live) setProblem("Cannot check deliveries. Your recovery data has been kept. Try again.");
    });
    return () => { live = false; };
  }, [backend, cursor, refresh]);

  useEffect(() => {
    let live = true;
    setDraft(undefined); setPreviewProblem(undefined); setConfirm(undefined); setAcknowledged(false);
    setPreviewing(!!selected);
    if (selected) void backend.deliveryAttentionDraft(selected.id).then((value) => {
      if (!live) return;
      setDraft(value);
      if (!value) setPreviewProblem("This delivery changed. Refresh the list before continuing.");
    }, () => {
      if (live) setPreviewProblem("The saved content cannot be previewed. Its data is retained until you explicitly finish review.");
    }).finally(() => { if (live) setPreviewing(false); });
    return () => { live = false; };
  }, [backend, selected]);

  const resolve = async () => {
    if (busy.current || !selected || !confirm || (confirm === "reviewed" && !acknowledged)) return;
    busy.current = true; setActing(true); setProblem(undefined);
    try {
      const result = await backend.resolveDelivery(selected.id, confirm);
      if (result.status === "sending") { setProblem("This delivery is still in progress. Wait for its outcome before reviewing it."); return; }
      if (result.status === "resolved" && result.recoveryId) onRecoveryChanged();
      setSelected(undefined); setCursor(undefined); setRefresh((value) => value + 1);
    } catch (error) {
      if (confirm === "copied") onRecoveryChanged();
      setProblem(`Could not confirm the review. ${errorText(error)}${confirm === "copied" ? " A separate recovery copy may already be saved below." : ""}`);
    }
    finally { busy.current = false; setActing(false); }
  };

  if (!selected && !problem && !cursor && (!page || page.entries.length === 0)) return null;
  return <section className={styles.panel} aria-label="Delivery recovery">
    <h2>Deliveries to review across all accounts</h2>
    {problem && <p role="alert">{problem}</p>}
    {!page && !problem && <p role="status">Checking deliveries…</p>}
    <Button variant="secondary" disabled={acting} onClick={() => { setSelected(undefined); setCursor(undefined); setRefresh((value) => value + 1); }}>Refresh deliveries</Button>
    {page && <ul className={styles.entries}>{page.entries.map((entry) => <li key={entry.id}>
      <span>{entry.state === "accepted" ? "Sent, local repair incomplete" : entry.state === "held" || entry.interruptedFrom === "held" ? "Unsent draft needs recovery" : "Delivery outcome unknown"}</span>
      <span><bdi>{accountName(entry.accountId)}</bdi><br />{new Date(entry.createdAt).toLocaleString()}</span>
      <Button variant="secondary" aria-label={`Review delivery for ${accountName(entry.accountId)} from ${new Date(entry.createdAt).toLocaleString()}`} disabled={acting} onClick={() => setSelected(entry)}>Review delivery</Button>
    </li>)}</ul>}
    {page?.next && <Button variant="secondary" disabled={acting} onClick={() => { setSelected(undefined); setCursor(page.next!); }}>Next deliveries</Button>}
    {selected && <div className={styles.review}>
      {selected.state === "accepted"
        ? <p>The server accepted this email. Do not send it again. Moss keeps retrying local repair while this recovery remains open. Discarding recovery files stops local Sent-copy repair, forwarded flags and reminders.</p>
        : selected.state === "held" || selected.interruptedFrom === "held"
          ? <p>This email was not submitted. You can keep a separate recovery copy to review in Drafts.</p>
          : <p>Delivery may have succeeded. Check Sent before sending another copy. A recovery copy does not prove the original was unsent.</p>}
      {previewing && <p role="status">Loading saved content…</p>}
      {previewProblem && <p role="alert">{previewProblem}</p>}
      {draft && <>
        <h3><bdi>{draft.subject || "No subject"}</bdi></h3>
        <DraftReview from={draft.fromAddress ?? accounts.find((account) => account.id === draft.fromAccountId)?.address ?? draft.fromAccountId} to={draft.to} cc={draft.cc} bcc={draft.bcc} attachments={draft.attachments} />
        <pre className={styles.body}>{draft.body}</pre>
      </>}
      {selected.state !== "accepted" && <Button variant="secondary" disabled={acting || previewing || !draft} onClick={() => setConfirm("copied")}>Keep a recovery copy</Button>}
      <Button variant="secondary" disabled={acting || previewing} onClick={() => { setAcknowledged(false); setConfirm("reviewed"); }}>Discard recovery files</Button>
      {confirm && <>
        <p>{selected.fileCount} recovery {selected.fileCount === 1 ? "file" : "files"} ({fileSize(selected.fileBytes)}) can be removed. Files still used elsewhere are kept.</p>
        <p>{confirm === "copied" ? "Keep a separate local copy, then stop recovery of this delivery. Review that copy in Drafts before sending anything."
          : "Stop repair without creating a copy. The attached recovery files are permanently removed. This may be the only local copy, especially with Resend. Saved text remains in the delivery record, but this review closes."}</p>
        {confirm === "reviewed" && <>
          <p>A recovery archive keeps this data in a private file. Moss cannot restore from an archive yet.</p>
          <Button variant="secondary" disabled={acting} onClick={() => useStore.getState().openSettings("storage")}>Open recovery archive settings</Button>
          <label><input type="checkbox" checked={acknowledged} disabled={acting} onChange={(event) => setAcknowledged(event.target.checked)} />I understand I am discarding recovery files without creating a copy</label>
        </>}
        <Button variant="secondary" disabled={acting || (confirm === "reviewed" && !acknowledged)} onClick={() => void resolve()}>Confirm review</Button>
        <Button variant="secondary" disabled={acting} onClick={() => setConfirm(undefined)}>Keep reviewing</Button>
      </>}
      <Button variant="secondary" disabled={acting} onClick={() => setSelected(undefined)}>Close delivery review</Button>
    </div>}
  </section>;
}
