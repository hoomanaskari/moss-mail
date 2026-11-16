import { useId, useState, type ReactNode } from "react";
import { BellOff, ChevronDown, Info, Leaf, PencilLine, Repeat, X, type LucideIcon } from "lucide-react";
import type { Fact, FactKind, SummaryContent } from "../../../services/summaries";
import { cx } from "../../../lib/cx";
import { openLink } from "../../../lib/links";
import { plainText } from "../../../lib/markdown";
import { Markdown } from "../../atoms/Markdown/Markdown";
import { Icon, Kbd } from "../../atoms";
import styles from "./SummaryCard.module.css";

export interface SummaryCardProps {
  /** Moss's leaf by default; a state's own icon otherwise. */
  icon?: LucideIcon;
  /** "Short summary", "Summarising 6 emails…". */
  label: string;
  /** Why it's there, in plain words. */
  why?: string;
  /** `repeat` for a rule. */
  whyIcon?: LucideIcon;
  /** Shown by an info icon after `why`, on hover or focus: why there's a summary at all. */
  info?: { title: string; text: string; hint: string };
  /** Change kind ⌄ S. */
  onChangeKind?: () => void;
  /** Stop rule, for a summary a rule wrote. */
  onStopRule?: () => void;
  /** Esc Stop, while it's being written. */
  onStop?: () => void;
  /** Discard X: takes the summary away. */
  onDiscard?: () => void;
  children?: ReactNode;
  /** Under a thin line: the "always" offer. */
  footer?: ReactNode;
}

/** A summary, or where one will be: under the subject, AI-teal, saying why it's there. */
export function SummaryCard({ icon = Leaf, label, why, whyIcon, info, onChangeKind, onStopRule, onStop, onDiscard, children, footer }: SummaryCardProps) {
  return (
    <section className={styles.card} aria-label={label}>
      <div className={styles.head}>
        <Icon icon={icon} size={15} tone="ai" />
        <span className={styles.label}>{label}</span>
        <span className={styles.why}>
          {whyIcon && <Icon icon={whyIcon} size={13} tone="ink-2" />}
          {why}
          {info && (
            <span className={styles.info} tabIndex={0} aria-label={info.title}>
              <Icon icon={Info} size={13} tone="ink-2" />
              <span role="tooltip" className={styles.tooltip}>
                <strong className={styles.tipTitle}>{info.title}</strong>
                <span className={styles.tipText}>{info.text}</span>
                <span className={styles.tipHint}>{info.hint}</span>
              </span>
            </span>
          )}
        </span>
        {onStop && (
          <button type="button" className={styles.quiet} onClick={onStop}>
            <Kbd>Esc</Kbd>
            Stop
          </button>
        )}
        {onChangeKind && (
          <button type="button" className={styles.switch} onClick={onChangeKind}>
            Change kind
            <Icon icon={ChevronDown} size={13} tone="inherit" />
            <Kbd>S</Kbd>
          </button>
        )}
        {onStopRule && (
          <button type="button" className={styles.quiet} onClick={onStopRule}>
            <Icon icon={BellOff} size={13} tone="ink-2" />
            Stop rule
          </button>
        )}
        {onDiscard && (
          <button type="button" className={styles.quiet} onClick={onDiscard}>
            <Icon icon={X} size={13} tone="ink-2" />
            Discard
            <Kbd>X</Kbd>
          </button>
        )}
      </div>
      {children}
      {footer && <div className={styles.footer}>{footer}</div>}
    </section>
  );
}

/** A teal-bordered action with its key, in a card: Always for Dana ↵, Try again ↵. */
export function SummaryAction({ keys, strong, onClick, children }: { keys: string; strong?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" className={cx(styles.action, strong && styles.strong)} onClick={onClick}>
      {children}
      <Kbd>{keys}</Kbd>
    </button>
  );
}

/** Words and actions for a card's states (model not downloaded, failed). */
export function SummaryNote({ text, children }: { text: string; children: ReactNode }) {
  return (
    <>
      <p className={styles.text}>{text}</p>
      <div className={styles.actions}>{children}</div>
    </>
  );
}

/** Three bars where the summary will be. */
export function SummarySkeleton() {
  return (
    <div className={styles.skeleton} aria-hidden>
      <span />
      <span />
      <span />
    </div>
  );
}

/** "Always summarise mail from … like this?" with Always for Dana ↵, Everyone at … ⇧↵ (not for public domains) and Not now Esc. */
export function SummaryOffer(p: {
  email: string;
  name: string;
  domain?: string;
  custom: boolean;
  problem?: string;
  onAlways: () => void;
  onDomain: () => void;
  onNotNow: () => void;
}) {
  return (
    <>
      <p className={styles.question}>
        <Icon icon={Repeat} size={14} tone="ai" />
        Always summarise mail from {p.email} {p.custom ? "this way" : "like this"}?
      </p>
      <div className={styles.actions}>
        <SummaryAction keys="↵" strong onClick={p.onAlways}>
          Always for {p.name}
        </SummaryAction>
        {p.domain && (
          <SummaryAction keys="⇧↵" onClick={p.onDomain}>
            Everyone at {p.domain}
          </SummaryAction>
        )}
        <SummaryAction keys="Esc" onClick={p.onNotNow}>
          Not now
        </SummaryAction>
      </div>
      {p.problem && <p className={styles.problem}>{p.problem}</p>}
    </>
  );
}

/** Custom's instruction: ↵ writes it again. `focus` puts the cursor in it, when you've just picked Custom. */
export function SummaryInstruction({ value, focus, onChange, onRedo }: { value: string; focus?: boolean; onChange: (v: string) => void; onRedo: () => void }) {
  return (
    <label className={styles.instruction}>
      <Icon icon={PencilLine} size={15} tone="ai" />
      <input
        className={styles.input}
        aria-label="How to summarise it"
        placeholder="Say how you want it, e.g. only what changed since my last email"
        value={value}
        autoFocus={focus}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key !== "Enter") return;
          e.preventDefault();
          e.stopPropagation();
          onRedo();
        }}
      />
      <Kbd>↵</Kbd>
      <span className={styles.hint}>Redo</span>
    </label>
  );
}

/** A run of text: under a heading, and one line or several. */
interface Block {
  heading?: string;
  lines: string[];
}

/** A list line's bullet or number, in Markdown or not: "- ", "* ", "+ ", "• ", "1. ", "2) ". */
const bullet = /^([-*+•]|\d+[.)])\s+/;
/** A Markdown heading's marks: "## ". */
const hashes = /^#{1,6}\s+/;
/** A heading: marked as one, or a short line ending in a colon (bold or not). */
const headingOf = (line: string) => {
  if (hashes.test(line)) return plainText(line.replace(hashes, ""));
  const words = plainText(line);
  return words.endsWith(":") && words.length <= 80 ? words.slice(0, -1) : undefined;
};

/**
 * Prose as the model wrote it, laid out to scan. Blank lines split it into blocks; a block's
 * lines are separate points; a heading (a short line ending in a colon, or a Markdown one)
 * heads what follows it. Each line keeps its Markdown for [`Markdown`] to show.
 */
export function blocks(text: string): Block[] {
  const groups = text
    .split(/\n{2,}/)
    .map((g) => g.split("\n").map((l) => l.trim().replace(bullet, "")).filter(Boolean))
    .filter((g) => g.length);
  const out: Block[] = [];
  let heading: string | undefined;
  groups.forEach((lines, i) => {
    const [first, ...rest] = lines;
    const named = headingOf(first);
    if (heading === undefined && named !== undefined && (rest.length || i < groups.length - 1)) {
      heading = named;
      lines = rest;
    }
    if (!lines.length) return;
    out.push({ heading, lines });
    heading = undefined;
  });
  return out;
}

function TextBlock({ heading, lines }: Block) {
  const id = useId();
  return (
    <div className={styles.block}>
      {heading && (
        <h3 id={id} className={styles.heading}>
          {heading}
        </h3>
      )}
      {lines.length > 1 ? (
        <ul className={styles.lines} aria-labelledby={heading && id}>
          {lines.map((l, i) => (
            <li key={i} className={styles.line}>
              <Markdown text={l} inline />
            </li>
          ))}
        </ul>
      ) : (
        <p className={styles.text}>
          <Markdown text={lines[0]} inline />
        </p>
      )}
    </div>
  );
}

const factRows: [FactKind, string][] = [
  ["person", "People"],
  ["date", "Dates"],
  ["amount", "Amounts"],
  ["link", "Links"],
  ["other", "Other"],
];

/** How many facts of a kind show before "N more". */
const SHOWN = 5;

/** "https://www.example.com/x" → "example.com". */
const site = (url: string) => url.replace(/^https?:\/\/(www\.)?/i, "").split(/[/?#]/)[0];

/** One fact: a web link reads as what it is, with the site it's on; anything else as itself, then what it is. */
function FactItem({ fact: f }: { fact: Fact }) {
  if (f.kind === "link" && /^https?:\/\//i.test(f.value)) {
    return (
      <li>
        <a
          className={styles.link}
          href={f.value}
          title={f.value}
          onClick={(e) => {
            e.preventDefault();
            openLink(f.value);
          }}
        >
          {f.detail ?? f.value.replace(/^https?:\/\//i, "")}
        </a>
        {f.detail && <> <span className={styles.factDetail}>{site(f.value)}</span></>}
      </li>
    );
  }
  return (
    <li>
      {f.kind === "link" ? <span className={styles.link}>{f.value}</span> : f.value}
      {f.detail && <> <span className={styles.factDetail}>{f.detail}</span></>}
    </li>
  );
}

/** One kind of fact, one per line: the first {@link SHOWN}, and the rest on asking. */
function FactRow({ label, facts }: { label: string; facts: Fact[] }) {
  const [all, setAll] = useState(false);
  const shown = all ? facts : facts.slice(0, SHOWN);
  return (
    <div className={styles.fact}>
      <dt className={styles.factLabel}>{label}</dt>
      <dd className={styles.factValue}>
        <ul className={styles.factList}>
          {shown.map((f, i) => (
            <FactItem key={i} fact={f} />
          ))}
        </ul>
        {shown.length < facts.length && (
          <button type="button" className={styles.more} onClick={() => setAll(true)}>
            {facts.length - shown.length} more
          </button>
        )}
      </dd>
    </div>
  );
}

/** Ticking is yours alone: it isn't saved. */
function Actions({ items }: { items: Extract<SummaryContent, { format: "actions" }>["items"] }) {
  const [ticked, setTicked] = useState<number[]>([]);
  if (!items.length) return <p className={styles.text}>Nothing for you to do.</p>;
  return (
    <ul className={styles.list}>
      {items.map((a, i) => (
        <li key={i} className={styles.action_}>
          <input
            type="checkbox"
            className={styles.check}
            aria-label={plainText(a.text)}
            checked={ticked.includes(i)}
            onChange={(e) => setTicked(e.target.checked ? [...ticked, i] : ticked.filter((t) => t !== i))}
          />
          <Markdown text={a.text} inline className={cx(styles.task, ticked.includes(i) && styles.ticked)} />
          {a.due && <span className={styles.due}>{a.due}</span>}
        </li>
      ))}
    </ul>
  );
}

/** What the writing model wrote, the way its kind reads best. */
export function SummaryBody({ content }: { content: SummaryContent }) {
  switch (content.format) {
    case "text":
      return (
        <div className={styles.body}>
          {blocks(content.text).map((b, i) => (
            <TextBlock key={i} {...b} />
          ))}
        </div>
      );
    case "actions":
      return <Actions items={content.items} />;
    case "facts": {
      const rows = factRows.map(([kind, label]) => [label, content.items.filter((f) => f.kind === kind)] as const).filter(([, f]) => f.length);
      if (!rows.length) return <p className={styles.text}>No people, dates, amounts or links in it.</p>;
      return (
        <dl className={styles.facts}>
          {rows.map(([label, facts]) => (
            <FactRow key={label} label={label} facts={facts} />
          ))}
        </dl>
      );
    }
  }
}
