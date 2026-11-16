/**
 * Files that go with an email: their sizes, and how much each mail service takes. The same
 * limits as `crates/moss-engine/src/attach.rs`, which refuses the send if this check is missed.
 */
import type { ProviderId } from "../data/types";

const MB = 1024 * 1024;

/**
 * The most file data an email from `provider` can carry. Gmail and Yahoo take 25 MB, Outlook
 * and iCloud 20 MB, Fastmail 50 MB. Resend takes 40 MB once encoded, about 30 MB of files.
 * Other servers (and demo accounts) say nothing ahead of time: 25 MB is the usual.
 */
export function sendLimit(provider: ProviderId | undefined): number {
  const mb: Partial<Record<ProviderId, number>> = { outlook: 20, icloud: 20, fastmail: 50, resend: 30 };
  return (mb[provider ?? "imap"] ?? 25) * MB;
}

const names: Partial<Record<ProviderId, string>> = {
  gmail: "Gmail",
  outlook: "Outlook",
  icloud: "iCloud",
  yahoo: "Yahoo",
  fastmail: "Fastmail",
  resend: "Resend",
};

/** "31.2 MB": a total, to one decimal place. */
export function megabytes(bytes: number) {
  return `${(bytes / MB).toFixed(1).replace(/\.0$/, "")} MB`;
}

/** Why `total` bytes of files are too many for `provider`, or nothing when they fit. */
export function tooBig(total: number, provider: ProviderId | undefined): string | undefined {
  const limit = sendLimit(provider);
  if (total <= limit) return undefined;
  const name = names[provider ?? "imap"] ?? "Your mail server";
  return `These files are ${megabytes(total)}. ${name} sends up to ${megabytes(limit)}.`;
}

/** "512 B", "240 KB", "1.2 MB": a file's size, as received files show theirs. */
export function fileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < MB) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < MB * 1024) return `${(bytes / MB).toFixed(1)} MB`;
  return `${(bytes / MB / 1024).toFixed(1)} GB`;
}
