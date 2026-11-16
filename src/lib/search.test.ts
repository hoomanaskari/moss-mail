import { describe, expect, it } from "vitest";
import { lena, person } from "../test/fixtures";
import { filterHits, groupHits, highlight, noFilters, searchDocs, words, type SearchDoc } from "./search";

const daniel = person("Daniel Okafor", "daniel@northwind.co");
const doc = (id: string, extra: Partial<SearchDoc> = {}): SearchDoc => ({
  id,
  accountId: "work",
  from: daniel,
  subject: `Subject ${id}`,
  summary: "",
  when: "Oct 1",
  date: "2026-10-01",
  hasAttachment: false,
  relevance: 0.5,
  ...extra,
});
const today = "2026-11-01";

describe("words", () => {
  it("splits a query into lower-case words", () => {
    expect(words("  Q4  Budget ")).toEqual(["q4", "budget"]);
    expect(words("")).toEqual([]);
  });
});

describe("highlight", () => {
  it("marks every match, whatever its case, and keeps the text around it", () => {
    expect(highlight("Budget fits the budget.", ["budget"])).toEqual([
      { text: "Budget", match: true },
      { text: " fits the ", match: false },
      { text: "budget", match: true },
      { text: ".", match: false },
    ]);
  });

  it("leaves text without matches whole, and treats regex characters as text", () => {
    expect(highlight("Costs (Q3)", ["(q3)"])).toEqual([
      { text: "Costs ", match: false },
      { text: "(Q3)", match: true },
    ]);
    expect(highlight("Nothing here", ["budget"])).toEqual([{ text: "Nothing here", match: false }]);
  });
});

describe("searchDocs", () => {
  const docs = [
    doc("old", { subject: "Budget 2025", date: "2025-12-01", relevance: 0.9 }),
    doc("new", { summary: "Under budget", date: "2026-10-30", hasAttachment: true }),
    doc("lena", { from: lena, summary: "budget notes", accountId: "personal", date: "2026-10-20" }),
    doc("none", { summary: "Lunch" }),
  ];

  it("finds every word in the subject, summary or sender, newest first", () => {
    expect(searchDocs(docs, "budget", noFilters, today).map((h) => h.doc.id)).toEqual(["new", "lena", "old"]);
    expect(searchDocs(docs, "daniel lunch", noFilters, today).map((h) => h.doc.id)).toEqual(["none"]);
  });

  it("flags highly relevant hits as best matches and highlights them", () => {
    const [, , old] = searchDocs(docs, "budget", noFilters, today);
    expect(old.best).toBe(true);
    expect(old.subject[0]).toEqual({ text: "Budget", match: true });
  });

  it("finds nothing without words, unless a sender is chosen", () => {
    expect(searchDocs(docs, " ", noFilters, today)).toEqual([]);
    const [hit] = searchDocs(docs, "", { ...noFilters, from: lena }, today);
    expect(hit.doc.id).toBe("lena");
    expect(hit.summary).toEqual([{ text: "budget notes", match: false }]);
  });

  it("narrows by sender, account, attachment and date", () => {
    const ids = (f: Partial<typeof noFilters>) => searchDocs(docs, "budget", { ...noFilters, ...f }, today).map((h) => h.doc.id);
    expect(ids({ from: daniel })).toEqual(["new", "old"]);
    expect(ids({ accountId: "personal" })).toEqual(["lena"]);
    expect(ids({ hasAttachment: true })).toEqual(["new"]);
    expect(ids({ date: "week" })).toEqual(["new"]);
    expect(ids({ date: "month" })).toEqual(["new", "lena"]);
    expect(ids({ date: "year" })).toEqual(["new", "lena", "old"]);
  });
});

describe("filterHits", () => {
  const hit = (d: SearchDoc) => ({ doc: d, subject: [], summary: [], best: false });
  const hits = [
    hit(doc("a", { date: "2026-10-30", hasAttachment: true })),
    hit(doc("b", { from: lena, accountId: "personal", date: "2025-01-01" })),
  ];
  const ids = (f: Partial<typeof noFilters>) => filterHits(hits, { ...noFilters, ...f }, today).map((h) => h.doc.id);

  it("keeps what the engine found that passes the filters", () => {
    expect(ids({})).toEqual(["a", "b"]);
    expect(ids({ from: lena })).toEqual(["b"]);
    expect(ids({ accountId: "work" })).toEqual(["a"]);
    expect(ids({ hasAttachment: true })).toEqual(["a"]);
    expect(ids({ date: "year" })).toEqual(["a"]);
  });
});

describe("groupHits", () => {
  const hits = searchDocs(
    [
      doc("best", { subject: "budget", date: "2026-10-14", relevance: 0.9 }),
      doc("tue", { subject: "budget", date: "2026-10-27" }),
      doc("oct", { subject: "budget", date: "2026-10-02" }),
      doc("sep", { subject: "budget", date: "2026-09-02", accountId: "personal" }),
      doc("last-year", { subject: "budget", date: "2025-10-02" }),
    ],
    "budget",
    noFilters,
    today,
  );
  const names: Record<string, string> = { work: "Work", personal: "Personal" };

  it("by date: best matches, this week, then each month", () => {
    const groups = groupHits(hits, "date", (id) => names[id], today);
    expect(groups.map((g) => [g.title, g.hits.map((h) => h.doc.id)])).toEqual([
      ["Best matches", ["best"]],
      ["This week", ["tue"]],
      ["October", ["oct"]],
      ["September", ["sep"]],
      ["October 2025", ["last-year"]],
    ]);
    expect(groups[0].note).toBe("Matches in the subject come first");
  });

  it("by account: best matches, then each account", () => {
    const groups = groupHits(hits, "account", (id) => names[id], today);
    expect(groups.map((g) => g.title)).toEqual(["Best matches", "Work", "Personal"]);
  });

  it("has no best matches group when nothing stands out", () => {
    expect(groupHits(hits.filter((h) => !h.best), "date", (id) => names[id], today)[0].title).toBe("This week");
  });
});
