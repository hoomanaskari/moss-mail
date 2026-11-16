import type { ReactNode } from "react";
import { cx } from "../../../lib/cx";
import styles from "./Pill.module.css";

/**
 * - `label`: AI-applied label on a thread ("Budget").
 * - `due`: a deadline ("Due Wed", "By Thursday").
 * - `waiting`: an answer you wait on hasn't come ("No reply"): outlined, so it isn't read as a due day.
 * - `flag`: a warning about a person ("Outside Northwind").
 * - `name`: a highlighted name inside a sentence.
 * - `context`: what the command palette is acting on.
 * - `ai`: the small solid "AI" mark.
 */
export type PillVariant = "label" | "due" | "waiting" | "flag" | "name" | "context" | "ai";

export interface PillProps {
  variant: PillVariant;
  children: ReactNode;
  className?: string;
}

/** A small rounded tag. */
export function Pill({ variant, children, className }: PillProps) {
  return <span className={cx(styles.pill, styles[variant], className)}>{children}</span>;
}
