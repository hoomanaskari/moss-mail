/**
 * New versions of Moss. In the desktop app this is Tauri's updater (`tauriUpdates`): it reads
 * latest.json from the newest GitHub Release, checks its signature and installs it. In the
 * browser, dev scenes and tests it is the demo below.
 */

/** A newer Moss than this one. */
export interface UpdateInfo {
  version: string;
  /** What's new, from the release. */
  notes?: string;
}

export interface UpdatesBackend {
  /** This Moss's version, e.g. "0.1.0". */
  version(): Promise<string>;
  /** A newer Moss, if there is one. */
  check(): Promise<UpdateInfo | null>;
  /** Downloads and installs what `check` found. `progress` gets bytes done and the total (0 when the server doesn't say). */
  install(progress: (done: number, total: number) => void): Promise<void>;
  /** Quits and opens the new Moss. */
  restart(): Promise<void>;
}

export const noUpdate = "No update to install. Check again.";

/** Moss `version` (0.1.0), with `next` (and what's new in it) waiting on the server if given. */
export function demoUpdates({ version = "0.1.0", next, notes }: { version?: string; next?: string; notes?: string } = {}): UpdatesBackend {
  let running = version;
  let waiting = next;
  let found: string | undefined;
  return {
    version: async () => running,
    check: async () => {
      found = waiting;
      return found ? { version: found, ...(notes && { notes }) } : null;
    },
    install: async (progress) => {
      if (!found) throw new Error(noUpdate);
      progress(5, 10);
      progress(10, 10);
    },
    restart: async () => {
      if (found) running = found;
      waiting = found = undefined;
    },
  };
}

let current: UpdatesBackend = demoUpdates();

export const updates = () => current;

export function setUpdates(backend: UpdatesBackend) {
  current = backend;
}
