import { forwardRef } from "react";
import type { LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, Kbd } from "../../atoms";
import styles from "./PaletteItem.module.css";

export interface PaletteItemProps {
  label: string;
  sub?: string;
  icon?: LucideIcon;
  shortcut?: string;
  /** AI suggestions are tinted green when highlighted. */
  ai?: boolean;
  highlighted: boolean;
  onChoose: () => void;
}

/** One result in the command palette. */
export const PaletteItem = forwardRef<HTMLButtonElement, PaletteItemProps>(function PaletteItem(
  { label, sub, icon, shortcut, ai, highlighted, onChoose },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      role="option"
      aria-selected={highlighted}
      className={cx(styles.item, highlighted && (ai ? styles.aiOn : styles.on))}
      onClick={onChoose}
    >
      {icon && <Icon icon={icon} size={18} tone={highlighted && ai ? "ai" : "ink-2"} />}
      <span className={styles.text}>
        <span className={styles.label}>{label}</span>
        {sub && <span className={styles.sub}>{sub}</span>}
      </span>
      {highlighted ? <Kbd>↵</Kbd> : shortcut && <Kbd>{shortcut}</Kbd>}
    </button>
  );
});
