import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { demoUpdates, setUpdates, type UpdatesBackend } from "../services/updates";
import { pristineUpdates, startUpdates, useUpdates } from "./updates";

beforeEach(() => useUpdates.setState(pristineUpdates));
afterEach(() => {
  vi.useRealTimers();
  setUpdates(demoUpdates());
});

/** A backend whose calls can be watched and made to fail. */
function backend(over: Partial<UpdatesBackend> = {}): UpdatesBackend {
  return { ...demoUpdates({ next: "0.2.0" }), ...over };
}

it("loads Moss's version", async () => {
  setUpdates(demoUpdates({ version: "0.3.1" }));
  await useUpdates.getState().load();
  expect(useUpdates.getState().version).toBe("0.3.1");
});

it("a check finds a newer Moss, downloads it and is ready to restart", async () => {
  setUpdates(backend());
  await useUpdates.getState().check();
  expect(useUpdates.getState().update).toEqual({ state: "ready", version: "0.2.0" });
});

it("shows the download as it comes in", async () => {
  const seen: unknown[] = [];
  const stop = useUpdates.subscribe((s) => seen.push(s.update));
  setUpdates(backend());
  await useUpdates.getState().check(true);
  stop();
  expect(seen).toEqual([
    { state: "checking" },
    { state: "downloading", version: "0.2.0", done: 0, total: 0 },
    { state: "downloading", version: "0.2.0", done: 5, total: 10 },
    { state: "downloading", version: "0.2.0", done: 10, total: 10 },
    { state: "ready", version: "0.2.0" },
  ]);
});

it("a check you didn't ask for says nothing when Moss is up to date", async () => {
  setUpdates(demoUpdates());
  await useUpdates.getState().check();
  expect(useUpdates.getState().update).toEqual({ state: "idle" });
});

it("a check you asked for says Moss is up to date, then goes quiet", async () => {
  vi.useFakeTimers();
  setUpdates(demoUpdates());
  await useUpdates.getState().check(true);
  expect(useUpdates.getState().update).toEqual({ state: "current" });
  vi.advanceTimersByTime(5000);
  expect(useUpdates.getState().update).toEqual({ state: "idle" });
});

it("a check you asked for says why it failed, then goes quiet", async () => {
  vi.useFakeTimers();
  setUpdates(backend({ check: () => Promise.reject(new Error("offline")) }));
  await useUpdates.getState().check(true);
  expect(useUpdates.getState().update).toEqual({ state: "failed", message: "Couldn’t check for updates. Try again later." });
  vi.advanceTimersByTime(5000);
  expect(useUpdates.getState().update).toEqual({ state: "idle" });
});

it("a quiet check that fails stays quiet", async () => {
  setUpdates(backend({ check: () => Promise.reject(new Error("offline")) }));
  await useUpdates.getState().check();
  expect(useUpdates.getState().update).toEqual({ state: "idle" });
});

it("a download that fails goes quiet, so the next check tries again", async () => {
  setUpdates(backend({ install: () => Promise.reject(new Error("cut off")) }));
  await useUpdates.getState().check();
  expect(useUpdates.getState().update).toEqual({ state: "idle" });
});

it("a quiet timer doesn't hide a newer result", async () => {
  vi.useFakeTimers();
  setUpdates(demoUpdates());
  await useUpdates.getState().check(true);
  setUpdates(backend());
  await useUpdates.getState().check();
  vi.advanceTimersByTime(5000);
  expect(useUpdates.getState().update).toEqual({ state: "ready", version: "0.2.0" });
});

it("doesn't check again while one is downloading or ready", async () => {
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check }));
  useUpdates.setState({ update: { state: "ready", version: "0.2.0" } });
  await useUpdates.getState().check(true);
  useUpdates.setState({ update: { state: "downloading", version: "0.2.0", done: 1, total: 2 } });
  await useUpdates.getState().check(true);
  expect(check).not.toHaveBeenCalled();
});

it("restarts into the new Moss", async () => {
  const restart = vi.fn(() => Promise.resolve());
  setUpdates(backend({ restart }));
  await useUpdates.getState().restart();
  expect(restart).toHaveBeenCalledOnce();
});

it("starts by loading the version and checking, then checks every few hours until stopped", async () => {
  vi.useFakeTimers();
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check, version: () => Promise.resolve("0.1.0") }));
  const stop = startUpdates(1000);
  await vi.advanceTimersByTimeAsync(0);
  expect(useUpdates.getState().version).toBe("0.1.0");
  expect(check).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1000);
  expect(check).toHaveBeenCalledTimes(2);
  stop();
  await vi.advanceTimersByTimeAsync(5000);
  expect(check).toHaveBeenCalledTimes(2);
});

it("checks every six hours by default", async () => {
  vi.useFakeTimers();
  const check = vi.fn(() => Promise.resolve(null));
  setUpdates(backend({ check }));
  const stop = startUpdates();
  await vi.advanceTimersByTimeAsync(6 * 60 * 60 * 1000);
  expect(check).toHaveBeenCalledTimes(2);
  stop();
});
