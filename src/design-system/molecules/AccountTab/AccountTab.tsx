import { Inbox } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { AccountColor } from "../../../data/types";
import { AccountBadge, Icon, Kbd } from "../../atoms";
import styles from "./AccountTab.module.css";

export interface AccountTabProps {
  name: string;
  detail: string;
  count: number;
  /** Why the account isn't syncing, e.g. "Not syncing". Shown in place of `detail`. */
  problem?: string;
  selected: boolean;
  /** Omit for the "All accounts" tab, which shows an inbox icon instead of a letter. */
  account?: { letter: string; color: AccountColor };
  /** The keys that pick this tab, e.g. "⌘1". */
  shortcut?: string;
  /** Show `shortcut` in place of the address, e.g. while ⌘ is held. */
  showShortcut?: boolean;
  onSelect: () => void;
}

/** One account in the switcher above the inbox. */
export function AccountTab({ name, detail, count, problem, selected, account, shortcut, showShortcut, onSelect }: AccountTabProps) {
  const isAll = !account;
  const keyShown = showShortcut && !!shortcut;
  return (
    <button
      type="button"
      role="tab"
      aria-selected={selected}
      className={cx(styles.tab, isAll && styles.all, selected && styles.selected)}
      style={account && selected ? { outlineColor: `var(--${account.color})` } : undefined}
      onClick={onSelect}
    >
      {account ? (
        <AccountBadge letter={account.letter} color={account.color} size={28} />
      ) : (
        <span className={styles.allIcon}>
          <Icon icon={Inbox} size={16} tone="ink" />
        </span>
      )}
      <span className={styles.text}>
        <span className={styles.name}>{name}</span>
        {/* The key floats over the address, which stays (unseen) to hold the tab's width steady. */}
        <span className={styles.detailRow}>
          <span
            className={cx(styles.detail, problem && styles.problem)}
            style={keyShown ? { visibility: "hidden" } : undefined}
            aria-hidden={keyShown || undefined}
          >
            {problem ?? detail}
          </span>
          {keyShown && <Kbd className={styles.key}>{shortcut}</Kbd>}
        </span>
      </span>
      <span className={styles.count}>{count}</span>
    </button>
  );
}
