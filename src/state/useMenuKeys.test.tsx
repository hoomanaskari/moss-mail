import { fireEvent, renderHook } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { resetStore } from "../test/fixtures";
import { useStore } from "./store";
import { useMenuKeys } from "./useMenuKeys";

/** What the app's own keys (listening after the menu) still hear. */
const heard: string[] = [];
const app = (e: KeyboardEvent) => heard.push(e.key);

beforeEach(() => {
  resetStore([]);
  heard.length = 0;
  window.addEventListener("keydown", app);
});
afterEach(() => window.removeEventListener("keydown", app));

it("↓ ↑ move through the items and go round the ends; ↵ picks the marked one", async () => {
  const onPick = vi.fn();
  const { result } = renderHook(() => useMenuKeys(3, onPick));
  expect(result.current).toBe(0);
  const user = userEvent.setup();
  await user.keyboard("{ArrowUp}");
  expect(result.current).toBe(2);
  await user.keyboard("{ArrowDown}");
  expect(result.current).toBe(0);
  await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
  expect(onPick).toHaveBeenCalledWith(2);
});

it("starts on the item you give it, and only the keys move it", async () => {
  const onPick = vi.fn();
  const { result } = renderHook(() => useMenuKeys(4, onPick, 2));
  expect(result.current).toBe(2);
  await userEvent.setup().keyboard("{Enter}");
  expect(onPick).toHaveBeenCalledWith(2);
});

it("keeps ↑ ↓ ↵ from the rest of the app, and stops their default", async () => {
  renderHook(() => useMenuKeys(2, vi.fn()));
  const prevented: boolean[] = [];
  const spy = (e: KeyboardEvent) => prevented.push(e.defaultPrevented);
  window.addEventListener("keydown", spy, true);
  // ⇧↓ too: the inbox reads it as ↓.
  await userEvent.setup().keyboard("{ArrowDown}{ArrowUp}{Enter}{Shift>}{ArrowDown}{/Shift}x");
  window.removeEventListener("keydown", spy, true);
  expect(heard).toEqual(["Shift", "x"]);
  expect(prevented).toEqual([true, true, true, false, true, false]);
});

it("leaves ⌘ and ⌥ arrows, and every key while the command palette is open, to their owners", async () => {
  const onPick = vi.fn();
  const { result } = renderHook(() => useMenuKeys(3, onPick));
  const user = userEvent.setup();
  await user.keyboard("{Meta>}{ArrowDown}{/Meta}{Alt>}{ArrowDown}{/Alt}");
  expect(result.current).toBe(0);
  useStore.setState({ paletteOpen: true });
  await user.keyboard("{ArrowDown}{Enter}");
  expect(result.current).toBe(0);
  expect(onPick).not.toHaveBeenCalled();
  expect(heard).toEqual(["Meta", "ArrowDown", "Alt", "ArrowDown", "ArrowDown", "Enter"]);
});

it("does nothing with an empty menu but still keeps the keys, and lets go when closed", () => {
  const onPick = vi.fn();
  const { result, unmount } = renderHook(() => useMenuKeys(0, onPick));
  fireEvent.keyDown(document.body, { key: "ArrowDown" });
  fireEvent.keyDown(document.body, { key: "Enter" });
  expect(result.current).toBe(0);
  expect(onPick).not.toHaveBeenCalled();
  expect(heard).toEqual([]);
  unmount();
  fireEvent.keyDown(document.body, { key: "ArrowDown" });
  expect(heard).toEqual(["ArrowDown"]);
});

it("uses the newest handler and count", async () => {
  const first = vi.fn();
  const second = vi.fn();
  const { result, rerender } = renderHook(({ n, pick }) => useMenuKeys(n, pick), { initialProps: { n: 2, pick: first } });
  rerender({ n: 4, pick: second });
  await userEvent.setup().keyboard("{ArrowUp}{Enter}");
  expect(result.current).toBe(3);
  expect(second).toHaveBeenCalledWith(3);
  expect(first).not.toHaveBeenCalled();
});

it("leaves Enter and arrows to the native quit modal even when a menu is open", async () => {
  const { useQuit } = await import("../services/quit");
  const onPick = vi.fn();const { result } = renderHook(() => useMenuKeys(3, onPick));
  useQuit.setState({ token: "quit" });
  try {
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(onPick).not.toHaveBeenCalled();expect(result.current).toBe(0);
    expect(heard).toEqual(["ArrowDown", "Enter"]);
  } finally {useQuit.setState({token:undefined});}
});
