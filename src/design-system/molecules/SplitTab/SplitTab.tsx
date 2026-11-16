import type { LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon } from "../../atoms";
import styles from "./SplitTab.module.css";

export interface SplitTabProps {
  label: string;
  count: number;
  /** How many are unread: shows a dot after the count. */
  unread?: number;
  /** Left out for a tab that opens another view (Drafts): it's then a plain button. */
  selected?: boolean;
  /** Before the label of one that opens another view, so it isn't read as a tab. */
  icon?: LucideIcon;
  onSelect: () => void;
}

/** One tab of the split inbox (Important, Other, Calendar…), or one that looks like it (Drafts). */
export function SplitTab({ label, count, unread = 0, selected, icon, onSelect }: SplitTabProps) {
  return (
    <button
      type="button"
      role={selected === undefined ? undefined : "tab"}
      aria-selected={selected}
      className={cx(styles.tab, unread > 0 && styles.unread, selected && styles.selected, icon && styles.link)}
      onClick={onSelect}
    >
      {icon && <Icon icon={icon} size={14} tone="ink-3" />}
      <span className={styles.label}>{label}</span>
      <span className={styles.count}>{count}</span>
      {unread > 0 && (
        <span className={styles.dot}>
          <span className={styles.hidden}>, {unread} unread</span>
        </span>
      )}
    </button>
  );
}
