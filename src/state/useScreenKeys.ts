import { listenAppKeys } from "./appKeyboard";
import { useEffect, useRef } from "react";
import { keyDigit } from "../lib/keyDigit";
import { useStore } from "./store";

export interface KeyInfo {
  /** Lower-case key. Letters pressed with ⌥ come from the physical key, since ⌥ changes e.key on a Mac. */
  key: string;
  /** The digit, from the number row or the numpad, with or without ⌘ or ⌥. Undefined for other keys. */
  digit?: number;
  /** ⌘ on a Mac, Ctrl elsewhere. */
  mod: boolean;
  alt: boolean;
  shift: boolean;
  /** True while the focus is in a text field. */
  typing: boolean;
}

function isTyping(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
}

/**
 * Keys for a screen that handles its own: Add account, Settings and Search.
 * Return true from `onKey` when the key did something, so the browser default is skipped.
 * The command palette keeps its keys while it is open.
 */
export function useScreenKeys(onKey: (k: KeyInfo, e: KeyboardEvent) => boolean | void) {
  const latest = useRef(onKey);
  latest.current = onKey;

  useEffect(() => {
    const listener = (e: KeyboardEvent) => {
      const s = useStore.getState();
      // The palette and the Remind and Move menus keep their keys while they are open.
      if (s.paletteOpen || s.remindingId || s.movingId) return;
      const letter = e.altKey && /^Key[A-Z]$/.test(e.code) ? e.code.slice(3) : e.key;
      const info: KeyInfo = {
        key: letter.toLowerCase(),
        digit: keyDigit(e),
        mod: e.metaKey || e.ctrlKey,
        alt: e.altKey,
        shift: e.shiftKey,
        typing: isTyping(e.target),
      };
      if (latest.current(info, e)) e.preventDefault();
    };
    const stopKeys = listenAppKeys(listener);
    return () => stopKeys();
  }, []);
}
