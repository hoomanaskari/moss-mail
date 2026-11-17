import { useEffect, useRef, type ReactNode } from "react";
import { CircleArrowUp, RotateCw } from "lucide-react";
import { Button, Markdown, ProgressBar } from "../design-system/atoms";
import { useUpdates } from "../state/updates";
import { percent } from "./AppStatus";
import styles from "./UpdateDialog.module.css";

/**
 * The update window: what a check you asked for from the app menu (Check for Updates…) or the
 * palette found. It always answers: up to date, a new Moss with what's new in it, or why it
 * couldn't check, with a way to try again. Closing it leaves a new Moss offered in the top bar.
 */
export function UpdateDialog() {
  const { open, update, version, check, install, restart, close } = useUpdates();
  const dialog = useRef<HTMLDialogElement>(null);
  const shown = open && update.state !== "idle";
  useEffect(() => {
    if (!shown) return;
    if (!dialog.current!.open) dialog.current!.showModal();
    // The answer's main button takes the focus, so Return does the likely thing.
    dialog.current!.querySelector<HTMLElement>("[data-main]")!.focus();
  }, [shown, update.state]);
  if (!shown) return null;

  const closeButton = (label = "Close", main = false) => (
    <Button variant="outline" data-main={main || undefined} onClick={close}>
      {label}
    </Button>
  );
  let title: string;
  let body: ReactNode = null;
  let actions: ReactNode;
  switch (update.state) {
    case "checking":
      title = "Checking for updates…";
      body = <p>Moss is looking for a newer version.</p>;
      actions = closeButton("Close", true);
      break;
    case "current":
      title = "Moss is up to date";
      body = <p>{version ? `You have Moss ${version}, the newest version.` : "You have the newest version."}</p>;
      actions = closeButton("OK", true);
      break;
    case "available":
      title = `Moss ${update.version} is available`;
      body = (
        <>
          <p>{version && `You have Moss ${version}. `}Moss downloads the new version, installs it and restarts.</p>
          {update.notes && (
            <section className={styles.notes} aria-label="What’s new">
              <h3>What’s new</h3>
              <Markdown text={update.notes} />
            </section>
          )}
        </>
      );
      actions = (
        <>
          {closeButton("Later")}
          <Button variant="primary" icon={CircleArrowUp} data-main onClick={() => void install()}>
            Update now
          </Button>
        </>
      );
      break;
    case "downloading": {
      const pct = percent(update.done, update.total);
      title = `Updating to Moss ${update.version}`;
      body = (
        <>
          <div className={styles.progress}>
            <ProgressBar value={pct === undefined ? 0 : pct / 100} label={`Downloading Moss ${update.version}`} />
          </div>
          <p>{pct === undefined ? "Downloading…" : `Downloaded ${pct}%.`} Moss restarts when it’s done.</p>
        </>
      );
      actions = closeButton("Hide", true);
      break;
    }
    case "ready":
      title = `Moss ${update.version} is ready`;
      body = <p>Restart to start using it. Your drafts are saved first.</p>;
      actions = (
        <>
          {closeButton("Later")}
          <Button variant="primary" icon={RotateCw} data-main onClick={() => void restart()}>
            Restart now
          </Button>
        </>
      );
      break;
    case "failed": {
      const download = update.during === "download";
      title = download ? `Couldn’t download Moss ${update.version}` : "Couldn’t check for updates";
      body = <p role="alert">{update.message}</p>;
      actions = (
        <>
          {closeButton()}
          <Button variant="primary" icon={RotateCw} data-main onClick={() => void (download ? install() : check({ asked: true }))}>
            Try again
          </Button>
        </>
      );
      break;
    }
  }

  return (
    <dialog
      ref={dialog}
      className={styles.dialog}
      aria-labelledby="update-title"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
      onKeyDownCapture={(event) => event.stopPropagation()}
    >
      <h2 id="update-title">{title}</h2>
      {body}
      <div className={styles.actions}>{actions}</div>
    </dialog>
  );
}
