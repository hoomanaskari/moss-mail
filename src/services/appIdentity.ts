import { create } from "zustand";

/**
 * Which Moss this is: the installed one, or Moss Dev, the separate app developers build to try
 * changes beside it (see src-tauri/src/identity.rs). In the browser, dev scenes and tests it is
 * the installed Moss, unless a scene says otherwise.
 */
export interface AppIdentity {
  /** "Moss" or "Moss Dev": the name in the menu bar, the Dock and the window title. */
  name: string;
  /** Moss Dev: it shows a Dev mark. */
  dev: boolean;
  /** Whether it looks for new versions. Moss Dev never does. */
  updates: boolean;
  /** The name agents add it as, so Moss Dev never replaces the installed Moss in an agent. */
  agentName: string;
}

export const installedMoss: AppIdentity = { name: "Moss", dev: false, updates: true, agentName: "moss" };
export const mossDev: AppIdentity = { name: "Moss Dev", dev: true, updates: false, agentName: "moss-dev" };

export const useAppIdentity = create<AppIdentity>()(() => installedMoss);
