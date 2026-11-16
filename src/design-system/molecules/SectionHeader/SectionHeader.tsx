import type { ReactNode } from "react";
import { SectionLabel } from "../../atoms";
import styles from "./SectionHeader.module.css";

export interface SectionHeaderProps {
  label: string;
  /** Pinned to the right end, e.g. "← → Change" or "This Mac: M3, 16 GB memory". */
  right?: ReactNode;
}

/** Small uppercase heading over a group of settings, with an optional note or key on the right. */
export function SectionHeader({ label, right }: SectionHeaderProps) {
  return (
    <div className={styles.header}>
      <SectionLabel as="h3">{label}</SectionLabel>
      <span className={styles.spacer} />
      {right}
    </div>
  );
}
