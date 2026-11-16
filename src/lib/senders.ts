/** Sender rules shared with the Rust store (`classify.rs`): keep the two in step. */

/** Free mail services: one person there says nothing about anyone else there. */
const publicDomains = new Set([
  "gmail.com", "googlemail.com", "icloud.com", "me.com", "mac.com", "aol.com", "proton.me",
  "protonmail.com", "pm.me", "fastmail.com", "gmx.com", "gmx.net", "mail.com", "zoho.com",
  "yandex.com", "hey.com", "tutanota.com", "tuta.io", "qq.com", "163.com", "msn.com",
]);
/** Free mail services with a domain per country, such as yahoo.co.uk or hotmail.fr. */
const publicPrefixes = ["yahoo.", "ymail.", "hotmail.", "outlook.", "live.", "gmx.", "web."];

/** Whether a domain belongs to a free mail service, so it can never be blocked as a whole. */
export function isPublicDomain(domain: string): boolean {
  const d = domain.toLowerCase();
  return publicDomains.has(d) || publicPrefixes.some((p) => d.startsWith(p));
}

/** The lowercase domain of an address, if it has one. */
export function domainOf(email: string): string | undefined {
  const at = email.trim().lastIndexOf("@");
  const domain = at < 0 ? "" : email.trim().slice(at + 1);
  return domain ? domain.toLowerCase() : undefined;
}
