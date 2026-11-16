import { Reply, Zap } from "lucide-react";
import { Button, Icon, Kbd } from "../../atoms";
import { ShortcutHint } from "../../molecules";
import styles from "./ReplyBar.module.css";

export interface ReplyBarProps {
  /** e.g. "3 suggested replies ready". */
  hint?: string;
  /** Who a reply goes to. With it, the bar becomes a full-width "Reply to Priya…" field. */
  replyTo?: string;
  onReply: () => void;
  onReplyAll: () => void;
  onForward: () => void;
  /** Quick reply (Q): Moss writes a reply in the docked card. Only with a reply field. */
  onQuickReply?: () => void;
}

/** Compact reply actions, shown when the instant reply box is folded away. */
export function ReplyBar({ hint, replyTo, onReply, onReplyAll, onForward, onQuickReply }: ReplyBarProps) {
  if (replyTo)
    return (
      <div className={styles.field}>
        <button type="button" className={styles.reply} onClick={onReply} title={`Reply to ${replyTo}`}>
          <Icon icon={Reply} size={18} tone="ink-3" />
          <span className={styles.placeholder}>Reply to {replyTo}…</span>
          <Kbd>R</Kbd>
        </button>
        <span className={styles.divider} />
        {onQuickReply && (
          <>
            <button type="button" className={styles.quick} onClick={onQuickReply}>
              <Kbd>Q</Kbd>
              <Icon icon={Zap} size={14} tone="ai" />
              <span className={styles.quickLabel}>Quick reply</span>
            </button>
            <span className={styles.divider} />
          </>
        )}
        <ShortcutHint keys="⇧R" label="Reply all" tone="ink-2" onClick={onReplyAll} />
        <ShortcutHint keys="F" label="Forward" tone="ink-2" onClick={onForward} />
      </div>
    );
  return (
    <div className={styles.bar}>
      <span className={styles.hint}>{hint}</span>
      <Button variant="outline" shortcut="R" onClick={onReply}>
        Reply
      </Button>
      <Button variant="outline" shortcut="⇧R" onClick={onReplyAll}>
        Reply all
      </Button>
      <Button variant="outline" shortcut="F" onClick={onForward}>
        Forward
      </Button>
    </div>
  );
}
