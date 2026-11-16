import { Fragment, memo, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type Ref, type RefObject } from "react";
import type { Account, AccountColor, Section, Thread } from "../../../data/types";
import { SectionLabel } from "../../atoms";
import { ThreadRow } from "../../molecules";
import rowStyles from "../../molecules/ThreadRow/ThreadRow.module.css";
import styles from "./ThreadList.module.css";

const sectionTitles: Record<Section, string> = {
  reply: "Needs your reply",
  info: "For your info",
  later: "Coming back later",
};

export interface ThreadListProps {
  threads: Thread[];
  accountOf: (id: string) => Account;
  cursorId?: string;
  /** Shown when there are no threads. */
  empty?: ReactNode;
  /** Group under "Needs your reply" / "For your info" / "Coming back later". Off: one plain list,
   * with reminded mail still apart at the bottom under "Coming back later". */
  grouped?: boolean;
  onOpen: (id: string) => void;
  onDone: (id: string) => void;
  onLater: (id: string) => void;
  onReply: (id: string) => void;
  onDelete: (id: string) => void;
  /** Blocks a new sender, from their row. */
  onBlock: (id: string) => void;
  /** Opens the Move menu (M), from the selected row. */
  onMove?: (id: string) => void;
}

type RowActions = Pick<ThreadListProps, "onOpen" | "onDone" | "onLater" | "onReply" | "onDelete" | "onBlock" | "onMove">;

interface RowProps {
  thread: Thread;
  letter: string;
  color: AccountColor;
  selected: boolean;
  position: number;
  setSize: number;
  movable: boolean;
  rowRef?: Ref<HTMLDivElement>;
  /** The list's latest callbacks: rows call through them, so new callbacks don't re-render every row. */
  actions: RefObject<RowActions>;
}

/** One row of the list. Re-renders only when its own email, account or place changes. */
const Row = memo(function Row({ thread: t, letter, color, selected, position, setSize, movable, rowRef, actions }: RowProps) {
  return (
    <ThreadRow
      ref={rowRef}
      account={{ letter, color }}
      from={t.from.name}
      subject={t.subject}
      summary={t.summary}
      time={t.time}
      due={t.due}
      noReply={!!t.waiting && !t.dueDate}
      section={t.section}
      unread={t.unread}
      selected={selected}
      position={position}
      setSize={setSize}
      onOpen={() => actions.current.onOpen(t.id)}
      onDone={() => actions.current.onDone(t.id)}
      onLater={() => actions.current.onLater(t.id)}
      onReply={() => actions.current.onReply(t.id)}
      onDelete={() => actions.current.onDelete(t.id)}
      onBlock={t.sender === "new" ? () => actions.current.onBlock(t.id) : undefined}
      onMove={movable ? () => actions.current.onMove!(t.id) : undefined}
      moved={t.split === "important" && t.movedByYou}
    />
  );
});

/** The inbox list, grouped into "Needs your reply", "For your info" and "Coming back later". */
export function ThreadList({ threads, accountOf, cursorId, empty, grouped = true, onOpen, onDone, onLater, onReply, onDelete, onBlock, onMove }: ThreadListProps) {
  const listRef = useRef<HTMLDivElement>(null);
  const rowRef = useRef<HTMLDivElement>(null);
  const laterRef = useRef<HTMLDivElement>(null);
  const headingRef = useRef<HTMLDivElement>(null);
  const [geometry, setGeometry] = useState({ generation: 0, top: 0, height: 720, row: 58, later: 52, heading: 42 });
  const measurementGeneration = useRef(0);
  const previousGeometry = useRef(geometry);
  const windowed = threads.length > 100;
  const pendingCursorScroll = useRef(false);
  const selectedRef = useRef<HTMLDivElement>(null);
  const actions = useRef<RowActions>({ onOpen, onDone, onLater, onReply, onDelete, onBlock, onMove });
  useLayoutEffect(() => {
    actions.current = { onOpen, onDone, onLater, onReply, onDelete, onBlock, onMove };
  });

  // Measure the actual CSS geometry, including reading-size and font changes.
  // Keeping the selected row mounted lets keyboard jumps use native scrolling.
  useLayoutEffect(() => {
    // The list and rulers are mounted for the lifetime of this layout effect.
    const list = listRef.current!;
    const generation = ++measurementGeneration.current;
    // Rulers stay mounted even for small lists, so crossing the windowing
    // threshold cannot temporarily shrink content and clamp the scroll position.
    const row = rowRef.current!;
    const later = laterRef.current!;
    const heading = headingRef.current!;
    const measure = () => setGeometry((previous) => {
      const next = {
        generation,
        top: list.scrollTop,
        height: list.clientHeight || previous.height,
        row: row.getBoundingClientRect().height || previous.row,
        later: later.getBoundingClientRect().height || previous.later,
        heading: heading.getBoundingClientRect().height || previous.heading,
      };
      return Object.keys(next).every((key) => next[key as keyof typeof next] === previous[key as keyof typeof next]) ? previous : next;
    });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(list);
    observer.observe(row);
    observer.observe(heading);
    observer.observe(later);
    return () => observer.disconnect();
  }, []);

  // Small lists scroll natively. Read their position once when windowing starts.
  useLayoutEffect(() => {
    if (windowed) {
      const top = listRef.current!.scrollTop;
      setGeometry((current) => current.top === top ? current : { ...current, top });
    }
  }, [windowed]);

  const scrollToCursor = () => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
    const top = listRef.current!.scrollTop;
    setGeometry((current) => current.top === top ? current : { ...current, top });
  };
  useLayoutEffect(() => {
    pendingCursorScroll.current = windowed && geometry.generation !== measurementGeneration.current;
    if (!pendingCursorScroll.current) scrollToCursor();
  }, [cursorId]);
  useLayoutEffect(() => {
    if (pendingCursorScroll.current && geometry.generation === measurementGeneration.current) {
      pendingCursorScroll.current = false;
      scrollToCursor();
    }
  }, [geometry.generation]);

  const groups = useMemo(() => {
    const of = (section: Section) => threads.filter((t) => t.section === section);
    // Reminded mail always sits apart at the bottom, so a reminder visibly takes it out of the way.
    return (
      grouped
        ? (["reply", "info", "later"] as Section[]).map((section) => ({ title: sectionTitles[section], items: of(section), heading: true }))
        : [
            { title: "Emails", items: [...of("reply"), ...of("info")], heading: false },
            { title: sectionTitles.later, items: of("later"), heading: true },
          ]
    ).filter((g) => g.items.length > 0);
  }, [threads, grouped]);
  // Keep the same row and fractional offset at the top when reading size changes.
  useLayoutEffect(() => {
    const old = previousGeometry.current;
    previousGeometry.current = geometry;
    const list = listRef.current;
    // The first measurement establishes a baseline, not a reading-size change.
    if (!list || !windowed || old.generation !== geometry.generation || (old.row === geometry.row && old.later === geometry.later && old.heading === geometry.heading)) return;
    let selectedWasVisible = false;
    let oldOffset = 8;
    for (const group of groups) {
      const height = group.items[0].section === "later" ? old.later : old.row;
      oldOffset += group.heading ? old.heading : 0;
      const index = group.items.findIndex((item) => item.id === cursorId);
      const top = oldOffset + index * height;
      if (index >= 0 && top >= old.top && top + height <= old.top + old.height) selectedWasVisible = true;
      oldOffset += group.items.length * height;
    }
    let oldStart = 8;
    let newStart = 8;
    for (const group of groups) {
      const later = group.items[0].section === "later";
      const oldRow = later ? old.later : old.row;
      const newRow = later ? geometry.later : geometry.row;
      const oldHeading = group.heading ? old.heading : 0;
      const newHeading = group.heading ? geometry.heading : 0;
      const end = oldStart + oldHeading + group.items.length * oldRow;
      if (old.top < end) {
        const within = old.top - oldStart;
        list.scrollTop = newStart + (within < oldHeading ? within : newHeading + (within - oldHeading) * newRow / oldRow);
        break;
      }
      oldStart = end;
      newStart += newHeading + group.items.length * newRow;
    }
    if (selectedWasVisible) selectedRef.current?.scrollIntoView({ block: "nearest" });
    const top = list.scrollTop;
    setGeometry((current) => current.top === top ? current : { ...current, top });
  }, [geometry, groups, windowed, cursorId]);
  // Matches the list's 8px block padding.
  let offset = 8;

  return (
    <div ref={listRef} className={styles.list} role="listbox" aria-label="Emails"
      onScroll={(event) => {
        const top = event.currentTarget.scrollTop;
        if (windowed) setGeometry((previous) => previous.top === top ? previous : { ...previous, top });
      }}>
      <div className={styles.measure} aria-hidden>
        <div ref={rowRef} data-testid="thread-row-height" className={rowStyles.row} />
        <div ref={laterRef} data-testid="thread-later-height" className={`${rowStyles.row} ${rowStyles.later}`} />
        <div ref={headingRef} data-testid="thread-heading-height" className={styles.heading}><SectionLabel>Measure</SectionLabel></div>
      </div>
      {groups.length === 0 && empty}
      {groups.map(({ title, items, heading }) => {
        const rowHeight = items[0].section === "later" ? geometry.later : geometry.row;
        const start = offset + (heading ? geometry.heading : 0);
        offset = start + items.length * rowHeight;
        const first = Math.max(0, Math.floor((geometry.top - start) / rowHeight) - 4);
        const last = Math.min(items.length, Math.ceil((geometry.top + geometry.height - start) / rowHeight) + 4);
        const selectedIndex = items.findIndex((item) => item.id === cursorId);
        const indices = windowed
          ? Array.from({ length: Math.max(0, last - first) }, (_, i) => first + i)
          : items.map((_, i) => i);
        // Keep groups stable in the accessibility tree, with at least one real option.
        if (windowed && !indices.includes(0)) indices.push(0);
        if (windowed && selectedIndex >= 0 && !indices.includes(selectedIndex)) indices.push(selectedIndex);
        indices.sort((a, b) => a - b);
        let previous = 0;
        return (
          <div key={title} role="group" aria-label={title} style={{ flexShrink: 0 }}>
            {heading && (
              <div className={styles.heading}>
                <SectionLabel>{title}</SectionLabel>
              </div>
            )}
            {indices.map((index) => {
              const t = items[index];
              const gap = index - previous;
              previous = index + 1;
              const a = accountOf(t.accountId);
              const selected = t.id === cursorId;
              return (
                <Fragment key={t.id}>
                  {gap > 0 && <div aria-hidden style={{ height: gap * rowHeight }} />}
                  <Row
                    thread={t}
                    letter={a.letter}
                    color={a.color}
                    selected={selected}
                    position={index + 1}
                    setSize={items.length}
                    movable={!!onMove}
                    rowRef={selected ? selectedRef : undefined}
                    actions={actions}
                  />
                </Fragment>
              );
            })}
            {windowed && previous < items.length && <div aria-hidden style={{ height: (items.length - previous) * rowHeight }} />}
          </div>
        );
      })}
    </div>
  );
}
