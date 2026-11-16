/**
 * Summaries of a conversation in the kind you pick, the ones kept for it, and "always
 * summarise mail from…" rules. In the desktop app this is the Rust side (`tauriSummaries`):
 * `moss_engine::summarize` and `moss_ai::summaries`. In the browser, dev scenes and tests it is
 * the demo below, which can be put in every state the UI draws.
 */
import { threads as demoThreads } from "../data/demo";
import { isPublicDomain } from "../lib/senders";
import { firstHalf } from "./replies";

/** Short: 1–2 lines. Detailed: a few paragraphs. Actions: what you're asked to do. Facts: people, dates, amounts, links. Custom: your own instruction. */
export type SummaryKind = "short" | "detailed" | "actions" | "facts" | "custom";

export const summaryKinds: SummaryKind[] = ["short", "detailed", "actions", "facts", "custom"];

/** A kind, and for Custom your instruction (other kinds have none). */
export interface SummaryRequest {
  kind: SummaryKind;
  instruction?: string;
}

/** One thing to do. `due` is the date as the email gives it: "Friday, 13 November". */
export interface ActionItem {
  text: string;
  due?: string;
}

export type FactKind = "person" | "date" | "amount" | "link" | "other";

/** One fact: `value` is the fact ("€18,400"), `detail` what it is ("new Q4 budget total"). Links are always ones in the email. */
export interface Fact {
  kind: FactKind;
  value: string;
  detail?: string;
}

/**
 * What the UI renders, by `format`. Text (Short, Detailed, Custom): paragraphs split by a
 * blank line, separate points on lines of their own. Actions: most pressing first; none means
 * nothing is asked of you. Facts: in the order found.
 */
export type SummaryContent =
  | { format: "text"; text: string }
  | { format: "actions"; items: ActionItem[] }
  | { format: "facts"; items: Fact[] };

/** Why a summary is there, for its label: you asked, a rule for its sender, or automatic (it's in Important, or you've written in it). */
export type SummaryWhy = "asked" | "rule" | "important" | "replied";

export interface SummaryView extends SummaryRequest {
  content: SummaryContent;
  why: SummaryWhy;
  /** The writing model that wrote it. */
  model: string;
  /** When (ms). */
  madeAt: number;
  /** A message came after it: show it, and offer to write it again. */
  stale: boolean;
}

/** "Always summarise mail from `sender` as `kind`". `sender` is a lowercase address, or a domain for everyone there and its subdomains. Rules are for every account. */
export interface SummaryRule extends SummaryRequest {
  id: number;
  sender: string;
  createdAt: number;
  /** The person's name, for an address Moss has mail from. */
  name?: string;
}

/** The summary kept for a conversation by the chosen writing model (the kind you chose last, else the automatic one-liner), and the rule for it. */
export interface ThreadSummaries {
  summary?: SummaryView;
  rule?: SummaryRule;
}

/**
 * Why a summary or rule wasn't made. `notReady`: the writing model isn't downloaded (offer
 * Settings → AI). `model` or `server`: it failed (offer Try again). `invalid`: a rule's sender
 * or a custom instruction it can't use; `message` says why, ready to show.
 */
export interface SummaryError {
  kind: "notReady" | "model" | "server" | "invalid" | "store" | (string & {});
  message: string;
}

export interface SummaryBackend {
  /** What's kept for a conversation: shown at once on opening, no model needed. */
  kept(threadId: string): Promise<ThreadSummaries>;
  /** A summary in this kind: the kept one when it's fresh (unless `again`), otherwise written now, replacing the one kept. Rejects with a {@link SummaryError}. */
  summarize(threadId: string, request: SummaryRequest, again?: boolean, onContent?: (content: SummaryContent) => void): Promise<SummaryView>;
  /** Discards a conversation's summary, and its one-liner in the list. None is written in the background until a new message comes. */
  discard(threadId: string): Promise<void>;
  rules(): Promise<SummaryRule[]>;
  /** Makes a rule (no `id`) or changes one. A sender has one rule: saving another for it replaces it. */
  saveRule(rule: SummaryRequest & { id?: number; sender: string }): Promise<SummaryRule>;
  /** Deletes a rule; false when there was none. What it wrote stays. */
  removeRule(id: number): Promise<boolean>;
  /** Whether Moss writes the one-liners of conversations that matter by itself. */
  automatic(): Promise<boolean>;
  /** Off removes the one-liners from the list; on writes them again in the background. */
  setAutomatic(on: boolean): Promise<void>;
  /** Calls `handler` with a thread's id when a rule wrote its summary in the background; returns a function that stops it. */
  onWritten(handler: (threadId: string) => void): () => void;
}

export const MAX_INSTRUCTION = 500;

const invalid = (message: string): SummaryError => ({ kind: "invalid", message });

/** A request as the Rust side takes it: Custom needs an instruction, trimmed; other kinds drop theirs. Like `SummaryRequest::new`. */
export function checkRequest(request: SummaryRequest): SummaryRequest {
  if (request.kind !== "custom") return { kind: request.kind };
  const instruction = (request.instruction ?? "").trim();
  if (!instruction) throw invalid("say how to summarise it");
  if ([...instruction].length > MAX_INSTRUCTION) throw invalid(`keep the instruction under ${MAX_INSTRUCTION} characters`);
  return { kind: "custom", instruction };
}

/** A rule's sender as the Rust side keeps it: a lowercase address, or a domain without "@". Like `summary_rules::sender`. */
export function ruleSender(text: string): string {
  let s = text.trim().toLowerCase();
  if (s.startsWith("mailto:")) s = s.slice(7);
  s = s.replace(/^@+/, "");
  const bad = () => invalid(`“${s}” isn’t an email address or a domain`);
  const at = s.indexOf("@");
  const [local, domain] = at < 0 ? [null, s] : [s.slice(0, at), s.slice(at + 1)];
  const domainOk = domain.includes(".") && !domain.startsWith(".") && !domain.endsWith(".") && /^[\p{L}\p{N}.-]+$/u.test(domain);
  if (!domainOk) throw bad();
  if (local !== null) {
    if (!local || /\s/.test(local)) throw bad();
    return s;
  }
  if (isPublicDomain(domain)) throw invalid(`${domain} is a public mail service: make the rule for one address there`);
  return domain;
}

/** The rule for mail from `email`: its address over a domain, a longer domain over a shorter one. Like `summary_rule_for`. */
export function ruleFor(rules: SummaryRule[], email: string): SummaryRule | undefined {
  const from = email.trim().toLowerCase();
  const applies = (r: SummaryRule) => r.sender === from || (!r.sender.includes("@") && (from.endsWith(`@${r.sender}`) || from.endsWith(`.${r.sender}`)));
  // A domain that applies is part of the address, so the longest rule is the most specific.
  return rules.filter(applies).sort((a, b) => b.sender.length - a.sender.length)[0];
}

/** The demo's summaries, of the Q4 budget conversation drawn in design.pen. */
export const demoContent: Record<SummaryKind, SummaryContent> = {
  short: { format: "text", text: "Daniel needs you to approve the €18,400 Q4 design budget by Thursday; user testing is the new line." },
  detailed: {
    format: "text",
    text:
      "Daniel updated the Q4 design budget after Tuesday’s planning call; the new total is €18,400.\nThe main change is a new line for user testing before the onboarding launch.\nHe needs your approval by Thursday, or what you’d change.\n\n" +
      "Lena, Jonas and Inês are copied in.\n\n" +
      "Other updates include:\nThe Q3 spend review is done and the vendor invoices are paid.\nThe planning call notes are in the shared folder.",
  },
  actions: {
    format: "actions",
    items: [
      { text: "Approve the Q4 design budget or say what to change", due: "Thursday" },
      { text: "Check the new user testing line" },
    ],
  },
  facts: {
    format: "facts",
    items: [
      { kind: "person", value: "Daniel Okafor", detail: "sent the budget" },
      { kind: "amount", value: "€18,400", detail: "new Q4 design budget total" },
      { kind: "date", value: "Thursday", detail: "approval needed by" },
      { kind: "other", value: "User testing", detail: "new budget line, before the onboarding launch" },
    ],
  },
  custom: { format: "text", text: "- Budget: €18,400\n- Needed by: Thursday\n- New: user testing" },
};

/** Where the demo starts. `state` is how asking for a summary goes: written at once, never answering (loading), part-way written and never finishing (writing), no model, or failing. */
export interface DemoSummaryStart {
  state?: "ready" | "loading" | "writing" | "notReady" | "failed";
  /** The summary already kept, by thread id. */
  kept?: Record<string, SummaryView>;
  rules?: SummaryRule[];
  automatic?: boolean;
  /** Who a thread is from, for its rule. Defaults to the demo mail. */
  senderOf?: (threadId: string) => string | undefined;
}

export const demoModel = "e4b";

/** A demo summary as it would come back. */
export function demoView(request: SummaryRequest, extra: Partial<SummaryView> = {}): SummaryView {
  return { ...request, content: demoContent[request.kind], why: "asked", model: demoModel, madeAt: Date.UTC(2026, 10, 1, 9, 20), stale: false, ...extra };
}

/** The first part of a summary as it streams in: half the text, or the first item. */
function partOf(content: SummaryContent): SummaryContent {
  return content.format === "text" ? { ...content, text: firstHalf(content.text) } : ({ ...content, items: content.items.slice(0, 1) } as SummaryContent);
}

const same = (a: SummaryRequest, b: SummaryRequest) => a.kind === b.kind && (a.instruction ?? "") === (b.instruction ?? "");

/** The demo summaries. Each kind comes back as {@link demoContent}; one is kept per thread, like the Rust side. */
export function demoSummaries(start: DemoSummaryStart = {}): SummaryBackend {
  const state = start.state ?? "ready";
  const kept = new Map(Object.entries(start.kept ?? {}));
  let rules = [...(start.rules ?? [])];
  let automatic = start.automatic ?? true;
  let nextId = Math.max(0, ...rules.map((r) => r.id)) + 1;
  const senderOf = start.senderOf ?? ((id: string) => demoThreads.find((t) => t.id === id)?.from.email);
  /** Like the Rust side, a person's name comes from the mail you have from them. */
  const nameOf = (sender: string) => demoThreads.find((t) => t.from.email.toLowerCase() === sender)?.from.name;
  const ruleOf = (threadId: string) => ruleFor(rules, senderOf(threadId) ?? "");
  return {
    kept: async (threadId) => {
      const rule = ruleOf(threadId);
      const summary = kept.get(threadId);
      return { ...(summary && { summary }), ...(rule && { rule }) };
    },
    summarize: async (threadId, sent, again = false, onContent) => {
      const request = checkRequest(sent);
      const found = kept.get(threadId);
      if (found && same(found, request) && !found.stale && !again) return found;
      if (state === "loading") return new Promise<SummaryView>(() => {});
      if (state === "notReady") throw { kind: "notReady", message: "the AI model isn't downloaded yet" } satisfies SummaryError;
      if (state === "failed") throw { kind: "model", message: "the AI model failed: it wrote no summary" } satisfies SummaryError;
      onContent?.(partOf(demoContent[request.kind]));
      if (state === "writing") return new Promise<SummaryView>(() => {});
      onContent?.(demoContent[request.kind]);
      const rule = ruleOf(threadId);
      const view = demoView(request, { why: rule && same(rule, request) ? "rule" : "asked", madeAt: Date.now() });
      kept.set(threadId, view);
      return view;
    },
    discard: async (threadId) => {
      kept.delete(threadId);
    },
    rules: async () => [...rules].sort((a, b) => a.sender.localeCompare(b.sender)),
    saveRule: async ({ id, sender: text, ...sent }) => {
      const request = checkRequest(sent);
      const sender = ruleSender(text);
      const existing = id === undefined ? rules.find((r) => r.sender === sender) : rules.find((r) => r.id === id);
      if (id !== undefined && !existing) throw { kind: "store", message: `no summary rule with id ${id}` } satisfies SummaryError;
      const name = nameOf(sender);
      const rule: SummaryRule = { id: existing?.id ?? nextId++, sender, ...request, createdAt: existing?.createdAt ?? Date.now(), ...(name && { name }) };
      rules = [...rules.filter((r) => r.id !== rule.id && r.sender !== sender), rule];
      return rule;
    },
    removeRule: async (id) => {
      const before = rules.length;
      rules = rules.filter((r) => r.id !== id);
      return rules.length < before;
    },
    automatic: async () => automatic,
    setAutomatic: async (on) => {
      automatic = on;
    },
    onWritten: () => () => {},
  };
}

let current: SummaryBackend = demoSummaries();

export const summaries = () => current;

export function setSummaries(backend: SummaryBackend) {
  current = backend;
}
