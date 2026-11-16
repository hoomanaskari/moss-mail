import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ChevronLeft, ChevronRight, Plus } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { Account } from "../../../data/types";
import { Icon } from "../../atoms";
import { AccountTab } from "../../molecules";
import styles from "./AccountSwitcher.module.css";

export interface AccountSwitcherProps {
  accounts: Account[];
  counts: Record<string, number>;
  /** Accounts that aren't syncing, by id: what their tab says instead of the address. */
  problems?: Record<string, string>;
  /** "all" or an account id. */
  selected: string;
  onSelect: (id: string) => void;
  onAdd: () => void;
  /** Show each tab's key (⌘0 for All accounts, ⌘1–⌘9 for the rest), e.g. while ⌘ is held. */
  showShortcuts?: boolean;
  /** Pinned to the right end of the row, e.g. progress for the selected account. */
  trailing?: ReactNode;
}

const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/**
 * Row of accounts above the inbox. "All accounts" merges every inbox into one list.
 * With more accounts than fit, the row scrolls sideways: a fade and an arrow show which
 * side has more, a mouse wheel scrolls it, and the selected account is always brought into view.
 */
export function AccountSwitcher({ accounts, counts, problems = {}, selected, onSelect, onAdd, showShortcuts, trailing }: AccountSwitcherProps) {
  const tabsRef = useRef<HTMLDivElement>(null);
  const [more, setMore] = useState({ left: false, right: false });

  const measure = useCallback(() => {
    const el = tabsRef.current;
    if (!el) return;
    const left = el.scrollLeft > 1;
    const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setMore((m) => (m.left === left && m.right === right ? m : { left, right }));
  }, []);

  useLayoutEffect(() => {
    const el = tabsRef.current;
    if (!el) return;
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    // A mouse wheel only scrolls up and down; turn that into sideways scrolling here.
    const onWheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX) || el.scrollWidth <= el.clientWidth) return;
      e.preventDefault();
      el.scrollLeft += e.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      observer.disconnect();
      el.removeEventListener("wheel", onWheel);
    };
  }, [measure, accounts.length]);

  // Bring the selected account into view, e.g. after ⌘5 picks one that is scrolled away.
  useEffect(() => {
    const el = tabsRef.current;
    const tab = el?.querySelector<HTMLElement>('[aria-selected="true"]');
    if (!el || !tab) return;
    const edge = 48; // keep it clear of the fade
    // Position of the tab within the scrolled content.
    const start = tab.getBoundingClientRect().left - el.getBoundingClientRect().left + el.scrollLeft;
    const end = start + tab.offsetWidth;
    if (start < el.scrollLeft + edge) el.scrollTo({ left: start - edge, behavior: reducedMotion() ? "auto" : "smooth" });
    else if (end > el.scrollLeft + el.clientWidth - edge)
      el.scrollTo({ left: end - el.clientWidth + edge, behavior: reducedMotion() ? "auto" : "smooth" });
  }, [selected, accounts.length]);

  const page = (direction: 1 | -1) => {
    const el = tabsRef.current;
    if (!el) return;
    el.scrollBy({ left: direction * el.clientWidth * 0.8, behavior: reducedMotion() ? "auto" : "smooth" });
  };

  return (
    <div className={styles.row}>
      <div className={styles.scroller}>
        <div
          ref={tabsRef}
          className={cx(styles.tabs, more.left && styles.fadeLeft, more.right && styles.fadeRight)}
          role="tablist"
          aria-label="Accounts"
          onScroll={measure}
        >
          <AccountTab
            name="All accounts"
            detail="Everything in one place"
            count={counts.all ?? 0}
            shortcut="⌘0"
            showShortcut={showShortcuts}
            selected={selected === "all"}
            onSelect={() => onSelect("all")}
          />
          {accounts.map((a, i) => (
            <AccountTab
              key={a.id}
              name={a.name}
              detail={a.address}
              problem={problems[a.id]}
              count={counts[a.id] ?? 0}
              account={{ letter: a.letter, color: a.color }}
              shortcut={i < 9 ? `⌘${i + 1}` : undefined}
              showShortcut={showShortcuts}
              selected={selected === a.id}
              onSelect={() => onSelect(a.id)}
            />
          ))}
        </div>
        {more.left && (
          <button type="button" className={cx(styles.arrow, styles.arrowLeft)} onClick={() => page(-1)} aria-label="Show earlier accounts">
            <Icon icon={ChevronLeft} size={16} tone="ink-2" />
          </button>
        )}
        {more.right && (
          <button type="button" className={cx(styles.arrow, styles.arrowRight)} onClick={() => page(1)} aria-label="Show more accounts">
            <Icon icon={ChevronRight} size={16} tone="ink-2" />
          </button>
        )}
      </div>
      <button type="button" className={styles.add} onClick={onAdd} aria-label="Add account">
        <Icon icon={Plus} size={18} tone="ink-2" />
      </button>
      {trailing && <div className={styles.trailing}>{trailing}</div>}
    </div>
  );
}
