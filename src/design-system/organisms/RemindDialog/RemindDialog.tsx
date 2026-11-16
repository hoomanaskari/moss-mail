import { listenAppKeys } from "../../../state/appKeyboard";
import { useEffect, useRef } from "react";
import { RemindMenu } from "../RemindMenu/RemindMenu";
import styles from "./RemindDialog.module.css";

export interface RemindDialogProps {
  /** What it asks: "Remind me", "Remind me if no reply", or "Send later". */
  title?: string;
  /** What picking your own time does: "Remind me", or "Send". */
  action?: string;
  /** The email it is for. */
  subject?: string;
  onPick: (at: number) => void;
  onClose: () => void;
  /** The times are worked out from this; the clock when left out. */
  now?: number;
}

/**
 * H in the mail list: when should this email come back? Or ⌘⇧↵ in the composer: when should it
 * go? It takes the keys from where you were typing. Esc or a click beside it closes it.
 */
export function RemindDialog({ title = "Remind me", action = title, subject, onPick, onClose, now }: RemindDialogProps) {
  const latest = useRef(onClose);
  latest.current = onClose;
  const card = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Where you were typing gets the keys back once this closes.
    const before = document.activeElement as HTMLElement | null;
    card.current?.focus();
    // Caught before the app's shortcuts, so Esc closes this and does nothing else.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      latest.current();
    };
    const stopKeys = listenAppKeys(onKey, true);
    return () => {
      stopKeys();
      before?.focus();
    };
  }, []);

  return (
    <div className={styles.layer}>
      <div className={styles.scrim} data-testid="remind-scrim" onClick={onClose} />
      <div ref={card} tabIndex={-1} className={styles.card} role="dialog" aria-modal="true" aria-labelledby="remind-title">
        <div className={styles.head}>
          <h2 id="remind-title" className={styles.title}>
            {title}
          </h2>
          {subject && <p className={styles.subject}>{subject}</p>}
        </div>
        <RemindMenu onPick={onPick} now={now} action={action} />
      </div>
    </div>
  );
}
