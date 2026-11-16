import { create } from "zustand";
import { demoToday, searchIndex } from "../data/demo";
import type { Person } from "../data/types";
import { mail } from "../services/mail";
import { preload } from "../services/preload";
import { noFilters, searchDocs, type DateRange, type GroupBy, type SearchBackend, type SearchFilters, type SearchHit } from "../lib/search";

/** Search over the demo mailbox. Swap for a Tauri command once the Rust store can search. */
export const demoSearch: SearchBackend = {
  search: async (query, filters) => searchDocs(searchIndex, query, filters, demoToday),
};

/** The engine's full-text index with real mail, the demo mailbox otherwise. */
export function searchBackend(): SearchBackend {
  const m = mail();
  return m.live ? { search: (query, filters) => m.search(query, filters) } : demoSearch;
}

/** "Today" for grouping results: the real date with real mail; the demo's own day otherwise. */
export function searchToday(now = new Date()): string {
  if (!mail().live) return demoToday;
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
}

export const dateLabels: Record<DateRange, string> = {
  any: "Any time",
  week: "This week",
  month: "Past month",
  year: "Past year",
};
const dateOrder: DateRange[] = ["any", "week", "month", "year"];

/** A result taken out of the results, and where it sat, so undo can put it back. */
export interface PutBack {
  hit: SearchHit;
  /** Its place in `hits`. */
  index: number;
}

interface SearchState {
  query: string;
  filters: SearchFilters;
  groupBy: GroupBy;
  hits: SearchHit[];
  cursorId?: string;
  setQuery(query: string): void;
  setCursor(id: string): void;
  /** Move along `order`, the hits as they appear on screen. */
  moveCursor(step: 1 | -1, order: string[]): void;
  /** Only mail from this person; again (or with nobody) to drop it. */
  toggleFrom(person: Person | undefined): void;
  /** All accounts → each account in turn → all again. */
  cycleAccount(accountIds: string[]): void;
  toggleAttachment(): void;
  cycleDate(): void;
  toggleGroupBy(): void;
  /** Empties the query and filters. False when there was nothing to clear. */
  clear(): boolean;
  /** Takes a handled email out of the results; the cursor moves to the next one in `order`. */
  removeHit(id: string, order: string[]): void;
  /** Undo: the result back where it was, selected. */
  putBack(back: PutBack): void;
}

export const useSearch = create<SearchState>()((set, get) => ({
  query: "",
  filters: noFilters,
  groupBy: "date",
  hits: [],

  setQuery: (query) => set({ query }),
  setCursor: (cursorId) => set({ cursorId }),
  moveCursor: (step, order) => {
    if (!order.length) return;
    const i = Math.max(0, order.indexOf(get().cursorId ?? ""));
    set({ cursorId: order[Math.min(order.length - 1, Math.max(0, i + step))] });
  },
  toggleFrom: (person) =>
    set((s) => ({ filters: { ...s.filters, from: s.filters.from?.email === person?.email ? undefined : person } })),
  cycleAccount: (ids) =>
    set((s) => {
      const order = ["all", ...ids];
      return { filters: { ...s.filters, accountId: order[(order.indexOf(s.filters.accountId) + 1) % order.length] } };
    }),
  toggleAttachment: () => set((s) => ({ filters: { ...s.filters, hasAttachment: !s.filters.hasAttachment } })),
  cycleDate: () =>
    set((s) => ({ filters: { ...s.filters, date: dateOrder[(dateOrder.indexOf(s.filters.date) + 1) % dateOrder.length] } })),
  toggleGroupBy: () => set((s) => ({ groupBy: s.groupBy === "date" ? "account" : "date" })),
  clear: () => {
    const { query, filters } = get();
    if (!query && filters === noFilters) return false;
    set({ query: "", filters: noFilters, hits: [], cursorId: undefined });
    return true;
  },
  removeHit: (id, order) => {
    const { hits, cursorId } = get();
    const i = order.indexOf(id);
    const rest = order.filter((x) => x !== id);
    set({
      hits: hits.filter((h) => h.doc.id !== id),
      cursorId: cursorId === id ? rest[Math.min(i, rest.length - 1)] : cursorId,
    });
  },
  putBack: ({ hit, index }) => {
    const id = hit.doc.id;
    const hits = get().hits.filter((h) => h.doc.id !== id);
    set({ hits: [...hits.slice(0, index), hit, ...hits.slice(index)], cursorId: id });
  },
}));

/** The latest search asked for: only its answer may reach the screen. */
let latest = 0;

/**
 * Asks `backend` for the current query and filters. An answer that arrives after they changed,
 * or after a newer search started (even for the same words), is dropped.
 */
export async function runSearch(backend: SearchBackend = searchBackend()) {
  const asked = ++latest;
  const { query, filters } = useSearch.getState();
  const hits = await backend.search(query, filters);
  const now = useSearch.getState();
  if (asked !== latest || now.query !== query || now.filters !== filters) return;
  const keep = hits.some((h) => h.doc.id === now.cursorId);
  useSearch.setState({ hits, cursorId: keep ? now.cursorId : hits[0]?.doc.id });
  // The top results are likely to be opened: have them here.
  if (mail().live) preload(hits.slice(0, 3).flatMap((h) => (h.doc.threadId ? [h.doc.threadId] : [])));
}
