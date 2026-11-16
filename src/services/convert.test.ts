import { describe, expect, it } from "vitest";
import {
  toAccount,
  toProgress,
  toSearchHits,
  toServerSettings,
  toThread,
  toUiServers,
  type RustAccount,
  type RustSearchHit,
  type RustSummary,
  type RustThread,
} from "./convert";

const now = new Date(2026, 10, 1, 15, 0).getTime();
const at = (d: number, h: number, m = 0) => new Date(2026, 10, d, h, m).getTime();

const daniel = { name: "Daniel Okafor", email: "daniel@northwind.co", initials: "DO", short: "Daniel" };
const maya = { name: "Maya Ross", email: "maya@gmail.com", initials: "MR", short: "Maya" };

const summary: RustSummary = {
  id: "7",
  accountId: "gmail",
  split: "important",
  section: "reply",
  from: daniel,
  subject: "Q4 budget",
  summary: "Can you sign off?",
  lastMessageAt: at(1, 9, 12),
  snoozedUntil: null,
  unread: true,
  done: false,
  labels: ["Finance"],
  messageCount: 2,
  hasAttachments: false,
  sender: "known",
};

const full: RustThread = {
  ...summary,
  messages: [
    {
      id: "1",
      threadId: "7",
      from: daniel,
      to: [maya],
      cc: [],
      bcc: [],
      subject: "Q4 budget",
      sentAt: at(1, 8, 0),
      paragraphs: ["Hi Maya,"],
      text: "Hi Maya,",
      attachments: [],
      read: true,
      imageTrust: "ask",
    },
    {
      id: "2",
      threadId: "7",
      from: daniel,
      to: [maya],
      cc: [],
      bcc: [],
      subject: "Re: Q4 budget",
      sentAt: at(1, 9, 12),
      paragraphs: ["Can you sign off?"],
      text: "Can you sign off?",
      html: "<p>Can you sign off?</p>",
      attachments: [{ index: 0, name: "budget.xlsx", size: "240 KB", bytes: 245760, contentType: "x", inline: false }],
      read: false,
      imageTrust: "you-wrote",
    },
  ],
};

describe("threads from the mail engine", () => {
  it("a list row becomes a thread with a preview message until it is opened", () => {
    const t = toThread(summary, now);
    expect(t).toMatchObject({
      id: "7",
      accountId: "gmail",
      split: "important",
      section: "reply",
      from: daniel,
      subject: "Q4 budget",
      summary: "Can you sign off?",
      time: "9:12 AM",
      labels: ["Finance"],
      lastMessageAt: at(1, 9, 12),
      partial: true,
    });
    expect(t.messages).toHaveLength(1);
    expect(t.messages[0]).toMatchObject({ id: "7-preview", paragraphs: ["Can you sign off?"], sentFull: "Sunday, 1 November, 9:12 AM" });
  });

  it("says when you moved it to its tab yourself, and when the latest email is yours", () => {
    expect(toThread(summary, now)).not.toHaveProperty("movedByYou");
    expect(toThread({ ...summary, movedByYou: true, youWroteLast: true }, now)).toMatchObject({ movedByYou: true, youWroteLast: true });
    expect(toThread({ ...summary, messages: [], movedByYou: true }, now).movedByYou).toBe(true);
  });

  it("keeps whether the thread has unread mail", () => {
    expect(toThread(summary, now).unread).toBe(true);
    expect(toThread({ ...summary, unread: false }, now).unread).toBe(false);
  });

  it("says whether the sender is new or blocked, on a list row as on a whole thread", () => {
    expect(toThread({ ...summary, messages: [], sender: "new" }, now).sender).toBe("new");
    expect(toThread({ ...summary, messages: [], sender: "known" }, now).sender).toBe("known");
    // The row knows too, so "Block this sender?" is there the moment the email opens.
    expect(toThread({ ...summary, sender: "new" }, now).sender).toBe("new");
    expect(toThread({ ...summary, sender: "blocked" }, now).sender).toBe("blocked");
  });

  it("a done thread keeps when it was done; an open one has no done time", () => {
    expect(toThread({ ...summary, done: true, doneAt: at(1, 9) }, now).doneAt).toBe(at(1, 9));
    expect(toThread({ ...summary, doneAt: null }, now)).not.toHaveProperty("doneAt");
    expect(toThread(summary, now)).not.toHaveProperty("doneAt");
  });

  it("the AI's line on a conversation replaces the preview in the list", () => {
    expect(toThread({ ...summary, aiSummary: "Daniel needs the Q4 budget signed off." }, now).summary).toBe("Daniel needs the Q4 budget signed off.");
    // A row has room for one line of words: a model's Markdown is taken out.
    expect(toThread({ ...summary, aiSummary: "**Daniel** needs the *Q4* budget.\n\n**What matters:**" }, now).summary).toBe("Daniel needs the Q4 budget. What matters:");
    expect(toThread(summary, now).summary).toBe(summary.summary);
  });

  it("the AI's line says why it was written, for its label", () => {
    const line = { ...summary, aiSummary: "Your order ships Friday.", aiSummaryWhy: "replied" as const };
    expect(toThread(line, now).summaryWhy).toBe("replied");
    expect(toThread(summary, now)).not.toHaveProperty("summaryWhy");
  });

  it("a thread the AI sorted says how sure it was", () => {
    expect(toThread({ ...summary, aiSure: 95 }, now).aiSure).toBe(95);
    expect(toThread(summary, now)).not.toHaveProperty("aiSure");
  });

  it("a deadline Moss found becomes the due chip, with its day and who found it", () => {
    const due = toThread({ ...summary, due: { date: "2026-11-06", why: { by: "rule", evidence: "by Friday" } } }, now);
    expect(due).toMatchObject({ due: "Due Fri", askDue: "Due Fri", dueDate: "2026-11-06", dueWhy: { by: "rule", evidence: "by Friday" } });
    const none = toThread(summary, now);
    for (const key of ["due", "askDue", "dueDate", "dueWhy", "splitWhy", "waiting"]) expect(none).not.toHaveProperty(key);
    expect(none.youWroteLast).toBe(false);
  });

  it("says when the AI moved it to its split", () => {
    expect(toThread({ ...summary, split: "receipts", splitWhy: { by: "ai", sure: 88 } }, now).splitWhy).toEqual({ by: "ai", sure: 88 });
  });

  it("your email waiting on an answer says when it's back, and once back that no one replied", () => {
    const waiting = { until: at(4, 15), why: { by: "you" as const } };
    const row = toThread({ ...summary, section: "later", snoozedUntil: at(4, 15), youWroteLast: true, waiting }, now);
    expect(row).toMatchObject({ waiting, youWroteLast: true, time: "Wed" });
    expect(row).not.toHaveProperty("due");
    const back = toThread({ ...summary, youWroteLast: true, waiting: { ...waiting, until: at(1, 9) } }, now);
    expect(back.due).toBe("No reply");
    expect(back).not.toHaveProperty("askDue");
  });

  it("knows which of your addresses it came to, when the engine could tell", () => {
    expect(toThread({ ...summary, toAddress: "hi@rossstudio.art" }, now).toAddress).toBe("hi@rossstudio.art");
    expect(toThread(summary, now)).not.toHaveProperty("toAddress");
  });

  it("a snoozed thread shows when it comes back", () => {
    const t = toThread({ ...summary, section: "later", snoozedUntil: at(2, 8) }, now);
    expect(t.time).toBe("Tomorrow");
    expect(toThread({ ...summary, summary: "" }, now).messages[0].paragraphs).toEqual([]);
  });

  it("a loaded thread has every message, the latest subject in full", () => {
    const t = toThread(full, now);
    expect(t.partial).toBe(false);
    expect(t.fullSubject).toBe("Re: Q4 budget");
    expect(t.messages.map((m) => m.id)).toEqual(["1", "2"]);
    expect(t.messages[1]).toMatchObject({
      time: "9:12 AM",
      sentFull: "Sunday, 1 November, 9:12 AM",
      html: "<p>Can you sign off?</p>",
      text: "Can you sign off?",
      attachments: [{ index: 0, name: "budget.xlsx", size: "240 KB", contentType: "x" }],
      imageTrust: "you-wrote",
    });
    expect(t.messages[0].html).toBeUndefined();
    expect(t.messages[0].imageTrust).toBe("ask");
    expect(t.messages[1].headersOnly).toBeUndefined();
  });

  it("knows a message whose body or images are still on the server", () => {
    const shallow = { ...full.messages[1], headersOnly: true };
    expect(toThread({ ...full, messages: [shallow] }, now).messages[0].headersOnly).toBe(true);
    const images = { ...full.messages[1], imagesMissing: true };
    expect(toThread({ ...full, messages: [images] }, now).messages[0].imagesMissing).toBe(true);
    expect(toThread(full, now).messages[1].imagesMissing).toBeUndefined();
  });

  it("knows a message whose sender the mail server didn't confirm", () => {
    const unverified = { ...full.messages[1], senderUnverified: true };
    expect(toThread({ ...full, messages: [unverified] }, now).messages[0].senderUnverified).toBe(true);
    expect(toThread(full, now).messages[1].senderUnverified).toBeUndefined();
  });

  it("knows where a message was forwarded, and whether it was forwarded elsewhere", () => {
    const rebecca = { name: "Rebecca Vale", email: "rebecca@vale.example", initials: "RV", short: "Rebecca" };
    const forwarded = { ...full.messages[1], forwards: [{ threadId: "77", to: [rebecca], sentAt: new Date(2026, 9, 31, 16, 12).getTime() }], forwarded: true };
    const m = toThread({ ...full, messages: [forwarded] }, now).messages[0];
    expect(m.forwards).toEqual([{ threadId: "77", to: [rebecca], time: "Saturday, 31 October, 4:12 PM" }]);
    expect(m.forwarded).toBe(true);
    const plain = toThread(full, now).messages[1];
    expect(plain.forwards).toBeUndefined();
    expect(plain.forwarded).toBeUndefined();
  });

  it("images shown in the body aren't listed as attachments", () => {
    const inline = { ...full.messages[1], attachments: [{ ...full.messages[1].attachments[0], inline: true }] };
    expect(toThread({ ...full, messages: [inline] }, now).messages[0].attachments).toEqual([]);
  });
});

describe("accounts", () => {
  const account: RustAccount = {
    id: "gmail",
    name: "Gmail",
    letter: "G",
    address: "maya@gmail.com",
    senderName: "Maya Ross",
    color: "work",
    provider: "gmail",
    connection: { provider: "gmail", summary: "Gmail · App password" },
  };

  it("keep their connection for Settings", () => {
    expect(toAccount(account)).toEqual({
      id: "gmail",
      name: "Gmail",
      letter: "G",
      address: "maya@gmail.com",
      senderName: "Maya Ross",
      color: "work",
      connection: { provider: "gmail", summary: "Gmail · App password" },
    });
    expect(toAccount({ ...account, connection: null, org: "Northwind", orgDomain: "northwind.co" })).toMatchObject({
      connection: undefined,
      org: "Northwind",
      orgDomain: "northwind.co",
    });
  });

  it("keep their other addresses, and have none when there are none", () => {
    const addresses = [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 3, blocked: false }];
    expect(toAccount({ ...account, addresses }).addresses).toEqual(addresses);
    expect(toAccount({ ...account, addresses: [] })).not.toHaveProperty("addresses");
  });
});

describe("server settings", () => {
  it("go to Rust with numeric ports and the email as the user name", () => {
    const rust = toServerSettings(
      {
        incoming: { host: "imap.posteo.de", port: "993", security: "ssl" },
        outgoing: { host: "smtp.posteo.de", port: "587", security: "starttls" },
      },
      "maya@posteo.de",
    );
    expect(rust).toEqual({
      imap: { host: "imap.posteo.de", port: 993, security: "tls", username: "maya@posteo.de" },
      smtp: { host: "smtp.posteo.de", port: 587, security: "startTls", username: "maya@posteo.de" },
    });
    // Moss never sends a password unencrypted: "none" still asks for STARTTLS.
    const none = toServerSettings({ incoming: { host: "h", port: "x", security: "none" }, outgoing: { host: "h", port: "25", security: "none" } }, "a@b.co");
    expect(none.imap).toMatchObject({ port: 0, security: "startTls" });
  });

  it("come back from discovery in the form's shape", () => {
    const ui = toUiServers({
      imap: { host: "imap.posteo.de", port: 993, security: "tls", username: "x" },
      smtp: { host: "smtp.posteo.de", port: 587, security: "startTls", username: "x" },
    });
    expect(ui).toEqual({
      incoming: { host: "imap.posteo.de", port: "993", security: "ssl" },
      outgoing: { host: "smtp.posteo.de", port: "587", security: "starttls" },
    });
  });
});

describe("search hits", () => {
  const hit = (subject: string, rank: number, extra: Partial<RustSummary> = {}): RustSearchHit => ({
    thread: { ...summary, subject, ...extra },
    messageId: `m-${subject}`,
    snippet: [
      { text: "about the ", matched: false },
      { text: "budget", matched: true },
    ],
    rank,
  });

  it("become the search screen's results, best matches first", () => {
    const hits = toSearchHits([hit("Q4 budget", -3), hit("Lunch", -1, { hasAttachments: true, lastMessageAt: new Date(2026, 9, 14).getTime() })], "budget", now);
    expect(hits).toHaveLength(2);
    expect(hits[0]).toMatchObject({
      best: true,
      doc: { id: "m-Q4 budget", threadId: "7", accountId: "gmail", from: daniel, subject: "Q4 budget", summary: "about the budget", when: "9:12 AM", date: "2026-11-01", hasAttachment: false },
      subject: [
        { text: "Q4 ", match: false },
        { text: "budget", match: true },
      ],
      summary: [
        { text: "about the ", match: false },
        { text: "budget", match: true },
      ],
    });
    expect(hits[0].doc.relevance).toBe(1);
    expect(hits[1]).toMatchObject({ best: false, doc: { hasAttachment: true, when: "Oct 14", date: "2026-10-14" } });
    expect(hits[1].doc.relevance).toBeCloseTo(1 / 3);
  });

  it("a match inside a file says which file first", () => {
    const [found] = toSearchHits([{ ...hit("Letter from the council", -2), file: { index: 0, name: "P2026-0457.pdf" } }], "budget", now);
    expect(found.summary).toEqual([
      { text: "In P2026-0457.pdf: ", match: false },
      { text: "about the ", match: false },
      { text: "budget", match: true },
    ]);
  });

  it("an empty answer stays empty", () => {
    expect(toSearchHits([], "x", now)).toEqual([]);
    const zero = toSearchHits([hit("A", 0)], "q", now);
    expect(zero[0].doc.relevance).toBe(0);
  });
});

describe("sync progress", () => {
  it("fills the Syncing screen", () => {
    expect(toProgress({ accountId: "gmail", done: 400, folders: 12, recent: 0.5, oldestYear: 2019, more: true })).toEqual({
      done: 400,
      total: 600,
      folders: 12,
      recent: 0.5,
      oldestYear: 2019,
    });
    // Done: the count is the total. Before any mail, the year is this one.
    expect(toProgress({ accountId: "gmail", done: 0, folders: 3, recent: 1, oldestYear: null, more: false })).toMatchObject({
      done: 0,
      total: 0,
      oldestYear: new Date().getFullYear(),
    });
  });
});
