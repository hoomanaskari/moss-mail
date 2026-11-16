import type { LucideIcon } from "lucide-react";
import { ChevronDown, X } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Avatar, Icon, Kbd } from "../../atoms";
import styles from "./FilterChip.module.css";

export interface FilterChipProps {
  label: string;
  /** Its shortcut, e.g. "⌥P". */
  keys: string;
  icon?: LucideIcon;
  /** Initials, for a filter on a person (shown instead of the icon). */
  person?: string;
  /** Narrowing the search right now. */
  active?: boolean;
  /** Shows a chevron: pressing it steps through choices. */
  menu?: boolean;
  /** Shows an × mark: pressing it drops the filter. */
  removable?: boolean;
  onClick: () => void;
}

/** One search filter, e.g. "Has attachment ⌥P". Active filters are raised with a teal outline. */
export function FilterChip({ label, keys, icon, person, active, menu, removable, onClick }: FilterChipProps) {
  return (
    <button
      type="button"
      aria-pressed={!!active}
      className={cx(styles.chip, active && styles.active, person && styles.person)}
      onClick={onClick}
    >
      {person ? <Avatar initials={person} size={22} /> : icon && <Icon icon={icon} size={15} tone={active ? "ink" : "ink-2"} />}
      <span className={styles.label}>{label}</span>
      {menu && <Icon icon={ChevronDown} size={14} tone="ink-3" />}
      {removable && <Icon icon={X} size={13} tone="ink-3" />}
      <Kbd>{keys}</Kbd>
    </button>
  );
}
