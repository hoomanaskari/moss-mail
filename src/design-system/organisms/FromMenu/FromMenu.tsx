import { listenAppKeys } from "../../../state/appKeyboard";
import { useEffect, useRef } from "react";
import { Plus } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { Account } from "../../../data/types";
import { sendingAddresses } from "../../../lib/addresses";
import { keyDigit } from "../../../lib/keyDigit";
import { useMenuKeys } from "../../../state/useMenuKeys";
import { AccountBadge, Button, Kbd, Radio } from "../../atoms";
import styles from "./FromMenu.module.css";

export interface FromMenuProps {
  accounts: Account[];
  /** The address the email goes from now. */
  chosen: { accountId: string; email: string };
  onChoose: (accountId: string, email: string) => void;
  onClose: () => void;
  /** Opens Settings → Accounts, where addresses are added. */
  onEdit: () => void;
}

/**
 * Which address the email goes from (⌥F): every address you send from, under its account.
 * The first nine are numbered, so one key picks one; ↑ ↓ ↵ pick from the one it goes from now.
 */
export function FromMenu({ accounts, chosen, onChoose, onClose, onEdit }: FromMenuProps) {
  const groups = accounts.map((account) => ({ account, addresses: sendingAddresses(account) }));
  const all = groups.flatMap((g) => g.addresses);
  // The newest handlers, for the key listener that lives while the menu is open.
  const latest = useRef({ all, onChoose, onClose });
  latest.current = { all, onChoose, onClose };

  useEffect(() => {
    // Capture on window runs before the composer's own keys (Esc would leave the composer).
    const onKey = (e: KeyboardEvent) => {
      const { all, onChoose, onClose } = latest.current;
      // ⌥ may still be down from ⌥F, so read the digit from the physical key.
      const n = keyDigit(e) ?? 0;
      if (e.key === "Escape") onClose();
      else if (n >= 1 && n <= all.length) onChoose(all[n - 1].accountId, all[n - 1].email);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    const stopKeys = listenAppKeys(onKey, true);
    return () => stopKeys();
  }, []);

  const isChosen = (a: { accountId: string; email: string }) => a.accountId === chosen.accountId && a.email === chosen.email;
  const active = useMenuKeys(all.length, (i) => onChoose(all[i].accountId, all[i].email), Math.max(0, all.findIndex(isChosen)));

  let number = 0;
  return (
    <div className={styles.menu}>
      <div className={styles.heading} id="from-menu-title">
        Send from
      </div>
      <div role="radiogroup" aria-labelledby="from-menu-title" className={styles.groups}>
        {groups.map(({ account, addresses }) => (
          <div key={account.id} className={styles.group}>
            <div className={styles.account}>
              <AccountBadge letter={account.letter} color={account.color} size={18} />
              <span>{account.name}</span>
            </div>
            {addresses.map((a) => {
              const on = isChosen(a);
              const index = number;
              const key = ++number <= 9 ? String(number) : undefined;
              return (
                <label key={a.email} className={cx(styles.option, index === active && styles.active)}>
                  <Radio name="from" checked={on} onChange={() => onChoose(a.accountId, a.email)} />
                  <span className={styles.text}>
                    <span className={styles.address}>{a.email}</span>
                    <span className={styles.name}>{a.name}</span>
                  </span>
                  {key && <Kbd>{key}</Kbd>}
                </label>
              );
            })}
          </div>
        ))}
      </div>
      <div className={styles.footer}>
        <Button variant="link" icon={Plus} onClick={onEdit}>
          Add an address
        </Button>
        <span className={styles.hint}>Custom domains on iCloud, Fastmail or Gmail’s “Send mail as”</span>
      </div>
    </div>
  );
}
