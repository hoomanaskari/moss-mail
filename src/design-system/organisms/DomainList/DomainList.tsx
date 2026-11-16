import type { ResendChoice } from "../../../state/addAccount";
import { cx } from "../../../lib/cx";
import { Checkbox, Select } from "../../atoms";
import { ListCard, ListRow } from "../../molecules";
import styles from "./DomainList.module.css";

export interface DomainListProps {
  domains: ResendChoice[];
  cursor: number;
  onToggle: (index: number) => void;
  /** null receives every address on the domain. */
  onAddresses: (index: number, only: string | null) => void;
  /** A click on a row moves the cursor there. Pointing at one never does. */
  onCursor: (index: number) => void;
}

/** Domains on a Resend key, each ticked on or off, with which addresses to receive. */
export function DomainList({ domains, cursor, onToggle, onAddresses, onCursor }: DomainListProps) {
  return (
    <ListCard as="ul" aria-label="Domains">
      {domains.map((d, i) => (
        <ListRow
          as="li"
          key={d.name}
          selected={i === cursor}
          className={cx(styles.row, !d.chosen && styles.off)}
          onClick={() => onCursor(i)}
        >
          <Checkbox aria-label={d.name} checked={d.chosen} onChange={() => onToggle(i)} />
          <span className={styles.text}>
            <span className={styles.name}>{d.name}</span>
            <span className={styles.detail}>{d.detail}</span>
          </span>
          <Select
            aria-label={`Addresses for ${d.name}`}
            disabled={!d.chosen}
            value={d.only ?? "all"}
            options={[{ value: "all", label: "All addresses" }, ...d.addresses.map((a) => ({ value: a, label: `Only ${a}@${d.name}` }))]}
            onChange={(v) => onAddresses(i, v === "all" ? null : v)}
          />
        </ListRow>
      ))}
    </ListCard>
  );
}
