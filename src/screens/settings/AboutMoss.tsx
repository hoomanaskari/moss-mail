import { CircleArrowUp, RefreshCw, RotateCw } from "lucide-react";
import { Button } from "../../design-system/atoms";
import { useAppIdentity } from "../../services/appIdentity";
import { checkedText, useUpdates } from "../../state/updates";
import { percent } from "../AppStatus";
import styles from "./AboutMoss.module.css";

/**
 * The foot of Settings: which Moss this is, and checking for updates, with the same answers as
 * the app menu's Check for Updates… and when Moss last checked.
 */
export function AboutMoss({ now = Date.now }: { now?: () => number }) {
  const { name, updates: checks } = useAppIdentity();
  const { version, update, checkedAt, check, install, restart } = useUpdates();
  const busy = update.state === "checking" || update.state === "downloading" || update.state === "ready";
  return (
    <div className={styles.about}>
      <p>
        {name}
        {version && ` ${version}`} · Free and open source
      </p>
      {checks ? (
        <>
          <Button variant="secondary" icon={RefreshCw} disabled={busy} onClick={() => void check({ asked: true })}>
            Check for updates
          </Button>
          <div role="status" className={styles.answer}>
            {answer()}
          </div>
          <p>{checkedText(checkedAt, now())}</p>
        </>
      ) : (
        <p>Updates are off in {name}.</p>
      )}
    </div>
  );

  function answer() {
    switch (update.state) {
      case "checking":
        return "Checking for updates…";
      case "current":
        return "Moss is up to date.";
      case "available":
        return (
          <>
            Moss {update.version} is available.
            <Button variant="secondary" icon={CircleArrowUp} onClick={() => void install()}>
              Update now
            </Button>
          </>
        );
      case "downloading": {
        const pct = percent(update.done, update.total);
        return `Updating to Moss ${update.version}${pct === undefined ? "…" : ` · ${pct}%`}`;
      }
      case "ready":
        return (
          <>
            Moss {update.version} is ready.
            <Button variant="secondary" icon={RotateCw} onClick={() => void restart()}>
              Restart now
            </Button>
          </>
        );
      case "failed":
        return update.during === "download" ? (
          <>
            Couldn’t download Moss {update.version}: {update.message}
            <Button variant="secondary" icon={RotateCw} onClick={() => void install()}>
              Try again
            </Button>
          </>
        ) : (
          `Couldn’t check for updates: ${update.message}`
        );
      default:
        return null;
    }
  }
}
