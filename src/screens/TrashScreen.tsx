import { useEffect, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { Trash2 } from "lucide-react";
import { PageTitle, SectionLabel } from "../design-system/atoms";
import { EmptyState, Notice, ShortcutHint, ThreadRow } from "../design-system/molecules";
import { ShortcutBar, TopBar } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import { doneGroups } from "../lib/done";
import { accountFor, useStore } from "../state/store";
import { moveTrashCursor, openFromTrash, restoreToInbox, useTrash, visibleTrash } from "../state/trash";
import { useScreenKeys } from "../state/useScreenKeys";
import styles from "./DoneScreen.module.css";

const hints = [
  { keys: "↵", label: "Open" },
  { keys: "E", label: "Back to inbox" },
  { keys: "C", label: "New email" },
  { keys: "/", label: "Search" },
  { keys: "Esc", label: "Back" },
];

/** The engine sends this many. */
const shownAtMost = 200;

/** Mail you deleted, newest first, so you can put one back. Reached from ⌘K. */
export function TrashScreen() {
  // Only what this screen shows, so a change elsewhere doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      accountFilter: x.accountFilter,
      // Rows show each account's letter and colour.
      accounts: x.accounts,
      leave: x.leave,
      compose: x.compose,
      openSearch: x.openSearch,
    })),
  );
  const { threads, cursorId } = useTrash();
  const list = visibleTrash(threads, s.accountFilter);
  const groups = doneGroups(list, Date.now(), (t) => t.lastMessageAt);

  const selectedRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [cursorId]);

  useScreenKeys((k) => {
    if (k.key === "escape") return s.leave(), true;
    if (k.mod) return;
    if (k.key === "arrowdown") return moveTrashCursor(1), true;
    if (k.key === "arrowup") return moveTrashCursor(-1), true;
    if ((k.key === "enter" || k.key === "o") && cursorId) return openFromTrash(cursorId), true;
    if (k.key === "e") return restoreToInbox(), true;
    if (k.key === "c") return s.compose("new"), true;
    if (k.key === "/") return s.openSearch(), true;
  });

  const filter = s.accountFilter;
  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>{filter === "all" ? "Trash" : `Trash in ${accountFor(filter).name}`}</PageTitle>}
          right={<ShortcutHint keys="Esc" label="Back" tone="ink-2" onClick={s.leave} />}
        />
      }
      footer={<ShortcutBar hints={hints} />}
    >
      <div className={styles.column}>
        <Notice variant="plain" icon={Trash2}>
          Deleted emails wait in each account’s Trash folder until your provider empties it. E puts one back in your inbox.
        </Notice>
        <div className={styles.list} role="listbox" aria-label="Trash">
          {!list.length && (
            <EmptyState
              icon={Trash2}
              title="Trash is empty"
              description="Emails you delete with ⌫ land here, so you can put them back."
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
                    // Deleted, so nothing here needs your reply.
                    section="info"
                    selected={selected}
                    onOpen={() => openFromTrash(t.id)}
                    actions={
                      <>
                        <ShortcutHint keys="↵" label="Open" tone="ink-2" gap={6} onClick={() => openFromTrash(t.id)} />
                        <ShortcutHint keys="E" label="Back to inbox" tone="ink-2" gap={6} onClick={() => restoreToInbox(t.id)} />
                      </>
                    }
                  />
                );
              })}
            </div>
          ))}
          {list.length >= shownAtMost && <p className={styles.more}>Older deleted emails are in your provider’s Trash folder.</p>}
        </div>
      </div>
    </AppFrame>
  );
}
