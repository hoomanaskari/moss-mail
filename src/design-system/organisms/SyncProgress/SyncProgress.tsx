import { CircleCheck, CircleDashed } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, ProgressBar, ProgressRing } from "../../atoms";
import styles from "./SyncProgress.module.css";

export interface SyncItem {
  label: string;
  state: "done" | "now" | "next";
  status?: string;
}

export interface SyncProgressProps {
  done: number;
  total: number;
  items: SyncItem[];
}

/** Download progress for a newly added account, with what is done and what comes next. */
export function SyncProgress({ done, total, items }: SyncProgressProps) {
  return (
    <section className={styles.card} aria-label="Downloading recent mail">
      <div className={styles.head}>
        <span className={styles.title}>Downloading recent mail</span>
        <span className={styles.count}>
          {done.toLocaleString("en-GB")} of {total.toLocaleString("en-GB")} emails
        </span>
      </div>
      <ProgressBar value={total ? done / total : 0} label="Recent mail downloaded" />
      <ul className={styles.list}>
        {items.map((item) => (
          <li key={item.label} className={cx(styles.item, styles[item.state])}>
            {item.state === "done" && <Icon icon={CircleCheck} size={18} tone="ai" />}
            {item.state === "now" && <ProgressRing value={0.3} size={18} spinning label="In progress" />}
            {item.state === "next" && <Icon icon={CircleDashed} size={18} tone="ink-3" />}
            <span className={styles.label}>{item.label}</span>
            {item.status && <span className={styles.status}>{item.status}</span>}
          </li>
        ))}
      </ul>
    </section>
  );
}
