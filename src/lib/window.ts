import { isTauri } from "./platform";

/** Closes the app window in the desktop app. In a browser tab there is nothing to close. */
export async function closeWindow() {
  if (!isTauri) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().close();
}
