import { Check, ExternalLink } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Button, Icon, SectionLabel } from "../../atoms";
import styles from "./SetupSteps.module.css";

export interface SetupStep {
  title: string;
  sub?: string;
  /** A page to open for this step. */
  url?: string;
  /** Button label; "Open" by default. */
  action?: string;
  /** Key for the button; by default ⌘1, ⌘2… or ⌘O when only one step has a page. */
  keys?: string;
}

export interface SetupStepsProps {
  steps: SetupStep[];
  /** How many steps are done; the next one is highlighted. */
  done: number;
  onOpen: (index: number, url: string) => void;
  /** A heading above the card, e.g. "How to fix it". */
  label?: string;
}

/** Numbered steps to follow on another website, each with a button that opens the page. */
export function SetupSteps({ steps, done, onOpen, label }: SetupStepsProps) {
  const pages = steps.filter((s) => s.url).length;
  const list = (
    <ol className={styles.card}>
      {steps.map((step, i) => {
        const state = i < done ? "done" : i === done ? "now" : "next";
        const keys = step.keys ?? (pages > 1 ? `⌘${i + 1}` : "⌘O");
        return (
          <li
            key={step.title}
            className={cx(styles.step, styles[state])}
            aria-current={state === "now" ? "step" : undefined}
            aria-label={state === "done" ? `${step.title}, done` : undefined}
          >
            <span className={styles.mark} aria-hidden>
              {state === "done" ? <Icon icon={Check} size={14} tone="ai" /> : i + 1}
            </span>
            <span className={styles.text}>
              <span className={styles.title}>{step.title}</span>
              {step.sub && <span className={styles.sub}>{step.sub}</span>}
            </span>
            {step.url && (
              <Button variant="secondary" icon={ExternalLink} shortcut={keys} onClick={() => onOpen(i, step.url!)}>
                {step.action ?? "Open"}
              </Button>
            )}
          </li>
        );
      })}
    </ol>
  );
  if (!label) return list;
  return (
    <section className={styles.section}>
      <SectionLabel>{label}</SectionLabel>
      {list}
    </section>
  );
}
