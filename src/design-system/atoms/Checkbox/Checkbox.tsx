import type { InputHTMLAttributes } from "react";
import { Check } from "lucide-react";
import { Icon } from "../Icon/Icon";
import styles from "./Checkbox.module.css";

export interface CheckboxProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "onChange"> {
  checked: boolean;
  onChange?: (checked: boolean) => void;
}

/** A square tick box: teal with a check when on, an outline when off. */
export function Checkbox({ checked, onChange, ...rest }: CheckboxProps) {
  return (
    <span className={styles.box}>
      <input
        type="checkbox"
        className={styles.input}
        checked={checked}
        onChange={(e) => onChange?.(e.target.checked)}
        {...rest}
      />
      {checked && <Icon icon={Check} size={14} tone="inherit" className={styles.check} />}
    </span>
  );
}
