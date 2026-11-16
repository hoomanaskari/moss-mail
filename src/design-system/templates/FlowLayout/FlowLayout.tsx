import type { ReactNode } from "react";
import { cx } from "../../../lib/cx";
import styles from "./FlowLayout.module.css";

export interface FlowLayoutProps {
  children: ReactNode;
  /** Space between blocks: 28 for short steps, 24 for forms, 22 for long advanced ones. */
  gap: 22 | 24 | 28;
  /** Less space above and below, for the long advanced steps. */
  tight?: boolean;
}

/** One 840px column for a step-by-step flow such as Add account, scrolling when it is tall. */
export function FlowLayout({ children, gap, tight }: FlowLayoutProps) {
  return (
    <div className={cx(styles.column, tight && styles.tight)} style={{ gap }}>
      {children}
    </div>
  );
}
