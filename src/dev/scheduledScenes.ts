/** Dev scenes for Send later: the time picker in the composer, the Scheduled list, and what it says once one has gone. */
import { people } from "../data/demo";
import { openScheduled, useScheduled, type ScheduledEmail } from "../state/scheduled";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

const blank = { cc: [], bcc: [], fieldsOpen: true, showCc: false, showBcc: false, remindIfNoReply: false, signatureId: null, updatedAt: Date.now() };

/** A day from today at a time, in ms. */
const dayAt = (days: number, hours: number) => {
  const d = new Date();
  d.setDate(d.getDate() + days);
  d.setHours(hours, 0, 0, 0);
  return d.getTime();
};

/** Three emails waiting: one for tomorrow morning, one for next week, and one that couldn't go yet. */
const demoScheduled = (): ScheduledEmail[] => [
  {
    revision: 1,
    operationId: "demo-schedule-1",
    draft: {
      ...blank,
      id: "later-offline",
      mode: "reply",
      threadId: "q4-budget",
      fromAccountId: "work",
      to: [people.daniel],
      subject: "Re: Q4 design budget",
      body: "Approved. Thanks Daniel.",
      bodyHtml: "<p>Approved. Thanks Daniel.</p>",
    },
    sendAt: Date.now() - 4 * 60_000,
    nextTry: Date.now() + 60_000,
    tries: 2,
    error: "can’t reach the server",
  },
  {
    revision: 1,
    operationId: "demo-schedule-2",
    draft: {
      ...blank,
      id: "later-shoot",
      mode: "new",
      fromAccountId: "studio",
      to: [people.priya],
      subject: "Call sheet for Friday",
      body: "Hi Priya, here’s the call sheet for the shoot.",
      bodyHtml: "<p>Hi Priya, here’s the call sheet for the shoot.</p>",
    },
    sendAt: dayAt(1, 8),
    nextTry: dayAt(1, 8),
    tries: 0,
  },
  {
    revision: 1,
    operationId: "demo-schedule-3",
    draft: {
      ...blank,
      id: "later-rent",
      mode: "new",
      fromAccountId: "personal",
      to: [people.mum],
      subject: "Sunday lunch",
      body: "Hi Mum, just a note that I can bring dessert.",
      bodyHtml: "<p>Hi Mum, just a note that I can bring dessert.</p>",
    },
    sendAt: dayAt(6, 9),
    nextTry: dayAt(6, 9),
    tries: 0,
  },
];

const keep = () => useScheduled.setState({ emails: demoScheduled(), picking: undefined, editing: undefined });

export const scheduledScenes: Record<string, Scene> = {
  "send-later": {
    shows: /Pick a date and time…/,
    apply: () => {
      useStore.getState().compose("reply-all", "q4-budget");
      useScheduled.setState({ picking: { kind: "compose" } });
    },
  },
  scheduled: {
    shows: /These wait on this Mac/,
    apply: () => {
      keep();
      useStore.setState({ view: "scheduled", history: ["inbox"] });
      useScheduled.setState({ cursorId: "later-shoot" });
    },
  },
  "scheduled-empty": {
    shows: /Nothing scheduled/,
    apply: () => {
      useScheduled.setState({ emails: [] });
      useStore.setState({ view: "scheduled", history: ["inbox"] });
    },
  },
  "inbox-scheduled": { shows: /^Scheduled$/, apply: keep },
  "composer-was-scheduled": {
    shows: /Was set to send/,
    apply: () => {
      useStore.getState().compose("reply", "q4-budget");
      const id = useStore.getState().draft!.id;
      useScheduled.setState({ editing: { id, sendAt: dayAt(1, 8) } });
    },
  },
  "sent-late": {
    shows: /Moss was closed at/,
    apply: () =>
      useStore.setState({ view: "inbox", toast: { id: 1, text: "Sent “Call sheet for Friday” late. Moss was closed at 8 AM.", undo: false } }),
  },
  "scheduled-failed": {
    shows: /Couldn’t send “Re: Q4 design budget”/,
    apply: () =>
      useStore.setState({
        view: "inbox",
        toast: { id: 1, text: "Couldn’t send “Re: Q4 design budget”. Trying again at 9:05 AM.", problem: true, stays: true, show: { label: "Scheduled", run: openScheduled } },
      }),
  },
};
