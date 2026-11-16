/**
 * Dev scenes for what Moss decides about mail: why an email is where it is, the due days it
 * found, and your email waiting on an answer. Uses the designed patterns (the reason line
 * under the subject, the amber due chip, the Later section); see the report for the frames
 * design.pen doesn't have yet.
 */
import { people } from "../data/demo";
import type { Thread } from "../data/types";
import { listTime } from "../lib/time";
import { dayOf, dueLabel } from "../lib/why";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

const DAY = 86_400_000;

/** The day `days` from today, as YYYY-MM-DD. */
function dayIn(days: number): string {
  const d = new Date(Date.now() + days * DAY);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** Your question to Lena, sent this morning: Moss brings it back in three days if she doesn't answer. */
function asked(until: number): Thread {
  const sent = Date.now() - 2 * 3_600_000;
  return {
    id: "testing-plan",
    accountId: "work",
    split: "important",
    section: until > Date.now() ? "later" : "reply",
    from: people.lena,
    subject: "Testing plan",
    summary: "Could you share the testing plan by Friday?",
    time: listTime(until > Date.now() ? until : sent, Date.now()),
    ...(until <= Date.now() && { due: "No reply" }),
    lastMessageAt: sent,
    youWroteLast: true,
    waiting: { until, why: { by: "rule", evidence: "Could you share the testing plan by Friday?" } },
    messages: [
      {
        id: "testing-plan-1",
        from: people.maya,
        to: [people.lena],
        cc: [],
        bcc: [],
        time: "9:40 AM",
        sentFull: "Today, 9:40 AM",
        paragraphs: ["Hi Lena,", "Could you share the testing plan by Friday?", "Thanks,\nMaya"],
        attachments: [],
      },
    ],
  };
}

/** "Thursday": the weekday a YYYY-MM-DD day falls on, as an email would say it. */
const weekday = (date: string) => new Date(dayOf(date)).toLocaleDateString("en-GB", { weekday: "long" });

/** Moss's decisions on the demo mail, as the engine gives them. Safe to run twice. */
function decide(waitingUntil = Date.now() + 3 * DAY) {
  const due = dayIn(2);
  useStore.setState((s) => ({
    threads: [
      ...s.threads
        .filter((t) => t.id !== "testing-plan")
        .map((t): Thread => {
          if (t.id === "q4-budget")
            return { ...t, aiSure: 92, due: dueLabel(due, Date.now()), askDue: dueLabel(due, Date.now()), dueDate: due, dueWhy: { by: "rule", evidence: `by ${weekday(due)}` } };
          if (t.id === "pebble-statement") return { ...t, split: "receipts", splitWhy: { by: "ai", sure: 88 } };
          if (t.id === "onboarding-v3") return { ...t, due: dueLabel(dayIn(5), Date.now()), dueDate: dayIn(5), dueWhy: { by: "ai", sure: 86 } };
          return t;
        }),
      asked(waitingUntil),
    ],
  }));
}

export const decisionScenes: Record<string, Scene> = {
  /** The inbox with the due chips Moss found, and your question to Lena waiting in Later. */
  "inbox-decided": { shows: /Testing plan/, apply: () => decide() },
  /** An email the AI sorted, with a deadline a rule found: one reason per decision. */
  "thread-why": {
    shows: /Needs your reply · AI, 92% sure/,
    apply: () => {
      decide();
      useStore.getState().open("q4-budget");
    },
  },
  /** A receipt the AI moved out of Important, and the way back. */
  "thread-moved": {
    shows: /Moved to Receipts · AI, 88% sure/,
    apply: () => {
      decide();
      useStore.getState().setSplit("receipts");
      useStore.getState().open("pebble-statement");
    },
  },
  /** Your email, waiting on Lena. */
  "thread-waiting": {
    shows: /if Lena doesn’t reply/,
    apply: () => {
      decide();
      useStore.getState().open("testing-plan");
    },
  },
  /** Three days on and no answer: back in Needs your reply, with the amber chip. */
  "inbox-no-reply": { shows: /No reply/, apply: () => decide(Date.now() - DAY) },
  /** ⌘K on an email Moss sorted: the corrections come first. */
  "palette-correct": {
    shows: /Move to For your information/,
    apply: () => {
      decide();
      useStore.getState().setCursor("q4-budget");
      useStore.setState({ paletteOpen: true, paletteQuery: "" });
    },
  },
};
