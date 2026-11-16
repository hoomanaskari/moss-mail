import { fireEvent, renderHook } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { resetStore } from "../test/fixtures";
import { digitFroms, pressDigit } from "../test/keys";
import { useStore } from "./store";
import { useScreenKeys, type KeyInfo } from "./useScreenKeys";

beforeEach(() => resetStore([]));

it("passes each key with its modifiers, and stops the default when handled", async () => {
  const seen: KeyInfo[] = [];
  const { unmount } = renderHook(() => useScreenKeys((k) => (seen.push(k), k.key === "j")));
  const prevented: boolean[] = [];
  const spy = (e: KeyboardEvent) => prevented.push(e.defaultPrevented);
  window.addEventListener("keydown", spy);
  await userEvent.setup().keyboard("j{Meta>}1{/Meta}{Alt>}a{/Alt}");
  // A Mac types "å" for ⌥A.
  fireEvent.keyDown(window, { key: "å", code: "KeyA", altKey: true });
  window.removeEventListener("keydown", spy);
  expect(seen.map((k) => [k.key, k.mod, k.alt, k.shift])).toEqual([
    ["j", false, false, false],
    ["meta", true, false, false],
    ["1", true, false, false],
    ["alt", false, true, false],
    // ⌥ changes e.key on a Mac, so letters come from e.code.
    ["a", false, true, false],
    ["a", false, true, false],
  ]);
  expect(prevented[0]).toBe(true);
  expect(prevented.at(-1)).toBe(false);
  unmount();
});

it("waits while the Move or Remind menu is open, which handle their own keys", async () => {
  const onKey = vi.fn();
  renderHook(() => useScreenKeys(onKey));
  useStore.setState({ movingId: "a" });
  await userEvent.setup().keyboard("e");
  useStore.setState({ movingId: undefined, remindingId: "a" });
  await userEvent.setup().keyboard("e");
  expect(onKey).not.toHaveBeenCalled();
  useStore.setState({ remindingId: undefined });
  await userEvent.setup().keyboard("e");
  expect(onKey).toHaveBeenCalledTimes(1);
});

it("knows when you are typing in a field", async () => {
  const onKey = vi.fn();
  renderHook(() => useScreenKeys(onKey));
  const input = document.body.appendChild(document.createElement("input"));
  await userEvent.setup().type(input, "x");
  expect(onKey.mock.calls[0][0]).toMatchObject({ key: "x", typing: true });
  input.remove();
});

it("leaves keys to the command palette while it is open", async () => {
  const onKey = vi.fn();
  renderHook(() => useScreenKeys(onKey));
  useStore.setState({ paletteOpen: true });
  await userEvent.setup().keyboard("j");
  expect(onKey).not.toHaveBeenCalled();
});

it.each(digitFroms)("gives the digit of a %s key, with or without ⌘ and ⌥", (from) => {
  const seen: KeyInfo[] = [];
  const { unmount } = renderHook(() => useScreenKeys((k) => void seen.push(k)));
  pressDigit(3, from);
  pressDigit(3, from, { meta: true });
  pressDigit(3, from, { alt: true });
  fireEvent.keyDown(window, { key: "a", code: "KeyA" });
  expect(seen.map((k) => k.digit)).toEqual([3, 3, 3, undefined]);
  unmount();
});

it("does not dispatch screen actions while the quit modal owns input", async () => {
  const { useQuit } = await import("./quitState");const action=vi.fn();renderHook(()=>useScreenKeys(action));
  useQuit.setState({token:"quit"});fireEvent.keyDown(window,{key:"Delete"});
  expect(action).not.toHaveBeenCalled();
});
