import type { ReactNode } from "react";
import { Check, CircleAlert } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon } from "../../atoms";
import styles from "./Field.module.css";

export interface FieldProps {
  label: string;
  /** The id of the input inside, so the label names it. */
  htmlFor: string;
  /** Quiet hint at the right end of the label row. */
  help?: ReactNode;
  /** A line under the input: a tick and grey text, or an amber error. */
  note?: ReactNode;
  noteTone?: "ok" | "error";
  children: ReactNode;
}

/** A labelled form field. */
export function Field({ label, htmlFor, help, note, noteTone = "ok", children }: FieldProps) {
  const error = noteTone === "error";
  return (
    <div className={styles.field}>
      <div className={styles.labelRow}>
        <label className={styles.label} htmlFor={htmlFor}>
          {label}
        </label>
        {help && <span className={styles.help}>{help}</span>}
      </div>
      {children}
      {note && (
        <p className={cx(styles.note, error && styles.error)} role={error ? "alert" : undefined}>
          <Icon icon={error ? CircleAlert : Check} size={14} tone={error ? "due" : "ink-3"} />
          {note}
        </p>
      )}
    </div>
  );
}
