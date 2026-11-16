import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, Kbd, WindowControls } from "../../atoms";
import styles from "./TopBar.module.css";

export interface TopBarProps {
  /** With `back`, the bar shows a back link and a bottom border (thread and composer pages). */
  back?: { label: string; onBack: () => void };
  /** Page title, lined up with the page content below (inbox). */
  title?: ReactNode;
  right?: ReactNode;
}

/**
 * The window's top strip: 54px tall, everything centred on the same line as the
 * macOS traffic lights. Also the drag handle for moving the window.
 */
export function TopBar({ back, title, right }: TopBarProps) {
  return (
    <header data-tauri-drag-region className={cx(styles.bar, back && styles.nav)}>
      <WindowControls />
      {title && (
        <div data-tauri-drag-region className={styles.title}>
          {title}
        </div>
      )}
      {back && (
        <button type="button" className={styles.back} onClick={back.onBack}>
          <Icon icon={ArrowLeft} size={16} tone="ink-2" />
          <span className={styles.backLabel}>{back.label}</span>
          <Kbd>Esc</Kbd>
        </button>
      )}
      <div data-tauri-drag-region className={styles.spacer} />
      {right}
    </header>
  );
}
