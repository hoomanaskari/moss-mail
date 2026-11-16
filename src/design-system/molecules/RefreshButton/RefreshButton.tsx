import { useState } from "react";
import { RotateCw } from "lucide-react";
import { Icon } from "../../atoms";
import styles from "./RefreshButton.module.css";

/** Checks every account for new mail now, beside the command hint in the top bar. */
export function RefreshButton({ onRefresh }: { onRefresh: () => void }) {
  // Each click turns the arrow once, so a check that finds nothing still shows it ran.
  const [turns, setTurns] = useState(0);
  return (
    <button
      type="button"
      className={styles.refresh}
      aria-label="Check for new mail"
      title="Check for new mail"
      onClick={() => {
        setTurns((n) => n + 1);
        onRefresh();
      }}
    >
      <span key={turns} className={turns ? styles.turn : undefined}>
        <Icon icon={RotateCw} size={15} tone="ink-3" />
      </span>
    </button>
  );
}
