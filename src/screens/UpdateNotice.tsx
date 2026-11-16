import { CircleCheck, RotateCw, TriangleAlert } from "lucide-react";
import { Button, Icon } from "../design-system/atoms";
import { useUpdates } from "../state/updates";
import styles from "./UpdateNotice.module.css";

/** The quiet update line in the top bar: no pop-ups. Speaks up when a new Moss is in, or when you asked it to check. */
export function UpdateNotice() {
  const { update, restart } = useUpdates();
  switch (update.state) {
    case "idle":
      return null;
    case "checking":
      return (
        <span role="status" className={styles.line}>
          Checking for updates…
        </span>
      );
    case "current":
      return (
        <span role="status" className={styles.line}>
          <Icon icon={CircleCheck} size={14} tone="ai" />
          Moss is up to date
        </span>
      );
    case "failed":
      return (
        <span role="alert" className={styles.line}>
          <Icon icon={TriangleAlert} size={14} tone="due" />
          {update.message}
        </span>
      );
    case "downloading":
      return (
        <span role="status" className={styles.line}>
          Getting Moss {update.version}
          {update.total > 0 && ` · ${Math.round((update.done / update.total) * 100)}%`}
        </span>
      );
    case "ready":
      return (
        <span className={styles.line}>
          Moss {update.version} is ready
          <Button variant="secondary" icon={RotateCw} onClick={() => void restart()}>
            Restart
          </Button>
        </span>
      );
  }
}
