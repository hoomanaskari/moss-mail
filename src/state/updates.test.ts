import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { installedMoss, mossDev, useAppIdentity } from "../services/appIdentity";
import { demoUpdates, setUpdates, type UpdatesBackend } from "../services/updates";
import { resetStore } from "../test/fixtures";
import { useDrafts } from "./drafts";
import { useStore } from "./store";
import { checkedText, pristineUpdates, startUpdates, useUpdates } from "./updates";

beforeEach(() => {
  resetStore([]);
  useUpdates.setState(pristineUpdates);
});
afterEach(() => {
  vi.useRealTimers();
  setUpdates(demoUpdates());
  useAppIdentity.setState(installedMoss);
});

/** A backend with 0.2.0 waiting, whose calls can be watched and made to fail. */
function backend(over: Partial<UpdatesBackend> = {}): UpdatesBackend {
  return { ...demoUpdates({ next: "0.2.0", notes: "Faster search" }), ...over };
}

const state = () => useUpdates.getState();

it("loads Moss's version", async () => {
  setUpdates(demoUpdates({ version: "0.3.1" }));
  await state().load();
  expect(state().version).toBe("0.3.1");
});

it("a quiet check finds a newer Moss and says so, without downloading it", async () => {
  const install = vi.fn(() => Promise.resolve());
  setUpdates(backend({ install }));
  vi.useFakeTimers({ now: new Date(2026, 10, 16, 10, 42) });
  await state().check();
  expect(state().update).toEqual({ state: "available", version: "0.2.0", notes: "Faster search" });
  expect(state().checkedAt).toBe(new Date(2026, 10, 16, 10, 42).getTime());
  expect(state().open).toBe(false);
  expect(install).not.toHaveBeenCalled();
});

it("a quiet check with nothing new stays quiet, and remembers when it checked", async () => {
  setUpdates(demoUpdates());
  await state().check();
  expect(state().update).toEqual({ state: "idle" });
  expect(state().checkedAt).toBeGreaterThan(0);
});

it("a check you asked for always answers: checking, then up to date", async () => {
  const seen: unknown[] = [];
  const stop = useUpdates.subscribe((s) => seen.push(s.update));
  setUpdates(demoUpdates());
  await state().check({ asked: true });
  stop();
  expect(seen).toEqual([{ state: "checking" }, { state: "current" }]);
});

it("a check you asked for says why it failed, and keeps saying it", async () => {
  vi.useFakeTimers();
  setUpdates(backend({ check: () => Promise.reject(new Error("The network connection was lost.")) }));
  await state().check({ asked: true });
  expect(state().update).toEqual({ state: "failed", during: "check", message: "The network connection was lost." });
  vi.advanceTimersByTime(60_000);
  expect(state().update.state).toBe("failed");
  expect(state().checkedAt).toBeUndefined();
});

it("a quiet check that fails leaves things as they were", async () => {
  setUpdates(backend({ check: () => Promise.reject(new Error("offline")) }));
  await state().check();
  expect(state().update).toEqual({ state: "idle" });
  useUpdates.setState({ update: { state: "available", version: "0.2.0" } });
  await state().check();
  expect(state().update).toEqual({ state: "available", version: "0.2.0" });
});

it("a check from the menu or the palette opens the update window, and closing it keeps the result", async () => {
  setUpdates(backend());
  await state().check({ asked: true, open: true });
  expect(state().open).toBe(true);
  state().close();
  expect(state().open).toBe(false);
  expect(state().update.state).toBe("available");
});

it("doesn't check again while checking, downloading or ready, but still opens the window", async () => {
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check }));
  for (const update of [{ state: "checking" }, { state: "downloading", version: "0.2.0", done: 1, total: 2 }, { state: "ready", version: "0.2.0" }] as const) {
    useUpdates.setState({ update, open: false });
    await state().check({ asked: true, open: true });
    expect(state().update).toEqual(update);
    expect(state().open).toBe(true);
  }
  expect(check).not.toHaveBeenCalled();
});

it("installing shows the download, then restarts into the new Moss", async () => {
  const restart = vi.fn(() => Promise.resolve());
  setUpdates(backend({ restart }));
  await state().check();
  const seen: unknown[] = [];
  const stop = useUpdates.subscribe((s) => seen.push(s.update));
  await state().install();
  stop();
  expect(seen).toEqual([
    { state: "downloading", version: "0.2.0", notes: "Faster search", done: 0, total: 0 },
    { state: "downloading", version: "0.2.0", notes: "Faster search", done: 5, total: 10 },
    { state: "downloading", version: "0.2.0", notes: "Faster search", done: 10, total: 10 },
    { state: "ready", version: "0.2.0" },
  ]);
  expect(restart).toHaveBeenCalledOnce();
});

it("waits for Restart now while you're writing", async () => {
  const writing: [string, () => void][] = [
    ["the composer", () => useStore.setState({ view: "compose" })],
    ["a Quick reply", () => useStore.setState({ replyDraft: "Thanks, see you" })],
    ["edits not yet saved", () => useDrafts.setState({ unsaved: { d1: { id: "d1", subject: "Plans" } as never } })],
  ];
  for (const [what, start] of writing) {
    resetStore([]);
    useDrafts.setState({ unsaved: {} });
    const restart = vi.fn(() => Promise.resolve());
    setUpdates(backend({ restart }));
    useUpdates.setState({ update: { state: "available", version: "0.2.0" } });
    await state().check();
    start();
    await state().install();
    expect(state().update, what).toEqual({ state: "ready", version: "0.2.0" });
    expect(restart, what).not.toHaveBeenCalled();
  }
  useDrafts.setState({ unsaved: {} });
});

it("a failed download says so, keeps the version, and can be tried again", async () => {
  let fails = true;
  const real = backend();
  setUpdates({ ...real, install: (progress) => (fails ? Promise.reject(new Error("The download was cut off.")) : real.install(progress)), restart: () => Promise.resolve() });
  await state().check();
  await state().install();
  expect(state().update).toEqual({ state: "failed", during: "download", message: "The download was cut off.", version: "0.2.0", notes: "Faster search" });
  fails = false;
  await state().install();
  expect(state().update).toEqual({ state: "ready", version: "0.2.0" });
});

it("installing with nothing to install does nothing", async () => {
  const install = vi.fn(() => Promise.resolve());
  setUpdates(backend({ install }));
  for (const update of [{ state: "idle" }, { state: "current" }, { state: "failed", during: "check", message: "offline" }] as const) {
    useUpdates.setState({ update });
    await state().install();
    expect(state().update).toEqual(update);
  }
  expect(install).not.toHaveBeenCalled();
});

it("restarts into the new Moss", async () => {
  const restart = vi.fn(() => Promise.resolve());
  setUpdates(backend({ restart }));
  await state().restart();
  expect(restart).toHaveBeenCalledOnce();
});

it("loads the version at once, checks a little after launch, then every few hours until stopped", async () => {
  vi.useFakeTimers();
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check, version: () => Promise.resolve("0.1.0") }));
  const stop = startUpdates({ delay: 500, every: 1000 });
  await vi.advanceTimersByTimeAsync(0);
  expect(state().version).toBe("0.1.0");
  expect(check).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(500);
  expect(check).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1000);
  expect(check).toHaveBeenCalledTimes(2);
  stop();
  await vi.advanceTimersByTimeAsync(5000);
  expect(check).toHaveBeenCalledTimes(2);
});

it("by default checks ten seconds after launch and every six hours", async () => {
  vi.useFakeTimers();
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check }));
  const stop = startUpdates();
  await vi.advanceTimersByTimeAsync(9_999);
  expect(check).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(check).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(6 * 60 * 60 * 1000);
  expect(check).toHaveBeenCalledTimes(2);
  stop();
});

it("checks when the window comes back after a long time away, as after the Mac slept", async () => {
  vi.useFakeTimers();
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check }));
  const stop = startUpdates({ delay: 0, every: 1000 });
  await vi.advanceTimersByTimeAsync(0);
  expect(check).toHaveBeenCalledTimes(1);
  window.dispatchEvent(new Event("focus"));
  await vi.advanceTimersByTimeAsync(0);
  expect(check).toHaveBeenCalledTimes(1);
  // The clock moved on while timers didn't run.
  vi.setSystemTime(Date.now() + 5000);
  window.dispatchEvent(new Event("focus"));
  await vi.advanceTimersByTimeAsync(0);
  expect(check).toHaveBeenCalledTimes(2);
  stop();
  vi.setSystemTime(Date.now() + 5000);
  window.dispatchEvent(new Event("focus"));
  await vi.advanceTimersByTimeAsync(0);
  expect(check).toHaveBeenCalledTimes(2);
});

it("never checks in Moss Dev", async () => {
  vi.useFakeTimers();
  useAppIdentity.setState(mossDev);
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check, version: () => Promise.resolve("0.1.0") }));
  const stop = startUpdates({ delay: 0, every: 1000 });
  await vi.advanceTimersByTimeAsync(10_000);
  vi.setSystemTime(Date.now() + 5000);
  window.dispatchEvent(new Event("focus"));
  await state().check({ asked: true, open: true });
  expect(check).not.toHaveBeenCalled();
  expect(state().update).toEqual({ state: "idle" });
  expect(state().version).toBe("0.1.0");
  stop();
});

it("says when it last checked, in a few words", () => {
  const now = new Date(2026, 10, 16, 15, 30).getTime();
  expect(checkedText(undefined, now)).toBe("Not checked yet");
  expect(checkedText(now - 20_000, now)).toBe("Last checked just now");
  expect(checkedText(now - 5 * 60_000, now)).toBe("Last checked 5 min ago");
  expect(checkedText(new Date(2026, 10, 16, 9, 5).getTime(), now)).toBe("Last checked today at 09:05");
  expect(checkedText(new Date(2026, 10, 14, 18, 0).getTime(), now)).toBe("Last checked on 14 Nov at 18:00");
});
