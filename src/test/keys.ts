import { fireEvent } from "@testing-library/react";

/** Where a digit is typed from: the number row or the numpad. */
export type DigitFrom = "Digit" | "Numpad";
export const digitFroms: DigitFrom[] = ["Digit", "Numpad"];

/** What a Mac types for ⌥0–⌥9 on the number row. */
const optionDigits = "º¡™£¢∞§¶•ª";

interface Mods {
  meta?: boolean;
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
}

/** The keydown a Mac sends for a digit: ⌥ changes e.key on the number row, not on the numpad. */
export function digitEvent(n: number, from: DigitFrom, mods: Mods = {}) {
  return {
    key: from === "Digit" && mods.alt ? optionDigits[n] : String(n),
    code: `${from}${n}`,
    metaKey: !!mods.meta,
    ctrlKey: !!mods.ctrl,
    altKey: !!mods.alt,
    shiftKey: !!mods.shift,
  };
}

/** Presses a digit on `target` (the window by default). False when a handler took it. */
export function pressDigit(n: number, from: DigitFrom, mods: Mods = {}, target: Element | Window = window) {
  return fireEvent.keyDown(target, digitEvent(n, from, mods));
}
