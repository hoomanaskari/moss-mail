import { splits } from "../data/demo";
import { domainOf, isPublicDomain } from "../lib/senders";
import { addressOf, replyFrom, sendingAddress, sendingAddresses } from "../lib/addresses";
import { fileSize } from "../lib/attachments";
import { initialsOf } from "../lib/people";
import { AtSign, Ban, MessageCircle, Plus } from "lucide-react";
import { Button, Kbd } from "../design-system/atoms";
import { Notice } from "../design-system/molecules";
import { memo, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Account, Message, Thread } from "../data/types";
import {
  ContactPanel,
  EarlierMessages,
  InstantReply,
  SignatureBlock,
  MessageView,
  ReplyBar,
  SenderBanner,
  ThreadActions,
  ThreadDraft,
  ThreadHeader,
  TopBar,
} from "../design-system/organisms";
import { AppFrame, ReadingLayout } from "../design-system/templates";
import { useAiSettings } from "../state/aiSettings";
import { useMossWriting } from "../state/mossWriting";
import { quickSignature, replyPick, useQuickReply } from "../state/quickReply";
import { chosenReason, offered, pickReason, useSignatures } from "../state/signatures";
import { useAskPanel } from "../state/askPanel";
import { threadDraft, useDrafts } from "../state/drafts";
import { isReplySubject } from "../lib/subject";
import { listTime } from "../lib/time";
import { escapeHtml } from "../lib/textHtml";
import { accountFor, openThread, useStore, visibleBlocked, visibleDone, visibleThreads, type Draft, type SendStep, type UndoStep } from "../state/store";
import styles from "./ThreadScreen.module.css";
import { SummariseButton, SummaryPanel } from "./SummaryPanel";
import { reasonsFor } from "../lib/why";

/** An earlier message opened in full; its recipients open on their own, apart from the newest message's. */
const EarlierMessage = memo(function EarlierMessage({ message, account, first }: { message: Message; account: Account; first: boolean }) {
  const [recipientsOpen, setRecipientsOpen] = useState(false);
  const theme = useStore((s) => s.theme);
  const readingSize = useStore((s) => s.readingSize);
  const imagesShown = useStore((s) => s.imagesShown.includes(message.id));
  const { showImages, allowImages, openFound } = useStore(useShallow((s) => ({ showImages: s.showImages, allowImages: s.allowImages, openFound: s.openFound })));
  return (
    <MessageView
      message={message}
      account={account}
      recipientsOpen={recipientsOpen}
      onToggleRecipients={() => setRecipientsOpen(!recipientsOpen)}
      theme={theme}
      readingSize={readingSize}
      imagesShown={imagesShown}
      onShowImages={() => showImages(message.id)}
      onAllowImages={() => allowImages(message.id)}
      onOpenForward={openFound}
      quoteIsNew={first}
    />
  );
});

/**
 * The reply bar, or the Quick replies in its place. On its own, so typing a reply or Moss writing
 * one redraws only this, not the email above it.
 */
function ThreadFooter({ thread, account, hasDraft, recipientsOpen }: { thread: Thread; account: Account; hasDraft: boolean; recipientsOpen: boolean }) {
  const s = useStore(
    useShallow((x) => ({
      replyChoice: x.replyChoice,
      replyDraft: x.replyDraft,
      setReplyChoice: x.setReplyChoice,
      setReplyDraft: x.setReplyDraft,
      editInstantReply: x.editInstantReply,
      sendInstantReply: x.sendInstantReply,
      openSettings: x.openSettings,
      compose: x.compose,
    })),
  );
  const quick = useQuickReply();
  const { signatures, recent } = useSignatures();
  const options = thread.replyOptions ?? [];
  const quickHere = quick.threadId === thread.id;
  // Ready replies, or Moss still at work on them. Replies that are gone leave the reply bar, and
  // so do ready ones when you have a draft here, unless you ask for Quick reply yourself.
  const showInstant = ((options.length > 0 && (!hasDraft || quickHere)) || (quickHere && !!quick.status)) && !recipientsOpen;
  if (!showInstant)
    return (
      <ReplyBar
        hint={options.length ? `${options.length} suggested replies ready` : undefined}
        replyTo={options.length ? undefined : (thread.from.short ?? thread.from.name)}
        onReply={() => s.compose("reply")}
        onReplyAll={() => s.compose("reply-all")}
        onForward={() => s.compose("forward")}
        onQuickReply={quick.start}
      />
    );
  const { signature, chosen, pick } = quickSignature(thread);
  const replyAddress = replyFrom(thread, account).address ?? account.address;
  const offer = offered(replyPick(thread).id, { signatures, recent });
  return (
    <InstantReply
      options={options}
      choice={s.replyChoice}
      draft={s.replyDraft ?? options[s.replyChoice]?.draft ?? ""}
      state={quickHere ? quick.status : undefined}
      focus={quickHere}
      from={{ letter: account.letter, color: account.color, address: replyAddress }}
      signature={
        <SignatureBlock
          minimal
          signature={signature}
          options={offer}
          {...(pick ? pickReason(pick, replyAddress) : chosenReason)}
          open={quick.signatureOpen}
          onToggle={quick.toggleSignature}
          onChoose={quick.chooseSignature}
          onEdit={() => s.openSettings("signatures")}
        />
      }
      onChoose={s.setReplyChoice}
      onDraftChange={s.setReplyDraft}
      onChangeIt={() => {
        s.editInstantReply(chosen);
        useMossWriting.getState().openMenu();
      }}
      onEdit={() => s.editInstantReply(chosen)}
      onSend={() => s.sendInstantReply(chosen)}
      onStop={quick.close}
      onDownload={() => {
        void useAiSettings.getState().download();
        quick.close();
      }}
      onSettings={() => s.openSettings("ai")}
      onRetry={quick.start}
      onDiscard={quick.discard}
    />
  );
}

export function ThreadScreen() {
  // You've answered: Moss's suggested replies step aside, as for a draft.
  const answering = useStore((x) => onTheirWay(x.undoSteps, x.openThreadId).length > 0);
  // Only what the email shows: a typed Quick reply, an Ask answer or a draft keystroke elsewhere
  // doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      thread: openThread(x),
      threads: x.threads,
      doneThreads: x.doneThreads,
      blockedThreads: x.blockedThreads,
      accountFilter: x.accountFilter,
      addressFilter: x.addressFilter,
      split: x.split,
      history: x.history,
      remindingId: x.remindingId,
      recipientsOpen: x.recipientsOpen,
      theme: x.theme,
      readingSize: x.readingSize,
      imagesShown: x.imagesShown,
      previews: x.previews,
      back: x.back,
      blockSender: x.blockSender,
      acceptSender: x.acceptSender,
      unblockSender: x.unblockSender,
      markDone: x.markDone,
      markNotDone: x.markNotDone,
      remindLater: x.remindLater,
      openRemind: x.openRemind,
      closeRemind: x.closeRemind,
      remindIfNoReply: x.remindIfNoReply,
      openMove: x.openMove,
      deleteThread: x.deleteThread,
      addAddress: x.addAddress,
      blockAddress: x.blockAddress,
      toggleRecipients: x.toggleRecipients,
      showImages: x.showImages,
      allowImages: x.allowImages,
      openAttachment: x.openAttachment,
      openFound: x.openFound,
      openDraft: x.openDraft,
      deleteDraft: x.deleteDraft,
      sendDraft: x.sendDraft,
    })),
  );
  const ask = useAskPanel(useShallow((a) => ({ open: a.open, toggle: a.toggle, reveal: a.reveal })));
  const drafts = useDrafts((d) => d.drafts);
  const thread = s.thread;
  const { threads, doneThreads, blockedThreads, accountFilter, addressFilter, split } = s;
  // Where it sits among its peers: worked out again only when the lists change.
  const peers = useMemo(() => {
    if (!thread) return [];
    if (blockedThreads.some((t) => t.id === thread.id)) return visibleBlocked({ blockedThreads, accountFilter });
    if (thread.doneAt !== undefined) return visibleDone({ doneThreads, accountFilter });
    return visibleThreads({ threads, accountFilter, split, addressFilter }).filter((t) => t.section === thread.section);
  }, [thread, threads, doneThreads, blockedThreads, accountFilter, addressFilter, split]);
  const latest = thread?.messages.at(-1);
  const shownMessage = useMemo(() => latest && withPreviews(latest, s.previews), [latest, s.previews]);
  // Your replies on their way read as the end of the conversation, as they will once gone.
  const sending = useStore(useShallow((x) => onTheirWay(x.undoSteps, thread?.id).map((step) => step.draft)));
  const copies = useMemo(() => sending.map(sendingCopy), [sending]);
  if (!thread) return null;
  const done = thread.doneAt !== undefined;
  const blocked = blockedThreads.some((t) => t.id === thread.id);
  const domain = domainOf(thread.from.email);
  const blockableDomain = domain && !isPublicDomain(domain) ? domain : undefined;

  const account = accountFor(thread.accountId);
  const message = thread.messages[thread.messages.length - 1];
  // Which of your addresses it came to, and whether that is one you send from or one you blocked.
  const to = thread.toAddress ?? account.address.toLowerCase();
  const toOther = to !== account.address.toLowerCase();
  const blockedAddress = toOther && addressOf(account, to)?.blocked ? to : undefined;
  const unknownAddress = toOther && !blockedAddress && !sendingAddresses(account).some((a) => a.email === to);
  const position = peers.findIndex((t) => t.id === thread.id) + 1;
  // Back goes to where you opened it: the Done list, Blocked, Search, or the split it is in.
  const from = s.history.at(-1);
  const backLabel =
    from === "done"
      ? "Done"
      : from === "blocked"
        ? "Blocked"
        : from === "search"
          ? "Search"
          : (splits.find((x) => x.id === thread.split)?.label ?? "Inbox");
  const block = () => s.blockSender(false, thread.id);
  const blockDomain = () => s.blockSender(true, thread.id);
  const accept = () => s.acceptSender(false, thread.id);
  const acceptDomain = () => s.acceptSender(true, thread.id);
  const unblock = () => s.unblockSender(thread.id);
  // Your unsent reply here, wherever you wrote it (the composer, Ask Moss, another device).
  const draft = threadDraft(drafts, thread.id);

  return (
    <AppFrame
      topBar={
        <TopBar
          back={{ label: backLabel, onBack: s.back }}
          right={
            <>
              <SummariseButton />
              <Button variant="toolbar" icon={MessageCircle} shortcut="⌘J" active={ask.open} aria-pressed={ask.open} onClick={ask.toggle}>
                Ask
              </Button>
              <span className={styles.divider} />
              <ThreadActions
                done={done}
                onDone={() => (done ? s.markNotDone(thread.id) : s.markDone(thread.id))}
                onRemind={(at) => s.remindLater(thread.id, at)}
                remindOpen={s.remindingId === thread.id}
                onRemindOpenChange={(open) => (open ? s.openRemind(thread.id) : s.closeRemind())}
                onRemindNoReply={() => s.remindIfNoReply(thread.id)}
                onMove={blocked ? undefined : () => s.openMove(thread.id)}
                // Blocked mail stays in the Blocked folder; unblock it first.
                onDelete={blocked ? undefined : () => s.deleteThread(thread.id)}
              />
              <span className={styles.divider} />
              <div className={styles.position}>
                <span className={styles.positionLabel}>
                  {position} of {peers.length}
                </span>
                <Kbd>↑</Kbd>
                <Kbd>↓</Kbd>
              </div>
            </>
          }
        />
      }
    >
      <ReadingLayout
        compact={s.recipientsOpen}
        footer={<ThreadFooter thread={thread} account={account} hasDraft={!!draft || answering} recipientsOpen={s.recipientsOpen} />}
      >
        <ThreadHeader
          account={{ letter: account.letter, color: account.color, name: account.name, address: toOther ? to : account.address }}
          labels={thread.labels}
          subject={thread.fullSubject ?? thread.subject}
          summary={<SummaryPanel threadId={thread.id} from={thread.from} emails={thread.messages.length} />}
          ask={thread.ask}
          askDue={thread.askDue}
          moved={thread.movedByYou ? splits.find((x) => x.id === thread.split)?.label : undefined}
          reasons={reasonsFor(thread, Date.now())}
        />
        {unknownAddress && !blocked && (
          <Notice
            variant="inset"
            icon={AtSign}
            action={
              <>
                <Button variant="secondary" icon={Plus} onClick={() => s.addAddress(account.id, to)}>
                  Send from it
                </Button>
                <Button variant="secondary" icon={Ban} onClick={() => s.blockAddress(account.id, to, true)}>
                  Block it
                </Button>
              </>
            }
          >
            {`It came to ${to}, which you don’t send from yet.`}
          </Notice>
        )}
        {(thread.sender === "new" || blocked) && (
          <SenderBanner
            status={blocked ? "blocked" : "new"}
            blockedAddress={blocked ? blockedAddress : undefined}
            email={thread.from.email}
            domain={blockableDomain}
            account={account.name}
            onBlock={block}
            onBlockDomain={blockDomain}
            onAccept={accept}
            onAcceptDomain={acceptDomain}
            onUnblock={unblock}
          />
        )}
        <EarlierMessages
          key={thread.id}
          messages={[...thread.messages, ...copies].slice(0, -1)}
          youAddress={account.address}
          renderMessage={(m) => <EarlierMessage message={m} account={account} first={m === thread.messages[0]} />}
          reveal={ask.reveal}
        />
        {copies.length ? (
          <div role="group" aria-label="Your reply, sending" className={styles.latest}>
            <MessageView
              message={copies.at(-1)!}
              account={account}
              recipientsOpen={s.recipientsOpen}
              onToggleRecipients={s.toggleRecipients}
              theme={s.theme}
              readingSize={s.readingSize}
              senderCard={
                <ContactPanel person={thread.from} recent={thread.recent} insight={thread.insight} between={thread.between} />
              }
            />
          </div>
        ) : message ? (
          // Ask Moss's source chips scroll to a message by this mark.
          <div className={styles.latest} data-message={message.id}>
            <MessageView
              message={shownMessage!}
              account={account}
              recipientsOpen={s.recipientsOpen}
              onToggleRecipients={s.toggleRecipients}
              theme={s.theme}
              readingSize={s.readingSize}
              imagesShown={s.imagesShown.includes(message.id)}
              onShowImages={() => s.showImages(message.id)}
              onAllowImages={() => s.allowImages(message.id)}
              onOpenAttachment={(index) => s.openAttachment(message.id, index)}
              onOpenForward={s.openFound}
              // A thread's first email quotes what you can't read above it (a forward, say), so its quote starts open.
              // Unless it answers an email that isn't here: that quote is old news.
              quoteIsNew={thread.messages.length === 1 && !isReplySubject(thread.fullSubject ?? thread.subject)}
              senderCard={
                <ContactPanel person={thread.from} recent={thread.recent} insight={thread.insight} between={thread.between} />
              }
            />
          </div>
        ) : (
          <p className={styles.preview}>{thread.summary}</p>
        )}
        {draft && (
          <ThreadDraft
            line={`To ${draft.to.map((p) => p.name || p.email).join(", ")} · from ${draft.fromAddress ?? accountFor(draft.fromAccountId).address}`}
            body={draft.body}
            details={{ ...draft, from: draft.fromAddress ?? accountFor(draft.fromAccountId).address }}
            saved={savedLabel(draft.updatedAt, Date.now())}
            onEdit={() => s.openDraft(draft.id)}
            onDelete={() => s.deleteDraft(draft.id)}
            onSend={() => s.sendDraft(draft)}
          />
        )}
      </ReadingLayout>
    </AppFrame>
  );
}

/** Replies to email `threadId` held for Undo, not yet gone or stopped. A forward starts a conversation of its own. */
const onTheirWay = (steps: UndoStep[], threadId: string | undefined) =>
  steps.filter((x): x is SendStep => x.kind === "send" && !x.failed && x.draft.mode !== "forward" && x.draft.threadId === threadId);

/** A held email as the thread shows it: from the address it goes from, its text, signature and files. */
function sendingCopy(d: Draft): Message {
  const from = sendingAddress(accountFor(d.fromAccountId), d.fromAddress);
  const signature = useSignatures.getState().signatures.find((x) => x.id === d.signatureId)?.lines.filter((l) => l.trim()) ?? [];
  // Like the engine's `compose::signature`: "-- " opens it in text, Gmail's mark in HTML.
  const [signText, signHtml] = signature.length
    ? [`\n\n-- \n${signature.join("\n")}`, `<br><div class="gmail_signature">${signature.map(escapeHtml).join("<br>")}</div>`]
    : ["", ""];
  const text = d.body + signText;
  const html = d.bodyHtml && d.bodyHtml + signHtml;
  return {
    id: `sending-${d.id}`,
    from: { name: from.name, email: from.email, initials: initialsOf(from.name) },
    to: d.to,
    cc: d.cc,
    bcc: d.bcc,
    time: "Sending…",
    sentFull: "Sending…",
    paragraphs: [],
    text,
    ...(html && { html }),
    attachments: (d.attachments ?? []).map((f) => ({ name: f.name, size: fileSize(f.bytes), contentType: f.contentType })),
  };
}

/** When a draft was last saved: "Saved just now", "Saved at 9:41 AM", "Saved on Oct 28". */
function savedLabel(at: number, now: number): string {
  if (now - at < 60_000) return "Saved just now";
  const when = listTime(at, now);
  return /\d:\d/.test(when) ? `Saved at ${when}` : `Saved on ${when}`;
}

/** The message with the thumbnails the mail engine has made for its files so far. */
function withPreviews(message: Message, previews: Record<string, string | null>): Message {
  return {
    ...message,
    attachments: message.attachments.map((a) => ({ ...a, preview: a.preview ?? previews[`${message.id}:${a.index}`] ?? undefined })),
  };
}
