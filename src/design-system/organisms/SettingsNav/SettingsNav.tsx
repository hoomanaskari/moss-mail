import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, Kbd } from "../../atoms";
import styles from "./SettingsNav.module.css";

export interface SettingsNavItem<T extends string> {
  id: T;
  label: string;
  icon: LucideIcon;
  keys: string;
}

export interface SettingsNavProps<T extends string> {
  items: SettingsNavItem<T>[];
  selected: T;
  onSelect: (id: T) => void;
  /** Quiet lines at the bottom: the version, and checking for updates. */
  footer?: ReactNode;
}

/** The list of Settings sections down the left, each with its ⌘ key. The current one is raised, like the inbox cursor. */
export function SettingsNav<T extends string>({ items, selected, onSelect, footer }: SettingsNavProps<T>) {
  return (
    <nav className={styles.nav} aria-label="Settings">
      {items.map((item) => {
        const on = item.id === selected;
        return (
          <button
            key={item.id}
            type="button"
            className={cx(styles.item, on && styles.on)}
            aria-current={on ? "page" : undefined}
            onClick={() => onSelect(item.id)}
          >
            {on && <span className={styles.cursor} aria-hidden />}
            <Icon icon={item.icon} size={17} tone={on ? "ink" : "ink-3"} />
            <span className={styles.label}>{item.label}</span>
            <Kbd>{item.keys}</Kbd>
          </button>
        );
      })}
      <span className={styles.push} />
      {footer && <div className={styles.footer}>{footer}</div>}
    </nav>
  );
}
