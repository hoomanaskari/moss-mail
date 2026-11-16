import { beforeEach, describe, expect, it, vi } from "vitest";
import { demoToday } from "../data/demo";
import { fakeMail } from "../test/fakeMail";
import { lena } from "../test/fixtures";
import { dateLabels, demoSearch, runSearch, searchToday, useSearch } from "./search";

const s = () => useSearch.getState();
const pristine = useSearch.getState();
beforeEach(() => useSearch.setState(pristine, true));

describe("search state", () => {
  it("runs the query through the backend and puts the cursor on the first hit", async () => {
    s().setQuery("budget");
    await runSearch();
    expect(s().hits.length).toBe(8);
    expect(s().cursorId).toBe("s-q4");
  });

  it("keeps the cursor on a hit that is still there", async () => {
    s().setQuery("budget");
    await runSearch();
    s().setCursor("s-q3");
    s().toggleAttachment();
    await runSearch();
    expect(s().cursorId).toBe("s-q4");
    s().setCursor("s-template");
    s().setQuery("budget template");
    await runSearch();
    expect(s().cursorId).toBe("s-template");
  });

  it("with real mail, has the top results' bodies fetched so they open at once", async () => {
    const { backend } = fakeMail();
    s().setQuery("budget");
    await runSearch(demoSearch);
    const ids = s().hits.slice(0, 3).flatMap((h) => (h.doc.threadId ? [h.doc.threadId] : []));
    expect(backend.preload).toHaveBeenCalledWith(ids);
    // Demo hits without a thread are left out.
    const { backend: other } = fakeMail();
    await runSearch({ search: async () => [{ ...s().hits[0], doc: { ...s().hits[0].doc, threadId: undefined } }] });
    expect(other.preload).not.toHaveBeenCalled();
  });

  it("drops an answer that arrives after the query changed", async () => {
    s().setQuery("budget");
    const slow = runSearch({ search: async (q, f) => (await new Promise((r) => setTimeout(r, 5)), demoSearch.search(q, f)) });
    s().setQuery("lunch");
    await runSearch();
    await slow;
    expect(s().hits.map((h) => h.doc.id)).toEqual(["s-lunch"]);
  });

  it("drops an answer once the words or filters have changed, even before their own search starts", async () => {
    s().setQuery("budget");
    const typing = runSearch();
    s().setQuery("budget tem");
    await typing;
    const filtering = runSearch();
    s().toggleAttachment();
    await filtering;
    expect(s().hits).toEqual([]);
  });

  it("drops an older answer for the same words, so it never replaces a newer one", async () => {
    s().setQuery("budget");
    const [older] = (await demoSearch.search("budget", s().filters)).slice(-1);
    const slow = runSearch({ search: async () => (await new Promise((r) => setTimeout(r, 5)), [older]) });
    // Back to the same words, e.g. "budgets" then ⌫: the newer search answers first.
    await runSearch();
    await slow;
    expect(s().hits.length).toBe(8);
  });

  it("moves the cursor along the order on screen, stopping at the ends", () => {
    const order = ["a", "b", "c"];
    s().moveCursor(1, order);
    expect(s().cursorId).toBe("b");
    s().moveCursor(1, order);
    s().moveCursor(1, order);
    expect(s().cursorId).toBe("c");
    s().moveCursor(-1, order);
    expect(s().cursorId).toBe("b");
    s().moveCursor(1, []);
    expect(s().cursorId).toBe("b");
  });

  it("toggles and cycles the filters", () => {
    s().toggleFrom(lena);
    expect(s().filters.from).toEqual(lena);
    s().toggleFrom(lena);
    expect(s().filters.from).toBeUndefined();
    s().toggleFrom(undefined);
    expect(s().filters.from).toBeUndefined();
    s().cycleAccount(["work", "personal"]);
    s().cycleAccount(["work", "personal"]);
    expect(s().filters.accountId).toBe("personal");
    s().cycleAccount(["work", "personal"]);
    expect(s().filters.accountId).toBe("all");
    for (const label of ["This week", "Past month", "Past year", "Any time"]) {
      s().cycleDate();
      expect(dateLabels[s().filters.date]).toBe(label);
    }
    s().toggleGroupBy();
    expect(s().groupBy).toBe("account");
    s().toggleGroupBy();
    expect(s().groupBy).toBe("date");
  });

  it("clears the query and filters, and says when there was nothing to clear", async () => {
    expect(s().clear()).toBe(false);
    s().setQuery("budget");
    s().toggleAttachment();
    await runSearch();
    expect(s().clear()).toBe(true);
    expect(s()).toMatchObject({ query: "", hits: [], filters: { hasAttachment: false } });
  });

  it("takes a handled email out of the results", async () => {
    s().setQuery("budget");
    await runSearch();
    s().removeHit("s-q4", ["s-q4", "s-q3", "s-template"]);
    expect(s().hits.some((h) => h.doc.id === "s-q4")).toBe(false);
    expect(s().cursorId).toBe("s-q3");
    s().removeHit("s-template", ["s-q3", "s-template"]);
    expect(s().cursorId).toBe("s-q3");
    s().removeHit("missing", []);
    expect(s().hits).toHaveLength(6);
  });
});

describe("with real mail", () => {
  it("searches the engine's index and groups by the real date", async () => {
    const hit = { doc: { id: "m1", threadId: "7", accountId: "gmail", from: lena, subject: "Budget", summary: "", when: "Fri", date: "2026-10-30", hasAttachment: false, relevance: 1 }, subject: [], summary: [], best: true };
    const { backend } = fakeMail({ search: vi.fn(async () => [hit]) });
    useSearch.setState({ query: "budget" });
    await runSearch();
    expect(backend.search).toHaveBeenCalledWith("budget", useSearch.getState().filters);
    expect(useSearch.getState().hits).toEqual([hit]);
    expect(searchToday(new Date(2026, 0, 5))).toBe("2026-01-05");
  });

  it("the demo keeps its own today", () => {
    expect(searchToday()).toBe(demoToday);
  });
});
