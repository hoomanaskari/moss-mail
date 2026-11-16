import { useState, type ReactNode } from "react";
import { ChevronsUpDown } from "lucide-react";
import type { Message } from "../../../data/types";
import { Icon } from "../../atoms";
import styles from "./EarlierMessages.module.css";

export interface EarlierMessagesProps {
  /** The messages before the newest one, oldest first. */
  messages: Message[];
  /** Your address on this account, so your own messages read "You". */
  youAddress: string;
  /** Draws a message in full once its row is opened. */
  renderMessage: (message: Message) => ReactNode;
  /** A message to show open, e.g. one Ask Moss cited: the fold opens to it. */
  reveal?: string;
}

/** Long conversations keep their first two and last message in view; the rest wait behind one button. */
const FOLD_ABOVE = 4;

const snippet = (m: Message) => (m.paragraphs.length ? m.paragraphs.join(" ") : (m.text ?? "")).replace(/\s+/g, " ").trim();

/** The conversation so far, one quiet row per message, above the newest one. */
export function EarlierMessages({ messages, youAddress, renderMessage, reveal }: EarlierMessagesProps) {
  const [opened, setOpened] = useState<string[]>([]);
  const [unfolded, setUnfolded] = useState(false);
  if (!messages.length) return null;

  const inFold = messages.slice(2, -1).some((m) => m.id === reveal);
  const folded = !unfolded && !inFold && messages.length > FOLD_ABOVE;
  const shown = folded ? [...messages.slice(0, 2), ...messages.slice(-1)] : messages;
  const hidden = messages.length - shown.length;

  return (
    <div className={styles.list}>
      {shown.map((m, i) => (
        <div key={m.id} className={styles.item} data-message={m.id}>
          {folded && i === 2 && (
            <div className={styles.more}>
              <button type="button" className={styles.pill} onClick={() => setUnfolded(true)}>
                <Icon icon={ChevronsUpDown} size={14} tone="ink-2" />
                {hidden} more messages
              </button>
            </div>
          )}
          {opened.includes(m.id) || m.id === reveal ? (
            <div className={styles.open}>{renderMessage(m)}</div>
          ) : (
            <button type="button" className={styles.row} onClick={() => setOpened([...opened, m.id])}>
              <span className={styles.name}>{m.from.email.toLowerCase() === youAddress.toLowerCase() ? "You" : m.from.name}</span>
              <span className={styles.snippet}>{snippet(m)}</span>
              <span className={styles.when}>{m.time}</span>
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
