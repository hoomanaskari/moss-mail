import type { LucideIcon } from "lucide-react";
import { Icon } from "../../atoms";
import { ShortcutHint } from "../ShortcutHint/ShortcutHint";
import styles from "./EmptyState.module.css";

export interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  /** What normally shows up here, so an empty list never looks broken. */
  description: string;
  /** One or two ways forward, each with its key. */
  steps?: { keys: string; label: string; onClick: () => void }[];
}

/** Shown in place of a list with nothing in it. Always says what belongs here and what to do next. */
export function EmptyState({ icon, title, description, steps = [] }: EmptyStateProps) {
  return (
    <div className={styles.empty} role="status">
      <span className={styles.tile}>
        <Icon icon={icon} size={22} tone="ink-2" />
      </span>
      <div className={styles.text}>
        <p className={styles.title}>{title}</p>
        <p className={styles.description}>{description}</p>
      </div>
      {steps.length > 0 && (
        <div className={styles.steps}>
          {steps.map((s) => (
            <ShortcutHint key={s.keys} keys={s.keys} label={s.label} tone="ink-2" onClick={s.onClick} />
          ))}
        </div>
      )}
    </div>
  );
}
