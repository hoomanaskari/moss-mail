import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { LucideIcon } from "lucide-react";
import { Lock, Search } from "lucide-react";
import { deviceName } from "../../../lib/platform";
import { Icon, Pill, SectionLabel } from "../../atoms";
import { PaletteItem, ShortcutHint } from "../../molecules";
import styles from "./CommandPalette.module.css";

export interface PaletteCommand {
  id: string;
  label: string;
  sub?: string;
  icon?: LucideIcon;
  shortcut?: string;
  ai?: boolean;
  run: () => void;
}

export interface PaletteGroup {
  title: string;
  items: PaletteCommand[];
}

export interface CommandPaletteProps {
  /** Built from the current query so AI suggestions can react to what you type. */
  groups: (query: string) => PaletteGroup[];
  /** What the palette acts on, e.g. the open email's subject. */
  context?: string;
  initialQuery?: string;
  onClose: () => void;
}

/** ⌘K: ask AI or run any command by typing. Everything in Moss is reachable from here. */
export function CommandPalette({ groups, context, initialQuery = "", onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState(initialQuery);
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const highlightedRef = useRef<HTMLButtonElement>(null);

  const visible = useMemo(() => groups(query).filter((g) => g.items.length > 0), [groups, query]);
  const flat = visible.flatMap((g) => g.items);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  // Only the arrow keys move the highlight (pointing never does), and they keep it in view.
  useEffect(() => {
    highlightedRef.current?.scrollIntoView({ block: "nearest" });
  }, [highlight]);

  const choose = (cmd?: PaletteCommand) => {
    if (!cmd) return;
    onClose();
    cmd.run();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      setHighlight((h) => (flat.length ? (h + 1) % flat.length : 0));
    } else if (e.key === "ArrowUp") {
      setHighlight((h) => (flat.length ? (h - 1 + flat.length) % flat.length : 0));
    } else if (e.key === "Enter") {
      // ⇧↵ is a key of its own: Ask about the email in context only.
      choose(e.shiftKey ? flat.find((c) => c.shortcut === "⇧↵") : flat[highlight]);
    } else if (e.key === "Escape") {
      onClose();
    } else {
      return;
    }
    e.preventDefault();
    // The screen a command opens listens for keys too: the ↵ that ran it would open an email there.
    e.stopPropagation();
  };

  let index = -1;
  return (
    <div className={styles.layer} onKeyDown={onKeyDown}>
      <div className={styles.scrim} onClick={onClose} />
      <div className={styles.palette} role="dialog" aria-modal aria-label="Command palette">
        <div className={styles.input}>
          <Icon icon={Search} size={20} tone="ink-3" />
          <input
            ref={inputRef}
            className={styles.query}
            value={query}
            placeholder="Ask AI or type a command"
            onChange={(e) => {
              setQuery(e.target.value);
              setHighlight(0);
            }}
            aria-label="Ask AI or type a command"
            aria-controls="palette-results"
          />
          {context && <Pill variant="context">{context}</Pill>}
        </div>
        <div className={styles.results} id="palette-results" role="listbox">
          {visible.map((g) => (
            <div key={g.title} className={styles.group} role="group" aria-label={g.title}>
              <div className={styles.heading}>
                <SectionLabel size="sm" as="div">
                  {g.title}
                </SectionLabel>
              </div>
              {g.items.map((cmd) => {
                index += 1;
                const on = index === highlight;
                return (
                  <PaletteItem
                    key={cmd.id}
                    ref={on ? highlightedRef : undefined}
                    label={cmd.label}
                    sub={cmd.sub}
                    icon={cmd.icon}
                    shortcut={cmd.shortcut}
                    ai={cmd.ai}
                    highlighted={on}
                    onChoose={() => choose(cmd)}
                  />
                );
              })}
            </div>
          ))}
          {flat.length === 0 && <p className={styles.empty}>Nothing matches “{query}”.</p>}
        </div>
        <div className={styles.footer}>
          <ShortcutHint keys="↑↓" label="Move" size="sm" />
          <ShortcutHint keys="↵" label="Choose" size="sm" />
          <ShortcutHint keys="Esc" label="Close" size="sm" onClick={onClose} />
          <ShortcutHint keys="⌘J" label="Ask Moss" size="sm" />
          <span className={styles.spacer} />
          <span className={styles.private}>
            <Icon icon={Lock} size={13} tone="ink-3" />
            AI runs on this {deviceName}
          </span>
        </div>
      </div>
    </div>
  );
}
