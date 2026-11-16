import { describe, expect, it } from "vitest";
import { thread } from "../test/fixtures";
import { dueLabel, reasonsFor, whyText } from "./why";

/** Monday 2 November 2026, 10:00 local time. */
const now = new Date(2026, 10, 2, 10, 0).getTime();
const DAY = 86_400_000;

describe("dueLabel", () => {
  it("says the day plainly, nearest first", () => {
    expect(dueLabel("2026-11-02", now)).toBe("Due today");
    expect(dueLabel("2026-11-03", now)).toBe("Due tomorrow");
    expect(dueLabel("2026-11-06", now)).toBe("Due Fri");
    expect(dueLabel("2026-11-13", now)).toBe("Due 13 Nov");
    expect(dueLabel("2027-01-05", now)).toBe("Due 5 Jan 2027");
    expect(dueLabel("2026-11-01", now)).toBe("Overdue");
  });
});

describe("whyText", () => {
  it("names who decided, and how", () => {
    expect(whyText({ by: "ai", sure: 88 })).toBe("AI, 88% sure");
    expect(whyText({ by: "rule", evidence: "by Friday" })).toBe("says “by Friday”");
    expect(whyText({ by: "rule" })).toBe("rule");
    expect(whyText({ by: "you" })).toBe("you asked");
  });

  it("cuts a long quote at a word, so the line stays short", () => {
    expect(whyText({ by: "rule", evidence: "Could you share the testing plan by Friday?" })).toBe("says “Could you share the testing…”");
    expect(whyText({ by: "rule", evidence: "Could you share the testing plan?" })).toBe("says “Could you share the testing plan?”");
    // No space to cut at: it's cut where the line ends.
    expect(whyText({ by: "rule", evidence: "https://example.com/plans/testing-plan-v3" })).toBe("says “https://example.com/plans/test…”");
  });
});

describe("reasonsFor", () => {
  it("has nothing to say about mail the rules sorted", () => {
    expect(reasonsFor(thread("t"), now)).toEqual([]);
  });

  it("says why it needs your reply, or doesn't, but not while snoozed", () => {
    expect(reasonsFor(thread("t", { aiSure: 95 }), now)).toEqual(["Needs your reply · AI, 95% sure"]);
    expect(reasonsFor(thread("t", { aiSure: 88, section: "info" }), now)).toEqual(["For your information · AI, 88% sure"]);
    expect(reasonsFor(thread("t", { aiSure: 88, section: "later" }), now)).toEqual([]);
  });

  it("says where the AI moved it, and the deadline it found", () => {
    const t = thread("t", {
      split: "receipts",
      section: "info",
      splitWhy: { by: "ai", sure: 91 },
      dueDate: "2026-11-06",
      dueWhy: { by: "rule", evidence: "by Friday" },
    });
    expect(reasonsFor(t, now)).toEqual(["Moved to Receipts · AI, 91% sure", "Due Fri · says “by Friday”"]);
  });

  it("says when your email comes back, and once it has, that no one replied", () => {
    const waiting = thread("t", { section: "later", youWroteLast: true, waiting: { until: now + 3 * DAY, why: { by: "rule", evidence: "Can you send it?" } } });
    expect(reasonsFor(waiting, now)).toEqual(["Back Thu 10 AM if Lena doesn’t reply · says “Can you send it?”"]);
    const back = { ...waiting, section: "reply" as const, waiting: { until: now - DAY, why: { by: "you" as const } } };
    expect(reasonsFor(back, now)).toEqual(["No reply from Lena yet · you asked"]);
    const nameless = { ...back, from: { name: "Dana Wu", email: "dana@acme.co", initials: "DW" } };
    expect(reasonsFor(nameless, now)).toEqual(["No reply from Dana Wu yet · you asked"]);
  });
});
