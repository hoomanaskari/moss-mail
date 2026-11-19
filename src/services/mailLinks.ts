/**
 * Mail links (`mailto:`) other apps send Moss once it is your default email app. Rust keeps each
 * one until the window takes it (src-tauri/src/mail_links.rs), so a link that started Moss waits
 * for the mail to load.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useStore } from "../state/store";

/** What Rust sends when a link arrives. */
export const mailLinkEvent = "moss://mail-link";

/** Opens a new email for each link waiting in Rust. */
async function openWaiting() {
  for (const url of await invoke<string[]>("take_mail_links")) useStore.getState().composeFromLink(url);
}

/** Opens the links waiting now and each one that arrives later. Returns a stop. */
export async function watchMailLinks(): Promise<() => void> {
  const stop = await listen(mailLinkEvent, () => void openWaiting());
  await openWaiting();
  return stop;
}
