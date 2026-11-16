import type { ReactNode } from "react";
import styles from "./SettingsLayout.module.css";

export interface SettingsLayoutProps {
  nav: ReactNode;
  children: ReactNode;
}

/** Settings body: the section list on the left, the chosen section on the right (scrolls if it is long). */
export function SettingsLayout({ nav, children }: SettingsLayoutProps) {
  return (
    <div className={styles.layout}>
      {nav}
      <div className={styles.content}>{children}</div>
    </div>
  );
}
