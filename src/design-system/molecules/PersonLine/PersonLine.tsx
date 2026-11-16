import { Copy, UserRound } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { AccountColor } from "../../../data/types";
import { AccountBadge, Button, Pill } from "../../atoms";
import styles from "./PersonLine.module.css";

export interface PersonLineProps {
  name: string;
  email: string;
  /** Shown when the person is you, so you can see which of your addresses got it. */
  account?: { letter: string; color: AccountColor };
  /** e.g. "Outside Northwind". */
  flag?: string;
  onCopy?: () => void;
  onContact?: () => void;
}

/** One person in an email's full recipient list. Hover shows Copy and Contact. */
export function PersonLine({ name, email, account, flag, onCopy, onContact }: PersonLineProps) {
  return (
    <div className={cx(styles.line)}>
      {account && <AccountBadge letter={account.letter} color={account.color} size={18} />}
      <span className={styles.name}>{name}</span>
      <span className={styles.email}>{email}</span>
      {flag && <Pill variant="flag">{flag}</Pill>}
      <span className={styles.spacer} />
      <span className={styles.actions}>
        <Button variant="mini" icon={Copy} onClick={onCopy}>
          Copy
        </Button>
        <Button variant="mini" icon={UserRound} onClick={onContact}>
          Contact
        </Button>
      </span>
    </div>
  );
}
