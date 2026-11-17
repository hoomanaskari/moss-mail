import { useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import { Calendar, Clock3, FilePen, Inbox, Layers, Newspaper, Receipt, TriangleAlert, type LucideIcon } from "lucide-react";
import type { SplitId } from "../data/types";
import { splits } from "../data/demo";
import { Button, PageTitle } from "../design-system/atoms";
import { AccountSwitcher, AddressBar, ShortcutBar, SplitTabs, ThreadList, TopBar } from "../design-system/organisms";
import { addressCounts } from "../lib/addresses";
import { CommandHint, EmptyState, Notice, ProgressSummary, RefreshButton, SplitTab } from "../design-system/molecules";
import { AppFrame } from "../design-system/templates";
import { useDrafts, visibleDrafts } from "../state/drafts";
import { openScheduled, useScheduled, visibleScheduled } from "../state/scheduled";
import { accountCountsFor, accountFor, progressFor, splitCountsFor, stillWaiting, unreadCountsFor, useStore, visibleThreads } from "../state/store";
import { useHeldKey } from "../state/useHeldKey";
import { modKey } from "../lib/platform";
import { syncProblemLabel, useSync } from "../services/live";
import { mail } from "../services/mail";
import styles from "./InboxScreen.module.css";
import { AiProgress } from "./AiProgress";
import { ServerChangesNotice } from "./ServerChangesNotice";

/** What each split holds, shown when it's empty. `title` is completed with " in Work" when filtered. */
const emptyCopy: Record<SplitId, { icon: LucideIcon; title: string; description: string }> = {
  important: { icon: Inbox, title: "Nothing needs you", description: "Emails that need a reply or your attention land here." },
  other: { icon: Layers, title: "Nothing else", description: "Updates and notifications that don’t need you land here." },
  calendar: { icon: Calendar, title: "No invites", description: "Invites and changes to events land here, so they never crowd your inbox." },
  newsletters: { icon: Newspaper, title: "No newsletters", description: "Subscriptions collect here, ready for when you have time to read." },
  receipts: { icon: Receipt, title: "No receipts", description: "Orders, invoices and payment confirmations land here." },
};

export function InboxScreen() {
  // Only what this screen shows, so a change elsewhere (a draft, an Ask answer) doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      threads: x.threads,
      doneThreads: x.doneThreads,
      accounts: x.accounts,
      accountFilter: x.accountFilter,
      addressFilter: x.addressFilter,
      split: x.split,
      cursorId: x.cursorId,
      paletteOpen: x.paletteOpen,
      setPaletteOpen: x.setPaletteOpen,
      setAccountFilter: x.setAccountFilter,
      setAddressFilter: x.setAddressFilter,
      setSplit: x.setSplit,
      cycleSplit: x.cycleSplit,
      openAddAccount: x.openAddAccount,
      openDone: x.openDone,
      openDrafts: x.openDrafts,
      showInboxZero: x.showInboxZero,
      open: x.open,
      markDone: x.markDone,
      openRemind: x.openRemind,
      compose: x.compose,
      deleteThread: x.deleteThread,
      blockSender: x.blockSender,
      openMove: x.openMove,
    })),
  );
  const { threads, doneThreads, accounts, accountFilter: filter, addressFilter, split } = s;
  // Worked out again only when the mail or the filters change, not when the cursor moves.
  const list = useMemo(() => visibleThreads({ threads, accountFilter: filter, split, addressFilter }), [threads, filter, split, addressFilter]);
  // "Done today" turns over at midnight, so the day is an input too.
  const today = new Date().toDateString();
  const p = useMemo(() => progressFor(threads, doneThreads, filter, Date.now()), [threads, doneThreads, filter, today]);
  const copy = emptyCopy[split];
  // Nothing waits in Important, in any account: inbox zero is a key away.
  const clear = useMemo(() => split === "important" && !stillWaiting(threads), [split, threads]);
  const nextSplit = splits[(splits.findIndex((x) => x.id === split) + 1) % splits.length];
  const problems = useSync((x) => x.problems);
  // Holding ⌘ shows which key picks each account.
  const modHeld = useHeldKey(modKey);
  const signedOut = accounts.filter((a) => problems[a.id]?.problem === "signedOut");
  const refused = accounts.filter((a) => problems[a.id]?.problem === "server");
  const allDrafts = useDrafts((x) => x.drafts);
  const drafts = useMemo(() => visibleDrafts(allDrafts, filter).length, [allDrafts, filter]);
  const allScheduled = useScheduled((x) => x.emails);
  const scheduled = useMemo(() => visibleScheduled(allScheduled, filter).length, [allScheduled, filter]);
  const accountCounts = useMemo(() => accountCountsFor(threads, accounts.map((a) => a.id)), [threads, accounts]);
  // An account with several addresses gets a row to see one address's mail at a time.
  const addresses = useMemo(() => {
    if (filter === "all") return [];
    const waitingAt = (address: string) => Object.values(splitCountsFor(threads, filter, address)).reduce((a, b) => a + b, 0);
    return addressCounts(threads, accountFor(filter)).map((a) => ({ email: a.email, count: waitingAt(a.email) }));
  }, [threads, filter, accounts]);
  const splitCounts = useMemo(() => splitCountsFor(threads, filter, addressFilter), [threads, filter, addressFilter]);
  const unreadCounts = useMemo(() => unreadCountsFor(threads, filter, addressFilter), [threads, filter, addressFilter]);
  const problemLabels = useMemo(
    () => Object.fromEntries(Object.entries(problems).map(([id, p]) => [id, syncProblemLabel[p.problem]])),
    [problems],
  );
  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>{filter === "all" ? "Inbox" : `${accountFor(filter).name} inbox`}</PageTitle>}
          right={
            <>
              <AiProgress />
              <RefreshButton onRefresh={() => void mail().syncNow()} />
              <CommandHint onOpen={() => s.setPaletteOpen(true)} />
            </>
          }
        />
      }
      footer={<ShortcutBar />}
    >
      <div className={styles.column}>
        <AccountSwitcher
          accounts={accounts}
          counts={accountCounts}
          problems={problemLabels}
          selected={filter}
          onSelect={s.setAccountFilter}
          onAdd={s.openAddAccount}
          showShortcuts={modHeld && !s.paletteOpen}
          trailing={<ProgressSummary done={p.done} total={p.total} eta={p.eta} onOpen={s.openDone} />}
        />
        {addresses.length > 0 && (
          <AddressBar addresses={addresses} total={accountCounts[filter]} selected={addressFilter} onSelect={s.setAddressFilter} />
        )}
        {signedOut.map((a) => (
          <Notice
            key={a.id}
            variant="inset"
            icon={TriangleAlert}
            tone="due"
            className={styles.syncProblem}
            action={
              <Button variant="secondary" onClick={s.openAddAccount}>
                Sign in again
              </Button>
            }
          >
            {a.name} isn’t syncing: Moss is signed out of {a.address}, so new mail can’t come in.{" "}
            <span className={styles.syncReason}>Why: {problems[a.id].message}</span>
          </Notice>
        ))}
        {refused.map((a) => (
          <Notice key={a.id} variant="inset" icon={TriangleAlert} tone="due" className={styles.syncProblem}>
            {a.name} isn’t syncing. Moss keeps trying.{" "}
            <span className={styles.syncReason}>Why: {problems[a.id].message}</span>
          </Notice>
        ))}
        <ServerChangesNotice />
        <SplitTabs
          splits={splits}
          counts={splitCounts}
          unread={unreadCounts}
          selected={split}
          onSelect={s.setSplit}
          trailing={
            <>
              {drafts > 0 && <SplitTab label="Drafts" count={drafts} icon={FilePen} onSelect={s.openDrafts} />}
              {scheduled > 0 && <SplitTab label="Scheduled" count={scheduled} icon={Clock3} onSelect={openScheduled} />}
            </>
          }
        />
        <ThreadList
          threads={list}
          grouped={split === "important"}
          accountOf={accountFor}
          cursorId={s.cursorId}
          onOpen={s.open}
          onDone={s.markDone}
          onLater={s.openRemind}
          onReply={(id) => s.compose("reply", id)}
          onDelete={s.deleteThread}
          onBlock={(id) => s.blockSender(false, id)}
          onMove={s.openMove}
          empty={
            <EmptyState
              icon={copy.icon}
              title={filter === "all" ? copy.title : `${copy.title} in ${accountFor(filter).name}`}
              description={copy.description}
              steps={[
                { keys: "Tab", label: `Next: ${nextSplit.label}`, onClick: () => s.cycleSplit(1) },
                ...(filter !== "all" ? [{ keys: "⌘0", label: "Show all accounts", onClick: () => s.setAccountFilter("all") }] : []),
                ...(clear ? [{ keys: "Z", label: "Inbox zero", onClick: s.showInboxZero }] : []),
              ]}
            />
          }
        />
      </div>
    </AppFrame>
  );
}
