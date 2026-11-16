import type { ReactNode } from "react";
import { cx } from "../../../lib/cx";
import styles from "./ReadingLayout.module.css";

export interface ReadingLayoutProps {
  children: ReactNode;
  /** Pinned to the bottom of the reading column (instant reply, reply bar). */
  footer?: ReactNode;
  /** Tighter spacing, used while the recipient list is open. */
  compact?: boolean;
}

/** An open email: a centred 740px reading column, with the whole window around it. */
export function ReadingLayout({ children, footer, compact }: ReadingLayoutProps) {
  return (
    <div className={styles.layout}>
      <div className={cx(styles.scroll, compact && styles.compact)}>
        <div className={styles.column}>
          {children}
          <div className={styles.push} />
          {footer}
        </div>
      </div>
    </div>
  );
}
