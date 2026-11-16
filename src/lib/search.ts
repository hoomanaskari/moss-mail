/**
 * Searching mail. Runs over demo data until the Rust store answers it; the screen only talks to
 * a `SearchBackend`, so a Tauri command can replace `demoSearch` without touching the UI.
 */
import type { Person } from "../data/types";

/** One email as the search index sees it. */
export interface SearchDoc {
  id: string;
  /** The thread to open, when it is in the mailbox on screen. */
  threadId?: string;
  accountId: string;
  from: Person;
  subject: string;
  summary: string;
  /** Short date as shown in lists, e.g. "Oct 14" or "Tue". */
  when: string;
  /** YYYY-MM-DD. */
  date: string;
  hasAttachment: boolean;
  /** 0–1 from the search engine; 0.8 and up counts as a best match. */
  relevance: number;
}

export type DateRange = "any" | "week" | "month" | "year";
export type GroupBy = "date" | "account";

export interface SearchFilters {
  from?: Person;
  /** "all" or an account id. */
  accountId: string;
  hasAttachment: boolean;
  date: DateRange;
}

export const noFilters: SearchFilters = { accountId: "all", hasAttachment: false, date: "any" };

export interface TextPart {
  text: string;
  match: boolean;
}

export interface SearchHit {
  doc: SearchDoc;
  subject: TextPart[];
  summary: TextPart[];
  best: boolean;
}

export interface ResultGroup {
  title: string;
  note?: string;
  hits: SearchHit[];
}

export interface SearchBackend {
  search(query: string, filters: SearchFilters): Promise<SearchHit[]>;
}

const DAY = 86_400_000;
const BEST = 0.8;
const months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

export function words(query: string) {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

const escape = (w: string) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Splits `text` into runs, marking each place a query word appears. */
export function highlight(text: string, ws: string[]): TextPart[] {
  if (!ws.length) return [{ text, match: false }];
  const re = new RegExp(`(${ws.map(escape).join("|")})`, "gi");
  return text
    .split(re)
    .filter(Boolean)
    .map((part) => ({ text: part, match: ws.includes(part.toLowerCase()) }));
}

const days = (from: string, to: string) => (Date.parse(to) - Date.parse(from)) / DAY;

/** Days since Monday of `today`'s week. */
const weekday = (today: string) => (new Date(`${today}T00:00:00Z`).getUTCDay() + 6) % 7;

function inRange(date: string, range: DateRange, today: string) {
  const age = days(date, today);
  if (range === "week") return age <= weekday(today);
  if (range === "month") return age <= 30;
  if (range === "year") return age <= 365;
  return true;
}

/** Keeps the hits that pass the filters (the search engine has already matched the words). */
export function filterHits(hits: SearchHit[], filters: SearchFilters, today: string): SearchHit[] {
  return hits.filter(
    ({ doc: d }) =>
      (!filters.from || d.from.email === filters.from.email) &&
      (filters.accountId === "all" || d.accountId === filters.accountId) &&
      (!filters.hasAttachment || d.hasAttachment) &&
      inRange(d.date, filters.date, today),
  );
}

/** Emails with every word of the query, newest first. */
export function searchDocs(docs: SearchDoc[], query: string, filters: SearchFilters, today: string): SearchHit[] {
  const ws = words(query);
  if (!ws.length && !filters.from) return [];
  return docs
    .filter((d) => {
      const haystack = `${d.subject} ${d.summary} ${d.from.name}`.toLowerCase();
      return (
        ws.every((w) => haystack.includes(w)) &&
        (!filters.from || d.from.email === filters.from.email) &&
        (filters.accountId === "all" || d.accountId === filters.accountId) &&
        (!filters.hasAttachment || d.hasAttachment) &&
        inRange(d.date, filters.date, today)
      );
    })
    .sort((a, b) => b.date.localeCompare(a.date))
    .map((doc) => ({
      doc,
      subject: highlight(doc.subject, ws),
      summary: highlight(doc.summary, ws),
      best: doc.relevance >= BEST,
    }));
}

function dateGroup(date: string, today: string) {
  if (days(date, today) <= weekday(today)) return "This week";
  const [y, m] = date.split("-").map(Number);
  return y === Number(today.slice(0, 4)) ? months[m - 1] : `${months[m - 1]} ${y}`;
}

/** Best matches first, then the rest by date ("This week", "October"…) or by account. */
export function groupHits(hits: SearchHit[], by: GroupBy, accountName: (id: string) => string, today: string): ResultGroup[] {
  const groups: ResultGroup[] = [];
  const best = hits.filter((h) => h.best);
  if (best.length) groups.push({ title: "Best matches", note: "Matches in the subject come first", hits: best });
  for (const h of hits.filter((x) => !x.best)) {
    const title = by === "date" ? dateGroup(h.doc.date, today) : accountName(h.doc.accountId);
    const group = groups.find((g) => g.title === title);
    if (group) group.hits.push(h);
    else groups.push({ title, hits: [h] });
  }
  return groups;
}
