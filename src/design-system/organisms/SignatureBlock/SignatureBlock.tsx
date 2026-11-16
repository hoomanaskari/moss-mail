import { listenAppKeys } from "../../../state/appKeyboard";
import { useEffect, useRef } from "react";
import { ChevronDown, Signature as SignatureIcon, X } from "lucide-react";
import { cx } from "../../../lib/cx";
import { keyDigit } from "../../../lib/keyDigit";
import { linkFor, preview, type Signature } from "../../../state/signatures";
import { useMenuKeys } from "../../../state/useMenuKeys";
import { Button, Icon, Kbd, Pill, Radio } from "../../atoms";
import styles from "./SignatureBlock.module.css";

export interface SignatureBlockProps {
  /** The signature on this email; none when undefined. */
  signature?: Signature;
  /** Signatures offered in the picker, keys 1–3 in this order. None is always 0. */
  options: Signature[];
  /** Next to the chip, e.g. "Your signature for hello@rossstudio.art", or "Left out: quick reply in a back-and-forth". */
  reason: string;
  /** In the picker: why Moss picked this one; left out when empty. */
  why: string;
  open: boolean;
  onToggle: () => void;
  onChoose: (id: string | null) => void;
  onEdit: () => void;
  /** On a Quick reply: the row and the lines, smaller; the picker opens upward. */
  minimal?: boolean;
}

interface Choice {
  id: string | null;
  name: string;
  sub: string;
  key: string;
}

/** The picker's choices, there only while it is open: ↑ ↓ ↵ pick from the one on the email. */
function Choices({ choices, chosen, onChoose }: { choices: Choice[]; chosen: string | null; onChoose: (id: string | null) => void }) {
  const active = useMenuKeys(choices.length, (i) => onChoose(choices[i].id), Math.max(0, choices.findIndex((c) => c.id === chosen)));
  return choices.map((c, i) => {
    const on = c.id === chosen;
    return (
      <label key={c.key} className={cx(styles.option, i === active && styles.active)}>
        <Radio name="signature" checked={on} onChange={() => onChoose(c.id)} />
        <span className={styles.text}>
          <span className={styles.optionName}>{c.name}</span>
          <span className={styles.preview}>{c.sub}</span>
        </span>
        <Kbd>{c.key}</Kbd>
      </label>
    );
  });
}

/**
 * The signature under an email body, always there so you can see whether one goes out: with one,
 * a chip to change it (⌥S) and Remove; with none, "No signature" and Add (⌥S), or a way to
 * Settings when you have none saved. The picker numbers the choices so one key picks a signature.
 */
export function SignatureBlock({ signature, options, reason, why, open, onToggle, onChoose, onEdit, minimal }: SignatureBlockProps) {
  const shown = options.slice(0, 3);
  // The newest handlers, for the key listener that lives while the picker is open.
  const latest = useRef({ shown, onChoose, onToggle });
  latest.current = { shown, onChoose, onToggle };

  useEffect(() => {
    if (!open) return;
    // Capture on window runs before the composer's own keys (Esc would leave the composer).
    const onKey = (e: KeyboardEvent) => {
      const { shown, onChoose, onToggle } = latest.current;
      // ⌥ may still be down from ⌥S, so read the digit from the physical key.
      const n = keyDigit(e);
      if (e.key === "Escape") onToggle();
      else if (n === 0) onChoose(null);
      else if (n && n <= shown.length) onChoose(shown[n - 1].id);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    const stopKeys = listenAppKeys(onKey, true);
    return () => stopKeys();
  }, [open]);

  const choices: Choice[] = [
    ...shown.map((x, i) => ({ id: x.id as string | null, name: x.name, sub: preview(x), key: String(i + 1) })),
    { id: null, name: "None", sub: "No signature on this email", key: "0" },
  ];

  return (
    <div className={cx(styles.block, minimal && styles.minimal)}>
      <div className={styles.row} role="group" aria-label="Signature on this email">
        {signature ? (
          <>
            <button
              type="button"
              className={styles.chip}
              onClick={onToggle}
              aria-expanded={open}
              aria-label={`Change signature, ${signature.name}`}
              aria-keyshortcuts="Alt+S"
            >
              <Icon icon={SignatureIcon} size={14} tone="ink-2" />
              <span className={styles.chipLabel}>{signature.name}</span>
              <Icon icon={ChevronDown} size={13} tone="ink-2" />
              <Kbd>⌥S</Kbd>
            </button>
            <Button variant="mini" icon={X} onClick={() => onChoose(null)} aria-label="Remove signature">
              Remove
            </Button>
          </>
        ) : (
          <>
            <span className={styles.none}>
              <Icon icon={SignatureIcon} size={14} tone="ink-3" />
              No signature
            </span>
            {options.length ? (
              <Button variant="link" shortcut="⌥S" onClick={onToggle} aria-expanded={open} aria-keyshortcuts="Alt+S">
                Add signature
              </Button>
            ) : (
              <Button variant="link" onClick={onEdit}>
                Set up a signature
              </Button>
            )}
          </>
        )}
        {reason && <span className={styles.reason}>{reason}</span>}
      </div>
      {signature && (
        <div className={styles.lines} role="group" aria-label="Signature">
          {signature.lines.map((line, i) =>
            linkFor(line) ? (
              <a key={i} className={cx(styles.line, styles.link)} href={linkFor(line)} target="_blank" rel="noreferrer">
                {line}
              </a>
            ) : (
              <span key={i} className={cx(styles.line, i === 0 && styles.name)}>
                {line}
              </span>
            ),
          )}
        </div>
      )}
      {open && (
        <div className={styles.picker}>
          <div className={styles.heading} id="signature-picker-title">
            Signature for this email
          </div>
          <div role="radiogroup" aria-labelledby="signature-picker-title" className={styles.options}>
            <Choices choices={choices} chosen={signature?.id ?? null} onChoose={onChoose} />
          </div>
          <div className={styles.footer}>
            {why && (
              <p className={styles.why}>
                <Pill variant="ai">AI</Pill>
                <span className={styles.whyText}>{why}</span>
              </p>
            )}
            <Button variant="link" shortcut="⌘," onClick={onEdit}>
              Edit signatures
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
