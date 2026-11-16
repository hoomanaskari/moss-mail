import { ShortcutHint } from "../../molecules";
import styles from "./ShortcutBar.module.css";

export interface ShortcutBarHint {
  keys: string;
  label: string;
}

const inboxHints: ShortcutBarHint[] = [
  { keys: "↵", label: "Open" },
  { keys: "E", label: "Done" },
  { keys: "H", label: "Remind me" },
  { keys: "C", label: "New email" },
  { keys: "/", label: "Search" },
  { keys: "⌘J", label: "Ask Moss" },
];

/** Always-visible reminder of the main keys, so nobody has to memorise them. */
export function ShortcutBar({ hints = inboxHints }: { hints?: ShortcutBarHint[] }) {
  return (
    <footer className={styles.bar}>
      {hints.map((h) => (
        <ShortcutHint key={h.label} keys={h.keys} label={h.label} />
      ))}
    </footer>
  );
}
