import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./design-system/tokens/global.css";
import { App } from "./App";
import { isTauri, markNativeTitleBar } from "./lib/platform";
import { setAccountSetup } from "./services/accountSetup";
import { setAgents } from "./services/agents";
import { setAi } from "./services/ai";
import { setAsk } from "./services/ask";
import { setMail } from "./services/mail";
import { setSummaries } from "./services/summaries";
import { setReplies } from "./services/replies";
import { setUpdates } from "./services/updates";
import { start } from "./state/startup";
import { startUpdates } from "./state/updates";
import { useStore } from "./state/store";

// Set the theme and reading size on <html> before React renders so there is no flash of the wrong ones.
const { theme, readingSize } = useStore.getState();
document.documentElement.dataset.theme = theme;
document.documentElement.dataset.reading = readingSize;
markNativeTitleBar();

/** Puts the desktop app's real services in place of the demo ones. Once: Try again only loads the mail again. */
async function wire() {
  const [{ tauriMail }, { tauriSetup }, { tauriAi }, { tauriSummaries }, { tauriReplies }, { tauriAsk }, { tauriUpdates }, { tauriAgents }, { loadAppIdentity }, { watchMenu }] = await Promise.all([
    import("./services/tauriMail"),
    import("./services/tauriSetup"),
    import("./services/tauriAi"),
    import("./services/tauriSummaries"),
    import("./services/tauriReplies"),
    import("./services/tauriAsk"),
    import("./services/tauriUpdates"),
    import("./services/tauriAgents"),
    import("./services/tauriIdentity"),
    import("./services/menu"),
  ]);
  // The installed Moss or Moss Dev: Moss Dev never checks for updates.
  await loadAppIdentity();
  setMail(tauriMail);
  setAccountSetup(tauriSetup);
  setAi(tauriAi);
  setSummaries(tauriSummaries);
  setReplies(tauriReplies);
  setAsk(tauriAsk);
  setUpdates(tauriUpdates);
  setAgents(tauriAgents);
  // The app menu's Check for Updates…
  watchMenu();
  // Looks for a newer Moss a little after launch and every six hours, and offers it in the top bar.
  startUpdates();
}

let wired: Promise<void> | undefined;

/**
 * In the desktop app, real mail loads before the first paint. The store starts with none there,
 * and a failure shows why (with Try again) instead of any mail. The browser keeps the demo.
 */
async function connect() {
  if (!isTauri) return;
  wired ??= wire().catch((error: unknown) => {
    wired = undefined;
    throw error;
  });
  await wired;
  const { startLive } = await import("./services/live");
  await startLive();
}

void start(connect).then(() =>
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  ),
);
