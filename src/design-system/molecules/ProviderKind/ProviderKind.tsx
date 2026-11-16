import { ProviderTile } from "../../atoms";
import styles from "./ProviderKind.module.css";

export interface ProviderKindProps {
  letter?: string;
  name: string;
  /** After a dot, e.g. "Advanced" or the account's address. */
  extra?: string;
}

/** Small "which provider" line above a sign-in step. */
export function ProviderKind({ letter, name, extra }: ProviderKindProps) {
  return (
    <div className={styles.kind}>
      <ProviderTile letter={letter} size={24} />
      <span className={styles.name}>{name}</span>
      {extra && (
        <>
          <span className={styles.dot} aria-hidden>
            ·
          </span>
          <span className={styles.extra}>{extra}</span>
        </>
      )}
    </div>
  );
}
