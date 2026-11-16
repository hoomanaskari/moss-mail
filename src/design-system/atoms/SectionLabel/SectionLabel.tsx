import type { ReactNode } from "react";
import { cx } from "../../../lib/cx";
import styles from "./SectionLabel.module.css";

export interface SectionLabelProps {
  children: ReactNode;
  /** `md` (13px) heads inbox sections; `sm` (12px) heads panels and palette groups. */
  size?: "sm" | "md";
  as?: "h2" | "h3" | "div";
  className?: string;
}

/** Small uppercase heading, e.g. "NEEDS YOUR REPLY". */
export function SectionLabel({ children, size = "md", as: Tag = "h2", className }: SectionLabelProps) {
  return <Tag className={cx(styles.label, styles[size], className)}>{children}</Tag>;
}
