import { FileText, ListChecks, List, Lock, Minus, PencilLine, type LucideIcon } from "lucide-react";
import type { SummaryKind } from "../../../services/summaries";
import { cx } from "../../../lib/cx";
import { useMenuKeys } from "../../../state/useMenuKeys";
import { Icon, Kbd } from "../../atoms";
import styles from "./SummaryMenu.module.css";

/** The kinds in menu order, with their keys 1–5. */
export const kindChoices: { kind: SummaryKind; label: string; sub: string; icon: LucideIcon }[] = [
  { kind: "short", label: "Short", sub: "One or two lines", icon: Minus },
  { kind: "detailed", label: "Detailed", sub: "The main points, in order", icon: FileText },
  { kind: "actions", label: "Action points", sub: "What you need to do, with dates", icon: ListChecks },
  { kind: "facts", label: "Key facts", sub: "People, dates, amounts, links", icon: List },
  { kind: "custom", label: "Custom…", sub: "Say how you want it", icon: PencilLine },
];

export interface SummaryMenuProps {
  /** The kind on the card now, marked. */
  current?: SummaryKind;
  onPick: (kind: SummaryKind) => void;
  onClose: () => void;
}

/** "Summarise as": the five kinds, 1–5 or ↑ ↓ ↵, under the Summarise button. Opens on the kind shown. */
export function SummaryMenu({ current, onPick, onClose }: SummaryMenuProps) {
  const active = useMenuKeys(kindChoices.length, (i) => onPick(kindChoices[i].kind), Math.max(0, kindChoices.findIndex((c) => c.kind === current)));
  return (
    <div role="menu" aria-label="Summarise as" className={styles.menu}>
      <div className={styles.head}>Summarise as</div>
      {kindChoices.map((c, i) => {
        const on = c.kind === current;
        return (
          <button
            key={c.kind}
            type="button"
            role="menuitemradio"
            aria-checked={on}
            className={cx(styles.item, on && styles.on, i === active && styles.active)}
            onClick={() => onPick(c.kind)}
          >
            <Icon icon={c.icon} size={16} tone={on ? "ai" : "ink-3"} />
            <span className={styles.words}>
              <span className={styles.label}>{c.label}</span>
              <span className={styles.sub}>{c.sub}</span>
            </span>
            <Kbd>{String(i + 1)}</Kbd>
          </button>
        );
      })}
      <div className={styles.foot}>
        <Icon icon={Lock} size={13} tone="ink-3" />
        <span className={styles.private}>Runs on this Mac</span>
        <button type="button" className={styles.close} onClick={onClose}>
          <Kbd>Esc</Kbd>
          Close
        </button>
      </div>
    </div>
  );
}
