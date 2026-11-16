import { Bell, Clock3, Paperclip, PenLine, Trash2 } from "lucide-react";
import { Button } from "../../atoms";
import styles from "./ComposerToolbar.module.css";

export interface ComposerToolbarProps {
  remindIfNoReply: boolean;
  /** Send was pressed: it shows so at once. */
  sending?: boolean;
  onAttach: () => void;
  /** Asks when to send it (⌘⇧↵). */
  onSendLater: () => void;
  onToggleRemind: () => void;
  onDiscard: () => void;
  onSend: () => void;
  /** Opens "Write with Moss" (⌘J). */
  onWriteWithMoss?: () => void;
}

/** Bottom row of the composer. */
export function ComposerToolbar({
  remindIfNoReply,
  sending,
  onAttach,
  onSendLater,
  onToggleRemind,
  onDiscard,
  onSend,
  onWriteWithMoss,
}: ComposerToolbarProps) {
  return (
    <div className={styles.toolbar}>
      {onWriteWithMoss && (
        <Button variant="ai" className={styles.moss} icon={PenLine} shortcut="⌘J" onClick={onWriteWithMoss}>
          Write with Moss
        </Button>
      )}
      <Button variant="toolbar" icon={Paperclip} shortcut="⌘⇧A" onClick={onAttach}>
        Attach
      </Button>
      <Button variant="toolbar" icon={Clock3} shortcut="⌘⇧↵" onClick={onSendLater}>
        Send later
      </Button>
      <Button variant="toolbar" icon={Bell} active={remindIfNoReply} onClick={onToggleRemind}>
        Remind me if no reply
      </Button>
      <span className={styles.spacer} />
      <Button variant="subtle" icon={Trash2} onClick={onDiscard}>
        Discard
      </Button>
      <Button variant="primary" size="lg" shortcut="⌘↵" busy={sending} onClick={onSend}>
        {sending ? "Sending…" : "Send"}
      </Button>
    </div>
  );
}
