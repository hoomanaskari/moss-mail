import { accountColorOrder, type AccountColor } from "../../../data/types";
import { cx } from "../../../lib/cx";
import styles from "./ColorSwatches.module.css";

export interface ColorSwatchesProps {
  value: AccountColor;
  onChange: (color: AccountColor) => void;
  label: string;
}

const names: Record<AccountColor, string> = {
  work: "Blue",
  personal: "Orange",
  studio: "Pink",
  green: "Green",
  violet: "Violet",
  sky: "Sky",
  sand: "Sand",
  olive: "Olive",
};

/** The eight account colours as round swatches; the chosen one has a ring. */
export function ColorSwatches({ value, onChange, label }: ColorSwatchesProps) {
  return (
    <div className={styles.row} role="radiogroup" aria-label={label}>
      {accountColorOrder.map((c) => (
        <button
          key={c}
          type="button"
          role="radio"
          aria-checked={c === value}
          aria-label={names[c]}
          tabIndex={c === value ? 0 : -1}
          className={cx(styles.swatch, c === value && styles.on)}
          onClick={() => onChange(c)}
        >
          <span className={styles.dot} style={{ background: `var(--${c})` }} />
        </button>
      ))}
    </div>
  );
}
