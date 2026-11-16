import { cx } from "../../../lib/cx";
import styles from "./Toggle.module.css";

export interface ToggleProps {
  on: boolean;
  onChange: (on: boolean) => void;
  label: string;
}

/** An on/off switch. */
export function Toggle({ on, onChange, label }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={cx(styles.toggle, on && styles.on)}
      onClick={() => onChange(!on)}
    >
      <span className={styles.knob} />
    </button>
  );
}
