import { quitOperationReasons, useQuit } from "../state/quitState";
export { protectQuit, useQuit } from "../state/quitState";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "../lib/platform";
import { currentDraftQuitBlockers, settleDraftsForQuit, type DraftQuitBlocker } from "../state/drafts";
import { errorText } from "./errorText";

async function check(token: string) {
  useQuit.setState({ token, exiting: false, checking: true, blockers: [], error: undefined });
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const blockers = await Promise.race([
      settleDraftsForQuit(),
      new Promise<DraftQuitBlocker[]>((resolve) => {
        timer = setTimeout(() => resolve([{ id: "save-timeout", subject: "Draft storage", reason: "Saving is taking longer than expected. Keep working and try quitting again." }]), 10_000);
      }),
    ]);
    if (useQuit.getState().token !== token) return;
    const remaining = [...new Map([...blockers, ...currentDraftQuitBlockers()].map(item => [item.id,item])).values(), ...quitOperationReasons().map((reason, i) => ({ id: `operation-${i}`, subject: "Work in progress", reason }))];
    useQuit.setState({ blockers: remaining, exiting: remaining.length === 0, checking: remaining.length === 0 });
    const accepted = await invoke<boolean>(remaining.length ? "quit_blocked" : "quit_ready", { token });
    if (accepted === false && useQuit.getState().token === token) useQuit.setState({ token: undefined, exiting: false, checking: false, blockers: [] });
  } catch (error) {
    if (useQuit.getState().token !== token) return;
    useQuit.setState({ exiting: false, checking: false, error: errorText(error) });
    // A lost IPC reply never grants permission to exit. The native gate stays armed.
    await invoke("quit_blocked", { token }).catch(() => {});
  } finally { clearTimeout(timer); }
}

export function watchQuit(): () => void {
  if (!isTauri) return () => {};
  let stop: (() => void) | undefined;
  let stopped = false;
  void listen<string>("moss://quit-requested", ({ payload }) => {
    if (!stopped) void check(payload);
  }).then((unlisten) => {
    if (stopped) unlisten(); else stop = unlisten;
  }).catch((error: unknown) => { console.warn("Moss quit listener:", errorText(error)); });
  return () => { stopped = true;stop?.();void keepWorking(); };
}

export async function keepWorking() {
  if (useQuit.getState().exiting) return;
  const token = useQuit.getState().token;
  useQuit.setState({ token: undefined, exiting: false, checking: false, blockers: [], error: undefined });
  if (token) await invoke("quit_cancel", { token }).catch((error: unknown) => console.warn("Moss quit cancellation:", errorText(error)));
}

export async function quitAnyway() {
  const token = useQuit.getState().token;
  if (!token) return;
  useQuit.setState({ exiting: true, checking: true, error: undefined });
  try {
    const accepted = await invoke<boolean>("quit_ready", { token });
    if (accepted === false && useQuit.getState().token === token) useQuit.setState({ token: undefined, exiting: false, checking: false, blockers: [] });
  }
  catch (error) {
    if (useQuit.getState().token === token) useQuit.setState({ exiting: false, checking: false, error: errorText(error) });
  }
}
