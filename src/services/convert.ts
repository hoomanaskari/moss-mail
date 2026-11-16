/**
 * The mail engine's JSON (moss-core's types, times in ms) turned into the UI's types
 * (`src/data/types.ts`, times as text). Kept pure so it is tested without the desktop app.
 */
import type { Account, AccountAddress, AccountColor, ImageTrust, Message, Person, ProviderId, Section, SenderStatus, SplitId, Thread, Why } from "../data/types";
import { dueLabel } from "../lib/why";
import { plainText } from "../lib/markdown";
import { highlight, words, type SearchHit } from "../lib/search";
import { fullTime, listTime } from "../lib/time";
import type { ServerSettings, SyncSnapshot } from "./accountSetup";

export interface RustPerson {
  name: string;
  email: string;
  initials: string;
  short: string;
}

export interface RustSummary {
  id: string;
  accountId: string;
  split: SplitId;
  section: Section;
  from: RustPerson;
  subject: string;
  summary: string;
  lastMessageAt: number;
  snoozedUntil: number | null;
  unread: boolean;
  done: boolean;
  /** When it was marked done (ms); only on done threads. */
  doneAt?: number | null;
  labels: string[];
  messageCount: number;
  hasAttachments: boolean;
  /** How sure the decision model was when it sorted it (percent); absent when the rules did. */
  aiSure?: number;
  /** One or two lines on the conversation, from the writing model. */
  aiSummary?: string;
  /** Which of your addresses it came to. */
  toAddress?: string;
  /** Why that line was written: the conversation is in Important, or you've written in it. */
  aiSummaryWhy?: "important" | "replied";
  /** Who the sender is to you. Every row says, so "Block this sender?" is decided before the email opens. */
  sender: SenderStatus;
  /** You moved it to its tab yourself; Moss's sorting leaves it there. */
  movedByYou?: boolean;
  /** A deadline in the latest email from someone else: its day (YYYY-MM-DD), and who found it. */
  due?: { date: string; why: Why };
  /** The decision model moved it to its split. */
  splitWhy?: Why;
  /** Your email is the latest and waits on an answer: back at `until` (ms) if none comes. */
  waiting?: { until: number; why: Why };
  /** The latest email is yours. Absent from an engine from before Moss waited on answers. */
  youWroteLast?: boolean;
}

export interface RustAttachment {
  index: number;
  name: string;
  size: string;
  bytes: number;
  contentType: string;
  inline: boolean;
}

export interface RustMessage {
  id: string;
  threadId: string;
  from: RustPerson;
  to: RustPerson[];
  cc: RustPerson[];
  bcc: RustPerson[];
  subject: string;
  sentAt: number;
  paragraphs: string[];
  text: string;
  html?: string;
  attachments: RustAttachment[];
  read: boolean;
  imageTrust: ImageTrust;
  senderUnverified?: boolean;
  headersOnly?: boolean;
  imagesMissing?: boolean;
  forwards?: { threadId: string; to: RustPerson[]; sentAt: number }[];
  forwarded?: boolean;
}

export interface RustThread extends RustSummary {
  messages: RustMessage[];
}

export interface RustAccount {
  id: string;
  name: string;
  letter: string;
  address: string;
  senderName: string;
  color: AccountColor;
  provider: string;
  org?: string;
  orgDomain?: string;
  connection: { provider: ProviderId; summary: string } | null;
  /** Absent from an engine from before addresses. */
  addresses?: AccountAddress[];
}

export interface RustServer {
  host: string;
  port: number;
  security: "tls" | "startTls";
  username: string;
}

export interface RustServerSettings {
  imap: RustServer;
  smtp: RustServer;
}

export interface RustSearchHit {
  thread: RustSummary;
  messageId: string;
  snippet: { text: string; matched: boolean }[];
  /** bm25: lower is better, and negative. */
  rank: number;
  /** The attachment the words were found in, when they were. */
  file?: { index: number; name: string };
}

export interface RustSyncProgress {
  accountId: string;
  done: number;
  folders: number;
  recent: number;
  oldestYear: number | null;
  more: boolean;
}

const person = (p: RustPerson): Person => ({ name: p.name, email: p.email, initials: p.initials, short: p.short });

function message(m: RustMessage, now: number): Message {
  return {
    id: m.id,
    from: person(m.from),
    to: m.to.map(person),
    cc: m.cc.map(person),
    bcc: m.bcc.map(person),
    time: listTime(m.sentAt, now),
    sentFull: fullTime(m.sentAt, now),
    paragraphs: m.paragraphs,
    ...(m.html !== undefined && { html: m.html }),
    text: m.text,
    attachments: m.attachments.filter((a) => !a.inline).map((a) => ({ index: a.index, name: a.name, size: a.size, bytes: a.bytes, contentType: a.contentType })),
    imageTrust: m.imageTrust,
    ...(m.senderUnverified && { senderUnverified: true }),
    ...(m.headersOnly && { headersOnly: true }),
    ...(m.imagesMissing && { imagesMissing: true }),
    ...(m.forwards?.length && { forwards: m.forwards.map((f) => ({ threadId: f.threadId, to: f.to.map(person), time: fullTime(f.sentAt, now) })) }),
    ...(m.forwarded && { forwarded: true }),
  };
}

/** A list row (messages still to load) or a whole thread. */
export function toThread(t: RustSummary | RustThread, now: number): Thread {
  const loaded = "messages" in t;
  const shownAt = t.section === "later" && t.snoozedUntil ? t.snoozedUntil : t.lastMessageAt;
  const messages = loaded
    ? t.messages.map((m) => message(m, now))
    : [
        {
          id: `${t.id}-preview`,
          from: person(t.from),
          to: [],
          cc: [],
          bcc: [],
          time: listTime(t.lastMessageAt, now),
          sentFull: fullTime(t.lastMessageAt, now),
          paragraphs: t.summary ? [t.summary] : [],
          attachments: [],
        },
      ];
  return {
    id: t.id,
    accountId: t.accountId,
    split: t.split,
    section: t.section,
    from: person(t.from),
    subject: t.subject,
    ...(loaded && { fullSubject: t.messages.at(-1)?.subject }),
    // The writing model's line on the conversation beats the preview of its latest email.
    summary: t.aiSummary ? plainText(t.aiSummary) : t.summary,
    time: listTime(shownAt, now),
    labels: t.labels,
    messages,
    lastMessageAt: t.lastMessageAt,
    partial: !loaded,
    unread: t.unread,
    ...(t.doneAt != null && { doneAt: t.doneAt }),
    ...(t.aiSure != null && { aiSure: t.aiSure }),
    ...(t.aiSummaryWhy && { summaryWhy: t.aiSummaryWhy }),
    sender: t.sender,
    ...(t.toAddress && { toAddress: t.toAddress }),
    ...(t.movedByYou && { movedByYou: true }),
    ...decided(t, now),
  };
}

/** What Moss decided about a thread: its due day, where the AI moved it, an answer you wait on. */
function decided(t: RustSummary, now: number): Partial<Thread> {
  // Back from waiting: no one replied. Amber like a due day, which comes first.
  const noReply = t.waiting && t.waiting.until <= now ? { due: "No reply" } : {};
  return {
    youWroteLast: !!t.youWroteLast,
    ...(t.splitWhy && { splitWhy: t.splitWhy }),
    ...(t.waiting && { waiting: t.waiting }),
    ...(t.due
      ? { due: dueLabel(t.due.date, now), askDue: dueLabel(t.due.date, now), dueDate: t.due.date, dueWhy: t.due.why }
      : noReply),
  };
}

export function toAccount(a: RustAccount): Account {
  return {
    id: a.id,
    name: a.name,
    letter: a.letter,
    address: a.address,
    senderName: a.senderName,
    color: a.color,
    ...(a.org && { org: a.org }),
    ...(a.orgDomain && { orgDomain: a.orgDomain }),
    connection: a.connection ?? undefined,
    ...(a.addresses?.length && { addresses: a.addresses }),
  };
}

/** The IMAP form's settings for the Rust sign-in. Moss never sends a password unencrypted. */
export function toServerSettings(s: ServerSettings, email: string): RustServerSettings {
  const server = (x: ServerSettings["incoming"]): RustServer => ({
    host: x.host.trim(),
    port: Number.parseInt(x.port, 10) || 0,
    security: x.security === "ssl" ? "tls" : "startTls",
    username: email,
  });
  return { imap: server(s.incoming), smtp: server(s.outgoing) };
}

export function toUiServers(s: RustServerSettings): ServerSettings {
  const server = (x: RustServer) => ({ host: x.host, port: String(x.port), security: x.security === "tls" ? ("ssl" as const) : ("starttls" as const) });
  return { incoming: server(s.imap), outgoing: server(s.smtp) };
}

const isoDate = (ms: number) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

/** Search results for the Search screen. Hits whose subject has every word are the best matches. */
export function toSearchHits(hits: RustSearchHit[], query: string, now: number): SearchHit[] {
  const ws = words(query);
  const best = Math.min(0, ...hits.map((h) => h.rank));
  return hits.map((h) => {
    const t = h.thread;
    const subject = t.subject.toLowerCase();
    return {
      doc: {
        id: h.messageId,
        threadId: t.id,
        accountId: t.accountId,
        from: person(t.from),
        subject: t.subject,
        summary: h.snippet.map((p) => p.text).join(""),
        when: listTime(t.lastMessageAt, now),
        date: isoDate(t.lastMessageAt),
        hasAttachment: t.hasAttachments,
        relevance: best ? h.rank / best : 0,
      },
      subject: highlight(t.subject, ws),
      summary: [...(h.file ? [{ text: `In ${h.file.name}: `, match: false }] : []), ...h.snippet.map((p) => ({ text: p.text, match: p.matched }))],
      best: ws.every((w) => subject.includes(w)),
    };
  });
}

/** One batch ahead while older mail is still coming, so the bar keeps moving. */
export function toProgress(p: RustSyncProgress): SyncSnapshot {
  return {
    done: p.done,
    total: p.more ? p.done + 200 : p.done,
    folders: p.folders,
    recent: p.recent,
    oldestYear: p.oldestYear ?? new Date().getFullYear(),
  };
}
