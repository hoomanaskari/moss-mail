import { useEffect, useRef, type KeyboardEvent, type ReactNode } from "react";
import { Leaf, PenLine, X } from "lucide-react";
import type { AccountColor, ReplyOption } from "../../../data/types";
import { Button, Icon, Kbd } from "../../atoms";
import { FromPicker, ShortcutHint } from "../../molecules";
import { MossNote } from "../MossNote/MossNote";
import styles from "./InstantReply.module.css";

export interface InstantReplyProps {
  options: ReplyOption[];
  choice: number;
  /** The reply as it stands: the chosen option's text, with your changes. */
  draft: string;
  from: { letter: string; color: AccountColor; address: string };
  /** Under the reply: the signature it's sent with, and the chip to change it. */
  signature?: ReactNode;
  /** You opened it (Q): the cursor goes to the end of the reply. Otherwise the reading view keeps its keys. */
  focus?: boolean;
  /** Moss is still writing, its model isn't downloaded, or it failed. Without it, the reply is ready. */
  state?: "writing" | "missing" | "failed";
  onChoose: (index: number) => void;
  onDraftChange: (text: string) => void;
  onChangeIt: () => void;
  onEdit: () => void;
  onSend: () => void;
  onStop?: () => void;
  onDownload?: () => void;
  onSettings?: () => void;
  onRetry?: () => void;
  /** Takes the replies away, with your changes; Esc does it too once you opened the card (Q). */
  onDiscard: () => void;
}

/** Quick reply (Q): replies Moss wrote under an email, editable in place. Nothing is sent until you press Send. */
export function InstantReply(props: InstantReplyProps) {
  const { options, choice, draft, from, signature, focus, state, onChoose, onDraftChange, onChangeIt, onEdit, onSend, onDiscard } = props;
  const box = useRef<HTMLTextAreaElement>(null);
  // Replies show as Moss writes them, one after another.
  const streaming = state === "writing" && options.length > 0;
  const note = state && (
    <MossNote state={{ kind: state }} onStop={props.onStop} onDownload={props.onDownload} onSettings={props.onSettings} onRetry={props.onRetry} />
  );

  // Grow with the text, and keep the cursor at the end of each new option.
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft]);
  useEffect(() => {
    const el = box.current;
    if (!el || !focus) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, [choice, state, focus]);

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Tab" || !options.length) return;
    e.preventDefault();
    onChoose((choice + (e.shiftKey ? options.length - 1 : 1)) % options.length);
  };

  return (
    <section className={styles.box} aria-label="Quick reply">
      <div className={styles.head}>
        <Icon icon={Leaf} size={15} tone="ai" />
        <span className={styles.title}>Quick reply</span>
        <span className={styles.note}>Moss wrote this. Nothing sends until you press Send.</span>
        {!state && <ShortcutHint keys="Tab" label="next option" size="sm" gap={6} />}
        {state !== "writing" && (
          <button type="button" className={styles.discard} onClick={onDiscard}>
            <Icon icon={X} size={13} tone="ink-2" />
            Discard
            {focus && <Kbd>Esc</Kbd>}
          </button>
        )}
      </div>
      {state && !streaming ? (
        note
      ) : (
        <>
          <div className={styles.options} role="radiogroup" aria-label="Reply options">
            {options.map((o, i) => (
              <Button key={o.label} variant="choice" active={i === choice} onClick={() => onChoose(i)} role="radio" aria-checked={i === choice}>
                {o.label}
              </Button>
            ))}
          </div>
          <textarea
            ref={box}
            className={styles.draft}
            aria-label="Reply"
            rows={1}
            value={draft}
            readOnly={streaming}
            onChange={(e) => onDraftChange(e.target.value)}
            onKeyDown={onKeyDown}
          />
          {signature}
          {streaming ? (
            note
          ) : (
            <div className={styles.footer}>
              <FromPicker variant="inline" account={from} text={from.address} />
              <span className={styles.spacer} />
              <Button variant="ai" icon={PenLine} shortcut="⌘J" onClick={onChangeIt}>
                Change it
              </Button>
              <Button variant="outline" shortcut="⌘E" onClick={onEdit}>
                Full editor
              </Button>
              <Button variant="primary" shortcut="⌘↵" onClick={onSend}>
                Send
              </Button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
