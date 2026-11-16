/**
 * Your addresses: one login can have several, e.g. custom domains on iCloud. Which ones you can
 * send from, which one a reply goes from, and which one an email came to.
 */
import type { Account, AccountAddress, Thread } from "../data/types";

/** An address you can send from, with the name mail from it shows. */
export interface SendingAddress {
  accountId: string;
  email: string;
  name: string;
  /** The address the account signs in with. */
  primary: boolean;
}

const lower = (email: string) => email.trim().toLowerCase();

/** One of an account's other addresses. */
export function addressOf(account: Account, email: string): AccountAddress | undefined {
  const key = lower(email);
  return account.addresses?.find((a) => a.email === key);
}

const sendable = (a: AccountAddress) => a.state === "added" && !a.blocked;

/** The account's own address first, then the ones you added and didn't block. */
export function sendingAddresses(account: Account): SendingAddress[] {
  const own = { accountId: account.id, email: account.address, name: account.senderName, primary: true };
  const others = (account.addresses ?? [])
    .filter(sendable)
    .map((a) => ({ accountId: account.id, email: a.email, name: a.name ?? account.senderName, primary: false }));
  return [own, ...others];
}

export const allSendingAddresses = (accounts: Account[]) => accounts.flatMap(sendingAddresses);

/** The address mail goes from: `email` when you can send from it, otherwise the account's own. */
export function sendingAddress(account: Account, email?: string): SendingAddress {
  const all = sendingAddresses(account);
  return (email && all.find((a) => a.email === lower(email))) || all[0];
}

/** Addresses Moss found mail to and asks you about. */
export const suggestedAddresses = (account: Account) =>
  (account.addresses ?? []).filter((a) => a.state === "suggested" && !a.blocked);

/** Addresses whose mail goes to Blocked. */
export const blockedAddresses = (account: Account) => (account.addresses ?? []).filter((a) => a.blocked);

/** Whether `email` is yours: an account's own address or one you added. */
export function isMine(email: string, accounts: Account[]) {
  const key = lower(email);
  return accounts.some((a) => lower(a.address) === key || addressOf(a, key)?.state === "added");
}

export const isValidAddress = (email: string) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());

/**
 * Which address a reply goes from: the one the email came to, when you can send from it. `reason`
 * is shown beside From. No address means the account's own.
 */
export function replyFrom(thread: Thread, account: Account): { address?: string; reason?: string } {
  const to = thread.toAddress;
  if (!to || to === lower(account.address)) return {};
  if (sendingAddresses(account).some((a) => a.email === to)) return { address: to, reason: `It came to ${to}` };
  return { reason: `It came to ${to}. Add it in Settings to reply from it` };
}

/** Whether a thread is in the list for this account (or "all") and, when given, this address. */
export function inView(t: Thread, accountFilter: string, addressFilter?: string) {
  if (accountFilter !== "all" && t.accountId !== accountFilter) return false;
  return !addressFilter || t.toAddress === addressFilter;
}

/**
 * The address row above an account's inbox: how many threads came to each address. The account's
 * own address first, then the rest, most mail first. Addresses you send from show even with no
 * mail; others only with some. Empty when there's nothing to choose between.
 */
export function addressCounts(threads: Thread[], account: Account): { email: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const t of threads) if (t.accountId === account.id && t.toAddress) counts.set(t.toAddress, (counts.get(t.toAddress) ?? 0) + 1);
  const own = lower(account.address);
  const sending = sendingAddresses(account).filter((a) => !a.primary).map((a) => a.email);
  const others = [...new Set([...sending, ...counts.keys()])].filter((e) => e !== own && !addressOf(account, e)?.blocked);
  if (!others.length) return [];
  const rest = others.map((email) => ({ email, count: counts.get(email) ?? 0 })).sort((a, b) => b.count - a.count);
  return [{ email: own, count: counts.get(own) ?? 0 }, ...rest];
}
