import { useState } from "react";
import { Check, GripVertical, Plus, Trash2, TriangleAlert } from "lucide-react";
import { accountColorOrder, type Account } from "../../data/types";
import { cx } from "../../lib/cx";
import { AccountBadge, Button, Icon, Kbd, Pill, TextInput } from "../../design-system/atoms";
import { ColorSwatches, ListCard, ListRow, ScreenHeading, ShortcutHint } from "../../design-system/molecules";
import { syncProblemLabel, useSync } from "../../services/live";
import { useStore } from "../../state/store";
import { useScreenKeys } from "../../state/useScreenKeys";
import { AddressesField } from "./AddressesField";
import shared from "./settings.module.css";
import styles from "./AccountsSection.module.css";

/** Settings → Accounts: their order (which is also ⌘1–⌘9), name, colour, and removing one. */
export function AccountsSection() {
  const accounts = useStore((s) => s.accounts);
  const problems = useSync((s) => s.problems);
  const { updateAccount, moveAccount, removeAccount, openAddAccount } = useStore.getState();
  const [selectedId, setSelectedId] = useState(accounts[0]?.id);
  const [confirmingId, setConfirmingId] = useState<string>();
  const removals = useStore((s) => s.accountRemovals);
  const index = Math.max(0, accounts.findIndex((a) => a.id === selectedId));
  const account = accounts[index] as Account | undefined;
  const confirming = account?.id === confirmingId;
  const removal = account && removals[account.id];
  // Resend keeps received mail for 30 days, so this Mac holds the only copy of older mail.
  const onlyHere = account?.connection?.provider === "resend";

  const select = (id: string) => {
    setSelectedId(id);
    setConfirmingId(undefined);
  };

  const remove = () => {
    if (!account || removal?.pending) return;
    if (!confirming) return setConfirmingId(account.id);
    const next = accounts[index + 1] ?? accounts[index - 1];
    const id = account.id;
    void removeAccount(id, removal?.unsentChanges).then((removed) => {
      if (!removed) return;
      setSelectedId((current) => current === id ? next?.id : current);
      setConfirmingId((current) => current === id ? undefined : current);
    });
  };

  const recolour = (a: Account, step: 1 | -1) => {
    const i = accountColorOrder.indexOf(a.color);
    updateAccount(a.id, { color: accountColorOrder[(i + step + accountColorOrder.length) % accountColorOrder.length] });
  };

  useScreenKeys((k, e) => {
    if (k.key === "escape" && confirming) {
      setConfirmingId(undefined);
      e.stopImmediatePropagation();
      return true;
    }
    if (k.mod && k.key === "n") return openAddAccount(), true;
    if (k.mod && k.key === "backspace") return remove(), true;
    if (!account || k.typing || k.mod) return;
    if (k.alt && (k.key === "arrowup" || k.key === "arrowdown")) return moveAccount(account.id, k.key === "arrowup" ? -1 : 1), true;
    if (k.alt) return;
    if (k.key === "arrowdown" || k.key === "arrowup") {
      const next = accounts[index + (k.key === "arrowdown" ? 1 : -1)];
      if (next) select(next.id);
      return true;
    }
    if (k.key === "arrowleft" || k.key === "arrowright") return recolour(account, k.key === "arrowright" ? 1 : -1), true;
  });

  return (
    <div className={shared.page}>
      <ScreenHeading
        title="Accounts"
        sub="They show in the inbox in this order. ⌘1 opens the first one, ⌘2 the second, and so on."
      />
      {accounts.length > 0 && (
        <ListCard role="listbox" aria-label="Accounts">
          {accounts.map((a, i) => {
            const on = a.id === account?.id;
            const problem = problems[a.id] && syncProblemLabel[problems[a.id].problem];
            return (
              <ListRow
                key={a.id}
                role="option"
                aria-selected={on}
                selected={on}
                className={styles.row}
                onClick={() => select(a.id)}
              >
                <Icon icon={GripVertical} size={16} tone="ink-3" />
                <AccountBadge letter={a.letter} color={a.color} size={28} />
                <div className={shared.text}>
                  <span className={shared.name}>{a.name}</span>
                  <span className={shared.small}>{a.address}</span>
                </div>
                <div className={styles.provider}>
                  <span className={styles.summary}>{a.connection?.summary ?? "Email account"}</span>
                  {a.connection?.signedOut ? (
                    <span className={styles.status}>
                      <Pill variant="due">Signed out</Pill>
                      <Button variant="link" onClick={openAddAccount}>
                        Sign in again
                      </Button>
                    </span>
                  ) : (
                    <span className={cx(styles.status, styles.ok)}>
                      <Icon icon={problem ? TriangleAlert : Check} size={13} tone={problem ? "due" : "ink-3"} />
                      <span className={shared.small}>{problem || "Up to date"}</span>
                    </span>
                  )}
                </div>
                {i < 9 && <Kbd>{`⌘${i + 1}`}</Kbd>}
              </ListRow>
            );
          })}
        </ListCard>
      )}
      {account && (
        <section className={styles.edit} aria-label={`Editing ${account.name}`}>
          <div className={styles.editHead}>
            <AccountBadge letter={account.letter} color={account.color} size={20} />
            <span className={shared.name}>Editing {account.name}</span>
            <span className={shared.sub}>{account.address}</span>
          </div>
          <div className={styles.field}>
            <label className={styles.label} htmlFor="account-name">
              Name
            </label>
            <TextInput
              id="account-name"
              compact
              className={styles.nameInput}
              value={account.name}
              onChange={(e) => {
                const name = e.target.value;
                updateAccount(account.id, { name, letter: name.trim()[0]?.toUpperCase() ?? account.letter });
              }}
            />
            <span className={shared.small}>Shown on the account tab</span>
          </div>
          <div className={cx(styles.field, styles.colours)}>
            <span className={styles.label}>Colour</span>
            <ColorSwatches label="Colour" value={account.color} onChange={(color) => updateAccount(account.id, { color })} />
            <span className={styles.spacer} />
            <ShortcutHint keys="← →" label="Change" onClick={() => recolour(account, 1)} />
          </div>
          <div className={styles.field}>
            <span className={styles.label}>Order</span>
            <ShortcutHint keys="⌥↑" label="Move up" tone="ink-2" onClick={() => moveAccount(account.id, -1)} />
            <ShortcutHint keys="⌥↓" label="Move down" tone="ink-2" onClick={() => moveAccount(account.id, 1)} />
          </div>
          <AddressesField account={account} />
          <div className={cx(styles.field, styles.remove)}>
            <span className={styles.label}>Remove</span>
            <p className={styles.removeText}>
              {onlyHere
                ? "Deletes this account’s mail from this Mac, then removes its API key from the Keychain. Moss will show a reminder if credential cleanup has to wait. Resend deletes received mail after 30 days, so older mail is only here: it’s gone for good."
                : "Deletes this account’s mail from this Mac, then removes its saved sign-in from the Keychain. Moss will show a reminder if credential cleanup has to wait. Nothing changes on the server."}
            </p>
            <p className={styles.removeText}>Local draft recovery copies remain on this Mac. Review or discard them in Drafts.</p>
            {removal?.error && <p role="alert" className={styles.removeText}>
              {removal.error}{removal.unsentChanges && " Your mail server keeps your mail as it was."}
            </p>}
            <Button variant="secondary" icon={Trash2} shortcut="⌘⌫" onClick={remove} disabled={removal?.pending}>
              {removal?.pending ? "Removing…" : !confirming ? "Remove account…"
                : removal?.unsentChanges ? `Remove ${account.name} and lose them`
                  : onlyHere ? `Yes, delete ${account.name} for good` : `Yes, remove ${account.name}`}
            </Button>
          </div>
        </section>
      )}
      <div>
        <Button variant="outline" size="lg" icon={Plus} shortcut="⌘N" onClick={openAddAccount}>
          Add account
        </Button>
      </div>
    </div>
  );
}
