import { Command } from "lucide-react";
import { Icon, Kbd } from "../../atoms";
import styles from "./CommandHint.module.css";

/** The "Ask AI or run a command" button in the top bar. Opens the command palette. */
export function CommandHint({ onOpen }: { onOpen: () => void }) {
  return (
    <button type="button" className={styles.hint} onClick={onOpen}>
      <Icon icon={Command} size={15} tone="ink-3" />
      <span className={styles.label}>Ask AI or run a command</span>
      <Kbd>⌘K</Kbd>
    </button>
  );
}
