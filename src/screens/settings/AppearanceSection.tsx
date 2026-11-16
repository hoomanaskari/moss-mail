import { RemoteImagesSettings } from "./RemoteImagesSettings";
import { useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { cx } from "../../lib/cx";
import { Radio } from "../../design-system/atoms";
import { ScreenHeading, SectionHeader, SegmentedControl, ShortcutHint } from "../../design-system/molecules";
import { useStore, type ReadingSize, type Theme, type ThemeChoice } from "../../state/store";
import { useScreenKeys } from "../../state/useScreenKeys";
import shared from "./settings.module.css";
import styles from "./AppearanceSection.module.css";

const themes: { id: ThemeChoice; name: string; sub: string }[] = [
  { id: "system", name: "System", sub: "Follows your Mac" },
  { id: "light", name: "Light", sub: "Warm paper" },
  { id: "dark", name: "Dark", sub: "Easy on the eyes at night" },
];

const sizes: ReadingSize[] = ["smaller", "default", "larger"];
const sizeLabels: Record<ReadingSize, string> = { smaller: "Smaller", default: "Default", larger: "Larger" };

/** Step through a list, wrapping around. */
function step<T>(list: T[], current: T, by: number) {
  return list[(list.indexOf(current) + by + list.length) % list.length];
}

/** A tiny inbox drawn in one theme. */
function Preview({ theme }: { theme: Theme }) {
  return (
    <div className={styles.preview} data-theme={theme} aria-hidden>
      <span className={styles.dots}>
        <i style={{ background: "#E8695E" }} />
        <i style={{ background: "#E9B949" }} />
        <i style={{ background: "#6CC055" }} />
      </span>
      {(["work", "personal", "studio"] as const).map((color, i) => (
        <span key={color} className={cx(styles.previewRow, i === 0 && styles.previewRowOn)}>
          <i className={styles.badge} style={{ background: `var(--${color})` }} />
          <i className={styles.from} />
          <i className={styles.subject} />
        </span>
      ))}
    </div>
  );
}

/** Settings → Appearance: theme and reading size. Changes apply right away. */
export function AppearanceSection() {
  const { themeChoice, readingSize, setTheme, setReadingSize } = useStore(
    useShallow((s) => ({ themeChoice: s.themeChoice, readingSize: s.readingSize, setTheme: s.setTheme, setReadingSize: s.setReadingSize })),
  );
  const sizeRef = useRef<HTMLDivElement>(null);

  // Native radios already move with ← → once focused (Tab moves between settings).
  // With nothing focused, ← → change the theme; inside Reading size they change the size.
  useScreenKeys((k) => {
    if (k.mod || k.alt || (k.key !== "arrowleft" && k.key !== "arrowright")) return;
    const by = k.key === "arrowright" ? 1 : -1;
    if (sizeRef.current!.contains(document.activeElement)) return setReadingSize(step(sizes, readingSize, by)), true;
    if (document.activeElement === document.body) return setTheme(step(themes.map((t) => t.id), themeChoice, by)), true;
  });

  const change = <ShortcutHint keys="← →" label="Change" />;

  return (
    <div className={shared.page}>
      <ScreenHeading title="Appearance" sub="Choose how Moss looks. Changes show right away." />

      <div className={shared.group}>
        <SectionHeader label="Theme" right={change} />
        <div className={styles.options} role="radiogroup" aria-label="Theme">
          {themes.map((t) => (
            <label key={t.id} className={cx(styles.card, styles.themeCard, themeChoice === t.id && styles.on)}>
              <span className={styles.previewBox}>
                {t.id === "system" ? (
                  <>
                    <Preview theme="dark" />
                    <Preview theme="light" />
                  </>
                ) : (
                  <Preview theme={t.id} />
                )}
              </span>
              <span className={styles.label}>
                <Radio name="theme" checked={themeChoice === t.id} onChange={() => setTheme(t.id)} />
                <span className={styles.text}>
                  <span className={shared.name}>{t.name}</span>
                  <span className={shared.small}>{t.sub}</span>
                </span>
              </span>
            </label>
          ))}
        </div>
      </div>

      <div className={shared.group}>
        <SectionHeader label="Reading size" right={change} />
        <div ref={sizeRef} className={styles.sizes}>
          <SegmentedControl
            label="Reading size"
            value={readingSize}
            onChange={setReadingSize}
            options={sizes.map((size) => ({
              value: size,
              label: sizeLabels[size],
              lead: <span className={cx(styles.aa, styles[size], size === readingSize && styles.aaOn)}>Aa</span>,
            }))}
          />
        </div>
      </div>
      <RemoteImagesSettings />
    </div>
  );
}
