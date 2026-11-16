import type { ReactNode } from "react";
import styles from "./ComposeLayout.module.css";

export interface ComposeLayoutProps {
  children: ReactNode;
  /** Pinned to the bottom: checks and the toolbar. */
  footer: ReactNode;
}

/** The composer: a centred 840px column. */
export function ComposeLayout({ children, footer }: ComposeLayoutProps) {
  return (
    <div className={styles.column}>
      {children}
      <div className={styles.push} />
      {footer}
    </div>
  );
}
