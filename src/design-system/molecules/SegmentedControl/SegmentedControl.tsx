import type { ReactNode } from "react";
import { cx } from "../../../lib/cx";
import styles from "./SegmentedControl.module.css";

export interface Segment<T extends string> {
  value: T;
  label: string;
  /** Shown before the label, e.g. an "Aa" size sample. */
  lead?: ReactNode;
}

export interface SegmentedControlProps<T extends string> {
  value: T;
  options: Segment<T>[];
  onChange: (value: T) => void;
  label: string;
  /** `sm` is the compact switch in a toolbar (Search's Group by). */
  size?: "md" | "sm";
  className?: string;
}

/** A few side-by-side options in one box; the chosen one is raised. */
export function SegmentedControl<T extends string>({ value, options, onChange, label, size = "md", className }: SegmentedControlProps<T>) {
  return (
    <div className={cx(styles.control, size === "sm" && styles.sm, className)} role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          tabIndex={o.value === value ? 0 : -1}
          className={cx(styles.segment, o.value === value && styles.on)}
          onClick={() => onChange(o.value)}
        >
          {o.lead && <span aria-hidden>{o.lead}</span>}
          <span className={styles.label}>{o.label}</span>
        </button>
      ))}
    </div>
  );
}
