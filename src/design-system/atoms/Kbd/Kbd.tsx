import type { ReactNode } from "react";
import { cx } from "../../../lib/cx";
import { isMac } from "../../../lib/platform";
import styles from "./Kbd.module.css";

export type KbdTone = "default" | "onAccent" | "onInvert" | "quiet";

export interface KbdProps {
  children: ReactNode;
  /** `default`: raised key. `onAccent`: inside the Send button. `onInvert`: on inverted buttons. `quiet`: small key inside chips. */
  tone?: KbdTone;
  className?: string;
}

const winNames: Record<string, string> = { "⌘": "Ctrl+", "⌥": "Alt+", "⇧": "Shift+", "⌫": "Backspace" };

/** The design writes shortcuts with Mac symbols; spell them out on Windows and Linux. */
export function platformKeys(keys: string): string {
  return isMac ? keys : keys.replace(/[⌘⌥⇧⌫]/g, (m) => winNames[m]);
}

/** A keyboard key cap. Every shortcut in Moss is shown with one of these. */
export function Kbd({ children, tone = "default", className }: KbdProps) {
  return (
    <kbd className={cx(styles.kbd, styles[tone], className)}>
      {typeof children === "string" ? platformKeys(children) : children}
    </kbd>
  );
}
