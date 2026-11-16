import { cx } from "../../../lib/cx";
import { Kbd } from "../../atoms";
import styles from "./AddressBar.module.css";

export interface AddressBarProps {
  /** The account's addresses, in the order shown, with how much mail waits at each. */
  addresses: { email: string; count: number }[];
  /** Waiting in the whole account. */
  total: number;
  /** The address shown alone; all of them when undefined. */
  selected?: string;
  onSelect: (email: string | undefined) => void;
}

/**
 * Under the account row, for an account with several addresses: the inbox of one address at a
 * time, like a separate account. ⌥0 shows all of them again, ⌥1–⌥9 picks one.
 */
export function AddressBar({ addresses, total, selected, onSelect }: AddressBarProps) {
  const tab = (label: string, count: number, on: boolean, keys: string | undefined, pick: () => void) => (
    <button key={label} type="button" role="tab" aria-selected={on} className={cx(styles.tab, on && styles.on)} onClick={pick}>
      <span className={styles.label}>{label}</span>
      {count > 0 && <span className={styles.count}>{count}</span>}
      {keys && <Kbd>{keys}</Kbd>}
    </button>
  );
  return (
    <div className={styles.bar} role="tablist" aria-label="Addresses">
      {tab("All addresses", total, !selected, "⌥0", () => onSelect(undefined))}
      {addresses.map((a, i) => tab(a.email, a.count, a.email === selected, i < 9 ? `⌥${i + 1}` : undefined, () => onSelect(a.email)))}
    </div>
  );
}
