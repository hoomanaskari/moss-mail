import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { HardDrive, PenLine, Smartphone } from "lucide-react";
import { PageTitle } from "../design-system/atoms";
import { EmptyState, Notice, ShortcutHint, ThreadRow } from "../design-system/molecules";
import { ShortcutBar, TopBar } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import { listTime } from "../lib/time";
import { adoptDraft, reloadDrafts, resolveLocalRecovery, useDrafts, visibleDrafts, type SavedDraft } from "../state/drafts";
import { accountFor, useStore } from "../state/store";
import { useScreenKeys } from "../state/useScreenKeys";
import { quietly } from "../services/mail";
import { DeliveryAttention } from "./DeliveryAttention";
import { LocalDraftRecoveries } from "./LocalDraftRecoveries";
import styles from "./DoneScreen.module.css";

const hints = [
  { keys: "↵", label: "Open" },
  { keys: "⌫", label: "Delete" },
  { keys: "C", label: "New email" },
  { keys: "Esc", label: "Back" },
];

/** "To Lena Vogt", "To Lena Vogt and 2 more", or "No one yet". */
export function recipients(d: SavedDraft) {
  const people = [...d.to, ...d.cc, ...d.bcc];
  if (!people.length) return "No one yet";
  return `To ${people[0].name}${people.length > 1 ? ` and ${people.length - 1} more` : ""}`;
}

/** Emails you started and haven't sent, most recently changed first. Reached from ⌘K and the inbox. */
export function DraftsScreen() {
  // Only what this screen shows, so a change elsewhere doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      accountFilter: x.accountFilter,
      accounts: x.accounts,
      leave: x.leave,
      openDraft: x.openDraft,
      deleteDraft: x.deleteDraft,
      compose: x.compose,
      openSearch: x.openSearch,
    })),
  );
  const [recoveryRefresh, setRecoveryRefresh] = useState(0);
  const { drafts, cursorId, setCursor, moveCursor } = useDrafts();
  const list = visibleDrafts(drafts, s.accountFilter);
  const now = Date.now();

  const selectedRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [cursorId]);

  useScreenKeys((k, event) => {
    if (event.target instanceof HTMLElement && event.target.closest("button, input, select, textarea, a, [contenteditable]")) return;
    if (k.key === "escape") return s.leave(), true;
    if (k.mod) return;
    if (k.key === "arrowdown") return moveCursor(1, list), true;
    if (k.key === "arrowup") return moveCursor(-1, list), true;
    if ((k.key === "enter" || k.key === "o") && cursorId) return s.openDraft(cursorId), true;
    if (k.key === "backspace" || k.key === "delete") return s.deleteDraft(), true;
    if (k.key === "c") return s.compose("new"), true;
    if (k.key === "/") return s.openSearch(), true;
  });

  const filter = s.accountFilter;
  // Resend has no Drafts folder: its drafts stay on this Mac.
  const inView = s.accounts.filter((a) => filter === "all" || a.id === filter);
  const onServer = !inView.length || inView.some((a) => a.connection?.provider !== "resend");
  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>{filter === "all" ? "Drafts" : `Drafts in ${accountFor(filter).name}`}</PageTitle>}
          right={<ShortcutHint keys="Esc" label="Back" tone="ink-2" onClick={s.leave} />}
        />
      }
      footer={<ShortcutBar hints={hints} />}
    >
      <div className={styles.column}>
        <DeliveryAttention onRecoveryChanged={() => setRecoveryRefresh((value) => value + 1)} />
        <LocalDraftRecoveries refresh={recoveryRefresh} accounts={s.accounts}
          onAdopt={(draft, id, version) => {
            resolveLocalRecovery(id, version, draft.id);
            if (!useDrafts.getState().drafts.some((row) => row.id === draft.id)) adoptDraft(draft);
            if (useStore.getState().view === "drafts") useStore.getState().openDraft(draft.id);
            quietly(reloadDrafts());
          }}
          onResolve={(id, version, adoptedId) => {
            resolveLocalRecovery(id, version, adoptedId);
            quietly(reloadDrafts());
          }}
        />
        <Notice variant="plain" icon={onServer ? Smartphone : HardDrive}>
          {onServer
            ? "Drafts save as you type, and show on your phone too. Sending one removes it."
            : "Drafts are saved on this Mac as you type. Sending one removes it from here."}
        </Notice>
        <div className={styles.list} role="listbox" aria-label="Drafts">
          {!list.length && (
            <EmptyState
              icon={PenLine}
              title="No drafts"
              description="Emails you start and don’t send wait here, so you can finish them later."
              steps={[
                { keys: "C", label: "New email", onClick: () => s.compose("new") },
                { keys: "Esc", label: "Back", onClick: s.leave },
              ]}
            />
          )}
          {list.map((d) => {
            const a = accountFor(d.fromAccountId);
            const selected = d.id === cursorId;
            return (
              <ThreadRow
                key={d.id}
                ref={selected ? selectedRef : undefined}
                account={{ letter: a.letter, color: a.color }}
                from={recipients(d)}
                subject={d.subject.trim() || "No subject"}
                summary={d.body.replace(/\s+/g, " ").trim()}
                time={listTime(d.updatedAt, now)}
                section="info"
                selected={selected}
                onOpen={() => {
                  setCursor(d.id);
                  s.openDraft(d.id);
                }}
                actions={
                  <>
                    <ShortcutHint keys="↵" label="Open" tone="ink-2" gap={6} onClick={() => s.openDraft(d.id)} />
                    <ShortcutHint keys="⌫" label="Delete" tone="ink-2" gap={6} onClick={() => s.deleteDraft(d.id)} />
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
