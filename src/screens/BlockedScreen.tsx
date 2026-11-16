import { useEffect, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { FolderLock, ShieldCheck } from "lucide-react";
import { PageTitle, SectionLabel } from "../design-system/atoms";
import { EmptyState, Notice, ShortcutHint, ThreadRow } from "../design-system/molecules";
import { ShortcutBar, TopBar } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import { doneGroups } from "../lib/done";
import { accountFor, useStore, visibleBlocked } from "../state/store";
import { useScreenKeys } from "../state/useScreenKeys";
import styles from "./DoneScreen.module.css";

const hints = [
  { keys: "↵", label: "Open" },
  { keys: "U", label: "Unblock" },
  { keys: "C", label: "New email" },
  { keys: "/", label: "Search" },
  { keys: "Esc", label: "Back" },
];

/** Mail from senders you blocked, newest first. Reached from ⌘K, like Spark's Blocked folder. */
export function BlockedScreen() {
  // Only what this screen shows, so a change elsewhere doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      blockedThreads: x.blockedThreads,
      accountFilter: x.accountFilter,
      // Rows show each account's letter and colour.
      accounts: x.accounts,
      blockedCursorId: x.blockedCursorId,
      leave: x.leave,
      moveBlockedCursor: x.moveBlockedCursor,
      open: x.open,
      unblockSender: x.unblockSender,
      compose: x.compose,
      openSearch: x.openSearch,
    })),
  );
  const list = visibleBlocked(s);
  const groups = doneGroups(list, Date.now(), (t) => t.lastMessageAt);
  const cursorId = s.blockedCursorId;

  const selectedRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [cursorId]);

  useScreenKeys((k) => {
    if (k.key === "escape") return s.leave(), true;
    if (k.mod) return;
    if (k.key === "arrowdown") return s.moveBlockedCursor(1), true;
    if (k.key === "arrowup") return s.moveBlockedCursor(-1), true;
    if ((k.key === "enter" || k.key === "o") && cursorId) return s.open(cursorId), true;
    if (k.key === "u") return s.unblockSender(), true;
    if (k.key === "c") return s.compose("new"), true;
    if (k.key === "/") return s.openSearch(), true;
  });

  const filter = s.accountFilter;
  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>{filter === "all" ? "Blocked" : `Blocked in ${accountFor(filter).name}`}</PageTitle>}
          right={<ShortcutHint keys="Esc" label="Back" tone="ink-2" onClick={s.leave} />}
        />
      }
      footer={<ShortcutBar hints={hints} />}
    >
      <div className={styles.column}>
        <Notice variant="plain" icon={FolderLock}>
          Blocked mail waits in each account’s Blocked folder. U unblocks the sender and moves their mail back to your inbox.
        </Notice>
        <div className={styles.list} role="listbox" aria-label="Blocked">
          {!list.length && (
            <EmptyState
              icon={ShieldCheck}
              title="No blocked mail"
              description="When you block a sender with B, their mail lands here instead of your inbox."
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
                    // Nothing here waits on you.
                    section="info"
                    selected={selected}
                    onOpen={() => s.open(t.id)}
                    actions={
                      <>
                        <ShortcutHint keys="↵" label="Open" tone="ink-2" gap={6} onClick={() => s.open(t.id)} />
                        <ShortcutHint keys="U" label="Unblock" tone="ink-2" gap={6} onClick={() => s.unblockSender(t.id)} />
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
