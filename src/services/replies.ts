/**
 * AI replies for the composer: quick replies to pick from, a reply from your own words, and
 * rewrites of what's in the composer. In the desktop app it is the Rust side (`tauriReplies`,
 * the writing model in `moss_ai::reply`); in the browser, dev scenes and tests it is the demo
 * below. The AI only writes text for the composer: it never sends and never marks mail Done.
 */
import type { ReplyOption } from "../data/types";

/** What to write. Rewrites change the composer's text; the rest write a reply, or rework the text when there is some. */
export type ReplyJob =
  | { kind: "instructions"; instructions: string }
  | { kind: "translate"; language: string }
  | { kind: "friendlier" | "formal" | "shorter" | "longer" | "fix" | "accept" | "decline" | "moreTime" };

export interface ReplyRequest {
  /** Names the stream, so `stop` can stop it. Any id the UI likes. */
  id: string;
  /** The conversation replied to; none for a new email. */
  threadId?: string;
  /** The account replying, so it sounds like you. */
  accountId: string;
  job: ReplyJob;
  /** The composer's text: plain text, paragraphs split by blank lines. */
  text: string;
  /** The lines of the signature that goes below the reply, so the reply ends to fit it. None when it gets none. */
  signature?: string[];
}

/** Why the AI couldn't write: `notReady` when the writing model isn't downloaded, `model` or `server` when it failed. */
export interface ReplyError {
  kind: "notReady" | "model" | "server" | "unknownAccount" | "unknownThread";
  message: string;
}

/** What Quick reply asks for: replies to `threadId` as `accountId`, to go above `signature` (its lines). */
export interface QuickRepliesRequest {
  /** Names the stream, so `stop` can stop it. */
  id: string;
  threadId: string;
  accountId: string;
  signature?: string[];
}

export interface ReplyBackend {
  /**
   * Two or three quick replies to a conversation, written one after another. `onOptions` hears
   * the replies so far each time one grows, the one being written last. `stop` keeps those.
   */
  options(request: QuickRepliesRequest, onOptions: (options: ReplyOption[]) => void): Promise<ReplyOption[]>;
  /**
   * Writes `request.job`. `onText` hears the whole text so far each time it grows; the promise
   * gives the finished text (also after `stop`). Rewriting an empty composer gives "".
   */
  write(request: ReplyRequest, onText: (text: string) => void): Promise<string>;
  /** Stops a write; what it wrote so far stays. `false` when it isn't running. */
  stop(id: string): Promise<boolean>;
}

/** Whether `job` rewrites the composer's text, so it does nothing when the composer is empty. Same rule as Rust's `reply::prompt`. */
export function rewritesText(job: ReplyJob): boolean {
  return !["instructions", "accept", "decline", "moreTime"].includes(job.kind);
}

/** Where the demo stands: `writing` stays mid-way through every write until it's stopped. */
export type DemoReplyState = "ready" | "writing" | "missing" | "failed";

export interface DemoRepliesStart {
  state?: DemoReplyState;
  options?: ReplyOption[];
}

const demoOptions: ReplyOption[] = [
  { label: "Yes", draft: "Hi,\n\nYes, that works for me. Thanks for sorting it out." },
  { label: "No", draft: "Hi,\n\nThanks for asking, but I can’t make it this time." },
  { label: "Need more info", draft: "Hi,\n\nCould you tell me a bit more before I decide? What would you need from me?" },
];

const answers = {
  accept: "Hi,\n\nYes, happy to. That works for me.",
  decline: "Hi,\n\nThanks for thinking of me, but I can’t this time.",
  moreTime: "Hi,\n\nCould I have a few more days on this? I’ll come back to you as soon as I can.",
};

const sentence = (text: string) => {
  const t = text.trim();
  return `${t.charAt(0).toUpperCase()}${t.slice(1)}${/[.!?]$/.test(t) ? "" : "."}`;
};

/** What the demo writes for `job`: canned answers, and rewrites you can recognise. */
function demoText(job: ReplyJob, text: string): string {
  switch (job.kind) {
    case "instructions":
      return `Hi,\n\nThanks for this. ${sentence(job.instructions)}`;
    case "accept":
    case "decline":
    case "moreTime":
      return answers[job.kind];
    case "shorter":
      return text.split("\n\n")[0];
    case "longer":
      return `${text}\n\nLet me know if you need anything else.`;
    case "friendlier":
      return `${text}\n\nThanks so much!`;
    case "formal":
      return text.replace(/^Hi\b/, "Dear");
    case "fix":
      return text.replace(/\bi\b/g, "I").replace(/ {2,}/g, " ");
    case "translate":
      return `(${job.language}) ${text}`;
  }
}

const notReady: ReplyError = { kind: "notReady", message: "the AI model isn't downloaded yet" };
const failed: ReplyError = { kind: "model", message: "the AI model failed: /v1/chat/completions answered 500" };

/** About the first half of `text`, cut where a word ends. */
export function firstHalf(text: string): string {
  const middle = Math.ceil(text.length / 2);
  const cut = text.lastIndexOf(" ", middle);
  return text.slice(0, cut > 0 ? cut : middle);
}

/** The demo replies. A ready demo writes at once, showing the first half on the way. */
export function demoReplies(start: DemoRepliesStart = {}): ReplyBackend {
  const state = start.state ?? "ready";
  /** Writes held mid-way, by id: each finishes when stopped. */
  const held = new Map<string, () => void>();
  const ready = () => {
    if (state === "missing") throw notReady;
    if (state === "failed") throw failed;
  };
  return {
    options: async (request, onOptions) => {
      ready();
      const done: ReplyOption[] = [];
      for (const option of start.options ?? demoOptions) {
        const part = [...done, { ...option, draft: firstHalf(option.draft) }];
        onOptions(part);
        if (state === "writing") {
          await new Promise<void>((resolve) => held.set(request.id, resolve));
          return part;
        }
        done.push(option);
        onOptions([...done]);
      }
      return done;
    },
    write: async (request, onText) => {
      if (rewritesText(request.job) && !request.text.trim()) return "";
      ready();
      const text = demoText(request.job, request.text);
      const half = text.slice(0, Math.ceil(text.length / 2));
      onText(half);
      if (state === "writing") {
        await new Promise<void>((resolve) => held.set(request.id, resolve));
        return half;
      }
      onText(text);
      return text;
    },
    stop: async (id) => {
      const finish = held.get(id);
      held.delete(id);
      finish?.();
      return !!finish;
    },
  };
}

let current: ReplyBackend = demoReplies();

export const replies = () => current;

export function setReplies(backend: ReplyBackend) {
  current = backend;
}
