/**
 * What Moss decided about an email, in a few plain words: the due day, and why each decision
 * was made ("AI, 91% sure", "says “by Friday”"). The engine decides; this only says it.
 */
import type { Thread, Why } from "../data/types";
import { splits } from "../data/demo";
import { remindTime } from "./time";

const splitLabel = Object.fromEntries(splits.map((s) => [s.id, s.label]));
const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** Midnight at the start of a YYYY-MM-DD day, on this computer's clock. */
export function dayOf(date: string): number {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(y, m - 1, d).getTime();
}

/** "Due today", "Due tomorrow", "Due Fri" this week, "Due 13 Nov", or "Overdue". */
export function dueLabel(date: string, now: number): string {
  const today = new Date(now);
  const days = Math.round((dayOf(date) - new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime()) / 86_400_000);
  const d = new Date(dayOf(date));
  if (days < 0) return "Overdue";
  if (days === 0) return "Due today";
  if (days === 1) return "Due tomorrow";
  if (days < 7) return `Due ${weekdays[d.getDay()]}`;
  const year = d.getFullYear() === today.getFullYear() ? "" : ` ${d.getFullYear()}`;
  return `Due ${d.getDate()} ${months[d.getMonth()]}${year}`;
}

/** A quote short enough for one line: a long one is cut at a word and ends in "…". */
function short(quote: string): string {
  if (quote.length <= 34) return quote;
  const cut = quote.lastIndexOf(" ", 30);
  return `${quote.slice(0, cut > 0 ? cut : 30)}…`;
}

/** Who decided, and how: "AI, 88% sure", "says “by Friday”", "you asked". */
export function whyText(why: Why): string {
  if (why.by === "ai") return `AI, ${why.sure}% sure`;
  if (why.by === "you") return "you asked";
  return why.evidence ? `says “${short(why.evidence)}”` : "rule";
}

/** One short line for each decision Moss made about the thread, for the top of the open email. */
export function reasonsFor(t: Thread, now: number): string[] {
  const out: string[] = [];
  if (t.aiSure !== undefined && t.section !== "later") {
    out.push(`${t.section === "reply" ? "Needs your reply" : "For your information"} · AI, ${t.aiSure}% sure`);
  }
  if (t.splitWhy) {
    out.push(`Moved to ${splitLabel[t.split]} · ${whyText(t.splitWhy)}`);
  }
  if (t.dueDate && t.dueWhy) out.push(`${dueLabel(t.dueDate, now)} · ${whyText(t.dueWhy)}`);
  if (t.waiting) {
    const who = t.from.short ?? t.from.name;
    const when = t.waiting.until > now ? `Back ${remindTime(t.waiting.until, now)} if ${who} doesn’t reply` : `No reply from ${who} yet`;
    out.push(`${when} · ${whyText(t.waiting.why)}`);
  }
  return out;
}
