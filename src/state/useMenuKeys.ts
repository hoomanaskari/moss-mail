import { listenAppKeys } from "./appKeyboard";
import { useEffect, useRef, useState } from "react";
import { useStore } from "./store";

/**
 * ↑ ↓ and ↵ for a menu while it is open: ↑ ↓ mark the next or previous item (going round the
 * ends) and ↵ picks the marked one. The menu hears them before the app does, so an open menu owns
 * ↑ ↓ ↵ and they never also move to another email. ⌘ and ⌥ arrows go on to the app, and the
 * command palette keeps its keys while it is open.
 *
 * Only these keys move the mark: hovering never does, so the mouse can't move what ↵ picks.
 * Call it from the component that is only there while the menu is open. Returns the marked item.
 */
export function useMenuKeys(count: number, onPick: (index: number) => void, start = 0) {
  const [active, setActive] = useState(start);
  const latest = useRef({ count, onPick, active });
  latest.current = { count, onPick, active };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (useStore.getState().paletteOpen || e.metaKey || e.ctrlKey || e.altKey) return;
      const { count, onPick, active } = latest.current;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        const step = e.key === "ArrowDown" ? 1 : count - 1;
        if (count) setActive((a) => (a + step) % count);
      } else if (e.key === "Enter") {
        if (count) onPick(active);
      } else return;
      e.preventDefault();
      e.stopPropagation();
    };
    // Capture on window runs before the app's shortcuts, which listen as the key bubbles up.
    const stopKeys = listenAppKeys(onKey, true);
    return () => stopKeys();
  }, []);

  return active;
}
