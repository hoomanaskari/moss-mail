import { useEffect, useReducer } from "react";
import { Check, CircleAlert } from "lucide-react";
import { Spinner } from "../design-system/atoms";
import { Toast } from "../design-system/molecules";
import { undoSendMs, useStore } from "../state/store";
import styles from "./UndoToast.module.css";

/** How long the note stays. ⌘Z still works after it has gone. */
const shownFor = 6000;

const RADIUS = 6;
const AROUND = 2 * Math.PI * RADIUS;

/** A ring that empties as the time to undo runs out. */
function Countdown({ left }: { left: number }) {
  return (
    <svg className={styles.countdown} width={16} height={16} viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="8" cy="8" r={RADIUS} fill="none" stroke="currentColor" strokeWidth="2" opacity="0.3" />
      <circle
        className={styles.left}
        cx="8"
        cy="8"
        r={RADIUS}
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeDasharray={`${AROUND * left} ${AROUND}`}
        transform="rotate(-90 8 8)"
      />
    </svg>
  );
}

/**
 * After done, not done, a snooze or Send: says where the email went, with Undo. A sent email
 * counts down until it goes. Or that a change didn't save, or a change Undo can't take back.
 */
export function UndoToast() {
  const toast = useStore((s) => s.toast);
  const undo = useStore((s) => s.undo);
  const dismiss = useStore((s) => s.dismissToast);
  const place = useStore((s) => (s.view === "zero" ? "top-right" : "bottom"));
  const [, tick] = useReducer((n: number) => n + 1, 0);
  const until = toast?.until;

  useEffect(() => {
    if (!toast || toast.stays || toast.until) return;
    const timer = setTimeout(dismiss, shownFor);
    return () => clearTimeout(timer);
  }, [toast, dismiss]);

  // A countdown redraws a few times a second until the email goes.
  useEffect(() => {
    if (!until) return;
    const timer = setInterval(tick, 200);
    return () => clearInterval(timer);
  }, [until]);

  if (!toast) return null;
  const action = { label: toast.undoLabel ?? "Undo", keys: "⌘Z", onClick: undo };
  const show = toast.show && { label: toast.show.label, keys: toast.show.keys, onClick: toast.show.run };
  let note;
  if (until) {
    const left = until - Date.now();
    note =
      left > 0 ? (
        <Toast lead={<Countdown left={left / undoSendMs} />} action={action}>
          {`Sending in ${Math.ceil(left / 1000)}s`}
        </Toast>
      ) : (
        <Toast lead={<Spinner />}>Sending…</Toast>
      );
  } else if (toast.problem) {
    note = (
      <Toast icon={CircleAlert} action={show || (toast.undoLabel ? action : undefined)}>
        {toast.text}
      </Toast>
    );
  } else {
    note = (
      <Toast icon={Check} action={show || (toast.undo === false ? undefined : action)}>
        {toast.text}
      </Toast>
    );
  }
  return (
    <div className={styles.dock} data-place={place}>
      {note}
    </div>
  );
}
