/** The Done list: emails you marked done, grouped by the day you did it. */
import type { Thread } from "../data/types";

export interface DoneGroup {
  title: string;
  threads: Thread[];
}

const startOfDay = (ms: number) => {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
};

function title(doneAt: number | undefined, now: number) {
  if (doneAt === undefined) return "Older";
  const days = Math.round((startOfDay(now) - startOfDay(doneAt)) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  return days < 7 ? "Earlier this week" : "Older";
}

/** Whether `doneAt` falls on the same day as `now`. */
export const doneToday = (doneAt: number, now: number) => title(doneAt, now) === "Today";

/**
 * Groups threads (already newest first) under Today, Yesterday, Earlier this week and Older:
 * by when they were done, or by another time `at` gives (when blocked mail came).
 */
export function doneGroups(threads: Thread[], now: number, at: (t: Thread) => number | undefined = (t) => t.doneAt): DoneGroup[] {
  const groups: DoneGroup[] = [];
  for (const t of threads) {
    const name = title(at(t), now);
    const group = groups.find((g) => g.title === name);
    if (group) group.threads.push(t);
    else groups.push({ title: name, threads: [t] });
  }
  return groups;
}
