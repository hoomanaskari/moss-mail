import { useState, type KeyboardEvent } from "react";
import type { LucideIcon } from "lucide-react";
import { Check, Hourglass, Languages, Leaf, Lock, Minimize2, Smile, SpellCheck, X, Zap } from "lucide-react";
import type { ReplyJob } from "../../../services/replies";
import { cx } from "../../../lib/cx";
import { Icon, Kbd } from "../../atoms";
import styles from "./MossMenu.module.css";

/** What to write, and the line the composer shows once Moss has written it. */
export interface MossPick {
  job: ReplyJob;
  lead: string;
  /** Your own note, when you typed one. */
  note?: string;
}

interface Row {
  label: string;
  sub: string;
  icon: LucideIcon;
  /** Changes what's written, so it needs text. */
  change?: boolean;
  /** What it writes, or the step it opens. */
  run: MossPick | "tone" | "translate";
}

type Step = "main" | "tone" | "translate";

const QUICK = "Reply to what they wrote: answer what they ask, briefly and warmly.";
const DIRECT = "Make it more direct: say the point first and cut the softening words. Keep what it says.";
const LANGUAGES = ["Dutch", "German", "French", "Persian", "Spanish", "English"];

const translate = (language: string): MossPick => ({
  job: { kind: "translate", language },
  lead: `Moss translated it into ${language}`,
});

function rows(step: Step, name?: string): Row[] {
  if (step === "tone")
    return [
      { label: "Friendlier", sub: "Warmer, same point", icon: Smile, change: true, run: { job: { kind: "friendlier" }, lead: "Moss made it friendlier" } },
      { label: "More formal", sub: "For clients and people you don’t know", icon: Smile, change: true, run: { job: { kind: "formal" }, lead: "Moss made it more formal" } },
      { label: "More direct", sub: "The point first, no softening", icon: Smile, change: true, run: { job: { kind: "instructions", instructions: DIRECT }, lead: "Moss made it more direct" } },
    ];
  if (step === "translate")
    return LANGUAGES.map((language) => ({ label: language, sub: "", icon: Languages, change: true, run: translate(language) }));
  return [
    { label: "Quick reply", sub: "Moss picks what to say", icon: Zap, run: { job: { kind: "instructions", instructions: QUICK }, lead: "Moss wrote a quick reply" } },
    { label: "Say yes", sub: name ? `Accept what ${name} asks` : "Accept what they ask", icon: Check, run: { job: { kind: "accept" }, lead: "Moss wrote a yes" } },
    { label: "Say no, kindly", sub: "Decline and keep it friendly", icon: X, run: { job: { kind: "decline" }, lead: "Moss wrote a kind no" } },
    { label: "Ask for more time", sub: "Suggests a new date", icon: Hourglass, run: { job: { kind: "moreTime" }, lead: "Moss asked for more time" } },
    { label: "Shorter", sub: "Same point, fewer words", icon: Minimize2, change: true, run: { job: { kind: "shorter" }, lead: "Moss made it shorter" } },
    { label: "Change tone", sub: "Friendlier, more formal, more direct", icon: Smile, change: true, run: "tone" },
    { label: "Fix spelling and grammar", sub: "Changes only mistakes", icon: SpellCheck, change: true, run: { job: { kind: "fix" }, lead: "Moss fixed spelling and grammar" } },
    { label: "Translate", sub: "Into Dutch, German, French, Persian…", icon: Languages, change: true, run: "translate" },
  ];
}

export interface MossMenuProps {
  /** The other person's first name, for "Accept what Dana asks". */
  name?: string;
  /** Whether the composer has text to change; without it, the changes are greyed out. */
  hasText: boolean;
  onPick: (pick: MossPick) => void;
  onClose: () => void;
}

/** "Write with Moss" (⌘J in the composer): your own note, or an action. ↑ ↓ pick, ↵ runs, Esc closes. */
export function MossMenu({ name, hasText, onPick, onClose }: MossMenuProps) {
  const [step, setStep] = useState<Step>("main");
  const [text, setText] = useState("");
  const [index, setIndex] = useState(0);
  const list = rows(step, name);
  const usable = (r: Row) => hasText || !r.change;

  const run = (row: Row) => {
    if (!usable(row)) return;
    if (typeof row.run === "string") {
      setStep(row.run);
      setIndex(0);
      setText("");
    } else onPick(row.run);
  };

  const move = (by: 1 | -1) => {
    for (let i = index + by; i >= 0 && i < list.length; i += by)
      if (usable(list[i])) return setIndex(i);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") move(e.key === "ArrowDown" ? 1 : -1);
    else if (e.key === "Escape") onClose();
    else if (e.key === "Enter") {
      const note = text.trim();
      if (!note) run(list[index]);
      else if (step === "translate") onPick(translate(note));
      else onPick({ job: { kind: "instructions", instructions: note }, lead: "Moss wrote this from your note", note });
    } else return;
    e.preventDefault();
    e.stopPropagation();
  };

  const groups: [string, Row[]][] =
    step === "main"
      ? [
          ["Reply for me", list.slice(0, 4)],
          ["Change what’s written", list.slice(4)],
        ]
      : [[step === "tone" ? "Change tone" : "Translate into", list]];
  const label = step === "translate" ? "Into which language?" : "Tell Moss what to say";
  const placeholder =
    step === "translate" ? "Into which language? Pick one or type it" : "Tell Moss what to say and how, e.g. “yes to the 17th, keep it short”";

  return (
    <div className={styles.menu} role="dialog" aria-label="Write with Moss">
      <div className={styles.instruction}>
        <Icon icon={Leaf} size={16} tone="ai" />
        <input
          key={step}
          className={styles.field}
          role="combobox"
          aria-label={label}
          aria-expanded
          aria-controls="moss-menu-list"
          placeholder={placeholder}
          value={text}
          autoFocus
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <Kbd>↵</Kbd>
        <span className={styles.hint}>Write</span>
      </div>
      <div id="moss-menu-list" role="listbox" aria-label="Moss can">
        {groups.map(([heading, group]) => (
          <div key={heading} role="group" aria-label={heading}>
            <div className={styles.head}>{heading}</div>
            {group.map((row) => {
              const i = list.indexOf(row);
              const on = i === index;
              return (
                <div
                  key={row.label}
                  role="option"
                  aria-selected={on}
                  aria-disabled={!usable(row)}
                  className={cx(styles.row, on && styles.on, !usable(row) && styles.off)}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => run(row)}
                >
                  <Icon icon={row.icon} size={16} tone="ai" />
                  <span className={styles.label}>{row.label}</span>
                  <span className={styles.sub}>{row.sub}</span>
                  {on && <Kbd>↵</Kbd>}
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <div className={styles.foot}>
        <Icon icon={Lock} size={13} tone="ink-3" />
        <span className={styles.footText}>Lands as text you can edit. Nothing sends by itself.</span>
        <Kbd>Esc</Kbd>
        <span className={styles.hint}>Close</span>
      </div>
    </div>
  );
}
