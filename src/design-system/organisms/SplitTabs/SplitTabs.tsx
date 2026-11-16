import type { ReactNode } from "react";
import type { Split, SplitId } from "../../../data/types";
import { SplitTab } from "../../molecules";
import styles from "./SplitTabs.module.css";

export interface SplitTabsProps {
  splits: Split[];
  counts: Record<string, number>;
  /** Unread emails in each split; a split with any shows a dot. */
  unread?: Record<string, number>;
  selected: SplitId;
  onSelect: (id: SplitId) => void;
  /** Sits at the far right of the row, outside the tablist (the Drafts tab). */
  trailing?: ReactNode;
}

/** The split inbox: Important, Other, Calendar, Newsletters, Receipts. */
export function SplitTabs({ splits, counts, unread = {}, selected, onSelect, trailing }: SplitTabsProps) {
  return (
    <div className={styles.row}>
      <div className={styles.tabs} role="tablist" aria-label="Inbox splits">
        {splits.map((s) => (
          <SplitTab key={s.id} label={s.label} count={counts[s.id] ?? 0} unread={unread[s.id]} selected={s.id === selected} onSelect={() => onSelect(s.id)} />
        ))}
      </div>
      {trailing}
    </div>
  );
}
