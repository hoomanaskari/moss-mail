import { useEffect, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { Clock3, HardDrive } from "lucide-react";
import { PageTitle } from "../design-system/atoms";
import { EmptyState, Notice, ShortcutHint, ThreadRow } from "../design-system/molecules";
import { ShortcutBar, TopBar } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import { remindTime } from "../lib/time";
import { cancelScheduled, editScheduled, sendNow, useScheduled, visibleScheduled, type ScheduledEmail } from "../state/scheduled";
import { accountFor, useStore } from "../state/store";
import { useScreenKeys } from "../state/useScreenKeys";
import { recipients } from "./DraftsScreen";
import styles from "./DoneScreen.module.css";

const rowHints = [
  { keys: "↵", label: "Edit" },
  { keys: "H", label: "New time" },
  { keys: "⌘↵", label: "Send now" },
  { keys: "⌫", label: "Back to drafts" },
];
const back = { keys: "Esc", label: "Back" };

const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

/** How it starts, or why it hasn't gone yet and when it is tried again, in normal ink so it's seen. */
function summary(e: ScheduledEmail, now: number) {
  if (e.error) {
    const reason = `${sentence(e.error)}${/[.!?]$/.test(e.error) ? "" : "."}`;
    return <span className={styles.problem}>{reason}{e.nextTry > now ? ` Trying again at ${remindTime(e.nextTry, now)}.` : ""}</span>;
  }
  return e.draft.body.replace(/\s+/g, " ").trim();
}

/**
 * Emails set to send later, the soonest first, with when each goes. Reached from ⌘K and the
 * inbox. Each can be opened to edit, given a new time, sent now, or put back in Drafts.
 */
export function ScheduledScreen() {
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
  const { emails, cursorId, setCursor, moveCursor, openMove } = useScheduled();
  const list = visibleScheduled(emails, s.accountFilter);
  const now = Date.now();

  const selectedRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [cursorId]);

  useScreenKeys((k) => {
    // The time picker has the keys while it is open.
    if (useScheduled.getState().picking) return;
    if (k.key === "escape") return s.leave(), true;
    const on = list.find((e) => e.draft.id === cursorId)?.draft.id;
    if (k.mod) {
      if (k.key === "enter" && on) return sendNow(on), true;
      return;
    }
    if (k.key === "arrowdown") return moveCursor(1, list), true;
    if (k.key === "arrowup") return moveCursor(-1, list), true;
    if (k.key === "c") return s.compose("new"), true;
    if (k.key === "/") return s.openSearch(), true;
    if (!on) return;
    if (k.key === "enter" || k.key === "o") return void editScheduled(on), true;
    if (k.key === "h") return openMove(on), true;
    if (k.key === "backspace" || k.key === "delete") return void cancelScheduled(on), true;
  });

  const filter = s.accountFilter;
  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>{filter === "all" ? "Scheduled" : `Scheduled in ${accountFor(filter).name}`}</PageTitle>}
          right={<ShortcutHint keys="Esc" label="Back" tone="ink-2" onClick={s.leave} />}
        />
      }
      footer={<ShortcutBar hints={list.length ? [...rowHints, back] : [{ keys: "C", label: "New email" }, back]} />}
    >
      <div className={styles.column}>
        <Notice variant="plain" icon={HardDrive}>
          These wait on this Mac. They go at their time while Moss is open, or as soon as it opens again.
        </Notice>
        <div className={styles.list} role="listbox" aria-label="Scheduled">
          {!list.length && (
            <EmptyState
              icon={Clock3}
              title="Nothing scheduled"
              description="To send an email later, press ⌘⇧↵ while you write it."
              steps={[
                { keys: "C", label: "New email", onClick: () => s.compose("new") },
                { keys: "Esc", label: "Back", onClick: s.leave },
              ]}
            />
          )}
          {list.map((e) => {
            const d = e.draft;
            const a = accountFor(d.fromAccountId);
            const selected = d.id === cursorId;
            return (
              <ThreadRow
                key={d.id}
                ref={selected ? selectedRef : undefined}
                account={{ letter: a.letter, color: a.color }}
                from={recipients(d)}
                subject={d.subject.trim() || "No subject"}
                summary={summary(e, now)}
                due={e.error && "Not sent yet"}
                time={remindTime(e.sendAt, now)}
                section="info"
                selected={selected}
                onOpen={() => {
                  setCursor(d.id);
                  void editScheduled(d.id);
                }}
                actions={
                  <>
                    <ShortcutHint keys="↵" label="Edit" tone="ink-2" gap={6} onClick={() => void editScheduled(d.id)} />
                    <ShortcutHint keys="H" label="New time" tone="ink-2" gap={6} onClick={() => openMove(d.id)} />
                    <ShortcutHint keys="⌘↵" label="Send now" tone="ink-2" gap={6} onClick={() => sendNow(d.id)} />
                    <ShortcutHint keys="⌫" label="Back to drafts" tone="ink-2" gap={6} onClick={() => void cancelScheduled(d.id)} />
                  </>
                }
              />
            );
          })}
        </div>
      </div>
    </AppFrame>
  );
}
