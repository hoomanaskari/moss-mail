import type { ButtonHTMLAttributes, ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon, type IconTone } from "../Icon/Icon";
import { Kbd, type KbdTone } from "../Kbd/Kbd";
import { Spinner } from "../Spinner/Spinner";
import styles from "./Button.module.css";

/**
 * - `primary`: the one main action, e.g. Send.
 * - `outline`: secondary actions (Reply, Edit).
 * - `outlineSm`: small bordered actions in field rows (Edit, + CC).
 * - `quiet`: an `outlineSm` without the border, for the lesser choice beside one (Just this once ⌥I).
 * - `toolbar`: borderless composer tools (Attach, Send later). `active` turns it AI-green.
 * - `subtle`: low-emphasis destructive actions (Discard).
 * - `mini`: tiny inline actions (Copy, Contact).
 * - `warn`: an action inside a warning (Attach file).
 * - `choice`: one of several options; `active` marks the chosen one.
 * - `invert` / `glass`: buttons that sit on a photo.
 * - `secondary`: a small boxed action beside text (Open ⌘1, Open again ⌘O).
 * - `link`: a teal text action with its key (Use your own Google OAuth client ⌥A). Never has an icon.
 * - `ai`: an `outline` with the AI border and a teal icon (Change it ⌘J, Write with Moss ⌘J).
 */
export type ButtonVariant =
  | "primary"
  | "outline"
  | "outlineSm"
  | "quiet"
  | "toolbar"
  | "subtle"
  | "mini"
  | "warn"
  | "choice"
  | "invert"
  | "glass"
  | "secondary"
  | "link"
  | "ai";

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  variant: ButtonVariant;
  /** `lg` is the composer's bigger Send, and the 38px buttons in forms (Connect, Add another account). */
  size?: "md" | "lg";
  active?: boolean;
  icon?: LucideIcon;
  shortcut?: ReactNode;
  /** Overrides the key cap style picked for the variant. */
  shortcutTone?: KbdTone;
  /** Working on it (Sending…, Connecting…): a spinner in place of the icon, and no second press. */
  busy?: boolean;
  children: ReactNode;
}

const iconSize: Partial<Record<ButtonVariant, number>> = { mini: 13, outlineSm: 13, secondary: 14 };

function iconTone(variant: ButtonVariant, active?: boolean): IconTone {
  if ((variant === "toolbar" && active) || variant === "ai") return "ai";
  if (variant === "subtle") return "ink-3";
  if (variant === "warn") return "due";
  return "ink-2";
}

function kbdTone(variant: ButtonVariant): KbdTone {
  if (variant === "primary") return "onAccent";
  if (variant === "invert") return "onInvert";
  return "default";
}

/** Every clickable action in Moss. Shows its keyboard shortcut when it has one. */
export function Button({
  variant,
  size = "md",
  active,
  icon,
  shortcut,
  shortcutTone,
  busy,
  children,
  className,
  type = "button",
  disabled,
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx(styles.button, styles[variant], size === "lg" && styles.lg, active && styles.active, className)}
      aria-pressed={variant === "choice" || variant === "toolbar" ? !!active : undefined}
      aria-busy={busy || undefined}
      disabled={disabled || busy}
      {...rest}
    >
      {busy ? (
        <Spinner size={iconSize[variant] ?? 16} />
      ) : (
        icon && variant !== "link" && <Icon icon={icon} size={iconSize[variant] ?? 16} tone={iconTone(variant, active)} />
      )}
      <span className={styles.label}>{children}</span>
      {shortcut && <Kbd tone={shortcutTone ?? kbdTone(variant)}>{shortcut}</Kbd>}
    </button>
  );
}
