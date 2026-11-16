import { ChevronDown } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { AccountColor } from "../../../data/types";
import { AccountBadge, Icon, Kbd } from "../../atoms";
import styles from "./FromPicker.module.css";

export interface FromPickerProps {
  account: { letter: string; color: AccountColor };
  /** What to show next to the badge, e.g. "maya@northwind.co" or "Maya Ross · maya@northwind.co". */
  text: string;
  /** `inline` sits in the instant reply footer with a "From" label; `field` sits in the composer's From row. */
  variant: "inline" | "field";
  /** Its shortcut, shown at the end, e.g. "⌥F". */
  keys?: string;
  /** The menu it opens is open. */
  expanded?: boolean;
  onClick?: () => void;
}

/** Chooses which of your addresses the email is sent from. */
export function FromPicker({ account, text, variant, keys, expanded, onClick }: FromPickerProps) {
  return (
    <button type="button" className={cx(styles.picker, styles[variant])} onClick={onClick} aria-expanded={expanded}>
      {variant === "inline" && <span className={styles.label}>From</span>}
      <AccountBadge letter={account.letter} color={account.color} size={variant === "inline" ? 18 : 22} />
      <span className={styles.address}>{text}</span>
      <Icon icon={ChevronDown} size={14} tone="ink-2" />
      {keys && <Kbd>{keys}</Kbd>}
    </button>
  );
}
