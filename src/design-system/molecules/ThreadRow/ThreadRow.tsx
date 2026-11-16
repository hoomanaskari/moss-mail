import { forwardRef, type ReactNode } from "react";
import { Clock3, Paperclip, Star } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { AccountColor, Section } from "../../../data/types";
import { AccountBadge, Icon, Pill } from "../../atoms";
import { ShortcutHint } from "../ShortcutHint/ShortcutHint";
import styles from "./ThreadRow.module.css";

export interface ThreadRowProps {
  account: { letter: string; color: AccountColor };
  from: string;
  /** Text, or marked-up text such as search matches. */
  subject: ReactNode;
  summary?: ReactNode;
  time: string;
  due?: string;
  /** `due` is "No reply": you wait on an answer, so it looks unlike a due day. */
  noReply?: boolean;
  /** Shows a paperclip before the time. */
  attachment?: boolean;
  /** Replaces the Done / Later / Reply / Delete shortcuts on the selected row. */
  actions?: ReactNode;
  /**
   * `reply`: needs your reply, shown bold.
   * `info`: for your information, shown lighter.
   * `later`: snoozed until a set time, shown dimmed with the time it returns.
   */
  section: Section;
  /** Shows a dot in the margin; read aloud as "Unread". */
  unread?: boolean;
  selected: boolean;
  /** Position in the complete option group, including rows outside the rendering window. */
  position?: number;
  setSize?: number;
  /** Clicking opens the row. Moving the mouse over it never selects it: only the arrow keys and clicks do. */
  onOpen: () => void;
  onDone?: () => void;
  onLater?: () => void;
  onReply?: () => void;
  onDelete?: () => void;
  /** Given for a new sender: the selected row offers Block too. */
  onBlock?: () => void;
  /** Given when it can go to another tab: the selected row offers Move (M). */
  onMove?: () => void;
  /** You moved it to Important: a star says so. */
  moved?: boolean;
}

/** One email thread in the inbox list. The selected row shows its shortcuts instead of the time. */
export const ThreadRow = forwardRef<HTMLDivElement, ThreadRowProps>(function ThreadRow(
  { account, from, subject, summary, time, due, noReply, attachment, actions, section, unread, selected, onOpen, onDone, onLater, onReply, onDelete, onBlock, onMove, moved, position, setSize },
  ref,
) {
  const later = section === "later";
  return (
    <div
      ref={ref}
      role="option"
      aria-selected={selected}
      aria-posinset={position}
      aria-setsize={setSize}
      className={cx(styles.row, styles[section], unread && styles.unread, selected && styles.selected)}
      onClick={onOpen}
    >
      {unread && (
        <span className={styles.dot}>
          <span className={styles.hidden}>Unread</span>
        </span>
      )}
      {selected && <span className={styles.cursor} aria-hidden />}
      <AccountBadge letter={account.letter} color={account.color} size={20} />
      <span className={styles.from}>{from}</span>
      {later ? (
        <span className={cx(styles.subject, styles.grow)}>{subject}</span>
      ) : (
        <span className={styles.content}>
          <span className={styles.subject}>{subject}</span>
          {summary && <span className={styles.summary}>{summary}</span>}
        </span>
      )}
      {moved && (
        <span role="img" aria-label="You moved it to Important" className={styles.attachment}>
          <Icon icon={Star} size={15} tone="ink-2" filled />
        </span>
      )}
      {due &&
        (noReply ? (
          <span title="You’re waiting for an answer">
            <Pill variant="waiting">
              <Icon icon={Clock3} size={12} tone="inherit" />
              {due}
            </Pill>
          </span>
        ) : (
          <Pill variant="due">{due}</Pill>
        ))}
      {attachment && (
        <span role="img" aria-label="Has an attachment" className={styles.attachment}>
          <Icon icon={Paperclip} size={15} tone="ink-3" />
        </span>
      )}
      {selected && !later ? (
        <span className={styles.shortcuts} onClick={(e) => e.stopPropagation()}>
          {actions ?? (
            <>
              <ShortcutHint keys="E" label="Done" tone="ink-2" gap={6} onClick={onDone} />
              <ShortcutHint keys="H" label="Later" tone="ink-2" gap={6} onClick={onLater} />
              <ShortcutHint keys="R" label="Reply" tone="ink-2" gap={6} onClick={onReply} />
              <ShortcutHint keys="⌫" label="Delete" tone="ink-2" gap={6} onClick={onDelete} />
              {onMove && <ShortcutHint keys="M" label="Move" tone="ink-2" gap={6} onClick={onMove} />}
              {onBlock && <ShortcutHint keys="B" label="Block" tone="ink-2" gap={6} onClick={onBlock} />}
            </>
          )}
        </span>
      ) : (
        <span className={styles.time}>{time}</span>
      )}
    </div>
  );
});
