import styles from "./ProgressBar.module.css";

export interface ProgressBarProps {
  /** 0 to 1. */
  value: number;
  label: string;
}

/** A thin teal bar showing how much is done. */
export function ProgressBar({ value, label }: ProgressBarProps) {
  const pct = Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <div className={styles.track} role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
      <div className={styles.value} style={{ width: `${pct}%` }} />
    </div>
  );
}
