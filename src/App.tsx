import { QuitDialog } from "./screens/QuitDialog";
import { watchQuit } from "./services/quit";
import { useEffect } from "react";
import { CommandPalette, MoveDialog, RemindDialog, ScreenErrorBoundary } from "./design-system/organisms";
import { splits } from "./data/demo";
import { ComposeScreen } from "./screens/ComposeScreen";
import { DoneScreen } from "./screens/DoneScreen";
import { DraftsScreen } from "./screens/DraftsScreen";
import { ScheduledScreen } from "./screens/ScheduledScreen";
import { InboxScreen } from "./screens/InboxScreen";
import { InboxZeroScreen } from "./screens/InboxZeroScreen";
import { AddAccountScreen } from "./screens/AddAccountScreen";
import { BlockedScreen } from "./screens/BlockedScreen";
import { SendersScreen } from "./screens/SendersScreen";
import { SearchScreen } from "./screens/SearchScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { ThreadScreen } from "./screens/ThreadScreen";
import { CredentialCleanupNotice } from "./screens/CredentialCleanupNotice";
import { StartupError } from "./screens/StartupError";
import { UndoToast } from "./screens/UndoToast";
import { AskSide } from "./screens/AskSide";
import { AppFrameSide } from "./design-system/templates";
import { TopBarStatus } from "./design-system/organisms";
import { AppStatus } from "./screens/AppStatus";
import { UpdateDialog } from "./screens/UpdateDialog";
import { useAppIdentity } from "./services/appIdentity";
import { useAskPanel, watchAsk } from "./state/askPanel";
import { useAiSettings, watchAi } from "./state/aiSettings";
import { contextThread, paletteGroups } from "./state/commands";
import { watchDrafts } from "./state/drafts";
import { closePicker, pickTime, useScheduled } from "./state/scheduled";
import { useStartup } from "./state/startup";
import { findThread, shownView, useStore, type View } from "./state/store";
import { useAppIcon } from "./state/useAppIcon";
import { usePreferences } from "./state/usePreferences";
import { useShortcuts } from "./state/useShortcuts";

/** Screens the Ask Moss panel sits beside. Not the composer, which has Write with Moss, nor setup. */
const askBeside = (view: View) => view !== "compose" && view !== "settings" && view !== "add-account";

/** The mail, or why it couldn't be loaded: then nothing of the mail app runs behind it. */
export function App() {
  const failed = useStartup((s) => s.failed);
  return failed === undefined ? <MailApp /> : <StartupError />;
}

function MailApp() {
  const storedView = useStore((s) => s.view);
  const view = useStore(shownView);
  const askOpen = useAskPanel((s) => s.open);
  const paletteOpen = useStore((s) => s.paletteOpen);
  const paletteQuery = useStore((s) => s.paletteQuery);
  const setPaletteOpen = useStore((s) => s.setPaletteOpen);
  // An open email drops the Remind menu from its own Remind button; anywhere else it is a dialog,
  // and so is "if no one replies", which that button doesn't ask.
  const reminding = useStore((s) =>
    s.remindingId && (s.remindNoReply || !(s.view === "thread" && s.openThreadId === s.remindingId)) ? findThread(s, s.remindingId) : undefined,
  );
  const noReply = useStore((s) => s.remindNoReply);
  const movingId = useStore((s) => s.movingId);
  const movingThread = useStore((s) => findThread(s, s.movingId));
  // Send later asks when about the email in the composer, or a scheduled one being moved.
  const picking = useScheduled((x) => x.picking);
  const moving = useScheduled(({ picking: p, emails }) => (p?.kind === "move" ? emails.find((e) => e.draft.id === p.id)?.draft.subject : undefined));
  // Only the subject: typing the body mustn't redraw the whole app.
  const composing = useStore((s) => s.draft?.subject);
  const sendLaterSubject = (picking?.kind === "move" ? moving : composing)?.trim() || "No subject";
  const name = useAppIdentity((i) => i.name);
  usePreferences();
  useAppIcon();
  useShortcuts();

  useEffect(() => {
    if (import.meta.env.DEV) import("./dev/scenes").then((m) => m.applyScene());
  }, []);

  // The AI's status, for the top bar's download line and the search hint.
  useEffect(() => {
    void useAiSettings.getState().loadStatus();
    return watchAi();
  }, []);

  // Unsent emails are saved as you type, so leaving the composer never loses one.
  useEffect(() => watchDrafts(useStore), []);
  useEffect(() => watchQuit(), []);

  // Answers to Ask Moss stream into the panel as they are written.
  useEffect(() => watchAsk(), []);

  // Keep the stored view in step when we had to fall back, so shortcuts act on what you see.
  useEffect(() => {
    if (view !== storedView) useStore.setState({ view });
  }, [view, storedView]);

  useEffect(() => {
    document.title = view === "zero" ? `${name} · Inbox zero` : name;
  }, [view, name]);

  return (
    <TopBarStatus.Provider value={<AppStatus />}>
    <AppFrameSide.Provider value={askOpen && askBeside(view) ? <AskSide /> : null}>
      <ScreenErrorBoundary key={view} onRecover={() => useStore.setState({ view: "inbox", paletteOpen: false })}>
        {view === "inbox" && <InboxScreen />}
        {view === "thread" && <ThreadScreen />}
        {view === "compose" && <ComposeScreen />}
        {view === "zero" && <InboxZeroScreen />}
        {view === "add-account" && <AddAccountScreen />}
        {view === "settings" && <SettingsScreen />}
        {view === "search" && <SearchScreen />}
        {view === "done" && <DoneScreen />}
        {view === "blocked" && <BlockedScreen />}
        {view === "drafts" && <DraftsScreen />}
        {view === "senders" && <SendersScreen />}
        {view === "scheduled" && <ScheduledScreen />}
      </ScreenErrorBoundary>
      <UndoToast />
      <CredentialCleanupNotice />
      {reminding && (
        <RemindDialog
          title={noReply ? "Remind me if no reply" : undefined}
          action="Remind me"
          subject={reminding.subject}
          onPick={(at) => (noReply ? useStore.getState().waitForReply(at, reminding.id) : useStore.getState().remindLater(reminding.id, at))}
          onClose={() => useStore.getState().closeRemind()}
        />
      )}
      {movingId && (
        <MoveDialog
          splits={splits}
          subject={movingThread?.subject}
          current={movingThread?.split}
          moved={movingThread?.movedByYou}
          onPick={(to) => useStore.getState().moveTo(to, movingId)}
          onClose={() => useStore.getState().closeMove()}
        />
      )}
      {picking && <RemindDialog title="Send later" action="Send" subject={sendLaterSubject} onPick={pickTime} onClose={closePicker} />}
      {paletteOpen && (
        <CommandPalette
          groups={paletteGroups}
          context={contextThread()?.subject}
          initialQuery={paletteQuery}
          onClose={() => setPaletteOpen(false)}
        />
      )}
      <UpdateDialog />
      <QuitDialog />
    </AppFrameSide.Provider>
    </TopBarStatus.Provider>
  );
}
