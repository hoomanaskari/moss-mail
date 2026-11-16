import styles from "./Spinner.module.css";

/** A turning arc in the colour around it, for "working on it" with no progress to show. The words beside it say what. */
export function Spinner({ size = 16 }: { size?: number }) {
  return (
    <svg className={styles.spinner} width={size} height={size} viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" strokeWidth="2" opacity="0.3" />
      <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeDasharray="10 40" />
    </svg>
  );
}
