import type { ReactNode } from "react";
import { ChevronDown, ChevronUp, Forward } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { Account, Message, Person } from "../../../data/types";
import { openLink } from "../../../lib/links";
import { recipientSummary } from "../../../lib/people";
import type { ReadingSize, Theme } from "../../../state/store";
import { Icon } from "../../atoms";
import { AttachmentChip, Popover } from "../../molecules";
import { EmailBody } from "../EmailBody/EmailBody";
import { senderImages } from "../EmailBody/senderImages";
import { RecipientDetails } from "../RecipientDetails/RecipientDetails";
import styles from "./MessageView.module.css";

export interface MessageViewProps {
  message: Message;
  account: Account;
  recipientsOpen: boolean;
  onToggleRecipients: () => void;
  theme?: Theme;
  readingSize?: ReadingSize;
  /** You chose to load this message's remote images, just this once. */
  imagesShown?: boolean;
  onShowImages?: () => void;
  /** Show images from this sender from now on. */
  onAllowImages?: () => void;
  /** Defaults to opening the link outside Moss. */
  onOpenLink?: (url: string) => void;
  /** Opens attachment `index` (see `Attachment.index`). */
  onOpenAttachment?: (index: number) => void;
  /** About the sender; with it, their name opens it as a card. */
  senderCard?: ReactNode;
  /** Opens a forward you sent of this email, by its thread id. */
  onOpenForward?: (threadId: string) => void;
}

const noop = () => {};

/** "Rebecca", "Rebecca and Jonas", "Rebecca, Jonas and Ines" */
function names(people: Person[]) {
  const all = people.map((p) => p.short ?? p.name);
  return all.length > 1 ? `${all.slice(0, -1).join(", ")} and ${all.at(-1)}` : all.join("");
}

/** That you forwarded this email: to whom and when, each opening the forward. */
function Forwards({ message, onOpen }: { message: Message; onOpen: (threadId: string) => void }) {
  const forwards = message.forwards ?? [];
  if (!forwards.length && !message.forwarded) return null;
  return (
    <div className={styles.forwards}>
      {forwards.length ? (
        forwards.map((f) => (
          <button key={f.threadId} type="button" className={styles.forward} onClick={() => onOpen(f.threadId)}>
            <Icon icon={Forward} size={14} tone="ink-3" />
            <span>
              You forwarded this to {names(f.to)}
              <span className={styles.forwardWhen}> · {f.time}</span>
            </span>
          </button>
        ))
      ) : (
        <span className={styles.forward}>
          <Icon icon={Forward} size={14} tone="ink-3" />
          You forwarded this
        </span>
      )}
    </div>
  );
}

/** One email: who sent it, to whom, the text and its attachments. */
export function MessageView({
  message,
  account,
  recipientsOpen,
  onToggleRecipients,
  theme = "dark",
  readingSize = "default",
  imagesShown = false,
  onShowImages = noop,
  onAllowImages = noop,
  onOpenLink = openLink,
  onOpenAttachment = noop,
  senderCard,
  onOpenForward = noop,
}: MessageViewProps) {
  const name = message.from.name;
  return (
    <article className={styles.message}>
      <div className={styles.sender}>
        {senderCard ? (
          <Popover
            role="dialog"
            label={`About ${name}`}
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className={cx(styles.name, styles.nameButton, open && styles.open)}
                aria-expanded={open}
                onClick={toggle}
              >
                {name}
              </button>
            )}
          >
            {() => senderCard}
          </Popover>
        ) : (
          <span className={styles.name}>{name}</span>
        )}
        <button
          type="button"
          className={cx(styles.toggle, recipientsOpen && styles.open)}
          onClick={onToggleRecipients}
          aria-expanded={recipientsOpen}
        >
          <span>{recipientSummary(message.to, message.cc, account.address)}</span>
          <Icon icon={recipientsOpen ? ChevronUp : ChevronDown} size={14} tone="ink-2" />
        </button>
        <span className={styles.spacer} />
        <span className={styles.when}>{message.time}</span>
      </div>
      {recipientsOpen && <RecipientDetails message={message} account={account} onHide={onToggleRecipients} />}
      <Forwards message={message} onOpen={onOpenForward} />
      <EmailBody
        message={message}
        theme={theme}
        readingSize={readingSize}
        imagesShown={imagesShown}
        {...senderImages(message)}
        recipient={account.address}
        onShowImages={onShowImages}
        onAllowImages={onAllowImages}
        onOpenLink={onOpenLink}
      />
      {message.attachments.length > 0 && (
        <div className={styles.attachments} role="group" aria-label="Attachments">
          {message.attachments.map(({ index, name, size, contentType, preview }) => (
            <AttachmentChip
              key={index ?? name}
              name={name}
              size={size}
              contentType={contentType}
              preview={preview}
              onOpen={index === undefined ? undefined : () => onOpenAttachment(index)}
            />
          ))}
        </div>
      )}
    </article>
  );
}
