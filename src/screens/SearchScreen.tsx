import { useEffect, useRef } from "react";
import { Calendar, Inbox, Paperclip, Search, SearchX, Sparkles, User } from "lucide-react";
import { Icon, Kbd, PageTitle, SectionLabel, HighlightedText } from "../design-system/atoms";
import { EmptyState, FilterChip, SearchBox, SegmentedControl, ShortcutHint, ThreadRow } from "../design-system/molecules";
import { ShortcutBar, TopBar } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import { groupHits, type SearchHit } from "../lib/search";
import { dateLabels, runSearch, searchToday, useSearch } from "../state/search";
import { useAiSettings } from "../state/aiSettings";
import { accountFor, useStore } from "../state/store";
import { meaningHint } from "./AiProgress";
import { useScreenKeys } from "../state/useScreenKeys";
import styles from "./SearchScreen.module.css";

const hints = [
  { keys: "↵", label: "Open" },
  { keys: "⌘⌫", label: "Delete" },
  { keys: "Tab", label: "Filters" },
  { keys: "⌘K", label: "Ask AI about these" },
  { keys: "Esc", label: "Clear search" },
];

/** How long typing pauses before Moss searches for the words. */
const typingPause = 200;

/** The results as grouped on screen, and the selected one (the first, if the cursor's has gone). */
function onScreen({ hits, groupBy, cursorId: cursor }: Pick<ReturnType<typeof useSearch.getState>, "hits" | "groupBy" | "cursorId">) {
  const groups = groupHits(hits, groupBy, (id) => accountFor(id).name, searchToday());
  const order = groups.flatMap((g) => g.hits.map((h) => h.doc.id));
  const cursorId = order.includes(cursor ?? "") ? cursor : order[0];
  return { groups, order, cursorId, current: hits.find((h) => h.doc.id === cursorId) };
}

export function SearchScreen() {
  const accounts = useStore((s) => s.accounts);
  const openFound = useStore((s) => s.openFound);
  const deleteThread = useStore((s) => s.deleteThread);
  const leave = useStore((s) => s.leave);
  const search = useSearch();
  const { query, filters, hits } = search;
  const getAi = meaningHint(useAiSettings((s) => s.status));

  // Each search can run the embedding model, so typing waits for a pause. Filters (and opening the
  // screen) search at once, and so does ↵, which needs results for the words just typed.
  const searched = useRef<string>(undefined);
  const pending = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => {
    const typed = searched.current !== undefined && searched.current !== query;
    searched.current = query;
    if (!typed) {
      void runSearch();
      return;
    }
    pending.current = setTimeout(() => {
      pending.current = undefined;
      void runSearch();
    }, typingPause);
    return () => clearTimeout(pending.current);
  }, [query, filters]);
  const searchNow = async () => {
    if (pending.current === undefined) return;
    clearTimeout(pending.current);
    pending.current = undefined;
    await runSearch();
  };

  const { groups, order, cursorId, current } = onScreen(search);

  const selectedRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [cursorId]);

  const escape = () => {
    if (!useSearch.getState().clear()) leave();
  };
  // Every result opens, even mail no list holds (only you sent it, or it's long done); one
  // that's gone since can't be shown, so it leaves the results.
  const openHit = async (hit?: SearchHit, among = order) => {
    if (hit?.doc.threadId && !(await openFound(hit.doc.threadId))) search.removeHit(hit.doc.id, among);
  };
  // ↵ straight after typing opens the top result for those words, not the last ones searched.
  const openTop = async () => {
    await searchNow();
    const now = onScreen(useSearch.getState());
    await openHit(now.current, now.order);
  };
  // To Trash, out of the results; ⌘Z puts it back in both. A result with no email behind it
  // (demo mail that isn't in the mailbox) has nothing to delete.
  const deleteHit = (hit?: SearchHit) => {
    if (!hit?.doc.threadId) return false;
    deleteThread(hit.doc.threadId, { hit, index: hits.indexOf(hit) });
    search.removeHit(hit.doc.id, order);
    return true;
  };
  // ⌘Z in the field undoes typing, unless the last thing you did was delete a result here.
  const undoHere = () => {
    const last = useStore.getState().undoSteps.at(-1);
    if (!last || last.kind === "send" || last.kind === "schedule" || last.view !== "search") return false;
    useStore.getState().undo();
    return true;
  };
  const toggleFrom = () => search.toggleFrom(filters.from ?? current?.doc.from);
  const cycleAccount = () => search.cycleAccount(accounts.map((a) => a.id));

  useScreenKeys((k) => {
    if (k.key === "escape") return escape(), true;
    if (k.mod) {
      if (k.key === "backspace") return deleteHit(current);
      if (k.key === "z" && !k.shift) return undoHere();
      return;
    }
    if (k.alt) {
      const actions: Record<string, () => void> = {
        f: toggleFrom,
        a: cycleAccount,
        p: search.toggleAttachment,
        d: search.cycleDate,
        g: search.toggleGroupBy,
      };
      actions[k.key]?.();
      return k.key in actions;
    }
    const down = k.key === "arrowdown";
    if (down || k.key === "arrowup") return search.moveCursor(down ? 1 : -1, order), true;
    if (k.key === "enter") return void openTop(), true;
    if ((k.key === "backspace" || k.key === "delete") && !k.typing) return deleteHit(current);
    // M: to another tab. In the field it's a letter.
    if (k.key === "m" && !k.typing && !k.shift && current?.doc.threadId) return useStore.getState().openMove(current.doc.threadId), true;
  });

  const words = query.trim() || filters.from;
  const count = words ? `${hits.length} ${hits.length === 1 ? "result" : "results"}` : undefined;

  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>Search</PageTitle>}
          right={<ShortcutHint keys="Esc" label="Clear search" tone="ink-2" onClick={escape} />}
        />
      }
      footer={<ShortcutBar hints={hints} />}
    >
      <div className={styles.column}>
        <SearchBox value={query} onChange={search.setQuery} count={count} />
        {getAi && (
          <p className={styles.meaning}>
            <Icon icon={Sparkles} size={14} tone="ink-3" />
            Search by meaning needs AI ·
            <button type="button" className={styles.getAi} onClick={() => void useAiSettings.getState().choosePlan("essentials")}>
              Get it ({getAi})
            </button>
          </p>
        )}
        <div className={styles.filters}>
          {filters.from ? (
            <FilterChip person={filters.from.initials} label={`From: ${filters.from.name}`} keys="⌥F" active removable onClick={toggleFrom} />
          ) : (
            <FilterChip icon={User} label="From: Anyone" keys="⌥F" onClick={toggleFrom} />
          )}
          <FilterChip
            icon={Inbox}
            label={`Account: ${filters.accountId === "all" ? "All" : accountFor(filters.accountId).name}`}
            keys="⌥A"
            active={filters.accountId !== "all"}
            menu
            onClick={cycleAccount}
          />
          <FilterChip icon={Paperclip} label="Has attachment" keys="⌥P" active={filters.hasAttachment} onClick={search.toggleAttachment} />
          <FilterChip
            icon={Calendar}
            label={`Date: ${dateLabels[filters.date]}`}
            keys="⌥D"
            active={filters.date !== "any"}
            menu
            onClick={search.cycleDate}
          />
          <span className={styles.spacer} />
          <span className={styles.groupLabel}>Group by</span>
          <SegmentedControl
            label="Group by"
            size="sm"
            value={search.groupBy}
            onChange={search.toggleGroupBy}
            options={[
              { value: "date", label: "Date" },
              { value: "account", label: "Account" },
            ]}
          />
          <Kbd>⌥G</Kbd>
        </div>
        <div className={styles.list} role="listbox" aria-label="Results">
          {!words && (
            <EmptyState icon={Search} title="Search all your mail" description="Type a name, a word or a phrase. The filters narrow it down." />
          )}
          {words && !hits.length && (
            <EmptyState
              icon={SearchX}
              title="Nothing found"
              description="No email matches all of these words and filters."
              steps={[{ keys: "Esc", label: "Clear search", onClick: escape }]}
            />
          )}
          {groups.map((g) => (
            <div key={g.title} role="group" aria-label={g.title}>
              <div className={styles.heading}>
                <SectionLabel>{g.title}</SectionLabel>
                {g.note && <span className={styles.note}>{g.note}</span>}
              </div>
              {g.hits.map((h) => {
                const a = accountFor(h.doc.accountId);
                const selected = h.doc.id === cursorId;
                return (
                  <ThreadRow
                    key={h.doc.id}
                    ref={selected ? selectedRef : undefined}
                    account={{ letter: a.letter, color: a.color }}
                    from={h.doc.from.name}
                    subject={<HighlightedText parts={h.subject} raised={selected} />}
                    summary={<HighlightedText parts={h.summary} raised={selected} />}
                    time={h.doc.when}
                    attachment={h.doc.hasAttachment}
                    section={h.best ? "reply" : "info"}
                    selected={selected}
                    onOpen={() => {
                      search.setCursor(h.doc.id);
                      void openHit(h);
                    }}
                    actions={
                      <>
                        <ShortcutHint keys="↵" label="Open" tone="ink-2" gap={6} onClick={() => void openHit(h)} />
                        {h.doc.threadId && <ShortcutHint keys="⌘⌫" label="Delete" tone="ink-2" gap={6} onClick={() => deleteHit(h)} />}
                      </>
                    }
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </AppFrame>
  );
}
