import { cx } from "../../../lib/cx";
import styles from "./Avatar.module.css";

export type AvatarSize = 22 | 24 | 28 | 56;

export interface AvatarProps {
  initials: string;
  size?: AvatarSize;
  /** `external` marks someone outside your organisation. */
  tone?: "default" | "external";
  className?: string;
}

/** A person's initials in a circle. */
export function Avatar({ initials, size = 24, tone = "default", className }: AvatarProps) {
  return (
    <span className={cx(styles.avatar, styles[`s${size}`], styles[tone], className)} aria-hidden>
      {initials}
    </span>
  );
}
