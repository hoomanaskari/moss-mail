import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Icon, Kbd } from "../../atoms";
import styles from "./Toast.module.css";

export interface ToastProps {
  icon?: LucideIcon;
  /** Shown in place of the icon, e.g. a countdown. */
  lead?: ReactNode;
  children: ReactNode;
  /** One way to act on it, such as Undo, with its key if it has one. */
  action?: { label: string; keys?: string; onClick: () => void };
}

/** A short note after an action, e.g. "Moved to Done · Undo ⌘Z". Read out without taking the focus. */
export function Toast({ icon, lead, children, action }: ToastProps) {
  return (
    <div className={styles.toast} role="status">
      {lead ?? (icon && <Icon icon={icon} size={16} tone="inherit" />)}
      <span className={styles.text}>{children}</span>
      {action && (
        <button type="button" className={styles.action} onClick={action.onClick}>
          {action.label}
          {action.keys && <Kbd tone="onInvert">{action.keys}</Kbd>}
        </button>
      )}
    </div>
  );
}
