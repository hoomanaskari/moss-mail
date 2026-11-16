import { X } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Avatar, Icon } from "../../atoms";
import styles from "./RecipientChip.module.css";

export interface RecipientChipProps {
  name: string;
  initials: string;
  /** Someone outside your organisation, highlighted so you notice before sending. */
  external?: boolean;
  onRemove?: () => void;
}

/** A person in the To / CC / BCC field. */
export function RecipientChip({ name, initials, external, onRemove }: RecipientChipProps) {
  return (
    <span className={cx(styles.chip, external && styles.external)}>
      <Avatar initials={initials} size={24} tone={external ? "external" : "default"} />
      <span className={styles.name}>{name}</span>
      {external && <span className={styles.ext}>· outside</span>}
      <button type="button" className={styles.remove} onClick={onRemove} aria-label={`Remove ${name}`}>
        <Icon icon={X} size={13} tone={external ? "due" : "ink-3"} />
      </button>
    </span>
  );
}
