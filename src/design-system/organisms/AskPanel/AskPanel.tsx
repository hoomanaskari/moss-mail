import { plainLabel } from "../../../lib/displayLabel";
import { useEffect, useRef, type KeyboardEvent } from "react";
import { Check, Leaf, Lock, Mail, Paperclip, PenLine, Reply } from "lucide-react";
import type { Account, Thread } from "../../../data/types";
import { cx } from "../../../lib/cx";
import { deviceName } from "../../../lib/platform";
import type { AskFile, ChatTurn } from "../../../services/ask";
import type { SavedDraft } from "../../../state/drafts";
import { AccountBadge, Button, Icon, Kbd, Markdown } from "../../atoms";
import styles from "./AskPanel.module.css";
import { DraftReview } from "../../molecules";

export interface AskPanelActions {
  type(text: string): void;
  ask(question?: string): void;
  close(): void;
  stop(): void;
  newChat(): void;
  draftReply(): void;
  /** ↑ ↓ among `count` things to pick. */
  move(step: 1 | -1, count: number): void;
  openEmail(threadId: string): void;
  /** A source chip: scroll to that message. */
  showMessage(messageId: string): void;
  /** A file chip: open the file the answer came from. */
  openFile(file: AskFile): void;
  editDraft(draft: SavedDraft): void;
  sendDraft(draft: SavedDraft): void;
  dismissDraft(draft: SavedDraft): void;
  download(): void;
  settings(): void;
  search(question: string): void;
}

export interface AskPanelProps {
  /** An email is open: Moss answers about it when a question is, and offers questions to start. */
  emailOpen: boolean;
  turns: ChatTurn[];
  input: string;
  /** What ↑ ↓ picked: an email card of the last answer. -1: none. */
  pick: number;
  /** With an email open, before anything is asked: what Moss can do with it. */
  intro: string;
  /** While it thinks, e.g. "Looking through 2,431 emails…". */
  thinking: string;
  /** When the writing model isn't downloaded, e.g. "Ask needs the writing model. …". */
  noModel: string;
  /** Draft cards closed with Not now, or sent. */
  dismissed: string[];
  accountOf(id: string): Pick<Account, "letter" | "color" | "address">;
  /** A cited message's chip, e.g. "Dana · Oct 28". Messages it can't name get no chip. */
  chip(messageId: string): string | undefined;
  /** A draft's head line, e.g. "Reply to Dana Whitfield" or "To Priya Nair · from hello@…". */
  draftLine(draft: SavedDraft): string;
  /** The email a draft replies to, shown in its card, when it isn't the one open. */
  replyingTo(draft: SavedDraft): Thread | undefined;
  /** When a draft you asked to send later goes, e.g. "Sat 8 AM". Its Send sends it then. */
  sendsAt(draft: SavedDraft): string | undefined;
  actions: AskPanelActions;
}

const busy = (turn?: ChatTurn) => turn?.state === "thinking" || turn?.state === "writing";
const firstName = (t: Thread) => t.from.short ?? t.from.name.split(" ")[0];

function Who() {
  return (
    <div className={styles.who}>
      <Icon icon={Leaf} size={13} tone="ai" />
      Moss
    </div>
  );
}

function EmailCard({ thread, account, picked, onOpen }: { thread: Thread; account: Pick<Account, "letter" | "color">; picked: boolean; onOpen(): void }) {
  // The newest email's file, as the list shows it.
  const file = [...thread.messages].reverse().find((m) => m.attachments.length)?.attachments[0];
  return (
    <button type="button" className={cx(styles.card, picked && styles.picked)} aria-current={picked || undefined} onClick={onOpen}>
      <span className={styles.cardTop}>
        <AccountBadge letter={account.letter} color={account.color} size={18} />
        <span className={styles.cardFrom}>{thread.from.name}</span>
        <span className={styles.cardWhen}>{thread.time}</span>
        {picked && <Kbd>↵</Kbd>}
      </span>
      <span className={styles.cardSubject}>{thread.fullSubject ?? thread.subject}</span>
      {file && (
        <span className={styles.cardFile}>
          <Icon icon={Paperclip} size={13} tone="ink-3" />
          {file.name}
        </span>
      )}
    </button>
  );
}

interface DraftCardProps {
  shortcuts: boolean;
  draft: SavedDraft;
  line: string;
  from: string;
  /** Send later, at this time. */
  at?: string;
  replyingTo?: Thread;
  account?: Pick<Account, "letter" | "color">;
  actions: AskPanelActions;
}

function DraftCard({ draft, line, from, at, replyingTo, account, actions, shortcuts }: DraftCardProps) {
  return (
    <div className={styles.draft} role="group" aria-label="Draft reply">
      {replyingTo && account && (
        <div className={styles.replyingTo}>
          <span className={styles.cardTop}>
            <AccountBadge letter={account.letter} color={account.color} size={18} />
            <span className={styles.replyFrom}>{replyingTo.from.name}</span>
            <span className={styles.replyWhen}>{replyingTo.time}</span>
            <button type="button" className={styles.open} onClick={() => actions.openEmail(replyingTo.id)}>
              <Kbd>⌘O</Kbd>
              Open
            </button>
          </span>
          <span className={styles.snippet}>
            {replyingTo.subject}
            {replyingTo.summary && `  ·  ${replyingTo.summary}`}
          </span>
        </div>
      )}
      <div className={styles.draftHead}>
        <Icon icon={Reply} size={14} tone="ink-3" />
        <span className={styles.draftLine}><bdi dir="ltr">{plainLabel(line)}</bdi></span>
        {/* Whatever the answer says: a small model may write "sent". */}
        <span className={styles.notSent}>Not sent yet</span>
      </div>
      <DraftReview {...draft} from={from} />
      <p className={styles.draftText}>{draft.body}</p>
      <div className={styles.draftActions}>
        <Button variant="outline" className={styles.small} shortcut={shortcuts ? "⌘E" : undefined} onClick={() => actions.editDraft(draft)}>
          Edit
        </Button>
        {replyingTo && (
          <Button variant="quiet" onClick={() => actions.dismissDraft(draft)}>
            Not now
          </Button>
        )}
        <span className={styles.spacer} />
        <Button variant="primary" className={cx(styles.small, styles.send)} shortcut={shortcuts ? "⌘↵" : undefined} onClick={() => actions.sendDraft(draft)}>
          {at ? `Send ${at}` : "Send"}
        </Button>
      </div>
    </div>
  );
}

interface TurnProps extends Omit<AskPanelProps, "turns" | "input" | "intro"> {
  turn: ChatTurn;
  last: boolean;
}

/** Moss's side of a turn: thinking, the answer and what it used, or why there is none. */
function MossTurn({ turn, last, emailOpen, pick, thinking, noModel, dismissed, accountOf, chip, draftLine, replyingTo, sendsAt, actions }: TurnProps) {
  if (turn.state === "thinking") {
    return (
      <div className={styles.moss} aria-busy>
        <Who />
        <p className={styles.text}>{turn.activity ?? thinking}</p>
        <div className={styles.skeleton} aria-hidden>
          <span />
          <span />
        </div>
        <button type="button" className={styles.keyHint} onClick={actions.stop}>
          <Kbd>Esc</Kbd>
          Stop
        </button>
      </div>
    );
  }
  if (turn.state === "failed") {
    const missing = turn.error?.kind === "notReady";
    return (
      <div className={styles.moss} role="alert">
        <Who />
        <p className={styles.text}>{missing ? noModel : "Something went wrong before I could answer."}</p>
        <div className={styles.row}>
          {missing ? (
            <>
              <Button variant="outlineSm" className={styles.aiButton} shortcut={last ? "↵" : undefined} onClick={actions.download}>
                Download
              </Button>
              <Button variant="outlineSm" shortcut="⌘," onClick={actions.settings}>
                AI settings
              </Button>
            </>
          ) : (
            <Button variant="outlineSm" className={styles.aiButton} shortcut={last ? "↵" : undefined} onClick={() => actions.ask(turn.question)}>
              Try again
            </Button>
          )}
        </div>
      </div>
    );
  }
  const drafts = turn.drafts.filter((d) => !dismissed.includes(d.id));
  const chips = turn.messages.flatMap((id) => (chip(id) ? [{ id, label: chip(id)! }] : []));
  const cards = turn.sources;
  const nothing = !emailOpen && found(turn);
  return (
    <div className={styles.moss}>
      <Who />
      {turn.text && <Markdown text={turn.text} className={styles.answer} />}
      {turn.state === "stopped" && <p className={styles.note}>Stopped.</p>}
      {turn.did.length > 0 && (
        <ul className={styles.did} aria-label="What Moss did">
          {turn.did.map((d, i) => (
            <li key={i} className={styles.didItem}>
              <Icon icon={Check} size={13} tone="ai" />
              <span className={styles.didText}>{d}</span>
            </li>
          ))}
        </ul>
      )}
      {(chips.length > 0 || turn.files.length > 0) && (
        <div className={styles.chips}>
          {chips.map((c) => (
            <button key={c.id} type="button" className={styles.chip} onClick={() => actions.showMessage(c.id)}>
              <Icon icon={Mail} size={12} tone="ink-3" />
              {c.label}
            </button>
          ))}
          {turn.files.map((f) => (
            <button
              key={`${f.messageId}-${f.index}`}
              type="button"
              className={styles.chip}
              title={f.name}
              aria-label={`Open ${f.name}`}
              onClick={() => actions.openFile(f)}
            >
              <Icon icon={Paperclip} size={12} tone="ink-3" />
              <span className={styles.chipName}>{f.name}</span>
            </button>
          ))}
        </div>
      )}
      {cards.length > 0 && !drafts.length && (
        <>
          {cards.map((t, i) => (
            <EmailCard key={t.id} thread={t} account={accountOf(t.accountId)} picked={last && i === pick} onOpen={() => actions.openEmail(t.id)} />
          ))}
          {last && <p className={styles.hint}>↑ ↓ to pick an email, ↵ to open it. Or just type to ask more.</p>}
        </>
      )}
      {drafts.map((d) => {
        const to = replyingTo(d);
        return (
          <div key={d.id} className={styles.moss}>
            <DraftCard shortcuts={last && drafts.length === 1} draft={d} from={d.fromAddress ?? accountOf(d.fromAccountId).address} line={draftLine(d)} at={sendsAt(d)} replyingTo={to} account={to && accountOf(to.accountId)} actions={actions} />
            {to && <p className={styles.note}>Sending doesn’t mark {firstName(to)}’s email Done. You decide that.</p>}
          </div>
        );
      })}
      {nothing && last && (
        <div className={styles.row}>
          <Button variant="outlineSm" shortcut="/" onClick={() => actions.search(turn.question)}>
            Search instead
          </Button>
        </div>
      )}
    </div>
  );
}

/** Done, and nothing to show for it: no emails, messages, files, drafts or changes. */
const found = (turn: ChatTurn) =>
  turn.state === "done" && !turn.sources.length && !turn.messages.length && !turn.files.length && !turn.drafts.length && !turn.did.length;

/** The Ask Moss panel: one right-hand panel, one chat about the open email or any other, and the box. */
export function AskPanel(props: AskPanelProps) {
  const { emailOpen, turns, input, pick, intro, actions } = props;
  const box = useRef<HTMLInputElement>(null);
  const messages = useRef<HTMLDivElement>(null);
  const last = turns.at(-1);
  const started = turns.length > 0;
  const sendable = last?.drafts.filter((d) => !props.dismissed.includes(d.id)) ?? [];
  // The email cards of the last answer: ↑ ↓ picks one, ↵ opens it.
  const picks: { run(): void }[] = last && !sendable.length ? last.sources.map((t) => ({ run: () => actions.openEmail(t.id) })) : [];
  const draft = sendable.length === 1 ? sendable[0] : undefined;
  const replyTo = draft && props.replyingTo(draft);
  const nothing = !emailOpen && !!last && found(last);

  useEffect(() => {
    box.current?.focus();
  }, []);
  // New text keeps the latest turn in view.
  useEffect(() => {
    const el = messages.current!;
    el.scrollTop = el.scrollHeight;
  }, [turns]);

  const onKeyDown = (e: KeyboardEvent) => {
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.toLowerCase();
    if (e.key === "Escape") {
      if (busy(last)) actions.stop();
      else actions.close();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      actions.move(e.key === "ArrowDown" ? 1 : -1, picks.length);
    } else if (mod && key === "r" && started) {
      actions.draftReply();
    } else if (mod && key === "e" && draft) {
      actions.editDraft(draft);
    } else if (mod && e.key === "Enter" && draft) {
      actions.sendDraft(draft);
    } else if (mod && key === "o" && replyTo) {
      actions.openEmail(replyTo.id);
    } else if (e.key === "Enter" && !mod && !e.shiftKey) {
      if (input.trim()) actions.ask();
      else if (picks[pick]) picks[pick].run();
      else if (last?.state === "failed") {
        if (last.error?.kind === "notReady") actions.download();
        else actions.ask(last.question);
      } else return;
    } else if (e.key === "/" && !input && nothing) {
      actions.search(last!.question);
    } else {
      return;
    }
    e.preventDefault();
    // The screen behind listens for keys too: ↵ would open an email there, Esc go back.
    e.stopPropagation();
  };

  const placeholder = started ? "Ask a follow-up…" : emailOpen ? "Ask about this email, or any other…" : "Ask about any email…";

  return (
    <aside className={cx(styles.panel, !emailOpen && styles.all)} aria-label="Ask Moss" onKeyDown={onKeyDown}>
      <header className={styles.head}>
        <div className={styles.titleRow}>
          <Icon icon={Leaf} size={16} tone="ai" />
          <h2 className={styles.title}>Ask Moss</h2>
          <button type="button" className={styles.keyHint} onClick={actions.close}>
            <Kbd>Esc</Kbd>
            Close
          </button>
        </div>
      </header>

      <div className={styles.messages} ref={messages} aria-live="polite">
        {!started && emailOpen && <p className={styles.intro}>{intro}</p>}
        {!started && !emailOpen && (
          <p className={styles.intro}>
            Ask about any email, or tell Moss what to do with your mail: who said what, where a file is, mark the newsletters Done, remind you about a thread.
          </p>
        )}
        {turns.map((turn, i) => (
          <div key={i} className={styles.turn}>
            <div className={styles.you}>
              <p className={styles.bubble}>{turn.question}</p>
            </div>
            <MossTurn {...props} turn={turn} last={i === turns.length - 1} />
          </div>
        ))}
      </div>

      <footer className={styles.inputArea}>
        {started && !busy(last) && (
          <div className={styles.chatActions}>
            <button type="button" className={styles.draftReply} onClick={actions.draftReply}>
              <Icon icon={PenLine} size={13} tone="ai" />
              Draft a reply from this chat
              <Kbd>⌘R</Kbd>
            </button>
            <button type="button" className={styles.newChat} onClick={actions.newChat}>
              New chat
            </button>
          </div>
        )}
        <div className={styles.input}>
          <input
            ref={box}
            className={styles.field}
            value={input}
            placeholder={placeholder}
            aria-label={placeholder}
            onChange={(e) => actions.type(e.target.value)}
          />
          <Kbd>↵</Kbd>
        </div>
        <p className={styles.foot}>
          <Icon icon={Lock} size={12} tone="ink-3" />
          Runs on this {deviceName}. Answers come only from your mail.
        </p>
      </footer>
    </aside>
  );
}
