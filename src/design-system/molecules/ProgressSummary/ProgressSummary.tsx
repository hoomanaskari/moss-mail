import { ProgressRing } from "../../atoms";
import styles from "./ProgressSummary.module.css";

export interface ProgressSummaryProps {
  done: number;
  total: number;
  /** e.g. "6 left", or "Inbox zero". */
  eta: string;
  /** Opens the Done list, to see (or bring back) what you've done. */
  onOpen?: () => void;
}

/** Today's progress toward inbox zero: "2 of 8 done" with a ring. A visible finish line. */
export function ProgressSummary({ done, total, eta, onOpen }: ProgressSummaryProps) {
  const Tag = onOpen ? "button" : "div";
  return (
    <Tag
      type={onOpen ? "button" : undefined}
      className={styles.progress}
      onClick={onOpen}
      aria-label={onOpen ? `${done} of ${total} done. See what you’ve done` : undefined}
    >
      <div className={styles.text}>
        <span className={styles.main}>
          {done} of {total} done
        </span>
        <span className={styles.sub}>{eta}</span>
      </div>
      <ProgressRing value={total ? done / total : 1} label={`${done} of ${total} done`} />
    </Tag>
  );
}
