import { CircleArrowUp, RotateCw, TriangleAlert } from "lucide-react";
import { Button, Icon, ProgressBar } from "../design-system/atoms";
import { useAppIdentity } from "../services/appIdentity";
import { useUpdates } from "../state/updates";
import styles from "./AppStatus.module.css";

/** How far a download is, as a whole percent; none until the size is known. */
export const percent = (done: number, total: number) => (total > 0 ? Math.round((done / total) * 100) : undefined);

/**
 * The top bar's own corner, on every screen: Moss Dev's mark, and a new Moss while there is one.
 * The update stays offered until you act on it: Update downloads it with its progress shown,
 * installs it and restarts into it.
 */
export function AppStatus() {
  const dev = useAppIdentity((i) => i.dev);
  return (
    <span className={styles.status}>
      {dev && (
        <span className={styles.dev} title="Moss Dev: the app for trying changes, apart from your Moss">
          Dev
        </span>
      )}
      <UpdateIndicator />
    </span>
  );
}

function UpdateIndicator() {
  const { update, install, restart } = useUpdates();
  switch (update.state) {
    case "available":
      return (
        <Button variant="primary" icon={CircleArrowUp} onClick={() => void install()}>
          Update to Moss {update.version}
        </Button>
      );
    case "downloading": {
      const pct = percent(update.done, update.total);
      return (
        <span role="status" className={styles.line}>
          Updating to Moss {update.version}
          {pct !== undefined && ` · ${pct}%`}
          <span className={styles.bar}>
            <ProgressBar value={pct === undefined ? 0 : pct / 100} label={`Downloading Moss ${update.version}`} />
          </span>
        </span>
      );
    }
    case "ready":
      return (
        <span className={styles.line}>
          Moss {update.version} is ready
          <Button variant="primary" icon={RotateCw} onClick={() => void restart()}>
            Restart now
          </Button>
        </span>
      );
    case "failed":
      return update.during === "download" ? (
        <span role="alert" className={styles.line}>
          <Icon icon={TriangleAlert} size={14} tone="due" />
          Couldn’t download Moss {update.version}
          <Button variant="secondary" icon={RotateCw} onClick={() => void install()}>
            Try again
          </Button>
        </span>
      ) : null;
    default:
      return null;
  }
}
