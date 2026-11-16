import { cx } from "../../../lib/cx";
import styles from "./AiDownloadBar.module.css";

export interface AiDownloadBarProps {
  /** e.g. "Getting AI · 2.1 of 6.4 GB". */
  label: string;
  /** How far, from 0 to 1. */
  value: number;
  /** Waiting (battery, hotspot): the bar goes grey. */
  paused?: boolean;
  onOpen: () => void;
}

/** The quiet download line in the top bar while the AI models come in. Opens Settings → AI. */
export function AiDownloadBar({ label, value, paused, onOpen }: AiDownloadBarProps) {
  const percent = Math.round(value * 100);
  return (
    <button type="button" className={styles.bar} onClick={onOpen}>
      <span className={styles.label}>{label}</span>
      <span
        className={styles.track}
        role="progressbar"
        aria-label="AI download"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <span className={cx(styles.value, paused && styles.paused)} style={{ width: `${percent}%` }} />
      </span>
    </button>
  );
}
