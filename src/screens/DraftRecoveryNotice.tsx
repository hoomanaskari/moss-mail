import { CircleAlert } from "lucide-react";
import { Button } from "../design-system/atoms";
import { Notice } from "../design-system/molecules";
import { keepDraft, keepResolvedEdits, localRecoveryFor, localRecoveryIsCurrent, localRecoveryIsPending, useDrafts } from "../state/drafts";
import type { Draft } from "../state/store";

export function DraftRecoveryNotice({ draft, onReview }: { draft: Draft; onReview: () => void }) {
  useDrafts((state) => state.recoveries);
  const recovery = localRecoveryFor(draft);
  if (!recovery) return null;
  const resolved = recovery.outcome?.status === "resolved";
  const current = localRecoveryIsCurrent(draft);
  const pending = localRecoveryIsPending(draft) && !recovery.error;
  return <Notice variant="warn" icon={CircleAlert}>
    <div role="status">
      {resolved ? "This recovery was resolved. Keep any new edits separately before leaving."
        : recovery.saving ? "Saving a local recovery…"
        : current ? "Your local changes are kept separately. Review them before sending or scheduling."
        : pending ? "Waiting to save your latest changes…"
        : `Local recovery is not confirmed. Keep this window open and retry. ${recovery.error ?? ""}`}
    </div>
    {resolved ? <Button variant="warn" onClick={() => keepResolvedEdits(draft)}>Keep new edits separately</Button>
      : !current && !pending && <Button variant="warn" disabled={recovery.saving} onClick={() => keepDraft(draft)}>Retry local recovery</Button>}
    <Button variant="warn" onClick={onReview}>Review in Drafts</Button>
  </Notice>;
}
