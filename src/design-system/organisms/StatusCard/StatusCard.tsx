import type { ReactNode } from "react";
import { Check } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, ProgressRing } from "../../atoms";
import styles from "./StatusCard.module.css";

export interface StatusCardProps {
  /** `waiting`: a spinner while something happens elsewhere. `ready`: a teal tick when it is done. */
  kind: "waiting" | "ready";
  title: string;
  sub: string;
  action?: ReactNode;
}

/** A card that says what Moss is waiting for, or that it is done. */
export function StatusCard({ kind, title, sub, action }: StatusCardProps) {
  return (
    <section className={cx(styles.card, styles[kind])} role="status">
      {kind === "waiting" ? (
        <ProgressRing value={110 / 360} size={40} spinning label="Waiting" />
      ) : (
        <span className={styles.tile}>
          <Icon icon={Check} size={22} tone="ai" />
        </span>
      )}
      <span className={styles.text}>
        <span className={styles.title}>{title}</span>
        <span className={styles.sub}>{sub}</span>
      </span>
      {action}
    </section>
  );
}
