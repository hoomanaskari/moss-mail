import type { Message, Person } from "../../../data/types";
import { openLink } from "../../../lib/links";
import type { ReadingSize, Theme } from "../../../state/store";
import { EmailBody } from "../EmailBody/EmailBody";
import { senderImages } from "../EmailBody/senderImages";
import styles from "./ForwardedMessage.module.css";

export interface ForwardedMessageProps {
  message: Message;
  subject: string;
  theme?: Theme;
  readingSize?: ReadingSize;
  /** You chose to load this message's remote images, just this once. */
  imagesShown?: boolean;
  onShowImages: () => void;
  /** Show images from this sender from now on. */
  onAllowImages: () => void;
  /** Defaults to opening the link outside Moss. */
  onOpenLink?: (url: string) => void;
}

/** "Lena Vogt <lena@northwind.co>", as the forward's header writes it. */
const who = (p: Person) => (p.name && p.name !== p.email ? `${p.name} <${p.email}>` : p.email);

/** The email a forward carries, shown under your text as it will be sent. */
export function ForwardedMessage({
  message,
  subject,
  theme = "dark",
  readingSize = "default",
  imagesShown = false,
  onShowImages,
  onAllowImages,
  onOpenLink = openLink,
}: ForwardedMessageProps) {
  const rows: [string, string][] = [
    ["From", who(message.from)],
    ["Date", message.sentFull],
    ["Subject", subject],
    ["To", message.to.map(who).join(", ")],
  ];
  if (message.cc.length) rows.push(["CC", message.cc.map(who).join(", ")]);
  return (
    <section className={styles.forwarded} aria-labelledby={`fwd-${message.id}`}>
      <div className={styles.head}>
        <span id={`fwd-${message.id}`} className={styles.title}>
          Forwarded message
        </span>
        <dl className={styles.rows}>
          {rows.map(([label, value]) => (
            <div key={label} className={styles.row}>
              <dt className={styles.label}>{label}</dt>
              <dd className={styles.value}>{value}</dd>
            </div>
          ))}
        </dl>
      </div>
      <EmailBody
        message={message}
        theme={theme}
        readingSize={readingSize}
        imagesShown={imagesShown}
        {...senderImages(message)}
        onShowImages={onShowImages}
        onAllowImages={onAllowImages}
        onOpenLink={onOpenLink}
      />
    </section>
  );
}
