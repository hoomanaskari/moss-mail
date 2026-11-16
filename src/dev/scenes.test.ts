import { afterEach, expect, it, vi } from "vitest";
import { resetDemoStore } from "../test/fixtures";
import { useStore } from "../state/store";
import { applyScene } from "./scenes";

afterEach(() => {
  vi.useRealTimers();
  history.replaceState(null, "", "/");
});

it("the scrolled scene copes when the account tabs aren’t on screen", () => {
  vi.useFakeTimers();
  resetDemoStore();
  history.replaceState(null, "", "/?scene=many-accounts-scrolled");
  applyScene();
  expect(() => vi.advanceTimersByTime(200)).not.toThrow();
  expect(useStore.getState().accounts.length).toBeGreaterThan(3);
});

it("ignores a theme it doesn’t know", () => {
  resetDemoStore();
  history.replaceState(null, "", "/?theme=sepia");
  applyScene();
  expect(useStore.getState().themeChoice).toBe("system");
});

it("moving the laptop email to Important happens once, though dev mode applies a scene twice", () => {
  resetDemoStore();
  history.replaceState(null, "", "/?scene=moved-important");
  applyScene();
  applyScene();
  const s = useStore.getState();
  expect(s.threads.find((t) => t.id === "drive-storage")).toMatchObject({ split: "important", movedByYou: true });
  expect(s.undoSteps.filter((u) => u.kind === "moved")).toHaveLength(1);
});

it("the reminded email in Other is reminded once, though dev mode applies a scene twice", () => {
  resetDemoStore();
  history.replaceState(null, "", "/?scene=other-reminded");
  applyScene();
  applyScene();
  const s = useStore.getState();
  expect(s.split).toBe("other");
  expect(s.threads.find((t) => t.id === "site-visits")?.section).toBe("later");
  expect(s.undoSteps.filter((u) => u.kind === "later")).toHaveLength(1);
});
