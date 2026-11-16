import { Check } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon } from "../../atoms";
import styles from "./FlowSteps.module.css";

export interface FlowStepsProps {
  steps: string[];
  /** Index of the step you are on. Earlier ones show a tick. */
  current: number;
}

/** Where you are in a short flow, e.g. Provider · Sign in · Sync. */
export function FlowSteps({ steps, current }: FlowStepsProps) {
  return (
    <ol className={styles.steps} aria-label="Steps">
      {steps.map((label, i) => {
        const state = i < current ? "done" : i === current ? "now" : "next";
        return (
          <li key={label} className={cx(styles.step, styles[state])} aria-current={state === "now" ? "step" : undefined}>
            {state === "done" ? <Icon icon={Check} size={14} tone="ai" /> : <span className={styles.dot} />}
            <span className={styles.label}>{label}</span>
          </li>
        );
      })}
    </ol>
  );
}
