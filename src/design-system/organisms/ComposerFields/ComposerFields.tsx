import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { ArrowRight, Plus } from "lucide-react";
import type { Account, Person, RecipientPick } from "../../../data/types";
import { sendingAddress, type SendingAddress } from "../../../lib/addresses";
import { isExternal, personFromAddress, rankByPicks } from "../../../lib/people";
import { AccountBadge, Button, Icon, Pill } from "../../atoms";
import { cx } from "../../../lib/cx";
import { AutocompleteOption, FromPicker, RecipientChip } from "../../molecules";
import { FromMenu } from "../FromMenu/FromMenu";
import styles from "./ComposerFields.module.css";
import { SEP } from "../../../lib/text";

type Field = "to" | "cc" | "bcc";

export interface ComposerFieldsProps {
  from: Account;
  /** Which of your addresses it goes from; the account's own when not given. */
  sending?: SendingAddress;
  /** Every account, for the From menu. */
  accounts?: Account[];
  /** The From menu (⌥F) is open. */
  fromOpen?: boolean;
  onToggleFrom?: () => void;
  onChooseFrom?: (accountId: string, email: string) => void;
  /** Settings → Accounts, to add an address. */
  onEditAddresses?: () => void;
  /** Shown on the right of the From row when the AI picked the account, e.g. "The account you use with Priya". */
  fromReason?: string;
  to: Person[];
  cc: Person[];
  bcc: Person[];
  showCc: boolean;
  showBcc: boolean;
  /** `false` folds every row into one summary line. */
  open: boolean;
  contacts: Person[];
  /** Who you chose from the suggestions before, newest first: they come first again. */
  picks?: RecipientPick[];
  /** You chose `person` from the suggestions after typing `typed` (trimmed, lower case). */
  onPick?: (typed: string, person: Person) => void;
  onChange: (field: Field, people: Person[]) => void;
  onShowCc: () => void;
  onShowBcc: () => void;
  onOpen: () => void;
}

/** From, To, CC and BCC. Outsiders are highlighted so they're hard to miss. */
export function ComposerFields(props: ComposerFieldsProps) {
  return props.open ? <OpenFields {...props} /> : <Summary {...props} />;
}

function Summary({ from, sending, to, cc, bcc, onOpen }: ComposerFieldsProps) {
  const short = (p: Person) => p.short ?? p.name;
  const other = sending && !sending.primary ? sending.email : undefined;
  return (
    <div className={styles.fields}>
      <div className={styles.summary}>
        <span className={styles.summaryFrom}>
          <AccountBadge letter={from.letter} color={from.color} size={22} />
          <span className={styles.summaryAccount}>{from.name}</span>
          {other && <span className={styles.small}>{other}</span>}
        </span>
        <Icon icon={ArrowRight} size={14} tone="ink-3" />
        <span className={styles.summaryPeople}>
          <span className={styles.summaryTo}>{to.map(short).join(", ")}</span>
          {cc.length > 0 && (
            <>
              <span className={styles.sep}>·</span>
              <span className={styles.small}>cc</span>
              {cc.map((p, i) =>
                isExternal(p, from) ? (
                  <Pill key={p.email} variant="name">
                    {short(p)}
                  </Pill>
                ) : (
                  <span key={p.email} className={styles.summaryCc}>
                    {short(p)}
                    {i < cc.length - 1 ? "," : ""}
                  </span>
                ),
              )}
            </>
          )}
          {bcc.length > 0 && (
            <>
              <span className={styles.sep}>·</span>
              <span className={styles.small}>{bcc.length} bcc</span>
            </>
          )}
        </span>
        <Button variant="outlineSm" shortcut="⌘⇧E" onClick={onOpen}>
          Edit
        </Button>
      </div>
    </div>
  );
}

function OpenFields(props: ComposerFieldsProps) {
  const { from, fromReason, to, cc, bcc, showCc, showBcc, contacts, onChange, onShowCc, onShowBcc } = props;
  const sending = props.sending ?? sendingAddress(from);
  const rows: Field[] = ["to", ...(showCc ? (["cc"] as const) : []), ...(showBcc ? (["bcc"] as const) : [])];
  const values = { to, cc, bcc };
  return (
    <div className={styles.fields}>
      <div className={cx(styles.row, styles.fromRow)}>
        <span className={styles.label}>From</span>
        <FromPicker
          variant="field"
          account={from}
          text={`${sending.name}${SEP}${sending.email}`}
          keys={props.onToggleFrom && "⌥F"}
          expanded={props.onToggleFrom && !!props.fromOpen}
          onClick={props.onToggleFrom}
        />
        <span className={styles.spacer} />
        {fromReason && <span className={styles.hint}>{fromReason}</span>}
        {props.fromOpen && (
          <FromMenu
            accounts={props.accounts ?? [from]}
            chosen={sending}
            onChoose={(accountId, email) => props.onChooseFrom?.(accountId, email)}
            onClose={() => props.onToggleFrom?.()}
            onEdit={() => props.onEditAddresses?.()}
          />
        )}
      </div>
      {rows.map((field, i) => (
        <RecipientRow
          key={field}
          field={field}
          people={values[field]}
          account={from}
          contacts={contacts}
          picks={props.picks}
          onPick={props.onPick}
          last={i === rows.length - 1}
          onChange={(list) => onChange(field, list)}
          trailing={
            field === "bcc" ? (
              <span className={styles.hint}>Hidden from everyone else</span>
            ) : field === "to" && (!showCc || !showBcc) ? (
              <>
                {!showCc && (
                  <Button variant="outlineSm" className={styles.add} icon={Plus} shortcut="⌘⇧C" shortcutTone="quiet" onClick={onShowCc}>
                    CC
                  </Button>
                )}
                {!showBcc && (
                  <Button variant="outlineSm" className={styles.add} icon={Plus} shortcut="⌘⇧B" shortcutTone="quiet" onClick={onShowBcc}>
                    BCC
                  </Button>
                )}
              </>
            ) : null
          }
        />
      ))}
    </div>
  );
}

const labels: Record<Field, string> = { to: "To", cc: "CC", bcc: "BCC" };

/** Commas and semicolons between addresses, but not inside a quoted name like "Vogt, Lena". */
const SEPARATOR = /[,;](?=(?:[^"]*"[^"]*")*[^"]*$)/;

interface RecipientRowProps {
  field: Field;
  people: Person[];
  account: Account;
  contacts: Person[];
  picks?: RecipientPick[];
  onPick?: (typed: string, person: Person) => void;
  last: boolean;
  trailing: ReactNode;
  onChange: (people: Person[]) => void;
}

function RecipientRow({ field, people, account, contacts, picks = [], onPick, last, trailing, onChange }: RecipientRowProps) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const [dropLeft, setDropLeft] = useState(0);

  const q = query.trim().toLowerCase();
  const taken = new Set(people.map((p) => p.email));
  const matches = q
    ? rankByPicks(
        contacts.filter((c) => !taken.has(c.email) && (c.name.toLowerCase().includes(q) || c.email.toLowerCase().includes(q))),
        q,
        picks,
      ).slice(0, 5)
    : [];

  useLayoutEffect(() => {
    if (inputRef.current) setDropLeft(inputRef.current.offsetLeft - 14);
  }, [query, people.length]);

  const add = (p: Person) => {
    onChange([...people, p]);
    setQuery("");
    setHighlight(0);
  };

  /** A suggestion chosen, by key or click. */
  const choose = (p: Person) => {
    onPick?.(q, p);
    add(p);
  };

  /**
   * Adds the full addresses among `parts`, as the contact when Moss knows them, and leaves
   * the rest (and `rest`) as text. Returns whether any part was an address.
   */
  const addTyped = (parts: string[], rest = "") => {
    const found = parts.map((text) => ({ text, person: personFromAddress(text) }));
    if (!found.some((f) => f.person)) return false;
    const seen = new Set(people.map((p) => p.email.toLowerCase()));
    const added: Person[] = [];
    for (const { person } of found) {
      const email = person?.email.toLowerCase();
      if (!email || seen.has(email)) continue;
      seen.add(email);
      added.push(contacts.find((c) => c.email.toLowerCase() === email) ?? person!);
    }
    if (added.length) onChange([...people, ...added]);
    const left = found.filter((f) => !f.person).map((f) => f.text.trim());
    setQuery([...left, rest.trimStart()].filter(Boolean).join(", "));
    setHighlight(0);
    return true;
  };

  const addAllTyped = () => addTyped(query.split(SEPARATOR));

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" && matches.length) {
      e.preventDefault();
      setHighlight((h) => (h + 1) % matches.length);
    } else if (e.key === "ArrowUp" && matches.length) {
      e.preventDefault();
      setHighlight((h) => (h - 1 + matches.length) % matches.length);
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      // ⌘↵ sends: take what's typed first, and let the key go on to Send.
      addAllTyped();
    } else if ((e.key === "Enter" || e.key === "Tab") && matches[highlight]) {
      e.preventDefault();
      choose(matches[highlight]);
    } else if ((e.key === "Enter" || e.key === "Tab") && addAllTyped()) {
      e.preventDefault();
    } else if (e.key === "Backspace" && !query && people.length) {
      onChange(people.slice(0, -1));
    }
  };

  return (
    <div className={last ? styles.rowLast : styles.row} onClick={() => inputRef.current?.focus()}>
      <span className={styles.label}>{labels[field]}</span>
      {people.map((p) => (
        <RecipientChip
          key={p.email}
          name={p.name}
          initials={p.initials}
          external={isExternal(p, account)}
          onRemove={() => onChange(people.filter((x) => x.email !== p.email))}
        />
      ))}
      <input
        ref={inputRef}
        className={styles.input}
        value={query}
        size={Math.max(1, query.length + 1)}
        onChange={(e) => {
          const parts = e.target.value.split(SEPARATOR);
          if (parts.length > 1 && addTyped(parts.slice(0, -1), parts.at(-1))) return;
          setQuery(e.target.value);
          setHighlight(0);
        }}
        onKeyDown={onKeyDown}
        onBlur={addAllTyped}
        aria-label={`${labels[field]} recipients`}
        data-shortcut-scope="field"
      />
      <span className={styles.spacer} />
      {trailing}
      {matches.length > 0 && (
        <div className={styles.autocomplete} style={{ left: dropLeft }} role="listbox">
          {matches.map((c, i) => (
            <AutocompleteOption
              key={c.email}
              name={c.name}
              email={c.email}
              initials={c.initials}
              external={isExternal(c, account)}
              highlighted={i === highlight}
              onChoose={() => choose(c)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
