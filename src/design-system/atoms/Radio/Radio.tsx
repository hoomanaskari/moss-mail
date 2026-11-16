import type { InputHTMLAttributes } from "react";
import styles from "./Radio.module.css";

export type RadioProps = Omit<InputHTMLAttributes<HTMLInputElement>, "type">;

/** A round choice mark: a teal ring and dot when chosen, a grey outline when not. */
export function Radio(props: RadioProps) {
  return <input type="radio" className={styles.radio} {...props} />;
}
