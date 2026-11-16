import { listenAppKeys } from "../../../state/appKeyboard";
import { useEffect, useRef } from "react";
import { CalendarDays, Inbox, Newspaper, Receipt, Sprout, Star, type LucideIcon } from "lucide-react";
import type { Split, SplitId } from "../../../data/types";
import { cx } from "../../../lib/cx";
import { useMenuKeys } from "../../../state/useMenuKeys";
import { Icon, Kbd } from "../../atoms";
import remind from "../RemindDialog/RemindDialog.module.css";
import menu from "../RemindMenu/RemindMenu.module.css";
import styles from "./MoveDialog.module.css";

export interface MoveDialogProps {
  splits: Split[];
  /** The email it is for. */
  subject?: string;
  /** The tab it is in now. */
  current?: SplitId;
  /** You placed it yourself: it can go back to Moss's sorting. */
  moved?: boolean;
  /** A tab, or null for Moss's sorting. */
  onPick: (split: SplitId | null) => void;
  onClose: () => void;
}

const icons: Record<SplitId, LucideIcon> = {
  important: Star,
  other: Inbox,
  calendar: CalendarDays,
  newsletters: Newspaper,
  receipts: Receipt,
};

/**
 * M: which tab an email belongs in, for good. ↵ moves it to Important at once (out of it to
 * Other when it is there); 1–5 pick a tab, 0 hands it back to Moss. Esc or a click beside it closes.
 */
export function MoveDialog({ splits, subject, current, moved, onPick, onClose }: MoveDialogProps) {
  const rows: (SplitId | null)[] = [...splits.map((s) => s.id), ...(moved ? [null] : [])];
  const start = current === "important" ? 1 : 0;
  const active = useMenuKeys(rows.length, (i) => onPick(rows[i]), start);
  const latest = useRef({ onPick, onClose, rows });
  latest.current = { onPick, onClose, rows };

  useEffect(() => {
    // Caught before the app's shortcuts, so these keys do nothing else.
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const { onPick, onClose, rows } = latest.current;
      const n = Number(e.key);
      if (e.key === "Escape") onClose();
      else if (/^[1-9]$/.test(e.key) && n <= rows.filter(Boolean).length) onPick(rows[n - 1]);
      else if (e.key === "0" && rows.includes(null)) onPick(null);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    const stopKeys = listenAppKeys(onKey, true);
    return () => stopKeys();
  }, []);

  return (
    <div className={remind.layer}>
      <div className={remind.scrim} data-testid="move-scrim" onClick={onClose} />
      <div className={remind.card} role="dialog" aria-modal="true" aria-labelledby="move-title">
        <div className={remind.head}>
          <h2 id="move-title" className={remind.title}>
            Move to
          </h2>
          {subject && <p className={remind.subject}>{subject}</p>}
        </div>
        <div className={menu.menu} role="menu" aria-label="Tabs">
          {splits.map((s, i) => (
            <button
              key={s.id}
              type="button"
              role="menuitemradio"
              aria-checked={s.id === current}
              data-active={i === active}
              className={cx(menu.item, i === active && menu.active)}
              onClick={() => onPick(s.id)}
            >
              <Icon icon={icons[s.id]} size={15} tone="ink-3" />
              <span className={menu.label}>{s.label}</span>
              {s.id === current && <span className={menu.detail}>Here</span>}
              <Kbd>{String(i + 1)}</Kbd>
            </button>
          ))}
          {moved && (
            <button
              type="button"
              role="menuitem"
              data-active={active === splits.length}
              className={cx(menu.item, styles.moss, active === splits.length && menu.active)}
              onClick={() => onPick(null)}
            >
              <Icon icon={Sprout} size={15} tone="ink-3" />
              <span className={menu.label}>Let Moss sort it</span>
              <Kbd>0</Kbd>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
