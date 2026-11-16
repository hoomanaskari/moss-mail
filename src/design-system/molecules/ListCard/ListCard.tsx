import type { HTMLAttributes } from "react";
import { cx } from "../../../lib/cx";
import styles from "./ListCard.module.css";

export interface ListCardProps extends HTMLAttributes<HTMLElement> {
  as?: "div" | "ul" | "ol";
}

/** A boxed list of rows: providers, domains, accounts, models, signatures. */
export function ListCard({ as: Tag = "div", className, ...rest }: ListCardProps) {
  return <Tag className={cx(styles.card, className)} {...rest} />;
}

export interface ListRowProps extends HTMLAttributes<HTMLElement> {
  as?: "div" | "li" | "label";
  /** The row under the cursor: raised, with the teal bar on its left edge, as in the inbox. */
  selected?: boolean;
}

/** One row in a ListCard. Its height and contents come from the caller. */
export function ListRow({ as: Tag = "div", selected, className, children, ...rest }: ListRowProps) {
  return (
    <Tag className={cx(styles.row, selected && styles.selected, className)} data-selected={selected || undefined} {...rest}>
      {selected && <span className={styles.cursor} aria-hidden />}
      {children}
    </Tag>
  );
}
