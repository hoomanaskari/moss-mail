/** Dev scenes for drafts: the inbox's Drafts tab, the Drafts list, a saved draft in the composer, and one at the end of its conversation. */
import { people } from "../data/demo";
import { useDrafts, type SavedDraft } from "../state/drafts";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

const minutes = (n: number) => Date.now() - n * 60_000;

const blank = { cc: [], bcc: [], fieldsOpen: true, showCc: false, showBcc: false, remindIfNoReply: false, signatureId: null };

/** Two unsent emails: a new one to Priya and a reply to Daniel's budget thread. */
const demoDrafts: SavedDraft[] = [
  {
    ...blank,
    id: "draft-shoot",
    mode: "new",
    fromAccountId: "studio",
    to: [people.priya],
    subject: "Call sheet for Friday",
    body: "Hi Priya,\n\nHere’s what I have so far for the shoot.",
    bodyHtml: "<p>Hi Priya,</p><p>Here’s what I have so far for the shoot.</p>",
    signatureId: "studio",
    updatedAt: minutes(12),
  },
  {
    ...blank,
    id: "draft-budget",
    mode: "reply",
    threadId: "q4-budget",
    fromAccountId: "work",
    to: [people.daniel],
    subject: "Re: Q4 design budget",
    body: "Thanks Daniel. One question about the user testing line",
    bodyHtml: "<p>Thanks Daniel. One question about the user testing line</p>",
    signatureId: "work-short",
    updatedAt: minutes(95),
  },
];

const keepDemoDrafts = () =>
  useDrafts.setState({ drafts: demoDrafts, status: Object.fromEntries(demoDrafts.map((d) => [d.id, "saved" as const])) });

export const draftScenes: Record<string, Scene> = {
  "inbox-drafts": { shows: /^Drafts$/, apply: keepDemoDrafts },
  "drafts-list": {
    shows: /show on your phone too/,
    apply: () => {
      keepDemoDrafts();
      useStore.getState().openDrafts();
    },
  },
  "drafts-empty": {
    shows: /No drafts/,
    apply: () => {
      useDrafts.setState({ drafts: [], status: {} });
      useStore.getState().openDrafts();
    },
  },
  // Your reply to Daniel, half written, at the end of his email.
  "thread-draft": {
    shows: /One question about the user testing line/,
    apply: () => {
      keepDemoDrafts();
      useStore.getState().open("q4-budget");
    },
  },
  "thread-draft-attachments": {
    shows: /contract-review.pdf/,
    apply: () => {
      keepDemoDrafts();
      useDrafts.setState((s) => ({ drafts: s.drafts.map((d) => d.id === "draft-budget" ? {
        ...d, cc: [people.priya], attachments: [{ id: "review-file", name: "contract-review.pdf", bytes: 2048, contentType: "application/pdf" }],
      } : d) }));
      useStore.getState().open("q4-budget");
    },
  },
  "composer-draft-saved": {
    shows: /Draft saved/,
    apply: () => {
      keepDemoDrafts();
      useStore.getState().openDraft("draft-shoot");
    },
  },
};
