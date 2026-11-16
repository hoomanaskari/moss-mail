import { createContext, useContext, type ReactNode } from "react";
import styles from "./AppFrame.module.css";

export interface AppFrameProps {
  topBar: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
}

/**
 * What sits beside every screen's body, under the top bar: the Ask Moss panel while it's open
 * (null while it's closed). The app fills it, so each screen doesn't have to.
 */
export const AppFrameSide = createContext<ReactNode>(null);

/** Full-window column: top bar, scrolling body (with the side beside it), optional footer. */
export function AppFrame({ topBar, children, footer }: AppFrameProps) {
  const side = useContext(AppFrameSide);
  return (
    // While a side is open, the top bar gets a hairline, so the side's edge meets a line.
    <div className={styles.frame} data-side={side ? "" : undefined}>
      {topBar}
      <div className={styles.row}>
        <main className={styles.body}>{children}</main>
        {side}
      </div>
      {footer}
    </div>
  );
}
