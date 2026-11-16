/**
 * Moss's icon has a light and a dark look, and the Mac switches between them itself (with Light
 * or Dark mode, or as its icon style in Settings → Appearance says). When Moss is set to Light
 * or Dark, the Dock shows that look instead while Moss runs.
 */
import type { Theme, ThemeChoice } from "../state/store";
import { isTauri } from "./platform";

/** The Dock icon: the Mac's own choice, or one look whatever the Mac shows. */
export type AppIcon = "system" | Theme;

/**
 * Which icon to show for Moss's theme choice. A fixed theme always shows its own look, even when
 * the Mac's mode matches it: the Mac's icon style can keep icons dark in Light mode.
 */
export function appIcon(choice: ThemeChoice): AppIcon {
  return choice;
}

/** Shows `icon` in the Dock, in the desktop app. If it can't, the icon stays as it was. */
export async function setAppIcon(icon: AppIcon) {
  if (!isTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_app_icon", { icon }).catch(() => {});
}
