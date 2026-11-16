import type { LucideIcon } from "lucide-react";
import { CircleAlert, Download, Leaf } from "lucide-react";
import { Icon, Kbd } from "../../atoms";
import styles from "./MossNote.module.css";

/** What Moss is doing with your text: done (`wrote`), still writing, missing its model, or failed. */
export type MossNoteState =
  | { kind: "wrote"; lead: string; note?: string }
  | { kind: "writing" }
  | { kind: "missing" }
  | { kind: "failed" };

export interface MossNoteProps {
  state: MossNoteState;
  onChangeIt?: () => void;
  onUndo?: () => void;
  onStop?: () => void;
  onDownload?: () => void;
  onSettings?: () => void;
  onRetry?: () => void;
}

interface Action {
  label: string;
  keys: string;
  run?: () => void;
}

/** The teal strip above text Moss writes, in the composer and in Quick reply. Every state shows here, never in a pop-up. */
export function MossNote({ state, onChangeIt, onUndo, onStop, onDownload, onSettings, onRetry }: MossNoteProps) {
  const [icon, lead, note, actions]: [LucideIcon, string, string | undefined, Action[]] =
    state.kind === "wrote"
      ? [Leaf, state.lead, state.note && `“${state.note}”`, [
          { label: "Change it", keys: "⌘J", run: onChangeIt },
          { label: "Undo", keys: "⌘Z", run: onUndo },
        ]]
      : state.kind === "writing"
        ? [Leaf, "Moss is writing…", "Your words stay as they are until it’s done.", [{ label: "Stop", keys: "Esc", run: onStop }]]
        : state.kind === "missing"
          ? [Download, "Writing needs the writing model", "Gemma 4 E4B runs on this Mac. About 5 GB, downloads in the background.", [
              { label: "Download", keys: "↵", run: onDownload },
              { label: "AI settings", keys: "⌘,", run: onSettings },
            ]]
          : [CircleAlert, "Couldn’t write this one", "The writing model stopped. Your draft is unchanged.", [
              { label: "Try again", keys: "↵", run: onRetry },
              { label: "Undo", keys: "⌘Z", run: onUndo },
            ]];
  // Missing and failed wait for you: ↵ presses their first button.
  const waits = state.kind === "missing" || state.kind === "failed";
  return (
    <div className={styles.note} role="status">
      <Icon icon={icon} size={15} tone="ai" />
      <div className={styles.words}>
        <span className={styles.lead}>{lead}</span>
        {note && <span className={styles.text}>{note}</span>}
      </div>
      {actions
        .filter((a) => a.run)
        .map((a, i) => (
          <button key={a.label} type="button" className={styles.action} onClick={a.run} autoFocus={waits && i === 0}>
            {a.label}
            <Kbd>{a.keys}</Kbd>
          </button>
        ))}
    </div>
  );
}
