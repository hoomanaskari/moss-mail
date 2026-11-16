import { mail, quietly } from "./mail";

/** Threads whose bodies were asked for already this session. */
const preloaded = new Set<string>();

/** Fetches the bodies of these threads that aren't on this Mac yet, each thread once a session. */
export function preload(ids: string[]) {
  const fresh = ids.filter((id) => !preloaded.has(id));
  if (!fresh.length) return;
  fresh.forEach((id) => preloaded.add(id));
  quietly(mail().preload(fresh));
}

/** Forgets what was preloaded: for tests. */
export function forgetPreloaded() {
  preloaded.clear();
}
