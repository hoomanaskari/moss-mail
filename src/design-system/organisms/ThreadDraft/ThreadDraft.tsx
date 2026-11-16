import { plainLabel } from "../../../lib/displayLabel";
import { PenLine } from "lucide-react";
import { useId } from "react";
import { Button, Icon } from "../../atoms";
import { DraftReview, type DraftReviewDetails } from "../../molecules";
import styles from "./ThreadDraft.module.css";

export interface ThreadDraftProps {
  /** Who it goes to and from: "To Alex Morgan Studio Build · from maya@example.test". */
  line: string;
  body: string;
  details: DraftReviewDetails;
  /** When it was last saved: "Saved 2 min ago". */
  saved: string;
  /** Send later, at this time. */
  at?: string;
  onEdit(): void;
  onDelete(): void;
  onSend(): void;
}

/** Your unsent reply, at the end of the conversation it answers: wherever it was written (the composer, Ask Moss, your phone). */
export function ThreadDraft({ line, body, details, saved, at, onEdit, onDelete, onSend }: ThreadDraftProps) {
  const id = useId();
  return (
    <section className={styles.draft} aria-labelledby={id}>
      <div className={styles.head}>
        <Icon icon={PenLine} size={14} tone="ink-3" />
        <h2 id={id} className={styles.title}>
          Your draft
        </h2>
        <span className={styles.line}><bdi dir="ltr">{plainLabel(line)}</bdi></span>
        <span className={styles.notSent}>Not sent yet · {saved}</span>
      </div>
      <DraftReview {...details} />
      {body.trim() ? <p className={styles.text}>{body}</p> : <p className={styles.empty}>No words yet.</p>}
      <div className={styles.actions}>
        <Button variant="outline" shortcut="⌘E" onClick={onEdit}>
          Edit
        </Button>
        <Button variant="quiet" onClick={onDelete}>
          Delete
        </Button>
        <span className={styles.spacer} />
        <Button variant="primary" shortcut="⌘↵" onClick={onSend}>
          {at ? `Send ${at}` : "Send"}
        </Button>
      </div>
    </section>
  );
}
