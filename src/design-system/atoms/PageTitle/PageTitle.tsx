import type { ReactNode } from "react";
import styles from "./PageTitle.module.css";

/** The page's name, e.g. "Inbox" or "Work inbox". Sits in the top bar. */
export function PageTitle({ children }: { children: ReactNode }) {
  return <h1 className={styles.title}>{children}</h1>;
}
