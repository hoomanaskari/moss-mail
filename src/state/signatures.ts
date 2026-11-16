import { create } from "zustand";
import type { Account, Thread } from "../data/types";
import { isMine } from "../lib/addresses";
import { mail, quietly } from "../services/mail";

export interface Signature {
  id: string;
  name: string;
  /** Plain text, one entry per line. Web addresses become links. */
  lines: string[];
}

/** Which signature an account adds; null means none. */
export interface AccountSignatures {
  newEmails: string | null;
  replies: string | null;
}

/** What the desktop app keeps between launches. */
export interface SavedSignatures {
  signatures: Signature[];
  byAccount: Record<string, AccountSignatures>;
  /** Addresses (lowercase) with a choice of their own; the rest use their account's. None from an older engine. */
  byAddress?: Record<string, AccountSignatures>;
  /** Moss decides per email whether to add one, starting from each account's choice. */
  letMossChoose: boolean;
  /** Signature ids, most recently used first. The composer's picker offers them in this order. */
  recent: string[];
}

interface SignaturesState extends SavedSignatures {
  byAddress: Record<string, AccountSignatures>;
  setChoice(accountId: string, kind: keyof AccountSignatures, signatureId: string | null): void;
  /** One address's choice; "same" follows its account for that kind of email. */
  setAddressChoice(accountId: string, email: string, kind: keyof AccountSignatures, signatureId: string | null | "same"): void;
  setLetMossChoose(on: boolean): void;
  /** Moves a signature to the front of `recent`, e.g. after you pick it for an email. */
  markUsed(id: string): void;
  updateSignature(id: string, patch: Partial<Pick<Signature, "name" | "lines">>): void;
  /** Adds a signature starting with your name, and returns its id. */
  addSignature(name: string): string;
  /** Deletes a signature; accounts that used it get none. */
  deleteSignature(id: string): void;
}

let nextId = 1;
let saving: Promise<unknown> = Promise.resolve();

/**
 * Saves one change after the ones before it, failed or not: the engine runs commands side by
 * side, so a quick edit could otherwise land before the one it follows.
 */
function save(change: () => Promise<void>) {
  saving = saving.then(change, change);
  quietly(saving);
}

/** Resolves once every change so far has been saved, or has failed. */
export const signaturesSaved = () => saving.catch(() => {});

/**
 * Signatures: the demo ones in the browser and dev scenes; in the desktop app `startLive`
 * swaps in the saved ones, and every change here is saved. Kept apart from the mail store
 * so Settings and the composer can share it.
 */
export const useSignatures = create<SignaturesState>()((set, get) => ({
  signatures: [
    { id: "work-full", name: "Work (full)", lines: ["Maya Ross", "Design lead, Northwind", "northwind.co"] },
    { id: "work-short", name: "Work (short)", lines: ["Maya"] },
    { id: "studio", name: "Studio", lines: ["Maya Ross", "Ross Studio · Photography and set design", "rossstudio.art"] },
    { id: "personal", name: "Personal", lines: ["Maya Ross"] },
  ],
  byAccount: {
    work: { newEmails: "work-full", replies: "work-short" },
    personal: { newEmails: null, replies: null },
    studio: { newEmails: "studio", replies: "studio" },
    gmail: { newEmails: "personal", replies: null },
  },
  byAddress: {},
  letMossChoose: true,
  setChoice: (accountId, kind, signatureId) => {
    set((s) => ({
      byAccount: {
        ...s.byAccount,
        [accountId]: { ...(s.byAccount[accountId] ?? { newEmails: null, replies: null }), [kind]: signatureId },
      },
    }));
    save(() => mail().setSignatureChoice(accountId, kind, signatureId));
  },
  setAddressChoice: (accountId, email, kind, signatureId) => {
    const account = get().byAccount[accountId] ?? { newEmails: null, replies: null };
    const own = { ...(get().byAddress[email] ?? account), [kind]: signatureId === "same" ? account[kind] : signatureId };
    // Both the same as the account's: it follows the account again.
    const choice = own.newEmails === account.newEmails && own.replies === account.replies ? null : own;
    set((s) => {
      const { [email]: _, ...rest } = s.byAddress;
      return { byAddress: choice ? { ...rest, [email]: choice } : rest };
    });
    save(() => mail().setAddressSignatures(accountId, email, choice));
  },
  setLetMossChoose: (letMossChoose) => {
    set({ letMossChoose });
    save(() => mail().setLetMossChoose(letMossChoose));
  },
  recent: ["work-short", "work-full", "studio", "personal"],
  markUsed: (id) => {
    set((s) => ({ recent: [id, ...s.recent.filter((x) => x !== id)] }));
    save(() => mail().markSignatureUsed(id));
  },
  updateSignature: (id, patch) => {
    set((s) => ({ signatures: s.signatures.map((x) => (x.id === id ? { ...x, ...patch } : x)) }));
    const saved = get().signatures.find((x) => x.id === id);
    if (saved) save(() => mail().saveSignature(saved));
  },
  addSignature: (sender) => {
    const taken = new Set(get().signatures.map((x) => x.id));
    let id = `signature-${nextId++}`;
    // Saved ones from before a restart keep their ids.
    while (taken.has(id)) id = `signature-${nextId++}`;
    const count = get().signatures.filter((x) => x.name.startsWith("New signature")).length;
    const added = { id, name: count ? `New signature ${count + 1}` : "New signature", lines: [sender] };
    set((s) => ({ signatures: [...s.signatures, added], recent: [...s.recent, id] }));
    save(() => mail().saveSignature(added));
    return id;
  },
  deleteSignature: (id) => {
    set((s) => ({
      signatures: s.signatures.filter((x) => x.id !== id),
      recent: s.recent.filter((x) => x !== id),
      byAccount: Object.fromEntries(
        Object.entries(s.byAccount).map(([account, c]) => [
          account,
          { newEmails: c.newEmails === id ? null : c.newEmails, replies: c.replies === id ? null : c.replies },
        ]),
      ),
      byAddress: Object.fromEntries(
        Object.entries(s.byAddress).map(([email, c]) => [
          email,
          { newEmails: c.newEmails === id ? null : c.newEmails, replies: c.replies === id ? null : c.replies },
        ]),
      ),
    }));
    save(() => mail().deleteSignature(id));
  },
}));

/**
 * Where a signature line links to, when it is a web address ("northwind.co"). Sent mail uses
 * the same rule (`link` in crates/moss-engine/src/compose.rs), so it links what the composer showed.
 */
export function linkFor(line: string) {
  if (!/^(https?:\/\/)?[\w-]+(\.[\w-]+)+(\/\S*)?$/i.test(line)) return undefined;
  return /^https?:\/\//i.test(line) ? line : `https://${line}`;
}

/** One-line summary for pickers: the first line and the first part of the second, e.g. "Maya Ross · Ross Studio". */
export function preview(sig: Signature) {
  return sig.lines
    .slice(0, 2)
    .map((l) => l.split(" · ")[0])
    .join(" · ");
}

/** "Used by Work · new emails", "Used by Studio" (both), or "Not used yet". */
export function usedBy(signatureId: string, accounts: Account[]) {
  const { byAccount } = useSignatures.getState();
  const uses = accounts.flatMap((a) => {
    const c = byAccount[a.id];
    if (!c) return [];
    const isNew = c.newEmails === signatureId;
    const isReply = c.replies === signatureId;
    if (isNew && isReply) return [a.name];
    if (isNew) return [`${a.name} · new emails`];
    if (isReply) return [`${a.name} · replies`];
    return [];
  });
  return uses.length ? `Used by ${uses.join(", ")}` : "Not used yet";
}

/** What an email is, for picking its signature. */
export type EmailKind = "new" | "reply" | "forward";

/** The email a signature is picked for. */
export interface SignatureEmail {
  accountId: string;
  /** The From address, when it isn't the account's own. */
  address?: string;
  kind: EmailKind;
  /** How many messages in the thread you already sent: a back-and-forth once there are some. */
  signedBefore: number;
}

/** Why an email got the signature it got: the address's choice for new emails or replies, a forward, a one-line one or none in a back-and-forth, or none set. */
export type PickWhy = "new" | "reply" | "forward" | "short" | "quick" | "noneSet";

/** The signature an email gets (null: none) and why. */
export interface SignaturePick {
  id: string | null;
  why: PickWhy;
}

/** What a pick reads: your signatures and which one each account and address adds. */
export type PickSettings = Pick<SavedSignatures, "signatures" | "byAccount" | "letMossChoose"> & {
  byAddress: Record<string, AccountSignatures>;
};

/**
 * Picks the signature for an email. Mirrors `pick_signature` in crates/moss-core/src/signatures.rs.
 *
 * Starts from the From address's choice (or its account's) for new emails or replies. With
 * "Let Moss choose" on, a forward gets the one for new emails, and once you've signed earlier in
 * the thread a reply gets a one-line one or none.
 */
export function pickSignature(settings: PickSettings, email: SignatureEmail): SignaturePick {
  const choice = (email.address && settings.byAddress[email.address.toLowerCase()]) ||
    settings.byAccount[email.accountId] || { newEmails: null, replies: null };
  const auto = settings.letMossChoose;
  const find = (id: string | null) => settings.signatures.find((x) => x.id === id);
  const [slot, why]: [string | null, PickWhy] =
    email.kind === "new"
      ? [choice.newEmails, "new"]
      : email.kind === "forward" && auto
        ? [choice.newEmails, "forward"]
        : [choice.replies, "reply"];
  const sig = find(slot);
  if (!sig) return { id: null, why: "noneSet" };
  if (!(auto && email.kind === "reply" && email.signedBefore > 0)) return { id: sig.id, why };
  const short = [sig, find(choice.newEmails)].find((x) => x && oneLine(x));
  return short ? { id: short.id, why: "short" } : { id: null, why: "quick" };
}

/** Just your name, say: blank lines don't count. */
const oneLine = (sig: Signature) => sig.lines.filter((l) => l.trim()).length <= 1;

/** How many messages in `thread` you sent: from any of your addresses. */
export function signedBefore(thread: Thread | undefined, accounts: Account[]) {
  return thread?.messages.filter((m) => isMine(m.from.email, accounts)).length ?? 0;
}

/**
 * The signatures the picker offers, in order: Moss's pick, then recently used ones, then any
 * other saved one, so every saved signature can be picked.
 */
export function offered(pickId: string | null, { signatures, recent }: Pick<SavedSignatures, "signatures" | "recent"> = useSignatures.getState()) {
  return [...new Set([pickId, ...recent, ...signatures.map((x) => x.id)])].flatMap((id) => signatures.filter((x) => x.id === id));
}

/** Picks from the signatures you have now. */
export const pickFor = (email: SignatureEmail) => pickSignature(useSignatures.getState(), email);

/** The line beside the signature chip and the picker's explanation, once you picked one (or none) yourself. */
export const chosenReason = { reason: "Chosen for this email", why: "You chose this one for this email." };

/** The line beside the signature chip and the picker's explanation, for a pick from `address`. */
export function pickReason(pick: SignaturePick, address: string) {
  switch (pick.why) {
    case "new":
      return { reason: `Your signature for ${address}`, why: `Picked for new emails from ${address}.` };
    case "reply":
      return { reason: `Your reply signature for ${address}`, why: `Picked for replies from ${address}.` };
    case "forward":
      return { reason: `Your signature for ${address}`, why: "A forward goes to someone new, so it gets your one for new emails." };
    case "short":
      return {
        reason: "Short one: you signed earlier in this thread",
        why: "You already signed in this thread, so Moss picked your one-line signature.",
      };
    case "quick":
      return { reason: "Left out: quick reply in a back-and-forth", why: "You already signed in this thread, so Moss left it out." };
    case "noneSet":
      return { reason: `None set for ${address}`, why: `No signature set for ${address} here. Pick one, or set one in Settings.` };
  }
}
