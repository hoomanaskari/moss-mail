import type { ReactNode } from "react";
import { Leaf, Star } from "lucide-react";
import type { AccountColor } from "../../../data/types";
import { AccountBadge, Icon, Pill } from "../../atoms";
import styles from "./ThreadHeader.module.css";
import { SEP } from "../../../lib/text";

export interface ThreadHeaderProps {
  /** `address` is the one it came to: the account's own, or another of its addresses. */
  account: { letter: string; color: AccountColor; name: string; address: string };
  labels?: string[];
  subject: string;
  /** AI's one-sentence "what do you need to do". */
  ask?: string;
  askDue?: string;
  /** Why Moss put it where it is: one short line per decision, e.g. "Needs your reply · AI, 91% sure". */
  reasons?: string[];
  /** The summary card: under the subject, before the ask. */
  summary?: ReactNode;
  /** The tab you moved it to yourself, e.g. "Important". What the AI thought no longer counts. */
  moved?: string;
}

/** Top of an open email: which inbox it's in, its labels, subject and the AI summary of the ask. */
export function ThreadHeader({ account, labels = [], subject, ask, askDue, reasons = [], summary, moved }: ThreadHeaderProps) {
  return (
    <>
      <div className={styles.meta}>
        <AccountBadge letter={account.letter} color={account.color} size={20} />
        <span className={styles.mailbox}>
          {account.name}
          {SEP}
          {account.address}
        </span>
        {labels.map((l) => (
          <Pill key={l} variant="label">
            {l}
          </Pill>
        ))}
        {moved && (
          <span className={styles.moved}>
            <Icon icon={Star} size={13} tone="ink-2" filled={moved === "Important"} />
            You moved this to {moved}
          </span>
        )}
        {reasons.length > 0 && (
          <ul className={styles.reasons} aria-label="Why Moss put it here">
            {reasons.map((reason) => (
              <li key={reason} className={styles.sorted}>
                <Icon icon={Leaf} size={13} tone="ai" />
                {reason}
              </li>
            ))}
          </ul>
        )}
      </div>
      <h1 className={styles.subject}>{subject}</h1>
      {summary}
      {ask && (
        <div className={styles.ask}>
          <p className={styles.askText}>{ask}</p>
          {askDue && <Pill variant="due">{askDue}</Pill>}
        </div>
      )}
    </>
  );
}
