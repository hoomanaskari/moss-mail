import { describe, expect, it } from "vitest";
import { keyDigit } from "./keyDigit";

describe("keyDigit", () => {
  it("reads 0–9 from the number row and the numpad", () => {
    for (let n = 0; n <= 9; n++) {
      expect(keyDigit({ key: String(n), code: `Digit${n}` })).toBe(n);
      expect(keyDigit({ key: String(n), code: `Numpad${n}` })).toBe(n);
    }
  });

  it("reads the physical key when ⌥ changes e.key, as on a Mac", () => {
    expect(keyDigit({ key: "£", code: "Digit3" })).toBe(3);
    expect(keyDigit({ key: "º", code: "Digit0" })).toBe(0);
    expect(keyDigit({ key: "#", code: "Digit3" })).toBe(3);
  });

  it("falls back to e.key when there is no digit code (another layout, or a synthetic event)", () => {
    expect(keyDigit({ key: "7", code: "" })).toBe(7);
    expect(keyDigit({ key: "7", code: "KeyQ" })).toBe(7);
  });

  it("is undefined for every other key", () => {
    expect(keyDigit({ key: "a", code: "KeyA" })).toBeUndefined();
    expect(keyDigit({ key: "+", code: "NumpadAdd" })).toBeUndefined();
    expect(keyDigit({ key: "Enter", code: "NumpadEnter" })).toBeUndefined();
    expect(keyDigit({ key: "F3", code: "F3" })).toBeUndefined();
    expect(keyDigit({ key: "12", code: "" })).toBeUndefined();
  });
});
