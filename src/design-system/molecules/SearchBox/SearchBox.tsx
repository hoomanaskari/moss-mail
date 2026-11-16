import { Search } from "lucide-react";
import { Icon } from "../../atoms";
import styles from "./SearchBox.module.css";

export interface SearchBoxProps {
  value: string;
  onChange: (value: string) => void;
  /** e.g. "7 results". */
  count?: string;
}

/** The big search field at the top of Search, focused as soon as it opens. */
export function SearchBox({ value, onChange, count }: SearchBoxProps) {
  return (
    <label className={styles.box}>
      <Icon icon={Search} size={20} tone="ink-3" />
      <input
        className={styles.input}
        type="search"
        aria-label="Search all mail"
        placeholder="Search all mail"
        autoFocus
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      {count && <span className={styles.count}>{count}</span>}
    </label>
  );
}
