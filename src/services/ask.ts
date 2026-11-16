/**
 * Ask Moss: one chat with your mail. The email on screen is its context when there is one, and
 * Moss works out from the question whether it's about that email or any other. It looks things
 * up and acts on mail as the window does (Done, remind, move, delete, block, drafts). In the
 * desktop app it is the Rust side (`tauriAsk`, see `docs/research/ask-agent.md`); in the browser,
 * dev scenes and tests it is the scripted demo below.
 *
 * Moss never sends: an email it writes waits under the answer until you press Send.
 */
import type { Thread } from "../data/types";
import type { SavedDraft } from "../state/drafts";
import { replySubject } from "../lib/subject";

/** Why an answer failed. `notReady`: the writing model isn't downloaded yet. */
export interface AskError {
  kind: string;
  message: string;
}

/** A file an answer took facts from: open it with its message and index. */
export interface AskFile {
  threadId: string;
  messageId: string;
  index: number;
  name: string;
}

/** A finished answer: the emails it used (show them as cards), the files it read, and the drafts it saved. */
export interface AskAnswer {
  text: string;
  sources: Thread[];
  /** Ask about this email: the ids of the messages it cites, for the source chips. */
  messages: string[];
  /** The files it cites or read: chips that open them. None from older answers. */
  files?: AskFile[];
  /** Saved in Drafts, waiting for you to check and send. */
  drafts: SavedDraft[];
  /** Drafts you asked to send later, by id: when (ms). None from older answers. */
  sendAt?: Record<string, number>;
  /** What it changed, in order: "Marked done: Lunch on Friday". None from older answers. */
  did?: string[];
  /** You stopped it: `text` is what it wrote until then. */
  stopped: boolean;
}

/** What streams in while an answer is written, tagged with its chat. */
export type AskEvent =
  /** The model is thinking (step 1, 2…). Text after this replaces the text before it. */
  | { type: "step"; chat: string; step: number }
  /** It's looking something up, e.g. "Searching mail for “Q4 budget”". */
  | { type: "tool"; chat: string; tool: string; label: string }
  | { type: "text"; chat: string; delta: string }
  | { type: "done"; chat: string; answer: AskAnswer }
  | { type: "failed"; chat: string; error: AskError };

export interface AskBackend {
  /** A question in `chat` (any id you pick; a new id is a new chat). `open`: the email on screen. */
  askMoss(chat: string, question: string, open?: string): Promise<AskAnswer>;
  /** Stops the answer being written in `chat`; it resolves with what it wrote. Says whether one was. */
  stop(chat: string): Promise<boolean>;
  /** Forgets a chat, so it starts over. */
  forget(chat: string): Promise<void>;
  /** How many emails are on this Mac to look through. */
  mailCount(): Promise<number>;
  /** Calls `handler` with every event, for every chat; returns a function that stops it. */
  onEvent(handler: (event: AskEvent) => void): () => void;
}

/** Where one question stands on screen. */
export type TurnState = "thinking" | "writing" | "done" | "stopped" | "failed";

/** One question and its answer, as the chat shows it. */
export interface ChatTurn {
  question: string;
  state: TurnState;
  /** What it's looking up now, while thinking. */
  activity?: string;
  text: string;
  sources: Thread[];
  messages: string[];
  files: AskFile[];
  drafts: SavedDraft[];
  sendAt: Record<string, number>;
  did: string[];
  error?: AskError;
}

export function startTurn(question: string): ChatTurn {
  return { question, state: "thinking", text: "", sources: [], messages: [], files: [], drafts: [], sendAt: {}, did: [] };
}

/** A turn after one more event of its chat. */
export function applyEvent(turn: ChatTurn, event: AskEvent): ChatTurn {
  const { activity: _, ...rest } = turn;
  switch (event.type) {
    case "step":
      return { ...rest, state: "thinking", text: "" };
    case "tool":
      return { ...rest, state: "thinking", activity: event.label };
    case "text":
      return { ...rest, state: "writing", text: turn.text + event.delta };
    case "done": {
      const { text, sources, messages, files = [], drafts, sendAt = {}, did = [], stopped } = event.answer;
      return { ...rest, state: stopped ? "stopped" : "done", text, sources, messages, files, drafts, sendAt, did };
    }
    case "failed":
      return { ...rest, state: "failed", error: event.error };
  }
}

/** The writing model isn't downloaded: show the way to get it, not an error. */
export const modelMissing = (turn: ChatTurn) => turn.state === "failed" && turn.error?.kind === "notReady";

/** What the demo answers: cited emails, a drafted reply, an answer about the open email, emails marked Done, nothing found, or never, or an error. */
export type DemoAnswer = "cited" | "draft" | "thread" | "acted" | "nothing" | "thinking" | "missing" | "failed";

export interface DemoAskStart {
  /** One answer for every question. Otherwise questions about replies get a draft, ones saying Done or archive mark emails Done, and the rest cite the open email's messages, or emails. */
  answer?: DemoAnswer;
  /** The mail its answers point at: the first two are the cited emails, the first gets the reply. */
  threads?: Thread[];
  /** Between events; 0 answers at once. */
  delayMs?: number;
}

const missing: AskError = { kind: "notReady", message: "the AI model isn't downloaded yet" };
const failure: AskError = { kind: "model", message: "the AI model failed: it stopped answering" };

let drafted = 0;

/** A demo reply to a thread, as the agent saves one. */
function draftFor(t: Thread): SavedDraft {
  const body = `Hi ${t.from.name.split(" ")[0]},\n\nYes, that works for me.\n\nBest,`;
  return {
    id: `ask-${t.id}-${++drafted}`,
    mode: "reply",
    threadId: t.id,
    fromAccountId: t.accountId,
    to: [t.from],
    cc: [],
    bcc: [],
    subject: replySubject(t.subject),
    body,
    bodyHtml: body.replace(/\n/g, "<br>"),
    fieldsOpen: true,
    showCc: false,
    showBcc: false,
    remindIfNoReply: false,
    signatureId: null,
    updatedAt: Date.now(),
  };
}

/** The events an answer streams, and how it ends ("never": it keeps thinking). */
interface Script {
  events: AskEvent[];
  end: { answer: AskAnswer } | { error: AskError } | "never";
}

function script(kind: DemoAnswer, chat: string, question: string, threads: Thread[], open?: Thread): Script {
  const step = (n: number): AskEvent => ({ type: "step", chat, step: n });
  const tool = (name: string, label: string): AskEvent => ({ type: "tool", chat, tool: name, label });
  const search = tool("search_mail", `Searching mail for “${question}”`);
  const say = (text: string) => text.split(/(?<= )/).map((delta): AskEvent => ({ type: "text", chat, delta }));
  const answered = (text: string, sources: Thread[], drafts: SavedDraft[] = [], messages: string[] = [], did: string[] = []) => ({
    answer: { text, sources, messages, drafts, sendAt: {}, did, stopped: false },
  });
  switch (kind) {
    case "cited": {
      const text = "Dana confirmed the Q4 budget is $182k, and Sam approved it on Friday.";
      return { events: [step(1), search, step(2), tool("read_thread", "Reading an email"), step(3), ...say(text)], end: answered(text, threads.slice(0, 2)) };
    }
    case "draft": {
      const t = open ?? threads[0];
      const text = t ? `I drafted a reply to ${t.from.name}. It’s in Drafts: check it, then send it yourself.` : "I couldn’t find an email to reply to.";
      const events = [step(1), search, step(2), tool("draft_reply", "Writing a draft reply"), step(3), ...say(text)];
      return { events, end: t ? answered(text, [t], [draftFor(t)]) : answered(text, []) };
    }
    case "thread": {
      const text = "They want you to confirm the $182k budget by Friday.";
      const cited = open ? [...new Set([open.messages[0].id, open.messages.at(-1)!.id])] : [];
      return { events: [step(1), ...say(text)], end: answered(text, [], [], cited) };
    }
    case "acted": {
      const done = threads.slice(0, 2);
      const text = done.length ? `Done. I marked ${done.length} emails Done.` : "I couldn’t find those emails.";
      const events = [step(1), search, step(2), ...done.map(() => tool("mark_done", "Marking an email Done")), step(3), ...say(text)];
      return { events, end: answered(text, [], [], [], done.map((t) => `Marked done: ${t.subject}`)) };
    }
    case "nothing": {
      const text = "I couldn’t find a PDF from a bank. Try searching for the bank’s name.";
      return { events: [step(1), search, step(2), ...say(text)], end: answered(text, []) };
    }
    case "thinking":
      return { events: [step(1), search], end: "never" };
    case "missing":
      return { events: [step(1)], end: { error: missing } };
    case "failed":
      return { events: [step(1)], end: { error: failure } };
  }
}

/** A turn as it stands once its script has played, with `open` on screen: for scenes. */
export function demoTurn(kind: DemoAnswer, question: string, threads: Thread[], open?: Thread): ChatTurn {
  const chat = "moss";
  const { events, end } = script(kind, chat, question, threads, open);
  const turn = events.reduce(applyEvent, startTurn(question));
  if (end === "never") return turn;
  return applyEvent(turn, "answer" in end ? { type: "done", chat, answer: end.answer } : { type: "failed", chat, error: end.error });
}

/** The demo: answers from a script, streamed like the real one. */
export function demoAsk(start: DemoAskStart = {}): AskBackend {
  const { threads = [], delayMs = 0 } = start;
  const handlers = new Set<(event: AskEvent) => void>();
  const running = new Map<string, { stopped: boolean; wake?: () => void }>();
  const emit = (event: AskEvent) => handlers.forEach((h) => h(event));
  const pause = () => (delayMs ? new Promise((r) => setTimeout(r, delayMs)) : Promise.resolve());

  async function play(chat: string, kind: DemoAnswer, question: string, open?: Thread): Promise<AskAnswer> {
    const { events, end } = script(kind, chat, question, threads, open);
    const run: { stopped: boolean; wake?: () => void } = { stopped: false };
    running.set(chat, run);
    let text = "";
    try {
      for (const event of events) {
        await pause();
        if (run.stopped) break;
        if (event.type === "step") text = "";
        if (event.type === "text") text += event.delta;
        emit(event);
      }
      if (end === "never" && !run.stopped) await new Promise<void>((wake) => (run.wake = wake));
      if (run.stopped || end === "never") {
        const answer = { text: text.trim(), sources: [], messages: [], drafts: [], sendAt: {}, did: [], stopped: true };
        emit({ type: "done", chat, answer });
        return answer;
      }
      if ("error" in end) {
        emit({ type: "failed", chat, error: end.error });
        throw end.error;
      }
      emit({ type: "done", chat, answer: end.answer });
      return end.answer;
    } finally {
      if (running.get(chat) === run) running.delete(chat);
    }
  }

  // An email open: questions are about it.
  const wants = (question: string, emailOpen: boolean): DemoAnswer => {
    if (/\b(reply|draft)\b/i.test(question)) return "draft";
    if (/\b(done|archive)\b/i.test(question)) return "acted";
    return emailOpen ? "thread" : "cited";
  };
  return {
    askMoss: (chat, question, id) =>
      play(chat, start.answer ?? wants(question, id !== undefined), question, threads.find((t) => t.id === id)),
    stop: async (chat) => {
      const run = running.get(chat);
      if (!run) return false;
      running.delete(chat);
      run.stopped = true;
      run.wake?.();
      return true;
    },
    forget: async () => {},
    mailCount: async () => 2431,
    onEvent: (handler) => {
      handlers.add(handler);
      return () => handlers.delete(handler);
    },
  };
}

let current: AskBackend = demoAsk();

export const ask = () => current;

export function setAsk(backend: AskBackend) {
  current = backend;
}
