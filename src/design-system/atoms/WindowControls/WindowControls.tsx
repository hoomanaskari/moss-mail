import { isMac, isTauri } from "../../../lib/platform";
import styles from "./WindowControls.module.css";

/**
 * The macOS traffic lights from the design.
 * In the desktop app on macOS the real ones are drawn by the OS at a fixed spot
 * (tauri.conf.json `trafficLightPosition`), so this only keeps their space free.
 * On Windows and Linux the OS draws its own title bar, so nothing is rendered.
 */
export function WindowControls() {
  if (isTauri && !isMac) return null;
  if (isTauri) return <div className={styles.reserve} aria-hidden />;
  return (
    <div className={styles.dots} aria-hidden>
      <span className={styles.dot} style={{ background: "#E8695E" }} />
      <span className={styles.dot} style={{ background: "#E9B949" }} />
      <span className={styles.dot} style={{ background: "#6CC055" }} />
    </div>
  );
}
