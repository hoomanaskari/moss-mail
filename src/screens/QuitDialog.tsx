import { useEffect, useRef } from "react";
import { Button } from "../design-system/atoms";
import { keepWorking, quitAnyway, useQuit } from "../services/quit";
import { reloadDrafts } from "../state/drafts";
import { quietly } from "../services/mail";
import { useStore } from "../state/store";
import styles from "./QuitDialog.module.css";

/** A native modal makes the rest of the document inert while edits are settled. */
export function QuitDialog() {
  const { token, checking, exiting, blockers, error } = useQuit();
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (token && !dialog.current!.open) dialog.current!.showModal();
  }, [token]);
  if (!token) return null;
  return <dialog ref={dialog} className={styles.dialog} aria-labelledby="quit-title"
    onCancel={(event) => { event.preventDefault();void keepWorking(); }}
    onKeyDownCapture={(event) => {
      event.stopPropagation();
      if (event.key !== "Tab") return;
      if (exiting) { event.preventDefault();return; }
      const buttons = event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)");
      const first = buttons[0];const last = buttons[buttons.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault();last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault();first.focus(); }
    }}>
    <h2 id="quit-title">{exiting ? "Quitting Moss" : checking ? "Saving before quitting" : "Drafts are not confirmed saved"}</h2>
    <p>{exiting ? "Your exit request has been handed to the app." : checking ? "Moss is checking your latest edits. This does not send any new email."
      : "Keep working to review these items. Quitting anyway can lose edits that have not been saved."}</p>
    {error && <p role="alert">{error}</p>}
    <ul>{blockers.map((item) => <li key={item.id}><strong>{item.subject}</strong><p>{item.reason}</p></li>)}</ul>
    {!exiting && <div className={styles.actions}>
      <Button variant="outline" onClick={() => { void keepWorking(); }}>Keep working</Button>
      {!checking && <>
        <Button variant="primary" onClick={() => { void keepWorking();useStore.getState().openDrafts();quietly(reloadDrafts()); }}>Review drafts</Button>
        <Button variant="subtle" onClick={() => { void quitAnyway(); }}>Quit anyway</Button>
      </>}
    </div>}
  </dialog>;
}
