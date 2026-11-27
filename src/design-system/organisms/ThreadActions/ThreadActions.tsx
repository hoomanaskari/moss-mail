import { AlarmClock, ArrowRightLeft, Check, ChevronDown, Inbox, RotateCcw, Trash2, Undo2, type LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, Kbd } from "../../atoms";
import { Popover } from "../../molecules";
import { RemindMenu } from "../RemindMenu/RemindMenu";
import styles from "./ThreadActions.module.css";

export interface ThreadActionsProps {
  /** A done email offers Not done instead of Done. */
  done: boolean;
  onDone: () => void;
  /** A time was picked in the Remind menu, in ms. */
  onRemind: (at: number) => void;
  onRemindNoReply: () => void;
  /** Set both to open the Remind menu from outside, e.g. with H. */
  remindOpen?: boolean;
  onRemindOpenChange?: (open: boolean) => void;
  /** Left out when the email can't be deleted (blocked mail). */
  onDelete?: () => void;
  /** Opens the Move menu (M): to Important, or out of it. */
  onMove?: () => void;
  /** Deleted mail: Back to inbox is all it offers. */
  onRestore?: () => void;
}

/** The icon and key say what it does; the name is there for screen readers and the tooltip. */
function Action({ icon, label, keys, title = label, onClick }: { icon: LucideIcon; label: string; keys: string; title?: string; onClick: () => void }) {
  return (
    <button type="button" className={styles.action} title={title} onClick={onClick}>
      <Icon icon={icon} size={16} tone="ink-2" />
      <span className={styles.name}>{label}</span>
      <Kbd>{keys}</Kbd>
    </button>
  );
}

/** What you can do with the open email, in the top bar beside the ↑/↓ position. */
export function ThreadActions({ done, onDone, onRemind, onRemindNoReply, remindOpen, onRemindOpenChange, onDelete, onMove, onRestore }: ThreadActionsProps) {
  if (onRestore)
    return (
      <div className={styles.actions}>
        <Action icon={Inbox} label="Back to inbox" keys="E" title="Put it back in your inbox" onClick={onRestore} />
      </div>
    );
  return (
    <div className={styles.actions}>
      {done ? (
        <Action icon={Undo2} label="Not done" keys="E" onClick={onDone} />
      ) : (
        <Action icon={Check} label="Done" keys="E" onClick={onDone} />
      )}
      <Popover
        role="group"
        label="Remind"
        align="end"
        open={remindOpen}
        onOpenChange={onRemindOpenChange}
        trigger={({ open, toggle }) => (
          <button
            type="button"
            className={cx(styles.action, open && styles.open)}
            title="Remind"
            aria-expanded={open}
            onClick={toggle}
          >
            <Icon icon={AlarmClock} size={16} tone="ink-2" />
            <span className={styles.name}>Remind</span>
            <Kbd>H</Kbd>
            <Icon icon={ChevronDown} size={12} tone="ink-3" />
          </button>
        )}
      >
        {(close) => (
          <RemindMenu
            onPick={(at) => {
              close();
              onRemind(at);
            }}
            extras={[
              {
                icon: RotateCcw,
                label: "Remind me if no reply",
                keys: "⇧H",
                onClick: () => {
                  close();
                  onRemindNoReply();
                },
              },
            ]}
          />
        )}
      </Popover>
      {onMove && <Action icon={ArrowRightLeft} label="Move" keys="M" title="Move to another tab" onClick={onMove} />}
      {onDelete && <Action icon={Trash2} label="Delete" keys="⌫" onClick={onDelete} />}
    </div>
  );
}
