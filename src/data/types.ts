/**
 * Account colour slots. Each maps to a design token of the same name.
 * New accounts take them in this order so neighbouring accounts always differ.
 */
export type AccountColor = "work" | "personal" | "studio" | "green" | "violet" | "sky" | "sand" | "olive";

export const accountColorOrder: AccountColor[] = ["work", "personal", "studio", "green", "violet", "sky", "sand", "olive"];

/** Where an account's mail comes from. Each has its own way to sign in. */
export type ProviderId = "gmail" | "outlook" | "icloud" | "yahoo" | "fastmail" | "resend" | "imap";

export interface Connection {
  provider: ProviderId;
  /** How it is connected, as shown in Settings, e.g. "Fastmail · App password". */
  summary: string;
  /** The provider stopped accepting the saved sign-in; the account needs signing in again. */
  signedOut?: boolean;
}

/**
 * Another address that comes into an account, e.g. a custom domain on iCloud. One login, many
 * addresses: Moss can send from the ones you added and sorts mail by the one it came to.
 */
export interface AccountAddress {
  /** Lowercase. */
  email: string;
  /** Your name on mail from it; the account's sender name when not set. */
  name?: string;
  /** `added`: you send from it. `suggested`: Moss found mail to it and asks. `dismissed`: you said no (shown only while blocked). */
  state: "added" | "suggested" | "dismissed";
  /** Who put it here: you, Moss from your mail, or Gmail's "Send mail as" list. */
  source: "you" | "mail" | "gmail";
  /** Emails from others that came to it. */
  seen: number;
  /** Mail to it goes to Blocked, e.g. a shop address that leaked to spammers. */
  blocked: boolean;
}

export interface Account {
  id: string;
  /** Short label, e.g. "Work". */
  name: string;
  letter: string;
  address: string;
  /** Your name as it appears on mail sent from this account. */
  senderName: string;
  color: AccountColor;
  /** Organisation name, used to flag people outside it. */
  org?: string;
  /** Email domain for that organisation. */
  orgDomain?: string;
  connection?: Connection;
  /** Its other addresses, not counting `address`, in the order you added them. */
  addresses?: AccountAddress[];
}

export type SplitId = "important" | "other" | "calendar" | "newsletters" | "receipts";

export interface Split {
  id: SplitId;
  label: string;
}

/** Where a thread sits in the Important split. */
export type Section = "reply" | "info" | "later";

/** Someone you chose from the recipient suggestions, and what you had typed when you did. */
export interface RecipientPick {
  typed: string;
  email: string;
}

export interface Person {
  name: string;
  email: string;
  initials: string;
  /** Short name used in compact lists, e.g. "Daniel". */
  short?: string;
  role?: string;
  location?: string;
  localTime?: string;
}

export interface Attachment {
  /** Which of the message's files it is, to open it. Demo attachments have none. */
  index?: number;
  name: string;
  size: string;
  /** The MIME type, for a file whose name has no extension. */
  contentType?: string;
  /** A thumbnail of what's in the file, as an image URL. */
  preview?: string;
  /** Its size in bytes, for how much a forward carries. Demo attachments have none. */
  bytes?: number;
}

/**
 * A file going with a draft: one you attached, kept on this Mac until it is sent, or one of
 * the forwarded email's files, fetched when it goes. Mirrors `DraftFile` in moss-core.
 */
export interface DraftFile {
  /** Its id among the files kept for drafts; for a forwarded file, "<message id>:<index>". */
  id: string;
  name: string;
  bytes: number;
  contentType: string;
  /** The forwarded email's file this is; none for a file you attached. */
  forwarded?: { messageId: string; index: number };
}

/** Whether a message's remote images load by themselves, and why. See docs/research/remote-images.md. */
export type ImageTrust = "ask" | "yours" | "you-allowed" | "you-wrote" | "same-company";

export interface Message {
  id: string;
  from: Person;
  to: Person[];
  cc: Person[];
  bcc: Person[];
  /** e.g. "9:12 AM" */
  time: string;
  /** e.g. "Sunday, 1 November, 9:12 AM" */
  sentFull: string;
  /** The text as paragraphs. Demo mail has only these. */
  paragraphs: string[];
  /** The HTML body as received, unsanitised. Shown sanitised in a sandboxed frame. */
  html?: string;
  /** The plain-text body as sent, line breaks kept. Shown when there is no HTML. */
  text?: string;
  attachments: Attachment[];
  /** Real mail only; demo mail asks. */
  imageTrust?: ImageTrust;
  /** The account's mail server didn't confirm it comes from who its From line says, so trusting them doesn't show its images. */
  senderUnverified?: boolean;
  /** Only the headers are on this device (older sent mail); the body loads when opened. */
  headersOnly?: boolean;
  /** Images the HTML shows aren't on this device yet; they load when opened. */
  imagesMissing?: boolean;
  /** Forwards of this email you sent from Moss, oldest first. */
  forwards?: Forward[];
  /** Marked forwarded on the server, by Moss or another mail app. */
  forwarded?: boolean;
}

/** A forward you sent: who it went to, when, and the conversation it started. */
export interface Forward {
  threadId: string;
  to: Person[];
  /** e.g. "Saturday, 31 October, 4:12 PM" */
  time: string;
}

export interface ReplyOption {
  label: string;
  draft: string;
}

/** What you and the sender have in common, worked out from local mail. */
export interface Between {
  /** How many emails you have exchanged. */
  emails: number;
  /** When the first one was, e.g. "March". */
  since: string;
  /** How quickly you usually answer them, e.g. "Same day". */
  youReply?: string;
  /** For a new sender instead: whether you have written to them, e.g. "Never". */
  youWrote?: string;
}

/** Who a thread's sender is to you, decided by rules in the engine (never by AI). */
export type SenderStatus = "known" | "new" | "blocked";

/** Someone you blocked or accepted, or everyone at a domain, as Settings → Senders lists them. */
export interface SenderRule {
  accountId: string;
  /** An address, or a domain when the rule is about everyone there. */
  sender: string;
  wholeDomain: boolean;
  /** Blocked; otherwise accepted. */
  blocked: boolean;
  createdAt: number;
}

/** Why Moss shows something: the decision model (how sure), a rule (in the email's own words), or you. */
export interface Why {
  by: "ai" | "rule" | "you";
  /** Percent, when the AI decided. */
  sure?: number;
  /** What a rule went by, e.g. "by Friday". */
  evidence?: string;
}

export interface Thread {
  id: string;
  accountId: string;
  split: SplitId;
  section: Section;
  from: Person;
  subject: string;
  /** Full subject shown on the thread page. */
  fullSubject?: string;
  summary?: string;
  /** Real mail only: why `summary` is the AI's line (it's in Important, or you've written in it). */
  summaryWhy?: "important" | "replied";
  time: string;
  due?: string;
  labels?: string[];
  /** One-line AI summary of what you need to do. */
  ask?: string;
  askDue?: string;
  recent?: { subject: string; when: string }[];
  insight?: string;
  between?: Between;
  replyOptions?: ReplyOption[];
  messages: Message[];
  /** Real mail only: when the latest message came (ms), so a reload can tell what changed. */
  lastMessageAt?: number;
  /** Real mail only: the messages are a preview until the thread is opened and loaded. */
  partial?: boolean;
  /** When you marked it done (ms). Only set on threads in the Done list. */
  doneAt?: number;
  /** Real mail only: the decision model sorted it into its section, this sure (percent). */
  aiSure?: number;
  /** Has a message you haven't opened. Demo mail is all read. */
  unread?: boolean;
  /** "new" shows "Block this sender?"; "blocked" shows Unblock. Real mail says it on every list row. */
  sender?: SenderStatus;
  /** Which of your addresses it came to (lowercase), when the engine could tell. */
  toAddress?: string;
  /** You moved it to this tab yourself (M), so Moss's sorting leaves it there. In Important it
   * needs you: it counts toward what's waiting and sits at the top. */
  movedByYou?: boolean;
  /** Real mail only: the day a deadline falls on (YYYY-MM-DD), and who found it. `due` is its label. */
  dueDate?: string;
  dueWhy?: Why;
  /** Real mail only: the decision model moved it to its split. */
  splitWhy?: Why;
  /** Your email is the latest and waits on an answer: it's back at `until` (ms) if none comes. */
  waiting?: { until: number; why: Why };
  /** The latest email is yours, so Moss can wait on an answer to it. */
  youWroteLast?: boolean;
}
