import { useEffect } from "react";
import { useStore, type ReadingSize, type Theme, type ThemeChoice } from "./store";

const KEY = "moss.preferences";
const LIGHT_QUERY = "(prefers-color-scheme: light)";

/** True when the URL picked the theme or reading size (dev scenes), so it is not saved as the user's choice. */
let fromUrl = false;

export function systemTheme(): Theme {
  return typeof window !== "undefined" && window.matchMedia?.(LIGHT_QUERY).matches ? "light" : "dark";
}

const readingSizes: readonly unknown[] = ["smaller", "default", "larger"] satisfies ReadingSize[];

/** Saved theme choice and reading size, with the theme resolved against the system setting. Read before first render to avoid a flash. */
export function initialPreferences(): { themeChoice: ThemeChoice; theme: Theme; readingSize: ReadingSize } {
  // Older builds saved a resolved `theme`, which pinned whatever the system was on first launch, and a `font`
  // (Figtree or Atkinson Hyperlegible) from before Figtree became the only font. Both are ignored.
  let saved: { themeChoice?: ThemeChoice; readingSize?: ReadingSize } = {};
  try {
    saved = JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    // Unreadable or unavailable storage: use defaults.
  }
  if (import.meta.env.DEV) {
    // Dev scenes pick the theme (?theme=light) and reading size (?reading=larger) from the URL.
    const params = new URLSearchParams(location.search);
    const forced = params.get("theme");
    if (forced === "light" || forced === "dark") {
      saved.themeChoice = forced;
      fromUrl = true;
    }
    const reading = params.get("reading");
    if (readingSizes.includes(reading)) {
      saved.readingSize = reading as ReadingSize;
      fromUrl = true;
    }
  }
  const themeChoice = saved.themeChoice ?? "system";
  return {
    themeChoice,
    theme: themeChoice === "system" ? systemTheme() : themeChoice,
    readingSize: readingSizes.includes(saved.readingSize) ? saved.readingSize! : "default",
  };
}

/** Applies theme and reading size to <html>, follows the system theme when chosen, and remembers the choice on this device. */
export function usePreferences() {
  const themeChoice = useStore((s) => s.themeChoice);
  const theme = useStore((s) => s.theme);
  const readingSize = useStore((s) => s.readingSize);

  useEffect(() => {
    if (themeChoice !== "system") return;
    const media = window.matchMedia?.(LIGHT_QUERY);
    if (!media) return;
    const sync = () => useStore.setState({ theme: media.matches ? "light" : "dark" });
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, [themeChoice]);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = theme;
    root.dataset.reading = readingSize;
  }, [theme, readingSize]);

  useEffect(() => {
    if (fromUrl) return;
    try {
      localStorage.setItem(KEY, JSON.stringify({ themeChoice, readingSize }));
    } catch {
      // Storage can be unavailable; preferences then last for this session only.
    }
  }, [themeChoice, readingSize]);
}
