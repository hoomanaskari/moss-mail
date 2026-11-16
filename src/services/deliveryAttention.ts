export interface DeliveryAttentionCursor { createdAt: number; id: string }
export interface DeliveryAttentionEntry extends DeliveryAttentionCursor {
  accountId: string;
  fileCount: number;
  fileBytes: number;
  state: "held" | "accepted" | "uncertain";
  interruptedFrom: "held" | "claimed" | null;
}
export interface DeliveryAttentionPage { entries: DeliveryAttentionEntry[]; next: DeliveryAttentionCursor | null }
export type DeliveryResolution = "reviewed" | "copied";
export type DeliveryResolutionOutcome = { status: "resolved"; recoveryId: string | null } | { status: "alreadyFinished" | "sending" };
