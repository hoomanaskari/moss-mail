/**
 * Dev-only deep links that put the app into each state drawn in design.pen,
 * e.g. http://localhost:1420/?scene=conversation&theme=light
 * Used for side-by-side and pixel comparison against the design exports.
 */
import { accounts, extraDemoAccounts, people } from "../data/demo";
import type { Person } from "../data/types";
import { useServerChanges } from "../services/serverChanges";
import { useSync } from "../services/live";
import { useStore, type Theme } from "../state/store";
import { useStartup } from "../state/startup";
import { addAccountScenes } from "./addAccountScenes";
import { addressScenes } from "./addressScenes";
import { askScenes } from "./askScenes";
import { attachmentScenes } from "./attachmentScenes";
import { blockScenes } from "./blockScenes";
import { decisionScenes } from "./decisionScenes";
import { doneScenes } from "./doneScenes";
import { recoveryScenes } from "./recoveryScenes";
import { draftScenes } from "./draftScenes";
import { mossScenes } from "./mossScenes";
import { moveScenes } from "./moveScenes";
import { aiScene, searchScenes } from "./searchScenes";
import { scheduledScenes } from "./scheduledScenes";
import { sendScenes } from "./sendScenes";
import { settingsScenes } from "./settingsScenes";
import { mcpScenes } from "./mcpScenes";
import { signatureScenes } from "./signatureScenes";
import { summaryScenes } from "./summaryScenes";
import { updateScenes } from "./updateScenes";

export interface Scene {
  /** Puts the store into the state drawn in design.pen. */
  apply: () => void;
  /** Text on screen once the scene renders, so App.test can check each one. */
  shows: RegExp;
}
import { sampleThreads } from "./sampleEmails";

const replyAllBody =
  "Hi Daniel,\n\nThanks for updating this. The user testing line makes sense, so I’m happy to approve the new total of €18,400.\n\nI’ve attached my notes on the vendor list. Could you share the testing plan by Friday, 13 November?\n\nBest,\nMaya";

/** Adds the sample HTML and plain-text emails to the demo mail and opens one. */
function openSample(id: string) {
  useStore.setState((s) => ({ threads: [...sampleThreads, ...s.threads] }));
  useStore.getState().open(id);
}

const you = { name: "Maya Ross", email: "hello@rossstudio.art", initials: "MR" };

/** The design frame "Conversation — Earlier messages": six earlier messages before Priya's latest. */
const shootHistory: [Person, string, string][] = [
  [people.priya, "Oct 27", "Hi Maya, a client wants a half-day studio shoot next week. Are you around?"],
  [you, "Oct 27", "Yes, I can do Thursday or Friday. Morning is best for the light."],
  [people.priya, "Oct 28", "Great, I’ll ask which day suits them."],
  [you, "Oct 28", "Sounds good."],
  [people.priya, "Oct 29", "They prefer Friday. Can you hold it?"],
  [people.priya, "Oct 30", "Friday it is. I’ll send the call sheet once the client confirms the time."],
];

/** Puts earlier messages in front of a demo thread's newest message and opens it. Safe to run twice. */
function openWithHistory(id: string, history: [Person, string, string][]) {
  useStore.setState((s) => ({
    threads: s.threads.map((t) =>
      t.id !== id
        ? t
        : {
            ...t,
            messages: [
              ...history.map(([from, time, text], i) => ({
                ...t.messages[0],
                id: `${id}-earlier-${i}`,
                from,
                time,
                paragraphs: [text],
              })),
              t.messages.at(-1)!,
            ],
          },
    ),
  }));
  useStore.getState().open(id);
}

const mailScenes: Record<string, Scene> = {
  "split-inbox": { shows: /^Inbox$/, apply: () => {} },
  "inbox-getting-ai": {
    shows: /Getting AI · 2\.1 of 6\.4 GB/,
    apply: () => aiScene({ download: { state: "downloading", done: 2_100_000_000, total: 6_409_244_946 } }),
  },
  "inbox-unread": {
    shows: /^Inbox$/,
    // Matches the design frame "Unread mail".
    apply: () => {
      const unread = new Set(["q4-budget", "sunday-lunch", "flight-booking", "framely-digest", "design-weekly"]);
      useStore.setState((s) => ({ threads: s.threads.map((t) => (unread.has(t.id) ? { ...t, unread: true } : t)) }));
    },
  },
  "inbox-not-syncing": {
    shows: /Personal isn’t syncing/,
    // Matches the design frame "Inbox · Account not syncing".
    apply: () => useSync.setState({ problems: { personal: { problem: "signedOut", message: "the server no longer accepts the saved sign-in: this account needs to sign in again" } } }),
  },
  "inbox-server-problem": {
    shows: /Work isn’t syncing\. Moss keeps trying\./,
    apply: () => useSync.setState({ problems: { work: { problem: "server", message: "The mail server refused the request." } } }),
  },
  "inbox-server-recovery": {
    shows: /may mean this change partly worked/,
    apply: () => useServerChanges.setState({ byAccount: { work: { unavailable: false, status: {
      accountId: "work", change: "0123456789abcdef0123456789abcdef", revision: 2, state: "blocked", failure: "uncertain", kind: "move",
      completed: 1, pending: 2, discarded: 0, dropped: 0,
    } } } }),
  },
  // A Done the server said no to: Try again, Discard, or check where it stands.
  "inbox-server-change-refused": {
    shows: /The server refused this change/,
    apply: () => useServerChanges.setState({ byAccount: { work: { unavailable: false, status: {
      accountId: "work", change: "0123456789abcdef0123456789abcdef", revision: 2, state: "blocked", failure: "refused", kind: "done",
      completed: 0, pending: 1, discarded: 0, dropped: 0,
    } } } }),
  },
  "single-account": { shows: /Work inbox/, apply: () => useStore.getState().setAccountFilter("work") },
  "many-accounts": { shows: /^Inbox$/, apply: () => useStore.setState({ accounts: [...accounts, ...extraDemoAccounts] }) },
  "many-accounts-scrolled": {
    shows: /^Inbox$/,
    apply: () => {
      useStore.setState({ accounts: [...accounts, ...extraDemoAccounts] });
      // Matches the design frame "Many accounts (overflow, scrolled)": the strip moved 482px.
      setTimeout(() => {
        const tabs = document.querySelector('[aria-label="Accounts"]');
        if (!tabs) return;
        tabs.scrollLeft = 482;
        // Headless screenshot tools don't always deliver scroll events; send one so the fades update.
        tabs.dispatchEvent(new Event("scroll"));
      }, 100);
    },
  },
  "empty-split": {
    shows: /No invites in Work/,
    apply: () => {
      useStore.getState().setAccountFilter("work");
      useStore.getState().setSplit("calendar");
    },
  },
  conversation: { shows: /Q4 design budget/, apply: () => useStore.getState().open("q4-budget") },
  "conversation-sorted": {
    shows: /AI, 95% sure/,
    apply: () => {
      useStore.setState((s) => ({ threads: s.threads.map((t) => (t.id === "q4-budget" ? { ...t, aiSure: 95 } : t)) }));
      useStore.getState().open("q4-budget");
    },
  },
  "conversation-sparse": { shows: /Reply to Priya/, apply: () => useStore.getState().open("fridays-shoot") },
  /** A sender with no name, only a long address: the reply field keeps it to one line. */
  "conversation-long-sender": {
    shows: /Reply to jordan\.ellison@mortgagebrokerage/,
    apply: () => {
      const email = "jordan.ellison@mortgagebrokerage.lending.example";
      const from = { name: email, email, initials: "J" };
      useStore.setState((s) => ({ threads: s.threads.map((t) => (t.id === "fridays-shoot" ? { ...t, from } : t)) }));
      useStore.getState().open("fridays-shoot");
    },
  },
  "conversation-history": { shows: /3 more messages/, apply: () => openWithHistory("fridays-shoot", shootHistory) },
  "conversation-recipients": {
    shows: /Q4 design budget/,
    apply: () => {
      useStore.getState().open("q4-budget");
      useStore.setState({ recipientsOpen: true });
    },
  },
  /** Forty people on one email: the line under the sender names a few and counts the rest, inside the column. */
  "conversation-many-recipients": {
    shows: /and 37 others/,
    apply: () => {
      const crowd: Person[] = Array.from({ length: 40 }, (_, i) => {
        const name = `Bartholomew Featherstonehaugh ${i + 1}`;
        return { name, email: `bartholomew.featherstonehaugh${i + 1}@example.com`, initials: "BF" };
      });
      useStore.setState((s) => ({
        threads: s.threads.map((t) =>
          t.id === "q4-budget" ? { ...t, messages: t.messages.map((m) => ({ ...m, to: crowd, cc: [] })) } : t,
        ),
      }));
      useStore.getState().open("q4-budget");
    },
  },
  "thread-forwarded": {
    shows: /You forwarded this/,
    apply: () => {
      const forwards = [{ threadId: "onboarding-v3", to: [people.lena], time: "Saturday, 31 October, 4:12 PM" }];
      useStore.setState((s) => ({
        threads: s.threads.map((t) =>
          // Its one message, forwarded.
          t.id === "q4-budget" ? { ...t, messages: [{ ...t.messages[0], forwards }] } : t,
        ),
      }));
      useStore.getState().open("q4-budget");
    },
  },
  "composer-reply-all": {
    shows: /Reply all/,
    apply: () => {
      const s = useStore.getState();
      s.compose("reply-all", "q4-budget");
      s.updateDraft({ bcc: [people.financeArchive], body: replyAllBody, remindIfNoReply: true, signatureId: null });
    },
  },
  "composer-collapsed": {
    shows: /Reply all/,
    apply: () => {
      const s = useStore.getState();
      s.compose("reply-all", "q4-budget");
      s.updateDraft({ bcc: [people.financeArchive], body: replyAllBody, remindIfNoReply: true, fieldsOpen: false, signatureId: null });
    },
  },
  "composer-forward": {
    shows: /Forwarded message/,
    apply: () => {
      const s = useStore.getState();
      s.compose("forward", "q4-budget");
      s.updateDraft({ to: [people.lena], body: "Lena, can you look at this before Friday?", signatureId: null });
    },
  },
  "composer-new": {
    shows: /New email/,
    apply: () => {
      const s = useStore.getState();
      s.compose("new");
      s.updateDraft({
        fromAccountId: accounts[2].id,
        fromReason: "The account you use with Priya",
        to: [people.priya],
        subject: "Friday’s shoot",
        body: "Hi Priya,\n\n10 AM on Friday works for me. I’ll let the client know the new time.\n\nSee you then,\nMaya",
        // Signed by hand in this design, so no signature underneath.
        signatureId: null,
      });
    },
  },
  "composer-signature-picker": {
    shows: /Signature for this email/,
    apply: () => {
      const s = useStore.getState();
      s.compose("new");
      s.updateDraft({
        fromAccountId: accounts[2].id,
        fromReason: "The account you use with Priya",
        to: [people.priya],
        subject: "Friday’s shoot",
        body: "Hi Priya,\n\n10 AM on Friday works for me. I’ll let the client know the new time.\n\nSee you then,",
        signatureOpen: true,
      });
    },
  },
  // The desktop app couldn't load your mail at launch.
  "startup-error": { shows: /Moss couldn’t open your mail/, apply: () => useStartup.setState({ failed: "The mail database is locked." }) },
  "inbox-zero": { shows: /Inbox zero|inbox zero/i, apply: () => useStore.setState({ view: "zero", handled: 8 }) },
  "inbox-zero-undo": {
    shows: /Moved to Done/,
    apply: () => {
      useStore.getState().markDone(useStore.getState().threads[0].id);
      useStore.setState({ view: "zero" });
    },
  },
  "command-palette": {
    shows: /remind/i,
    apply: () => useStore.setState({ paletteOpen: true, paletteQuery: "remind me before Daniel’s deadline" }),
  },
  remind: { shows: /Pick a date and time/, apply: () => useStore.getState().openRemind() },
  "remind-thread": {
    shows: /Pick a date and time/,
    apply: () => {
      useStore.getState().open(useStore.getState().threads[0].id);
      useStore.getState().openRemind();
    },
  },
  "html-newsletter": { shows: /Images are hidden/, apply: () => openSample("sample-newsletter") },
  // A sender you allowed, on an email their domain didn't vouch for: it asks, and says why.
  "html-newsletter-unconfirmed": {
    shows: /Images are off because Moss couldn't confirm who sent this/,
    apply: () => {
      const unconfirmed = sampleThreads.map((t) =>
        t.id === "sample-newsletter" ? { ...t, messages: t.messages.map((m) => ({ ...m, imageTrust: "you-allowed" as const, senderUnverified: true })) } : t,
      );
      useStore.setState((s) => ({ threads: [...unconfirmed, ...s.threads] }));
      useStore.getState().open("sample-newsletter");
    },
  },
  "html-receipt": { shows: /Your Harbour Bikes order/, apply: () => openSample("sample-receipt") },
  "html-dark-classes": { shows: /A bonus on every transfer/, apply: () => openSample("sample-canvas") },
  "html-dark-half-done": { shows: /Your table is booked/, apply: () => openSample("sample-half-dark") },
  "html-reply": { shows: /Show quoted text/, apply: () => openSample("sample-reply") },
  "text-email": { shows: /Show quoted text/, apply: () => openSample("sample-text") },
};

/** Every designed state, by name. Each feature keeps its own list next to this one. */
export const scenes: Record<string, Scene> = {
  ...mailScenes,
  ...addAccountScenes,
  ...settingsScenes,
  ...mcpScenes,
  ...searchScenes,
  ...doneScenes,
  ...blockScenes,
  ...attachmentScenes,
  ...draftScenes,
  ...recoveryScenes,
  ...sendScenes,
  ...scheduledScenes,
  ...mossScenes,
  ...signatureScenes,
  ...addressScenes,
  ...summaryScenes,
  ...askScenes,
  ...moveScenes,
  ...decisionScenes,
  ...updateScenes,
};

export function applyScene() {
  const params = new URLSearchParams(location.search);
  const theme = params.get("theme") as Theme | null;
  if (theme === "dark" || theme === "light") useStore.getState().setTheme(theme);
  const scene = scenes[params.get("scene") ?? ""];
  if (!scene) return;
  document.documentElement.dataset.scene = params.get("scene")!;
  scene.apply();
}
