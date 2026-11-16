/** True when running inside the Tauri desktop shell (not a plain browser tab). */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

/** The key that ⌘ shortcuts use here, as KeyboardEvent.key names it: ⌘ on a Mac, Ctrl elsewhere. */
export const modKey = isMac ? "Meta" : "Control";

/** What to call this machine in copy like "AI runs on this Mac". */
export const deviceName = isMac ? "Mac" : "computer";

/**
 * Marks <html> with who draws the window controls, so the top bar can leave the right
 * amount of room: "mac" (real traffic lights) or "native" (Windows/Linux title bar).
 */
export function markNativeTitleBar() {
  if (isTauri) document.documentElement.dataset.titlebar = isMac ? "mac" : "native";
}
