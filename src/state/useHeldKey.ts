import { useEffect, useState } from "react";

/** How long a key must be held alone before it counts, so quick shortcuts like ⌘C don't flash anything. */
export const holdDelay = 300;

/**
 * True while `key` (a KeyboardEvent.key name, e.g. "Meta") is held down on its own for a moment.
 * Once on, it stays on while other keys are pressed with it, and goes off when it is let go.
 */
export function useHeldKey(key: string) {
  const [held, setHeld] = useState(false);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const release = () => {
      clearTimeout(timer);
      setHeld(false);
    };
    const onDown = (e: KeyboardEvent) => {
      if (e.key === key) {
        if (!e.repeat) timer = setTimeout(() => setHeld(true), holdDelay);
      } else if (!e.getModifierState(key)) {
        // The key-up can go missing, e.g. when the window lost focus mid-press.
        release();
      } else {
        // Another key with it, straight away: that was a shortcut, not a hold.
        clearTimeout(timer);
      }
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.key === key) release();
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    window.addEventListener("blur", release);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
      window.removeEventListener("blur", release);
    };
  }, [key]);

  return held;
}
