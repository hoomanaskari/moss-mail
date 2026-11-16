import type { Provider } from "../../../data/providers";
import { Kbd, ProviderTile } from "../../atoms";
import { ListCard, ListRow, ShortcutHint } from "../../molecules";
import styles from "./ProviderList.module.css";

export interface ProviderListProps {
  providers: Provider[];
  cursor: number;
  onPick: (index: number) => void;
}

/** The email providers Moss can add, numbered so 1–7 pick one. Pointing at a row never moves the cursor. */
export function ProviderList({ providers, cursor, onPick }: ProviderListProps) {
  return (
    <ListCard role="listbox" aria-label="Providers">
      {providers.map((p, i) => (
        <ListRow
          key={p.id}
          role="option"
          aria-selected={i === cursor}
          selected={i === cursor}
          className={styles.row}
          onClick={() => onPick(i)}
        >
          <ProviderTile letter={p.letter} raised={i === cursor} />
          <span className={styles.text}>
            <span className={styles.name}>{p.name}</span>
            <span className={styles.how}>{p.how}</span>
          </span>
          {i === cursor && <ShortcutHint keys="↵" label="Continue" tone="ink-2" />}
          <Kbd>{String(i + 1)}</Kbd>
        </ListRow>
      ))}
    </ListCard>
  );
}
