/**
 * What a mail link (`mailto:`) asks for: who to write to, a subject and the start of the email.
 * Websites and other apps make these links, so they are read the way RFC 6068 writes them and
 * forgiven where they aren't.
 */
import type { Person } from "../data/types";
import { personFromAddress } from "./people";

export interface MailLink {
  to: Person[];
  cc: Person[];
  bcc: Person[];
  subject: string;
  body: string;
}

/** Percent-encoded text, or the text as written when a website encoded it badly. */
function decode(text: string) {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

/** "a@x.io, \"Vogt, Lena\" <b@x.io>" → each address, splitting only on commas outside quotes. */
function split(list: string): string[] {
  const parts = [""];
  let quoted = false;
  for (const char of list) {
    if (char === '"') quoted = !quoted;
    if (char === "," && !quoted) parts.push("");
    else parts[parts.length - 1] += char;
  }
  return parts;
}

/** Everyone in `lists`, once each, leaving out what isn't an address. */
function people(lists: string[]): Person[] {
  const seen = new Set<string>();
  return lists
    .flatMap((list) => split(decode(list)))
    .map(personFromAddress)
    .filter((p): p is Person => !!p && !seen.has(p.email.toLowerCase()) && !!seen.add(p.email.toLowerCase()));
}

const text = (value: string) => decode(value).replace(/\r\n?/g, "\n");

/** The new email `url` asks for, or nothing when it isn't a mail link. */
export function parseMailLink(url: string): MailLink | undefined {
  const link = /^mailto:([^?]*)(?:\?(.*))?$/is.exec(url.trim());
  if (!link) return undefined;
  const fields: Record<string, string[]> = { to: [link[1]], cc: [], bcc: [], subject: [], body: [] };
  for (const part of (link[2] ?? "").split("&")) {
    const at = part.indexOf("=");
    if (at < 0) continue;
    fields[decode(part.slice(0, at)).toLowerCase()]?.push(part.slice(at + 1));
  }
  return {
    to: people(fields.to),
    cc: people(fields.cc),
    bcc: people(fields.bcc),
    subject: fields.subject.length ? text(fields.subject[0]) : "",
    body: fields.body.map(text).join("\n"),
  };
}
