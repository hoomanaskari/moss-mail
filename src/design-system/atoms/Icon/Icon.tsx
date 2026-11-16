import type { LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import styles from "./Icon.module.css";

export type IconTone = "ink" | "ink-2" | "ink-3" | "ai" | "due" | "personal" | "inherit";

export interface IconProps {
  icon: LucideIcon;
  size?: number;
  tone?: IconTone;
  className?: string;
  /** Solid in its colour, e.g. the star on mail you moved to Important. */
  filled?: boolean;
}

/** A Lucide icon with a design-token colour. design.pen uses the Lucide set throughout. */
export function Icon({ icon: Glyph, size = 16, tone = "ink-2", className, filled = false }: IconProps) {
  const color = tone === "inherit" ? "currentColor" : `var(--${tone})`;
  return (
    <Glyph
      className={cx(styles.icon, className)}
      width={size}
      height={size}
      strokeWidth={2}
      color={color}
      fill={filled ? color : "none"}
      aria-hidden
    />
  );
}
