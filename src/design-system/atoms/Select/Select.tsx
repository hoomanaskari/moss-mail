import { ChevronDown } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon } from "../Icon/Icon";
import styles from "./Select.module.css";

export interface SelectOption {
  value: string;
  label: string;
}

export interface SelectProps {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  /** `outline`: a bordered chip in a list row. `field`: a compact form field (IMAP security). */
  variant?: "outline" | "field";
  disabled?: boolean;
  "aria-label": string;
  className?: string;
}

/**
 * A drop-down choice. Shows the chosen option with a chevron; the native menu underneath
 * keeps it keyboard- and screen-reader-friendly.
 */
export function Select({ value, options, onChange, variant = "outline", disabled, className, ...aria }: SelectProps) {
  const current = options.find((o) => o.value === value) ?? options[0];
  return (
    <span className={cx(styles.select, styles[variant], disabled && styles.disabled, className)}>
      <span className={styles.label}>{current.label}</span>
      <Icon icon={ChevronDown} size={14} tone="ink-2" />
      <select
        className={styles.native}
        value={current.value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        {...aria}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </span>
  );
}
