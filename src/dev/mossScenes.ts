/** Dev scenes for AI replies: Quick reply in the reading view, and Write with Moss in the composer (design.pen "AI ·" frames). */
import { useMossWriting } from "../state/mossWriting";
import { useQuickReply, type QuickReplyState } from "../state/quickReply";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

const shoot = "fridays-shoot";

const options = [
  { label: "Yes, 10 works", draft: "Hi Priya,\n\nYes, 10 AM works for me. See you on Friday." },
  { label: "Keep it at 9", draft: "Hi Priya,\n\nI’d rather keep 9 AM if we can. Could the stylist start a little earlier instead?" },
  { label: "Ask about the stylist", draft: "Hi Priya,\n\nThat could work. How much time does the stylist need before we start?" },
];

/** Priya's email open, with Quick reply in `status` (or its replies in), and `shown` of them so far. */
function quickReply(status?: QuickReplyState["status"], shown = status ? [] : options) {
  const s = useStore.getState();
  if (shown.length) useStore.setState({ threads: s.threads.map((t) => (t.id === shoot ? { ...t, replyOptions: shown } : t)) });
  useStore.getState().open(shoot);
  useQuickReply.setState({ threadId: shoot, status });
}

/** A reply to Priya in the composer, with `body` written so far. */
function composing(body: string) {
  const s = useStore.getState();
  s.open(shoot);
  s.compose("reply");
  s.updateDraft({ body, bodyHtml: "", fieldsOpen: false });
  return useStore.getState().draft!.id;
}

const note = "yes to 10, and ask if the stylist can come at 9:30";
const written =
  "Hi Priya,\n\nYes, 10 AM works for me. Thanks for checking with the client.\n\nCould the stylist come in at 9:30, so we’re ready to shoot at 10?";

export const mossScenes: Record<string, Scene> = {
  "quick-reply": { shows: /Moss wrote this\. Nothing sends until you press Send\./, apply: () => quickReply() },
  "quick-reply-writing": { shows: /Moss is writing…/, apply: () => quickReply("writing") },
  // The first reply streaming in; the others follow it.
  "quick-reply-streaming": { shows: /Yes, 10 works/, apply: () => quickReply("writing", [{ ...options[0], draft: "Hi Priya,\n\nYes, 10 AM works for" }]) },
  "quick-reply-no-model": { shows: /Writing needs the writing model/, apply: () => quickReply("missing") },
  "quick-reply-failed": { shows: /Couldn’t write this one/, apply: () => quickReply("failed") },
  "compose-moss-menu": {
    shows: /Moss picks what to say/,
    apply: () => {
      const draftId = composing("Hi Priya,");
      useMossWriting.setState({ draftId, menuOpen: true, status: undefined });
    },
  },
  "compose-moss-wrote": {
    shows: /Moss wrote this from your note/,
    apply: () => {
      const draftId = composing(written);
      const pick = { job: { kind: "instructions" as const, instructions: note }, lead: "Moss wrote this from your note", note };
      useMossWriting.setState({ draftId, menuOpen: false, status: "wrote", pick, before: "Hi Priya," });
    },
  },
  "compose-moss-writing": {
    shows: /Your words stay as they are until it’s done\./,
    apply: () => {
      const draftId = composing("Hi Priya,");
      useMossWriting.setState({ draftId, menuOpen: false, status: "writing", streamed: "Hi Priya, thanks for checking. 10 AM works for" });
    },
  },
  "compose-moss-no-model": {
    shows: /Writing needs the writing model/,
    apply: () => useMossWriting.setState({ draftId: composing("Hi Priya,"), menuOpen: false, status: "missing" }),
  },
  "compose-moss-failed": {
    shows: /The writing model stopped\. Your draft is unchanged\./,
    apply: () => useMossWriting.setState({ draftId: composing("Hi Priya,"), menuOpen: false, status: "failed" }),
  },
};
