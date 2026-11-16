import { act, fireEvent, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { holdDelay, useHeldKey } from "./useHeldKey";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function hold() {
  const view = renderHook(() => useHeldKey("Meta"));
  fireEvent.keyDown(window, { key: "Meta", metaKey: true });
  return view;
}

const wait = (ms: number) => act(() => vi.advanceTimersByTime(ms));

describe("useHeldKey", () => {
  it("is false until the key has been held a moment, then true", () => {
    const { result } = hold();
    expect(result.current).toBe(false);
    wait(holdDelay - 1);
    expect(result.current).toBe(false);
    wait(1);
    expect(result.current).toBe(true);
  });

  it("goes back to false when the key is let go", () => {
    const { result } = hold();
    wait(holdDelay);
    fireEvent.keyUp(window, { key: "Meta" });
    expect(result.current).toBe(false);
  });

  it("a quick tap never turns it on", () => {
    const { result } = hold();
    fireEvent.keyUp(window, { key: "Meta" });
    wait(holdDelay);
    expect(result.current).toBe(false);
  });

  it("a shortcut pressed straight away (⌘C) doesn't turn it on", () => {
    const { result } = hold();
    fireEvent.keyDown(window, { key: "c", metaKey: true });
    wait(holdDelay);
    expect(result.current).toBe(false);
  });

  it("stays on while you press keys with it held, e.g. ⌘1 then ⌘2", () => {
    const { result } = hold();
    wait(holdDelay);
    fireEvent.keyDown(window, { key: "1", metaKey: true });
    fireEvent.keyUp(window, { key: "1", metaKey: true });
    fireEvent.keyDown(window, { key: "2", metaKey: true });
    expect(result.current).toBe(true);
  });

  it("ignores the key repeating while held, so letting go early still keeps it off", () => {
    const { result } = hold();
    wait(holdDelay / 2);
    fireEvent.keyDown(window, { key: "Meta", metaKey: true, repeat: true });
    fireEvent.keyUp(window, { key: "Meta" });
    wait(holdDelay);
    expect(result.current).toBe(false);
  });

  it("other keys alone do nothing", () => {
    const { result } = renderHook(() => useHeldKey("Meta"));
    fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
    wait(holdDelay);
    expect(result.current).toBe(false);
  });

  it("turns off when the window loses focus, since the key-up never arrives (⌘Tab)", () => {
    const { result } = hold();
    wait(holdDelay);
    fireEvent.blur(window);
    expect(result.current).toBe(false);
  });

  it("turns off if a key arrives without the modifier, in case the key-up went missing", () => {
    const { result } = hold();
    wait(holdDelay);
    fireEvent.keyDown(window, { key: "a" });
    expect(result.current).toBe(false);
  });
});
