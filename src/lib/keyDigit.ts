/**
 * The digit a key stands for, from the number row or the numpad, or undefined for any other key.
 * Reads the physical key first, since ⌥ changes e.key on a Mac (⌥3 types "£") but not e.code.
 */
export function keyDigit(e: Pick<KeyboardEvent, "key" | "code">): number | undefined {
  const digit = /^(?:Digit|Numpad)([0-9])$/.exec(e.code) ?? /^[0-9]$/.exec(e.key);
  return digit ? Number(digit[1] ?? digit[0]) : undefined;
}
