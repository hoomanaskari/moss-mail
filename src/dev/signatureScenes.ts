/** Dev scenes for the signature row under a draft: the one Moss picks in a back-and-forth, one from webmail, or none and why. */
import { textToHtml } from "../lib/textHtml";
import { useSignatures } from "../state/signatures";
import { accountFor, useStore } from "../state/store";
import { useQuickReply } from "../state/quickReply";
import type { Scene } from "./scenes";

const shoot = "fridays-shoot";

/** Your reply sits in `threadId` already, then theirs after it: a back-and-forth. Safe to run twice. */
function signedOnce(threadId: string) {
  useStore.setState((s) => ({
    threads: s.threads.map((t) => {
      if (t.id !== threadId || t.messages.some((m) => m.id === `${threadId}-yours`)) return t;
      const theirs = t.messages[t.messages.length - 1];
      const account = accountFor(t.accountId);
      const you = { name: account.senderName, email: account.address, initials: "MR" };
      const yours = { ...theirs, id: `${threadId}-yours`, from: you, to: [t.from], paragraphs: ["Let me check with the stylist."] };
      return { ...t, messages: [...t.messages.slice(0, -1), yours, theirs] };
    }),
  }));
}

const reply = "Hi Priya,\n\nYes, 10 AM works for me. See you on Friday.";

export const signatureScenes: Record<string, Scene> = {
  // Studio's signature is a full one, so a later reply goes without.
  "composer-signature-left-out": {
    shows: /Left out: quick reply in a back-and-forth/,
    apply: () => {
      signedOnce(shoot);
      const s = useStore.getState();
      s.open(shoot);
      s.compose("reply", shoot);
      s.updateDraft({ body: reply, bodyHtml: textToHtml(reply) });
    },
  },
  // Work has a one-line one for replies, so it stays.
  "composer-signature-short": {
    shows: /Short one: you signed earlier in this thread/,
    apply: () => {
      signedOnce(shoot);
      const s = useStore.getState();
      s.open(shoot);
      s.compose("reply", shoot);
      s.updateDraft({ fromAccountId: "work", body: reply, bodyHtml: textToHtml(reply) });
    },
  },
  // Personal sets none for new emails: the row says so and offers to add one.
  "composer-signature-none": {
    shows: /None set for maya\.ross@fastmail\.com/,
    apply: () => {
      const s = useStore.getState();
      s.compose("new");
      s.updateDraft({ fromAccountId: "personal", to: [], subject: "Saturday", body: "Are you free for lunch?", bodyHtml: textToHtml("Are you free for lunch?") });
    },
  },
  // A reply saved in webmail, whose text ended with Studio's signature: Moss took it out of the
  // text and made it the draft's signature, so it goes once and Remove takes it off.
  "composer-signature-from-webmail": {
    shows: /Your reply signature for/,
    apply: () => {
      const s = useStore.getState();
      s.open(shoot);
      s.compose("reply", shoot);
      s.updateDraft({ body: reply, bodyHtml: textToHtml(reply), signatureId: "studio" });
    },
  },
  // No signatures saved at all: the row leads to Settings.
  "composer-signature-none-saved": {
    shows: /Set up a signature/,
    apply: () => {
      useSignatures.setState({ signatures: [], recent: [] });
      const s = useStore.getState();
      s.open(shoot);
      s.compose("reply", shoot);
      s.updateDraft({ body: reply, bodyHtml: textToHtml(reply) });
    },
  },
  "quick-reply-signature-left-out": {
    shows: /Left out: quick reply in a back-and-forth/,
    apply: () => {
      signedOnce(shoot);
      const options = [{ label: "Yes, 10 works", draft: reply }];
      useStore.setState((s) => ({ threads: s.threads.map((t) => (t.id === shoot ? { ...t, replyOptions: options } : t)) }));
      useStore.getState().open(shoot);
      useQuickReply.setState({ threadId: shoot, status: undefined });
    },
  },
};
