import { cx } from "../../../lib/cx";
import styles from "./HighlightedText.module.css";

export interface HighlightedTextProps {
  parts: { text: string; match: boolean }[];
  /** On a raised (selected) row the marks use a stronger grey so they still stand out. */
  raised?: boolean;
  className?: string;
}

/** Text with the searched-for words marked: bold ink on a neutral grey, since amber means due and teal means AI. */
export function HighlightedText({ parts, raised, className }: HighlightedTextProps) {
  return (
    <span className={cx(styles.text, className)}>
      {parts.map((p, i) =>
        p.match ? (
          <mark key={i} className={cx(styles.mark, raised && styles.raised)}>
            {p.text}
          </mark>
        ) : (
          p.text
        ),
      )}
    </span>
  );
}
