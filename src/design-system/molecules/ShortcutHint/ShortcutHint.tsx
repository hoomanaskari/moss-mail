import { cx } from "../../../lib/cx";
import { Kbd } from "../../atoms";
import styles from "./ShortcutHint.module.css";

export interface ShortcutHintProps {
  keys: string;
  label: string;
  /** Label text size: `sm` 13px, `md` 14px. */
  size?: "sm" | "md";
  tone?: "ink-2" | "ink-3" | "ai";
  /** Put the label before the key ("Hide I") instead of after ("E Done"). */
  labelFirst?: boolean;
  /** Space between key and label. The design uses 6 or 8. */
  gap?: 6 | 8;
  onClick?: () => void;
  className?: string;
}

/** A key cap with what it does, e.g. [E] Done. */
export function ShortcutHint({
  keys,
  label,
  size = "md",
  tone = "ink-3",
  labelFirst,
  gap = 8,
  onClick,
  className,
}: ShortcutHintProps) {
  const text = <span className={cx(styles.label, styles[size], styles[tone])}>{label}</span>;
  const Tag = onClick ? "button" : "span";
  return (
    <Tag type={onClick ? "button" : undefined} className={cx(styles.hint, className)} style={{ gap }} onClick={onClick}>
      {labelFirst && text}
      <Kbd>{keys}</Kbd>
      {!labelFirst && text}
    </Tag>
  );
}
