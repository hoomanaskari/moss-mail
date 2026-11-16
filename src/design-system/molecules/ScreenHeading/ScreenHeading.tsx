import type { ReactNode } from "react";
import styles from "./ScreenHeading.module.css";

export interface ScreenHeadingProps {
  title: ReactNode;
  sub: ReactNode;
}

/** The big heading at the top of a page's content, with one sentence explaining it. */
export function ScreenHeading({ title, sub }: ScreenHeadingProps) {
  return (
    <div className={styles.head}>
      <h2 className={styles.title}>{title}</h2>
      <p className={styles.sub}>{sub}</p>
    </div>
  );
}
