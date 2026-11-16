import { Fragment, useRef, useState, type KeyboardEvent } from "react";
import { AtSign, Plus, Signature, Trash2 } from "lucide-react";
import type { Account } from "../../data/types";
import { sendingAddresses } from "../../lib/addresses";
import { cx } from "../../lib/cx";
import { AccountBadge, Button, Icon, Pill, Select, TextInput, Toggle } from "../../design-system/atoms";
import { ListCard, ListRow, ScreenHeading, SectionHeader, ShortcutHint } from "../../design-system/molecules";
import { useSignatures, usedBy, type AccountSignatures, type Signature as SignatureData } from "../../state/signatures";
import { useStore } from "../../state/store";
import { useScreenKeys } from "../../state/useScreenKeys";
import shared from "./settings.module.css";
import styles from "./SignaturesSection.module.css";

/** Web addresses and email addresses are shown as links. */
const isLink = (line: string) => /^(https?:\/\/)?[\w.-]+\.[a-z]{2,}(\/\S*)?$/i.test(line.trim()) || /\S+@\S+\.\S+/.test(line);

const MAX_LINES = 4;

/** Settings → Signatures: write them, and pick one per account for new emails and for replies. */
export function SignaturesSection() {
  const accounts = useStore((s) => s.accounts);
  const { signatures, byAccount, byAddress, letMossChoose, setLetMossChoose, setChoice, setAddressChoice, updateSignature, addSignature, deleteSignature } =
    useSignatures();
  const [selectedId, setSelectedId] = useState(signatures.find((x) => x.id === "studio")?.id ?? signatures[0]?.id);
  const nameRef = useRef<HTMLInputElement>(null);
  const lineRefs = useRef<(HTMLInputElement | null)[]>([]);
  const index = Math.max(0, signatures.findIndex((x) => x.id === selectedId));
  const sig = signatures[index];

  const create = () => {
    // New signatures start with your name, as you sign your mail.
    const id = addSignature(accounts.find((a) => a.senderName)?.senderName ?? "");
    setSelectedId(id);
    setTimeout(() => nameRef.current?.select());
  };

  const remove = () => {
    if (!sig) return;
    const next = signatures[index + 1] ?? signatures[index - 1];
    deleteSignature(sig.id);
    setSelectedId(next?.id);
  };

  useScreenKeys((k) => {
    if (k.mod && k.key === "n") return create(), true;
    if (k.mod && k.key === "backspace" && !k.typing) return remove(), true;
    if (k.typing || k.mod || k.alt || !sig) return;
    if (k.key === "arrowdown" || k.key === "arrowup") {
      const next = signatures[index + (k.key === "arrowdown" ? 1 : -1)];
      if (next) setSelectedId(next.id);
      return true;
    }
    if (k.key === "enter") {
      // The name field is always there while a signature is selected.
      nameRef.current!.focus();
      return true;
    }
  });

  const editLine = (sig: SignatureData, i: number, value: string) =>
    updateSignature(sig.id, { lines: sig.lines.map((l, j) => (j === i ? value : l)) });

  const onLineKey = (sig: SignatureData, i: number, e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && sig.lines.length < MAX_LINES) {
      e.preventDefault();
      updateSignature(sig.id, { lines: [...sig.lines.slice(0, i + 1), "", ...sig.lines.slice(i + 1)] });
      setTimeout(() => lineRefs.current[i + 1]?.focus());
    } else if (e.key === "Backspace" && sig.lines[i] === "" && sig.lines.length > 1) {
      e.preventDefault();
      updateSignature(sig.id, { lines: sig.lines.filter((_, j) => j !== i) });
      setTimeout(() => lineRefs.current[Math.max(0, i - 1)]?.focus());
    }
  };

  const options = [{ value: "", label: "None" }, ...signatures.map((x) => ({ value: x.id, label: x.name }))];
  const pick = (accountId: string, kind: keyof AccountSignatures, label: string) => (
    <div className={styles.cell}>
      <Select
        aria-label={label}
        value={byAccount[accountId]?.[kind] ?? ""}
        options={options}
        onChange={(v) => setChoice(accountId, kind, v || null)}
      />
    </div>
  );

  // An address follows its account until it has a choice of its own.
  const pickForAddress = (a: Account, email: string, kind: keyof AccountSignatures, label: string) => (
    <div className={styles.cell}>
      <Select
        aria-label={label}
        value={byAddress[email] ? (byAddress[email][kind] ?? "") : "same"}
        options={[{ value: "same", label: `Same as ${a.name}` }, ...options]}
        onChange={(v) => setAddressChoice(a.id, email, kind, v === "same" ? "same" : v || null)}
      />
    </div>
  );

  return (
    <div className={cx(shared.page, styles.page)}>
      <ScreenHeading
        title="Signatures"
        sub="Write them once. Moss adds the right one to each email. Change it in any email with ⌥S."
      />
      <div className={styles.choose}>
        <div className={styles.chooseText}>
          <span className={styles.chooseHead}>
            <Pill variant="ai">AI</Pill>
            <span className={styles.chooseTitle}>Let Moss choose</span>
          </span>
          <p className={styles.chooseSub}>
            Starts from your choices below. Later replies in a thread get your one-line one, or none. Forwards get the one
            for new emails. Your pick in an email always wins.
          </p>
        </div>
        <Toggle label="Let Moss choose" on={letMossChoose} onChange={setLetMossChoose} />
      </div>

      <div className={shared.group}>
        <SectionHeader
          label="Your signatures"
          right={sig && <ShortcutHint keys="⌘N" label="New signature" onClick={create} />}
        />
        {!sig ? (
          <div className={styles.empty} role="status">
            <span className={styles.emptyTile}>
              <Icon icon={Signature} size={20} tone="ink-2" />
            </span>
            <div className={styles.emptyText}>
              <p className={styles.emptyTitle}>No signatures yet</p>
              <p className={styles.emptySub}>
                Write your name and how to reach you once. Moss adds it to the end of your emails.
              </p>
            </div>
            <Button variant="outline" icon={Plus} shortcut="⌘N" onClick={create}>
              New signature
            </Button>
          </div>
        ) : (
        <div className={styles.columns}>
          <ListCard className={styles.list} role="listbox" aria-label="Your signatures">
            {signatures.map((x) => {
              const on = x.id === sig?.id;
              return (
                <ListRow
                  key={x.id}
                  role="option"
                  aria-selected={on}
                  selected={on}
                  className={styles.item}
                  onClick={() => setSelectedId(x.id)}
                >
                  <Icon icon={Signature} size={16} tone={on ? "ink" : "ink-3"} />
                  <div className={cx(shared.text, styles.itemText)}>
                    <span className={cx(styles.itemName, on && styles.itemNameOn)}>{x.name}</span>
                    <span className={shared.small}>{usedBy(x.id, accounts)}</span>
                  </div>
                </ListRow>
              );
            })}
          </ListCard>
          {sig && (
            <div className={styles.editor}>
              <div className={styles.nameRow}>
                <label className={styles.label} htmlFor="signature-name">
                  Name
                </label>
                <TextInput
                  id="signature-name"
                  ref={nameRef}
                  compact
                  value={sig.name}
                  onChange={(e) => updateSignature(sig.id, { name: e.target.value })}
                />
              </div>
              <div className={styles.body}>
                {sig.lines.map((line, i) => (
                  <input
                    key={i}
                    ref={(el) => {
                      lineRefs.current[i] = el;
                    }}
                    aria-label={`Line ${i + 1}`}
                    className={cx(styles.line, i === 0 && styles.first, i > 0 && isLink(line) && styles.link)}
                    value={line}
                    spellCheck={false}
                    onChange={(e) => editLine(sig, i, e.target.value)}
                    onKeyDown={(e) => onLineKey(sig, i, e)}
                  />
                ))}
              </div>
              <div className={styles.foot}>
                <span className={cx(shared.small, styles.hint)}>Text and links only. Two to four lines read best.</span>
                <Button variant="secondary" icon={Trash2} shortcut="⌘⌫" onClick={remove}>
                  Delete
                </Button>
              </div>
            </div>
          )}
        </div>
        )}
      </div>

      <div className={shared.group}>
        <SectionHeader label="Which one for each account" />
        <div className={styles.table} role="table" aria-label="Signature for each account">
          <div className={cx(styles.tr, styles.th)} role="row">
            <span className={styles.accountCell} role="columnheader">
              Account
            </span>
            <span className={styles.cell} role="columnheader">
              New emails
            </span>
            <span className={styles.cell} role="columnheader">
              Replies
            </span>
          </div>
          {accounts.map((a) => (
            <Fragment key={a.id}>
              <div className={styles.tr} role="row">
                <span className={styles.accountCell} role="cell">
                  <AccountBadge letter={a.letter} color={a.color} size={20} />
                  <span className={shared.name}>{a.name}</span>
                  <span className={shared.small}>{a.address}</span>
                </span>
                {pick(a.id, "newEmails", `${a.name}: new emails`)}
                {pick(a.id, "replies", `${a.name}: replies`)}
              </div>
              {sendingAddresses(a)
                .filter((x) => !x.primary)
                .map((x) => (
                  <div key={x.email} className={cx(styles.tr, styles.addressRow)} role="row">
                    <span className={styles.accountCell} role="cell">
                      <Icon icon={AtSign} size={14} tone="ink-3" />
                      <span className={shared.small}>{x.email}</span>
                    </span>
                    {pickForAddress(a, x.email, "newEmails", `${x.email}: new emails`)}
                    {pickForAddress(a, x.email, "replies", `${x.email}: replies`)}
                  </div>
                ))}
            </Fragment>
          ))}
        </div>
      </div>
    </div>
  );
}
