import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetStore } from "../test/fixtures";
import { useStore } from "./store";
import { initialPreferences, systemTheme, usePreferences } from "./usePreferences";

const KEY = "moss.preferences";

/** A fake system light/dark setting that can be flipped mid-test. */
function stubSystemTheme(light: boolean) {
  const listeners = new Set<() => void>();
  const media = {
    matches: light,
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
  };
  vi.stubGlobal("matchMedia", () => media);
  return {
    listeners,
    set(next: boolean) {
      media.matches = next;
      listeners.forEach((fn) => fn());
    },
  };
}

beforeEach(() => {
  localStorage.clear();
  resetStore([]);
});
afterEach(() => {
  vi.unstubAllGlobals();
  history.replaceState(null, "", "/");
});

describe("systemTheme", () => {
  it("reads the system setting, dark when unknown", () => {
    stubSystemTheme(true);
    expect(systemTheme()).toBe("light");
    vi.stubGlobal("matchMedia", undefined);
    expect(systemTheme()).toBe("dark");
  });
});

describe("initialPreferences", () => {
  it("follows the system at the default reading size on first launch", () => {
    stubSystemTheme(true);
    expect(initialPreferences()).toEqual({ themeChoice: "system", theme: "light", readingSize: "default" });
  });

  it("restores a saved choice", () => {
    localStorage.setItem(KEY, JSON.stringify({ themeChoice: "dark", readingSize: "larger" }));
    expect(initialPreferences()).toEqual({ themeChoice: "dark", theme: "dark", readingSize: "larger" });
  });

  // Older builds offered Atkinson Hyperlegible; Figtree is now the only font, so the saved choice is dropped.
  it("ignores a font saved by an older build", () => {
    localStorage.setItem(KEY, JSON.stringify({ themeChoice: "dark", font: "atkinson", readingSize: "smaller" }));
    expect(initialPreferences()).toEqual({ themeChoice: "dark", theme: "dark", readingSize: "smaller" });
  });

  it("falls back to the default size when the saved one isn't a size", () => {
    localStorage.setItem(KEY, JSON.stringify({ readingSize: "huge" }));
    expect(initialPreferences()).toMatchObject({ readingSize: "default" });
  });

  // Older builds saved the resolved theme, which pinned the first-launch theme forever.
  it("ignores the old saved `theme` and keeps following the system", () => {
    stubSystemTheme(true);
    localStorage.setItem(KEY, JSON.stringify({ theme: "dark" }));
    expect(initialPreferences()).toMatchObject({ themeChoice: "system", theme: "light" });
  });

  it("falls back to defaults when storage is unreadable", () => {
    localStorage.setItem(KEY, "{not json");
    expect(initialPreferences()).toMatchObject({ themeChoice: "system", readingSize: "default" });
  });

  it("lets a dev scene pick the theme from the URL without saving it", async () => {
    history.replaceState(null, "", "/?theme=light");
    vi.resetModules();
    // The store must load first: it reads initialPreferences() while it is created.
    const { useStore: freshStore } = await import("./store");
    const fresh = await import("./usePreferences");
    expect(fresh.initialPreferences()).toMatchObject({ themeChoice: "light", theme: "light" });
    freshStore.setState({ themeChoice: "light", theme: "light" });
    renderHook(() => fresh.usePreferences());
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it("lets a dev scene pick the reading size from the URL without saving it", async () => {
    history.replaceState(null, "", "/?scene=conversation&reading=larger");
    vi.resetModules();
    const { useStore: freshStore } = await import("./store");
    const fresh = await import("./usePreferences");
    expect(fresh.initialPreferences()).toMatchObject({ readingSize: "larger" });
    expect(freshStore.getState().readingSize).toBe("larger");
    renderHook(() => fresh.usePreferences());
    expect(document.documentElement.dataset.reading).toBe("larger");
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it("ignores a URL reading size that isn't one", () => {
    history.replaceState(null, "", "/?reading=huge");
    expect(initialPreferences()).toMatchObject({ readingSize: "default" });
  });

  it("ignores the URL theme and reading size outside dev builds", () => {
    stubSystemTheme(false);
    history.replaceState(null, "", "/?theme=light&reading=larger");
    vi.stubEnv("DEV", false);
    try {
      expect(initialPreferences()).toMatchObject({ themeChoice: "system", theme: "dark", readingSize: "default" });
    } finally {
      vi.unstubAllEnvs();
    }
  });
});

describe("usePreferences", () => {
  it("puts the theme and reading size on <html> and remembers the choice", () => {
    useStore.setState({ themeChoice: "light", theme: "light", readingSize: "smaller" });
    renderHook(() => usePreferences());
    expect(document.documentElement.dataset).toMatchObject({ theme: "light", reading: "smaller" });
    expect(document.documentElement.dataset.font).toBeUndefined();
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual({ themeChoice: "light", readingSize: "smaller" });
  });

  it("follows the system theme as it changes, and stops once a theme is pinned", () => {
    const system = stubSystemTheme(false);
    renderHook(() => usePreferences());
    expect(useStore.getState().theme).toBe("dark");
    act(() => system.set(true));
    expect(useStore.getState().theme).toBe("light");
    act(() => useStore.getState().setTheme("dark"));
    expect(system.listeners.size).toBe(0);
    act(() => system.set(true));
    expect(useStore.getState().theme).toBe("dark");
  });

  it("copes without matchMedia", () => {
    vi.stubGlobal("matchMedia", undefined);
    expect(() => renderHook(() => usePreferences())).not.toThrow();
  });

  it("keeps working when storage is full or blocked", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("QuotaExceeded");
    });
    expect(() => renderHook(() => usePreferences())).not.toThrow();
    vi.restoreAllMocks();
  });
});
