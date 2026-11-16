import { listenAppKeys } from "../../../state/appKeyboard";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { cx } from "../../../lib/cx";
import styles from "./Popover.module.css";

export interface PopoverProps {
  /** The button that opens it. */
  trigger: (p: { open: boolean; toggle: () => void }) => ReactNode;
  /** What shows while open; given `close` for choices that should shut it. */
  children: (close: () => void) => ReactNode;
  role: "dialog" | "group";
  label: string;
  /** Which edge of the trigger it lines up with. */
  align?: "start" | "end";
  /** Set both to open and close it from outside, e.g. from a shortcut; it then only asks. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /**
   * Floats over the page instead of sitting in its card: never clipped, never pushing or
   * scrolling anything. Opens downward when there's room, else upward, its height capped to
   * the room there. Closing gives focus back to the trigger.
   */
  floating?: boolean;
}

/** Room between the trigger and the card, and between the card and the window's edge. */
const GAP = 8;
const EDGE = 12;
/** Below this much room underneath, the card opens upward if there's more room above. */
const ROOM = 320;

/** Where a floating card goes for a trigger at `r`: below it if there's room, else above, capped to the room. */
export function placeFor(r: DOMRect, align: "start" | "end", width: number, height: number): { placement: "down" | "up"; style: CSSProperties } {
  const below = height - r.bottom - GAP - EDGE;
  const above = r.top - GAP - EDGE;
  const down = below >= ROOM || below >= above;
  const side = align === "start" ? { left: r.left } : { right: width - r.right };
  return down
    ? { placement: "down", style: { position: "fixed", top: r.bottom + GAP, maxHeight: below, ...side } }
    : { placement: "up", style: { position: "fixed", bottom: height - r.top + GAP, maxHeight: above, ...side } };
}

/** A card that drops down from a button. A click outside it or Esc closes it. */
export function Popover({ trigger, children, role, label, align = "start", open: given, onOpenChange, floating }: PopoverProps) {
  const [own, setOwn] = useState(false);
  const open = given ?? own;
  const latest = useRef(onOpenChange ?? setOwn);
  latest.current = onOpenChange ?? setOwn;
  const ref = useRef<HTMLDivElement>(null);
  const card = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<ReturnType<typeof placeFor>>();
  const setOpen = (o: boolean) => {
    latest.current(o);
    // A floating card lives elsewhere in the page, so focus comes back to the trigger by hand.
    if (!o && floating) ref.current?.querySelector<HTMLElement>("button, [tabindex]")?.focus({ preventScroll: true });
  };

  // Lined up with the trigger before it paints, and again whenever the trigger may have moved.
  useLayoutEffect(() => {
    if (!open || !floating) return;
    const follow = () => setPlace(placeFor(ref.current!.getBoundingClientRect(), align, window.innerWidth, window.innerHeight));
    follow();
    window.addEventListener("resize", follow);
    document.addEventListener("scroll", follow, true);
    return () => {
      window.removeEventListener("resize", follow);
      document.removeEventListener("scroll", follow, true);
    };
  }, [open, floating, align]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (!ref.current!.contains(target) && !card.current?.contains(target)) setOpen(false);
    };
    // Caught before the app's shortcuts, so Esc closes this and doesn't also leave the email.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    const stopKeys = listenAppKeys(onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      stopKeys();
    };
  }, [open]);

  const body = () => children(() => setOpen(false));
  return (
    <div ref={ref} className={styles.anchor}>
      {trigger({ open, toggle: () => setOpen(!open) })}
      {open && !floating && (
        <div role={role} aria-label={label} className={cx(styles.pop, styles[align])}>
          {body()}
        </div>
      )}
      {open &&
        floating &&
        createPortal(
          <div ref={card} role={role} aria-label={label} data-placement={place?.placement} className={styles.floating} style={place?.style}>
            {body()}
          </div>,
          document.body,
        )}
    </div>
  );
}
