import { cx } from "../../../lib/cx";
import type { AccountColor } from "../../../data/types";
import styles from "./AccountBadge.module.css";

export type AccountBadgeSize = 18 | 20 | 22 | 28;

export interface AccountBadgeProps {
  letter: string;
  color: AccountColor;
  size?: AccountBadgeSize;
  className?: string;
}

/** The coloured square with an account's letter. Tells you which inbox an email belongs to. */
export function AccountBadge({ letter, color, size = 20, className }: AccountBadgeProps) {
  return (
    <span
      className={cx(styles.badge, styles[`s${size}`], className)}
      style={{ background: `var(--${color})` }}
      aria-hidden
    >
      {letter}
    </span>
  );
}
