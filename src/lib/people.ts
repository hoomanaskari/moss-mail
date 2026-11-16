import type { Account, Person, RecipientPick } from "../data/types";
import { SEP } from "./text";

/** Someone outside the account's organisation. Accounts without an organisation have no outsiders. */
export function isExternal(person: Person, account: Account): boolean {
  if (!account.orgDomain) return false;
  return !person.email.toLowerCase().endsWith(`@${account.orgDomain.toLowerCase()}`);
}

const ADDRESS = /^[^\s@<>,;"]+@[^\s@<>,;".]+(\.[^\s@<>,;".]+)+$/;

/** "Lena Vogt" → "LV"; an address → its first letter. */
export const initialsOf = (name: string) =>
  (name.includes("@") ? name[0] : name.split(/\s+/).map((w) => w[0]).slice(0, 2).join("")).toUpperCase();

/**
 * Someone to send to from a typed or pasted address, "lena@x.io" or "Lena Vogt <lena@x.io>".
 * Nothing until the address is complete. With no name, the address is the name, which Rust sends bare.
 */
export function personFromAddress(text: string): Person | undefined {
  const named = /^(.*?)\s*<([^>]*)>$/.exec(text.trim());
  const email = (named ? named[2] : text).trim();
  if (!ADDRESS.test(email)) return undefined;
  const name = named?.[1].replace(/^"(.*)"$/, "$1").trim() || email;
  return { name, email, initials: initialsOf(name) };
}

/** "to you, Lena · cc Jonas, Ines" */
export function recipientSummary(to: Person[], cc: Person[], myAddress: string): string {
  const name = (p: Person) => (p.email === myAddress ? "you" : (p.short ?? p.name));
  const parts = [`to ${to.map(name).join(", ")}`];
  if (cc.length) parts.push(`cc ${cc.map(name).join(", ")}`);
  return parts.join(SEP);
}

/**
 * `people` with whoever you chose before for the same or similar text first, the newest
 * choice first. Similar is text that starts the same: "ro" and "ross" either way round.
 */
export function rankByPicks(people: Person[], typed: string, picks: RecipientPick[]): Person[] {
  const q = typed.trim().toLowerCase();
  const order = new Map<string, number>();
  picks.forEach((p, i) => {
    if ((p.typed.startsWith(q) || q.startsWith(p.typed)) && !order.has(p.email)) order.set(p.email, i);
  });
  const rank = (p: Person) => order.get(p.email.toLowerCase()) ?? Infinity;
  return [...people].sort((a, b) => rank(a) - rank(b));
}
