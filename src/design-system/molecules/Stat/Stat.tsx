import styles from "./Stat.module.css";

/** A big number with a label, shown on the inbox zero photo. */
export function Stat({ value, label }: { value: number | string; label: string }) {
  return (
    <div className={styles.stat}>
      <span className={styles.value}>{value}</span>
      <span className={styles.label}>{label}</span>
    </div>
  );
}
