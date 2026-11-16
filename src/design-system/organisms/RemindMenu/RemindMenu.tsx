import { useState, type FormEvent } from "react";
import { Briefcase, CalendarClock, ChevronLeft, Clock3, Sofa, Sunrise, type LucideIcon } from "lucide-react";
import { cx } from "../../../lib/cx";
import { remindChoices, remindTime, tomorrowMorning, type RemindChoice } from "../../../lib/time";
import { useMenuKeys } from "../../../state/useMenuKeys";
import { Button, Icon, Kbd, TextInput } from "../../atoms";
import styles from "./RemindMenu.module.css";

/** A row under the times that isn't a time, e.g. "Remind me if no reply". */
export interface RemindExtra {
  icon: LucideIcon;
  label: string;
  keys: string;
  onClick: () => void;
}

export interface RemindMenuProps {
  /** A time was picked, in ms. */
  onPick: (at: number) => void;
  extras?: RemindExtra[];
  /** The times are worked out from this; the clock when left out. */
  now?: number;
  /** What picking your own time does, on its button: "Remind me" or "Send". */
  action?: string;
}

const icons: Record<RemindChoice["id"], LucideIcon> = {
  today: Clock3,
  tomorrow: Sunrise,
  weekend: Sofa,
  "next-week": Briefcase,
};

interface Row {
  icon: LucideIcon;
  label: string;
  /** When it comes back, or the row's key. */
  detail?: string;
  keys?: string;
  run: () => void;
}

/** The ready-made times as a menu: a click, or ↑ ↓ ↵, picks one. Tomorrow starts marked. */
function Choices({ rows, start }: { rows: Row[]; start: number }) {
  const active = useMenuKeys(rows.length, (i) => rows[i].run(), start);
  return (
    <div className={styles.menu}>
      {rows.map((r, i) => (
        <button key={r.label} type="button" className={cx(styles.item, i === active && styles.active)} onClick={r.run}>
          <Icon icon={r.icon} size={15} tone="ink-3" />
          <span className={styles.label}>{r.label}</span>
          {r.detail && <span className={styles.detail}>{r.detail}</span>}
          {r.keys && <Kbd>{r.keys}</Kbd>}
        </button>
      ))}
    </div>
  );
}

const pad = (n: number) => String(n).padStart(2, "0");
const dateValue = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const timeValue = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;

/** "2026-11-04" and "08:00" as a local time in ms, or undefined while either is empty. */
function fromFields(date: string, time: string): number | undefined {
  if (!date || !time) return undefined;
  const [y, m, d] = date.split("-").map(Number);
  const [h, min] = time.split(":").map(Number);
  return new Date(y, m - 1, d, h, min).getTime();
}

/** Your own date and time, starting at tomorrow 8 AM. Only a time still ahead can be picked. */
function PickForm({ now, action, onPick, onBack }: { now: number; action: string; onPick: (at: number) => void; onBack: () => void }) {
  const start = new Date(tomorrowMorning(now));
  const [date, setDate] = useState(dateValue(start));
  const [time, setTime] = useState(timeValue(start));
  const at = fromFields(date, time);
  const passed = at !== undefined && at <= now;
  const ok = at !== undefined && !passed;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (ok) onPick(at);
  };

  return (
    <form className={styles.form} aria-label="Pick a date and time" onSubmit={submit}>
      <button type="button" className={styles.back} onClick={onBack}>
        <Icon icon={ChevronLeft} size={14} tone="ink-3" />
        Back
      </button>
      <div className={styles.fields}>
        <TextInput compact type="date" aria-label="Date" value={date} onChange={(e) => setDate(e.target.value)} autoFocus />
        <TextInput compact type="time" aria-label="Time" value={time} onChange={(e) => setTime(e.target.value)} invalid={passed} />
      </div>
      {passed && <p className={styles.passed}>That time has passed.</p>}
      <Button variant="primary" type="submit" disabled={!ok}>
        {ok ? `${action} ${remindTime(at, now)}` : action}
      </Button>
    </form>
  );
}

/** When an email should come back, or go out: Later today, Tomorrow, This weekend, Next week, or a time you pick. */
export function RemindMenu({ onPick, extras = [], now: given, action = "Remind me" }: RemindMenuProps) {
  const [now] = useState(() => given ?? Date.now());
  const [picking, setPicking] = useState(false);
  if (picking) return <PickForm now={now} action={action} onPick={onPick} onBack={() => setPicking(false)} />;

  const choices = remindChoices(now);
  const rows: Row[] = [
    ...choices.map((c) => ({ icon: icons[c.id], label: c.label, detail: remindTime(c.at, now), run: () => onPick(c.at) })),
    { icon: CalendarClock, label: "Pick a date and time…", run: () => setPicking(true) },
    ...extras.map((x) => ({ icon: x.icon, label: x.label, keys: x.keys, run: x.onClick })),
  ];
  return <Choices rows={rows} start={choices.findIndex((c) => c.id === "tomorrow")} />;
}
