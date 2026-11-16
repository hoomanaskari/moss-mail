/**
 * The Ask Moss panel beside any mail screen (see `AppFrame`'s side slot). Joins the panel's
 * state (`state/askPanel.ts`) to the `AskPanel` organism, and words what it shows. The email
 * open is its context; there is nothing to switch.
 */
import { AskPanel, type AskPanelActions } from "../design-system/organisms";
import { deviceName } from "../lib/platform";
import { remindTime } from "../lib/time";
import { useAiSettings } from "../state/aiSettings";
import { editDraft, emailThread, openEmail, sendDraft, turnsOf, useAskPanel } from "../state/askPanel";
import type { SavedDraft } from "../state/drafts";
import { useSearch } from "../state/search";
import { accountFor, findThread, useStore } from "../state/store";

/** A chip's day: "today" for a message with only a time, else its date ("Oct 28"). */
const dayOf = (time: string) => (/\d:\d\d/.test(time) ? "today" : time);

export function AskSide() {
  const panel = useAskPanel();
  // The open email is what a question can be about.
  useStore((s) => s.view);
  useStore((s) => s.openThreadId);
  useStore((s) => s.cursorId);
  useStore((s) => s.threads);
  const status = useAiSettings((s) => s.status);
  if (!panel.open) return null;

  const thread = emailThread();
  const turns = turnsOf(panel);
  const count = panel.mailCount?.toLocaleString("en");
  const n = thread?.messages.length ?? 0;
  const writer = status?.writers.find((w) => w.id === status.writer);
  const noModel = writer
    ? `Ask needs the writing model. ${writer.name} runs on this ${deviceName}, ${writer.size.replace(/^About/, "about")}.`
    : `Ask needs the writing model. It runs on this ${deviceName}.`;

  const actions: AskPanelActions = {
    type: panel.type,
    ask: (question) => void panel.ask(question),
    close: panel.hide,
    stop: panel.stop,
    newChat: panel.newChat,
    draftReply: () => void panel.draftReply(),
    move: panel.move,
    openEmail: (id) => void openEmail(id),
    showMessage: panel.showMessage,
    openFile: (file) => useStore.getState().openAttachment(file.messageId, file.index),
    editDraft,
    sendDraft: (d) => sendDraft(d, sendAt(d)),
    dismissDraft: (draft) => panel.dismiss(draft.id),
    download: () => void useAiSettings.getState().download(),
    settings: () => useStore.getState().openSettings("ai"),
    search: (question) => {
      panel.hide();
      useStore.getState().openSearch();
      useSearch.getState().setQuery(question);
    },
  };

  // When the turn that wrote a draft says it should go.
  function sendAt(d: SavedDraft) {
    return turns.find((t) => t.drafts.some((x) => x.id === d.id))?.sendAt[d.id];
  }
  const replyingTo = (d: SavedDraft) => (d.threadId === thread?.id ? undefined : findThread(useStore.getState(), d.threadId));
  const draftLine = (d: SavedDraft) => {
    const to = d.to.map((p) => p.name).join(", ");
    return `${thread && d.threadId === thread.id ? "Reply to" : "To"} ${to} · from ${d.fromAddress ?? accountFor(d.fromAccountId).address}`;
  };
  const chip = (id: string) => {
    const m = thread?.messages.find((x) => x.id === id);
    return m && `${m.from.short ?? m.from.name.split(" ")[0]} · ${dayOf(m.time)}`;
  };

  return (
    <AskPanel
      emailOpen={!!thread}
      turns={turns}
      input={panel.input}
      pick={panel.pick}
      intro={`Ask about this conversation or any other email, or tell Moss what to do with your mail. Moss reads ${n === 1 ? "this email" : `all ${n} emails here`} first.`}
      thinking={thread ? `Reading ${n === 1 ? "the email" : `${n} emails`}…` : count ? `Looking through ${count} emails…` : "Looking through your mail…"}
      noModel={noModel}
      dismissed={panel.dismissed}
      accountOf={accountFor}
      chip={chip}
      draftLine={draftLine}
      replyingTo={replyingTo}
      sendsAt={(d) => {
        const at = sendAt(d);
        return at === undefined ? undefined : remindTime(at, Date.now());
      }}
      actions={actions}
    />
  );
}
