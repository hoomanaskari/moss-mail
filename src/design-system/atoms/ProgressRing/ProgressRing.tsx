import styles from "./ProgressRing.module.css";

export interface ProgressRingProps {
  /** 0 to 1. */
  value: number;
  size?: number;
  label?: string;
  /** Turns slowly, for "waiting" states where there is no real progress to show. */
  spinning?: boolean;
}

// design.pen draws the ring as two ellipses with innerRadius 0.78.
const INNER = 0.78;

/** Circular progress toward inbox zero. Starts at 12 o'clock and fills clockwise. */
export function ProgressRing({ value, size = 40, label, spinning }: ProgressRingProps) {
  const thickness = ((1 - INNER) * size) / 2;
  const r = size / 2 - thickness / 2;
  const circumference = 2 * Math.PI * r;
  const clamped = Math.min(1, Math.max(0, value));
  return (
    <svg
      className={styles.ring}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      role="img"
      aria-label={label}
      data-spinning={spinning || undefined}
    >
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--raise)" strokeWidth={thickness} />
      <circle
        className={styles.value}
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke="var(--ai)"
        strokeWidth={thickness}
        strokeDasharray={`${circumference * clamped} ${circumference}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
    </svg>
  );
}
