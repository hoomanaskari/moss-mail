import { cx } from "../../../lib/cx";
import { Avatar, Kbd } from "../../atoms";
import styles from "./AutocompleteOption.module.css";

export interface AutocompleteOptionProps {
  name: string;
  email: string;
  initials: string;
  external?: boolean;
  highlighted: boolean;
  onChoose: () => void;
}

/** A suggested person while typing in a recipient field. */
export function AutocompleteOption({ name, email, initials, external, highlighted, onChoose }: AutocompleteOptionProps) {
  return (
    <button
      type="button"
      role="option"
      aria-selected={highlighted}
      className={cx(styles.option, highlighted && styles.on)}
      onMouseDown={(e) => {
        e.preventDefault();
        onChoose();
      }}
    >
      <Avatar initials={initials} size={28} />
      <span className={styles.text}>
        <span className={styles.name}>{name}</span>
        <span className={cx(styles.email, external && styles.external)}>{email}</span>
      </span>
      {highlighted && <Kbd>↵</Kbd>}
    </button>
  );
}
