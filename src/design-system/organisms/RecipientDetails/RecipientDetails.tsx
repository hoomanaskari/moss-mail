import { Info } from "lucide-react";
import type { Account, Message, Person } from "../../../data/types";
import { isExternal } from "../../../lib/people";
import { Notice, PersonLine, ShortcutHint } from "../../molecules";
import styles from "./RecipientDetails.module.css";

export interface RecipientDetailsProps {
  message: Message;
  /** The account that received the email, to show "You" and spot outsiders. */
  account: Account;
  onHide: () => void;
}

/** Everyone on an email, by role, with a warning when reply all would reach outsiders. */
export function RecipientDetails({ message, account, onHide }: RecipientDetailsProps) {
  const total = 1 + message.to.length + message.cc.length + message.bcc.length;
  const outsiders = [...message.to, ...message.cc].filter((p) => isExternal(p, account));

  const line = (p: Person) => {
    const you = p.email === account.address;
    return (
      <PersonLine
        key={p.email}
        name={you ? "You" : p.name}
        email={p.email}
        account={you ? { letter: account.letter, color: account.color } : undefined}
        flag={isExternal(p, account) ? `Outside ${account.org}` : undefined}
        onCopy={() => navigator.clipboard?.writeText(p.email)}
      />
    );
  };

  const group = (label: string, people: Person[]) =>
    people.length > 0 && (
      <div className={styles.group}>
        <span className={styles.label}>{label}</span>
        <div className={styles.people}>{people.map(line)}</div>
      </div>
    );

  return (
    <section className={styles.card} aria-label="People on this email">
      <div className={styles.head}>
        <span className={styles.title}>{total} people on this email</span>
        <ShortcutHint keys="I" label="Hide" size="sm" labelFirst gap={6} onClick={onHide} />
      </div>
      {group("From", [message.from])}
      {group("To", message.to)}
      {group("CC", message.cc)}
      {group("BCC", message.bcc)}
      <div className={styles.group}>
        <span className={styles.label}>Sent</span>
        <div className={styles.people}>
          <div className={styles.sent}>
            <span className={styles.sentText}>{message.sentFull}</span>
            <span className={styles.zone}>your time</span>
          </div>
        </div>
      </div>
      {outsiders.length > 0 && (
        <div className={styles.note}>
          <Notice variant="inset" icon={Info}>
            Reply all includes {outsiders.map((p) => p.short ?? p.name).join(" and ")}, who{" "}
            {outsiders.length > 1 ? "are" : "is"} outside {account.org}.
          </Notice>
        </div>
      )}
    </section>
  );
}
