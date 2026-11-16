import styles from "./Caret.module.css";

/** The AI-coloured text cursor used in the design's typing states. */
export function Caret({ height = 18 }: { height?: number }) {
  return <span className={styles.caret} style={{ height }} aria-hidden />;
}
