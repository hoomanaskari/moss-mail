/** Times as the mail list and the email view show them. Mail times come from Rust as ms since the epoch. */

const weekdays = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

const startOfDay = (ms: number) => {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
};

/** Days from `now`'s day to `ms`'s day: 0 today, -1 yesterday, 1 tomorrow. */
function dayOffset(ms: number, now: number) {
  return Math.round((startOfDay(ms) - startOfDay(now)) / 86_400_000);
}

/** "Thursday" */
export const weekdayOf = (ms: number) => weekdays[new Date(ms).getDay()];

/** "9:12 AM" */
function clock(d: Date) {
  const h = d.getHours() % 12 || 12;
  return `${h}:${String(d.getMinutes()).padStart(2, "0")} ${d.getHours() < 12 ? "AM" : "PM"}`;
}

/** "9:12 AM" today, "Tomorrow", "Fri" this week, "Oct 14" this year, "Oct 14, 2025" before. */
export function listTime(ms: number, now: number): string {
  const d = new Date(ms);
  const offset = dayOffset(ms, now);
  if (offset === 0) return clock(d);
  if (offset === 1) return "Tomorrow";
  // Past mail this week, or a snoozed email coming back this week.
  if (offset > -7 && offset < 7) return weekdays[d.getDay()].slice(0, 3);
  const date = `${months[d.getMonth()].slice(0, 3)} ${d.getDate()}`;
  return d.getFullYear() === new Date(now).getFullYear() ? date : `${date}, ${d.getFullYear()}`;
}

/** "Sunday, 1 November, 9:12 AM", with the year when it isn't this year. */
export function fullTime(ms: number, now: number): string {
  const d = new Date(ms);
  const year = d.getFullYear() === new Date(now).getFullYear() ? "" : ` ${d.getFullYear()}`;
  return `${weekdays[d.getDay()]}, ${d.getDate()} ${months[d.getMonth()]}${year}, ${clock(d)}`;
}

/** `days` after `now`'s day, at `hour` o'clock. */
function dayAt(now: number, days: number, hour: number) {
  const d = new Date(now);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + days, hour, 0).getTime();
}

/** When "Remind me" brings an email back by default: 8 AM tomorrow. */
export function tomorrowMorning(now: number): number {
  return dayAt(now, 1, 8);
}

/** "6 PM", or "6:30 PM" off the hour. */
function hourClock(d: Date) {
  return d.getMinutes() ? clock(d) : `${d.getHours() % 12 || 12} ${d.getHours() < 12 ? "AM" : "PM"}`;
}

/** When a reminder is due: "6 PM" today, "Sat 8 AM" this week, "Sun 8 Nov, 9:15 AM" after that. */
export function remindTime(ms: number, now: number): string {
  const d = new Date(ms);
  const offset = dayOffset(ms, now);
  if (offset === 0) return hourClock(d);
  const day = weekdays[d.getDay()].slice(0, 3);
  if (offset > 0 && offset < 7) return `${day} ${hourClock(d)}`;
  const year = d.getFullYear() === new Date(now).getFullYear() ? "" : ` ${d.getFullYear()}`;
  return `${day} ${d.getDate()} ${months[d.getMonth()].slice(0, 3)}${year}, ${hourClock(d)}`;
}

export interface RemindChoice {
  id: "today" | "tomorrow" | "weekend" | "next-week";
  label: string;
  at: number;
}

/**
 * The ready-made times in the Remind menu. Mornings are 8 AM. A choice that would be the same
 * day as Tomorrow is left out, and so is Later today once it would land after 9 PM.
 */
export function remindChoices(now: number): RemindChoice[] {
  const d = new Date(now);
  const weekday = d.getDay(); // 0 Sunday … 6 Saturday
  const inThree = d.getHours() + 3 + (d.getMinutes() || d.getSeconds() || d.getMilliseconds() ? 1 : 0);
  const choices: RemindChoice[] = [];
  if (inThree <= 21) choices.push({ id: "today", label: "Later today", at: dayAt(now, 0, inThree) });
  choices.push({ id: "tomorrow", label: "Tomorrow", at: dayAt(now, 1, 8) });
  if (weekday >= 1 && weekday <= 4) choices.push({ id: "weekend", label: "This weekend", at: dayAt(now, 6 - weekday, 8) });
  if (weekday !== 0) choices.push({ id: "next-week", label: "Next week", at: dayAt(now, 8 - weekday, 8) });
  return choices;
}

/** 4 PM the day before the weekday a deadline names ("By Thursday"), if that is still ahead. */
export function dayBefore(due: string, now: number): number | undefined {
  const named = weekdays.findIndex((w) => due.toLowerCase().includes(w.toLowerCase()));
  if (named < 0) return undefined;
  const days = (named - new Date(now).getDay() + 7) % 7;
  const at = dayAt(now, days - 1, 16);
  return at > now ? at : undefined;
}

/** 4 PM the day before a deadline's day (YYYY-MM-DD), if that is still ahead. */
export function dayBeforeDate(date: string, now: number): number | undefined {
  const [y, m, d] = date.split("-").map(Number);
  const at = new Date(y, m - 1, d - 1, 16, 0).getTime();
  return at > now ? at : undefined;
}
