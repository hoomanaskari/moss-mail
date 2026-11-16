import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, type IconTone } from "../../atoms/Icon/Icon";
import styles from "./Notice.module.css";

/**
 * - `card`: a bordered note on the page ("Claude writes for you now…").
 * - `inset`: a note inside a panel ("Reply all includes Ines…").
 * - `warn`: something to fix before sending ("You mention attached notes…").
 * - `plain`: a quiet line with no background ("Goes to 4 people…").
 * - `note`: a bordered note that may wrap ("Work or school account? Your IT admin…").
 * - `caution`: an amber heads-up that is not an error ("In Testing mode, Google signs you out…").
 * - `footnote`: a small grey line at the end of a form ("Moss keeps your passwords in the Keychain").
 */
export type NoticeVariant = "card" | "inset" | "warn" | "plain" | "note" | "caution" | "footnote";

export interface NoticeProps {
  variant: NoticeVariant;
  icon: LucideIcon;
  children: ReactNode;
  action?: ReactNode;
  /** The icon's colour, when it isn't the variant's own. */
  tone?: IconTone;
  className?: string;
}

const tones: Record<NoticeVariant, IconTone> = {
  card: "ink-2",
  inset: "ink-2",
  warn: "due",
  plain: "ink-3",
  note: "ink-2",
  caution: "due",
  footnote: "ink-3",
};

/** A one-line message with an icon and an optional action on the right. */
export function Notice({ variant, icon, children, action, tone, className }: NoticeProps) {
  return (
    <div className={cx(styles.notice, styles[variant], className)} role={variant === "warn" ? "alert" : undefined}>
      <Icon icon={icon} size={variant === "footnote" ? 14 : 16} tone={tone ?? tones[variant]} />
      <span className={styles.text}>{children}</span>
      {action}
    </div>
  );
}
