import type { LocalDraftRecoveryWrite, MailBackend } from "../services/mail";
import type { SavedDraft } from "./drafts";

type Resolved = Extract<LocalDraftRecoveryWrite, { status: "resolved" }>;

/** One immutable base and recovery identity per conflicting composer chain. */
export class LocalDraftRecoveryWriter {
  private readonly originalId: string;
  private readonly baseRevision: string | undefined;
  private version: string | undefined;
  private pending: { draft: SavedDraft; previous: string | undefined } | undefined;
  private resolved: Resolved | undefined;
  private acknowledged: { key: string; result: LocalDraftRecoveryWrite } | undefined;
  private writing: Promise<unknown> = Promise.resolve();

  constructor(readonly id: string, base: SavedDraft, private readonly backend: () => Pick<MailBackend, "keepLocalDraftRecovery">) {
    this.originalId = base.id;
    this.baseRevision = base.revision;
  }

  /** A reviewed terminal receipt releases text while retaining replay protection. */
  resolve(adoptedId: string | null) {
    this.resolved = { status: "resolved", adoptedId };
    this.pending = undefined;
    this.acknowledged = undefined;
    this.writing = Promise.resolve();
  }

  keep(draft: SavedDraft): Promise<LocalDraftRecoveryWrite> {
    if (draft.id !== this.originalId) return Promise.reject(new Error("This recovery belongs to a different draft."));
    if (this.resolved) return Promise.resolve(this.resolved);
    const snapshot = structuredClone({ ...draft, id: this.originalId, revision: this.baseRevision });
    const save = async (): Promise<LocalDraftRecoveryWrite> => {
      if (this.resolved) return this.resolved;
      // The first response may have been lost after commit. Never change its
      // content or predecessor until that exact request has been acknowledged.
      if (this.pending) await this.flushPending();
      if (this.resolved) return this.resolved;
      if (this.acknowledged?.key === snapshotKey(snapshot)) return this.acknowledged.result;
      this.pending = { draft: snapshot, previous: this.version };
      return this.flushPending();
    };
    const result = this.writing.then(save, save);
    this.writing = result;
    return result;
  }

  private async flushPending(): Promise<LocalDraftRecoveryWrite> {
    const pending = this.pending!;
    const result = await this.backend().keepLocalDraftRecovery(this.id, pending.previous, pending.draft).catch((error: unknown) => {
      // Only this native error certifies a refused transaction. Transport errors
      // and unknown store failures keep the exact pending request for replay.
      if (typeof error === "object" && error !== null && "kind" in error && error.kind === "localRecoveryRejected") this.pending = undefined;
      throw error;
    });
    if (this.resolved) return this.resolved;
    if (result.status === "saved") {
      this.version = result.recovery.version;
      this.acknowledged = { key: snapshotKey(pending.draft), result };
    }
    else this.resolve(result.adoptedId);
    this.pending = undefined;
    return result;
  }
}

function snapshotKey(draft: SavedDraft): string {
  return JSON.stringify({ ...draft, updatedAt: 0 });
}
