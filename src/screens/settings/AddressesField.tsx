import { useState } from "react";
import { AtSign, Ban, Plus, Search, Undo2, X } from "lucide-react";
import type { Account, ProviderId } from "../../data/types";
import { blockedAddresses, isValidAddress, suggestedAddresses } from "../../lib/addresses";
import { Button, Icon, Pill, TextInput } from "../../design-system/atoms";
import { useStore } from "../../state/store";
import shared from "./settings.module.css";
import styles from "./AddressesField.module.css";

/** Where an account's other addresses come from, by provider. */
const where: Partial<Record<ProviderId, string>> = {
  icloud: "Set it up on icloud.com first (Custom Email Domain), then add it here. Moss also finds the ones you get mail at.",
  gmail: "Add it in Gmail first (Settings → Accounts → Send mail as). Signed in with your browser, Moss brings them in by itself.",
  fastmail: "Set it up in Fastmail first, then add it here. Moss also finds the ones you get mail at.",
  resend: "Any address at a domain you verified in Resend.",
};
const elsewhere = "Your provider has to let this account send from it. Moss also finds the ones you get mail at.";

/**
 * The name on mail from one address. The store keeps it trimmed, so the box holds what you typed
 * (a space at the end included) for as long as it still trims to the saved name.
 */
function AddressName({ name = "", placeholder, email, onRename }: { name?: string; placeholder: string; email: string; onRename: (name: string) => void }) {
  const [typed, setTyped] = useState(name);
  return (
    <TextInput
      compact
      className={styles.name}
      aria-label={`Name on mail from ${email}`}
      placeholder={placeholder}
      value={typed.trim() === name ? typed : name}
      onChange={(e) => {
        setTyped(e.target.value);
        onRename(e.target.value);
      }}
    />
  );
}

const emails = (n: number) => `${n} ${n === 1 ? "email" : "emails"} came to it`;

/**
 * Settings → Accounts → Addresses: one login, many addresses. The ones you send from (and the
 * name each shows), the ones Moss found in your mail, and the ones whose mail goes to Blocked.
 */
export function AddressesField({ account }: { account: Account }) {
  const { addAddress, renameAddress, removeAddress, blockAddress, discoverAddresses } = useStore.getState();
  const [typed, setTyped] = useState("");
  const [problem, setProblem] = useState<string>();
  const [looking, setLooking] = useState(false);
  const sending = (account.addresses ?? []).filter((a) => a.state === "added" && !a.blocked);
  const found = suggestedAddresses(account);
  const blocked = blockedAddresses(account);

  const add = () => {
    const email = typed.trim().toLowerCase();
    if (email === account.address.toLowerCase()) return setProblem(`That’s the address ${account.name} signs in with`);
    if (!isValidAddress(email)) return setProblem("That isn’t a full address, like hi@yourdomain.com");
    addAddress(account.id, email);
    setTyped("");
    setProblem(undefined);
  };

  const look = () => {
    setLooking(true);
    void discoverAddresses(account.id).finally(() => setLooking(false));
  };

  return (
    <div className={styles.field}>
      <span className={styles.label}>Addresses</span>
      <div className={styles.body}>
        <ul className={styles.list} aria-label="Addresses you send from">
          <li className={styles.row}>
            <Icon icon={AtSign} size={15} tone="ink-3" />
            <span className={styles.email}>{account.address}</span>
            <span className={shared.small}>Signs in</span>
          </li>
          {sending.map((a) => (
            <li key={a.email} className={styles.row}>
              <Icon icon={AtSign} size={15} tone="ink-3" />
              <span className={styles.email}>{a.email}</span>
              {a.source === "gmail" && <Pill variant="label">From Gmail</Pill>}
              <span className={styles.spacer} />
              <AddressName name={a.name} placeholder={account.senderName} email={a.email} onRename={(name) => renameAddress(account.id, a.email, name)} />
              <Button variant="quiet" icon={X} aria-label={`Remove ${a.email}`} onClick={() => removeAddress(account.id, a.email)}>
                Remove
              </Button>
            </li>
          ))}
        </ul>

        {found.length > 0 && (
          <ul className={styles.list} aria-label="Found in your mail">
            {found.map((a) => (
              <li key={a.email} className={styles.row}>
                <Pill variant="ai">Found</Pill>
                <span className={styles.text}>
                  <span className={styles.email}>{a.email}</span>
                  <span className={shared.small}>{emails(a.seen)}</span>
                </span>
                <Button variant="secondary" icon={Plus} aria-label={`Send from ${a.email}`} onClick={() => addAddress(account.id, a.email)}>
                  Send from it
                </Button>
                <Button variant="quiet" aria-label={`${a.email} isn’t mine`} onClick={() => removeAddress(account.id, a.email)}>
                  Not mine
                </Button>
                <Button variant="quiet" icon={Ban} aria-label={`Block mail to ${a.email}`} onClick={() => blockAddress(account.id, a.email, true)}>
                  Block
                </Button>
              </li>
            ))}
          </ul>
        )}

        {blocked.length > 0 && (
          <ul className={styles.list} aria-label="Blocked addresses">
            {blocked.map((a) => (
              <li key={a.email} className={styles.row}>
                <Icon icon={Ban} size={15} tone="ink-3" />
                <span className={styles.text}>
                  <span className={styles.email}>{a.email}</span>
                  <span className={shared.small}>Its mail goes to Blocked</span>
                </span>
                <Button variant="quiet" icon={Undo2} aria-label={`Unblock ${a.email}`} onClick={() => blockAddress(account.id, a.email, false)}>
                  Unblock
                </Button>
              </li>
            ))}
          </ul>
        )}

        <div className={styles.add}>
          <TextInput
            compact
            className={styles.addInput}
            aria-label="Another address"
            placeholder="hi@yourdomain.com"
            value={typed}
            invalid={!!problem}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Enter") return;
              e.preventDefault();
              add();
            }}
          />
          <Button variant="outline" icon={Plus} onClick={add}>
            Add
          </Button>
          <Button variant="quiet" icon={Search} onClick={look} disabled={looking}>
            {looking ? "Looking…" : "Look in my mail"}
          </Button>
        </div>
        {problem && (
          <p className={styles.problem} role="alert">
            {problem}
          </p>
        )}
        <p className={styles.hint}>{where[account.connection?.provider as ProviderId] ?? elsewhere}</p>
      </div>
    </div>
  );
}
