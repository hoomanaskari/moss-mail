import type { Scene } from "./scenes";
import { accounts } from "../data/demo";
import { demoMail, setMail, type LocalDraftRecovery } from "../services/mail";
import { useQuit } from "../services/quit";
import { useStore } from "../state/store";

const copy: LocalDraftRecovery = {
  id: "scene-recovery", version: "scene-version", updatedAt: 1_794_412_800_000, missingFiles: ["missing"],
  draft: { id: "scene-original", mode: "reply", fromAccountId: "removed-account", to: [{ name: "Alex", email: "alex@example.test", initials: "A" }], cc: [], bcc: [], subject: "Cabinet plans", body: "Hi Alex,\n\nHere are my latest notes on the cabinet dimensions. Please keep the wider drawer in the revised plan.\n\nThanks!", bodyHtml: "", remindIfNoReply: false, fieldsOpen: true, showCc: false, showBcc: false, updatedAt: 1, signatureId: null, attachments: [{ id: "missing", name: "cabinet-plans.pdf", contentType: "application/pdf", bytes: 2048 }] },
};
export const recoveryScenes: Record<string, Scene> = {
  "delivery-recovery": {
    apply: () => {
      setMail({ ...demoMail,
        deliveryAttention: async () => ({ entries: [
          { id: "scene-accepted", accountId: accounts[0].id, state: "accepted", interruptedFrom: null, createdAt: copy.updatedAt, fileCount: 1, fileBytes: 2048 },
          { id: "scene-uncertain", accountId: accounts[0].id, state: "uncertain", interruptedFrom: "claimed", createdAt: copy.updatedAt + 60_000, fileCount: 1, fileBytes: 2048 },
        ], next: null }),
        deliveryAttentionDraft: async () => ({ ...copy.draft, fromAccountId: accounts[0].id }),
      });
      useStore.setState({ view: "drafts", accounts });
    },
    shows: /Deliveries to review across all accounts/,
  },
  "local-draft-recovery": {
    apply: () => {
      setMail({ ...demoMail,
        localDraftRecoveryIndex: async () => [{ id: copy.id, version: copy.version, originalId: copy.draft.id, updatedAt: copy.updatedAt }],
        localDraftRecovery: async () => copy,
      });
      useStore.setState({ view: "drafts", accounts });
    },
    shows: /Local draft recoveries/,
  },
  "quit-unsaved-drafts": {
    apply: () => { useQuit.setState({ token: "scene-quit", checking: false, blockers: [{ id: "scene-original", subject: "Cabinet plans", reason: "Your latest changes have not been confirmed saved." }], error: undefined }); },
    shows: /Drafts are not confirmed saved/,
  },
};
