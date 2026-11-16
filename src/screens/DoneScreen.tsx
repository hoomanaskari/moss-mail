import { useEffect, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { Archive, CheckCheck } from "lucide-react";
import { PageTitle, SectionLabel } from "../design-system/atoms";
import { EmptyState, Notice, ShortcutHint, ThreadRow } from "../design-system/molecules";
import { ShortcutBar, TopBar } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import { doneGroups } from "../lib/done";
import { accountFor, useStore, visibleDone } from "../state/store";
import { useScreenKeys } from "../state/useScreenKeys";
import styles from "./DoneScreen.module.css";

const hints = [
  { keys: "↵", label: "Open" },
  { keys: "E", label: "Not done" },
  { keys: "⌘Z", label: "Undo" },
  { keys: "C", label: "New email" },
  { keys: "/", label: "Search" },
  { keys: "Esc", label: "Back" },
];

/** The engine sends this many; older done mail is found with Search. */
const shownAtMost = 200;

/** Everything you marked done, most recent first, so nothing feels lost. */
export function DoneScreen() {
  // Only what this screen shows, so a change elsewhere doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      doneThreads: x.doneThreads,
      accountFilter: x.accountFilter,
      // Rows show each account's letter and colour.
      accounts: x.accounts,
      doneCursorId: x.doneCursorId,
      leave: x.leave,
      undo: x.undo,
      moveDoneCursor: x.moveDoneCursor,
      open: x.open,
      markNotDone: x.markNotDone,
      openMove: x.openMove,
      deleteThread: x.deleteThread,
      compose: x.compose,
      openSearch: x.openSearch,
    })),
  );
  const list = visibleDone(s);
  const groups = doneGroups(list, Date.now());
  const cursorId = s.doneCursorId;

  const selectedRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [cursorId]);

  useScreenKeys((k) => {
    if (k.key === "escape") return s.leave(), true;
    if (k.mod) {
      if (k.key === "z") return s.undo(), true;
      return;
    }
    if (k.key === "arrowdown") return s.moveDoneCursor(1), true;
    if (k.key === "arrowup") return s.moveDoneCursor(-1), true;
    if ((k.key === "enter" || k.key === "o") && cursorId) return s.open(cursorId), true;
    if (k.key === "e") return s.markNotDone(), true;
    if (k.key === "m") return s.openMove(), true;
    if (k.key === "backspace" || k.key === "delete") return s.deleteThread(), true;
    if (k.key === "c") return s.compose("new"), true;
    if (k.key === "/") return s.openSearch(), true;
  });

  const filter = s.accountFilter;
  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>{filter === "all" ? "Done" : `Done in ${accountFor(filter).name}`}</PageTitle>}
          right={<ShortcutHint keys="Esc" label="Back" tone="ink-2" onClick={s.leave} />}
        />
      }
      footer={<ShortcutBar hints={hints} />}
    >
      <div className={styles.column}>
        <Notice variant="plain" icon={Archive}>
          Done emails are archived, not deleted. E brings one back to your inbox.
        </Notice>
        <div className={styles.list} role="listbox" aria-label="Done">
          {!list.length && (
            <EmptyState
              icon={CheckCheck}
              title="Nothing done yet"
              description="Emails you mark done with E land here, so you can always find them again."
              steps={[{ keys: "Esc", label: "Back", onClick: s.leave }]}
            />
          )}
          {groups.map((g) => (
            <div key={g.title} role="group" aria-label={g.title}>
              <div className={styles.heading}>
                <SectionLabel>{g.title}</SectionLabel>
              </div>
              {g.threads.map((t) => {
                const a = accountFor(t.accountId);
                const selected = t.id === cursorId;
                return (
                  <ThreadRow
                    key={t.id}
                    ref={selected ? selectedRef : undefined}
                    account={{ letter: a.letter, color: a.color }}
                    from={t.from.name}
                    subject={t.subject}
                    summary={t.summary}
                    time={t.time}
                    // Handled, so nothing here needs your reply.
                    section="info"
                    selected={selected}
                    onOpen={() => s.open(t.id)}
                    actions={
                      <>
                        <ShortcutHint keys="↵" label="Open" tone="ink-2" gap={6} onClick={() => s.open(t.id)} />
                        <ShortcutHint keys="E" label="Not done" tone="ink-2" gap={6} onClick={() => s.markNotDone(t.id)} />
                        <ShortcutHint keys="⌫" label="Delete" tone="ink-2" gap={6} onClick={() => s.deleteThread(t.id)} />
                      </>
                    }
                  />
                );
              })}
            </div>
          ))}
          {list.length >= shownAtMost && <p className={styles.more}>Older done emails are in Search.</p>}
        </div>
      </div>
    </AppFrame>
  );
}
