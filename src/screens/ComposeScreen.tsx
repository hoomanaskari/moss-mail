import { lazy, Suspense, useMemo, useRef, useState, type DragEvent } from "react";
import { useShallow } from "zustand/react/shallow";
import { Check, CircleAlert, Clock3, Paperclip } from "lucide-react";
import type { Account, DraftFile, Message, Person } from "../data/types";
import { Button, Icon } from "../design-system/atoms";
import { AttachmentChip, Notice } from "../design-system/molecules";
import {
  BeforeYouSend,
  ComposerFields,
  ComposerToolbar,
  ForwardedMessage,
  MossMenu,
  MossNote,
  SignatureBlock,
  TopBar,
  type MossNoteState,
} from "../design-system/organisms";
import { AppFrame, ComposeLayout } from "../design-system/templates";
import { isExternal } from "../lib/people";
import { fileSize } from "../lib/attachments";
import { sendingAddress } from "../lib/addresses";
import { hasContent, useDrafts } from "../state/drafts";
import { DraftRecoveryNotice } from "./DraftRecoveryNotice";
import { openSendLater, useScheduled } from "../state/scheduled";
import { remindTime } from "../lib/time";
import { useMossWriting, type MossWritingState } from "../state/mossWriting";
import { chosenReason, offered, pickReason, useSignatures, type SignaturePick } from "../state/signatures";
import { accountFor, contactsFrom, filesTooBig, findThread, signaturePick, useStore, type ComposeMode, type Draft } from "../state/store";
import { useFileDrop } from "../state/useFileDrop";
import styles from "./ComposeScreen.module.css";

// TipTap is large; load it only when the composer opens.
const DraftEditor = lazy(() => import("../design-system/molecules/DraftEditor/DraftEditor").then((m) => ({ default: m.DraftEditor })));

const kinds: Record<ComposeMode, string> = { reply: "Reply", "reply-all": "Reply all", forward: "Forward", new: "New email" };

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "Goes to 4 people and 1 BCC. Ines is outside Northwind." */
function audience(to: Person[], cc: Person[], bcc: Person[], from: Account) {
  const visible = to.length + cc.length;
  if (!visible && !bcc.length) return "No one added yet.";
  let text = `Goes to ${plural(visible, "person", "people")}`;
  if (bcc.length) text += ` and ${bcc.length} BCC`;
  text += ".";
  const outsiders = [...to, ...cc, ...bcc].filter((p) => isExternal(p, from));
  if (outsiders.length) {
    const names = outsiders.map((p) => p.short ?? p.name).join(" and ");
    text += ` ${names} ${outsiders.length > 1 ? "are" : "is"} outside ${from.org}.`;
  }
  return text;
}

// The AI check will replace this keyword test.
const mentionsAttachment = (body: string) => /\battach(ed|ment|ing)?\b/i.test(body);

/** The chip's reason line and the picker's explanation for the draft's signature. */
function signatureReasons(d: Draft, pick: SignaturePick, address: string) {
  if (d.signatureChosen) return chosenReason;
  // One the draft came with (or none), e.g. from before you changed your signatures: nothing to
  // explain. Drafts from the server get theirs as they come in (sign_server_draft in the engine).
  if (pick.id !== d.signatureId) return { reason: "", why: "" };
  return pickReason(pick, address);
}

/** The strip above the text for where Moss stands in this draft, if anywhere. */
function noteState(moss: MossWritingState, draftId: string): MossNoteState | undefined {
  if (moss.draftId !== draftId) return undefined;
  if (moss.status === "wrote") return { kind: "wrote", lead: moss.pick!.lead, note: moss.pick!.note };
  return moss.status && { kind: moss.status };
}

/** A file's size: from its bytes, or as the forwarded email shows it (demo mail has no byte counts). */
function sizeOf(f: DraftFile, forwarded?: Message) {
  if (f.bytes || !f.forwarded) return fileSize(f.bytes);
  return forwarded?.attachments.find((a, i) => (a.index ?? i) === f.forwarded!.index)?.size ?? "";
}

/** Whether a drag carries files from outside, not text moved within the email. */
const carriesFiles = (e: DragEvent) => e.dataTransfer.types.includes("Files");

/** "Dana", for "Accept what Dana asks". */
const firstName = (p?: Person) => p && (p.short ?? p.name.split(" ")[0]);

export function ComposeScreen() {
  // Only what the composer shows, so a change elsewhere (an Ask answer, a sync) doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      draft: x.draft,
      thread: findThread(x, x.draft?.threadId),
      accounts: x.accounts,
      threads: x.threads,
      doneThreads: x.doneThreads,
      knownPeople: x.knownPeople,
      recipientPicks: x.recipientPicks,
      pickRecipient: x.pickRecipient,
      sending: x.sending,
      attachProblem: x.attachProblem,
      sendError: x.sendError,
      noSubjectAsked: x.noSubjectAsked,
      previews: x.previews,
      theme: x.theme,
      readingSize: x.readingSize,
      imagesShown: x.imagesShown,
      attach: x.attach,
      back: x.back,
      openDrafts: x.openDrafts,
      updateDraft: x.updateDraft,
      send: x.send,
      discardDraft: x.discardDraft,
      toggleFromPicker: x.toggleFromPicker,
      chooseFrom: x.chooseFrom,
      openSettings: x.openSettings,
      toggleSignaturePicker: x.toggleSignaturePicker,
      chooseSignature: x.chooseSignature,
      removeAttachment: x.removeAttachment,
      showImages: x.showImages,
      allowImages: x.allowImages,
    })),
  );
  // Everyone in your mail: built again when the mail changes, not on every typed character.
  const { threads, doneThreads, accounts, knownPeople } = s;
  const open = !!s.draft;
  const contacts = useMemo(
    () => (open ? contactsFrom([...threads, ...doneThreads], accounts, knownPeople) : []),
    [open, threads, doneThreads, accounts, knownPeople],
  );
  const moss = useMossWriting();
  const { signatures, recent } = useSignatures();
  const d = s.draft;
  const saved = useDrafts((x) => (d ? x.status[d.id] : undefined));
  const editing = useScheduled((x) => x.editing);
  const subjectRef = useRef<HTMLInputElement>(null);
  const [dropping, setDropping] = useState(false);
  // In the desktop app, files dropped from Finder come as paths, through the window.
  useFileDrop(!!d, s.attach, setDropping);
  if (!d) return null;
  const note = noteState(moss, d.id);
  const mine = moss.draftId === d.id;
  // A new email starts in Subject, so it isn't missed.
  const subjectFirst = d.mode === "new" && !d.subject.trim();

  const from = accountFor(d.fromAccountId);
  const sending = sendingAddress(from, d.fromAddress);
  const thread = s.thread;
  // Sending forwards the thread's latest message, so that's the one shown.
  const forwarded = d.mode === "forward" ? thread?.messages[thread.messages.length - 1] : undefined;
  const pick = signaturePick(d);
  // Moss's pick first, then the ones you used most recently.
  const offer = offered(pick.id, { signatures, recent });
  const files = d.attachments ?? [];
  const tooBig = filesTooBig(d);
  const drop = {
    onDragEnter: (e: DragEvent) => carriesFiles(e) && setDropping(true),
    onDragOver: (e: DragEvent) => {
      if (!carriesFiles(e)) return;
      e.preventDefault();
      setDropping(true);
    },
    onDragLeave: (e: DragEvent) => {
      // Moving onto something inside the composer isn't leaving it.
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false);
    },
    onDrop: (e: DragEvent) => {
      if (!carriesFiles(e)) return;
      e.preventDefault();
      setDropping(false);
      const dropped = [...e.dataTransfer.files];
      if (dropped.length) s.attach(dropped);
    },
  };

  return (
    <AppFrame
      topBar={
        <TopBar
          back={{ label: thread ? "Back to thread" : "Back to inbox", onBack: s.back }}
          right={
            <span className={styles.saved} role="status">
              {s.sending ? (
                "Sending…"
              ) : !hasContent(d) ? null : saved === "saved" ? (
                <>
                  <Icon icon={Check} size={14} tone="ink-3" />
                  Draft saved
                </>
              ) : (
                saved === "failed" && "Couldn’t save draft"
              )}
            </span>
          }
        />
      }
    >
      <div className={styles.dropZone} data-testid="compose-drop" {...drop}>
      {dropping && (
        <div className={styles.dropping} aria-live="polite">
          <Icon icon={Paperclip} size={22} tone="inherit" />
          Drop to attach
        </div>
      )}
      <ComposeLayout
        footer={
          <>
            <DraftRecoveryNotice draft={d} onReview={s.openDrafts} />
            {tooBig && (
              <Notice variant="warn" icon={Paperclip}>
                {tooBig}
              </Notice>
            )}
            {s.attachProblem && (
              <Notice variant="warn" icon={CircleAlert}>
                {s.attachProblem}
              </Notice>
            )}
            {s.sendError && (
              <Notice variant="warn" icon={CircleAlert}>
                {`Not sent: ${s.sendError}. Your email is still here, so you can try again.`}
              </Notice>
            )}
            {s.noSubjectAsked && (
              <Notice
                variant="warn"
                icon={CircleAlert}
                action={
                  <>
                    <Button
                      variant="quiet"
                      onClick={() => {
                        // Setting the subject, even to itself, puts the question away.
                        s.updateDraft({ subject: d.subject });
                        subjectRef.current?.focus();
                      }}
                    >
                      Add subject
                    </Button>
                    <Button variant="warn" onClick={s.send}>
                      Send anyway
                    </Button>
                  </>
                }
              >
                This email has no subject.
              </Notice>
            )}
            <BeforeYouSend
              missingAttachment={
                mentionsAttachment(d.body) && !files.length ? "You mention an attachment, but nothing is attached." : undefined
              }
              audience={audience(d.to, d.cc, d.bcc, from)}
              onAttach={() => s.attach()}
            />
            <ComposerToolbar
              remindIfNoReply={d.remindIfNoReply}
              sending={s.sending}
              onAttach={() => s.attach()}
              onSendLater={openSendLater}
              onToggleRemind={() => s.updateDraft({ remindIfNoReply: !d.remindIfNoReply })}
              onDiscard={s.discardDraft}
              onSend={s.send}
              onWriteWithMoss={moss.openMenu}
            />
          </>
        }
      >
        <div className={styles.head}>
          <span className={styles.kind}>{kinds[d.mode]}</span>
          {d.mode === "new" ? (
            <input
              ref={subjectRef}
              autoFocus={subjectFirst}
              className={styles.subjectInput}
              value={d.subject}
              placeholder="Subject"
              onChange={(e) => s.updateDraft({ subject: e.target.value })}
              aria-label="Subject"
            />
          ) : (
            <h1 className={styles.subject}>{d.subject}</h1>
          )}
        </div>
        {/* Under the subject, where you look first. */}
        {editing?.id === d.id && (
          <Notice variant="plain" icon={Clock3}>
            {`Was set to send ${remindTime(editing.sendAt, Date.now())}. It won’t go until you send it or pick a time again.`}
          </Notice>
        )}
        <ComposerFields
          from={from}
          sending={sending}
          accounts={s.accounts}
          fromOpen={!!d.fromOpen}
          onToggleFrom={s.toggleFromPicker}
          onChooseFrom={s.chooseFrom}
          onEditAddresses={() => s.openSettings("accounts")}
          fromReason={d.fromReason}
          to={d.to}
          cc={d.cc}
          bcc={d.bcc}
          showCc={d.showCc || d.cc.length > 0}
          showBcc={d.showBcc || d.bcc.length > 0}
          open={d.fieldsOpen}
          contacts={contacts}
          picks={s.recipientPicks}
          onPick={(typed, person) => s.pickRecipient(typed, person.email)}
          onChange={(field, people) => s.updateDraft({ [field]: people })}
          onShowCc={() => s.updateDraft({ showCc: true })}
          onShowBcc={() => s.updateDraft({ showBcc: true })}
          onOpen={() => s.updateDraft({ fieldsOpen: true })}
        />
        <div className={styles.body}>
          {note && (
            <MossNote
              state={note}
              onChangeIt={moss.openMenu}
              onUndo={moss.undo}
              onStop={moss.stop}
              onDownload={moss.download}
              onSettings={moss.settings}
              onRetry={moss.retry}
            />
          )}
          {mine && moss.status === "writing" ? (
            // The text streams in where it will land; your own stays as it is until it's done.
            <p className={styles.streaming} aria-live="polite">
              {moss.streamed}
              <span className={styles.streamCaret} aria-hidden />
            </p>
          ) : (
            <Suspense fallback={<div className={styles.editorPlaceholder} />}>
              <DraftEditor
                key={`${d.id}-${moss.version}`}
                initial={d.body}
                autoFocus={!subjectFirst && !(mine && moss.menuOpen) && (!d.fieldsOpen || d.to.length > 0)}
                onChange={(body, bodyHtml) => {
                  s.updateDraft({ body, bodyHtml });
                  moss.dismiss();
                }}
              />
            </Suspense>
          )}
          {mine && moss.menuOpen && (
            <div className={styles.menu}>
              <MossMenu name={firstName(d.to[0])} hasText={!!d.body.trim()} onPick={moss.write} onClose={moss.closeMenu} />
            </div>
          )}
          <SignatureBlock
            signature={signatures.find((x) => x.id === d.signatureId)}
            options={offer}
            {...signatureReasons(d, pick, sending.email)}
            open={!!d.signatureOpen}
            onToggle={s.toggleSignaturePicker}
            onChoose={s.chooseSignature}
            onEdit={() => s.openSettings("signatures")}
          />
        </div>
        {files.length > 0 && (
          <div className={styles.files} role="group" aria-label="Attached files">
            {files.map((f) => (
              <AttachmentChip
                key={f.id}
                name={f.name}
                size={sizeOf(f, forwarded)}
                contentType={f.contentType}
                preview={s.previews[f.forwarded ? `${f.forwarded.messageId}:${f.forwarded.index}` : `draft:${d.id}:${f.id}`] ?? undefined}
                onRemove={() => s.removeAttachment(f.id)}
              />
            ))}
          </div>
        )}
        {forwarded && (
          <ForwardedMessage
            message={forwarded}
            subject={thread!.fullSubject ?? thread!.subject}
            theme={s.theme}
            readingSize={s.readingSize}
            imagesShown={s.imagesShown.includes(forwarded.id)}
            onShowImages={() => s.showImages(forwarded.id)}
            onAllowImages={() => s.allowImages(forwarded.id)}
          />
        )}
      </ComposeLayout>
      </div>
    </AppFrame>
  );
}
