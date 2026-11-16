import type { InputHTMLAttributes, ReactNode, Ref } from "react";
import { cx } from "../../../lib/cx";
import styles from "./TextInput.module.css";

export interface TextInputProps extends InputHTMLAttributes<HTMLInputElement> {
  /** `compact` (34px, page-coloured) sits inside cards and settings rows; the default (45px) is a form field. */
  compact?: boolean;
  /** Draws the amber error outline and tells screen readers the value is wrong. */
  invalid?: boolean;
  /** Shown after the text, e.g. a ⌘V key or "Key works". */
  trailing?: ReactNode;
  ref?: Ref<HTMLInputElement>;
}

/** A single-line text box. */
export function TextInput({ compact, invalid, trailing, className, ...rest }: TextInputProps) {
  return (
    <span className={cx(styles.box, compact && styles.compact, invalid && styles.invalid, className)}>
      <input className={styles.input} aria-invalid={invalid || undefined} spellCheck={false} {...rest} />
      {trailing}
    </span>
  );
}
