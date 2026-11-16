/**
 * Opening the mail at launch. In the desktop app nothing is shown until it has loaded; if it
 * can't be, Moss says why and offers to try again, and never falls back to the demo mailbox.
 */
import { create } from "zustand";
import { errorText } from "../services/errorText";

interface StartupState {
  /** Why the mail couldn't be loaded, while it couldn't. */
  failed?: string;
  /** Loading it again after it failed. */
  trying: boolean;
}

export const useStartup = create<StartupState>()(() => ({ failed: undefined, trying: false }));

let load: () => Promise<void> = async () => {};

/** Loads the mail with `connect`. Never throws: a failure is kept for the screen to show. */
export async function start(connect: () => Promise<void>) {
  load = connect;
  useStartup.setState({ trying: true });
  try {
    await connect();
    useStartup.setState({ failed: undefined, trying: false });
  } catch (error) {
    console.error("Moss: couldn’t load mail", error);
    useStartup.setState({ failed: errorText(error), trying: false });
  }
}

/** Loads the mail again the same way, after it failed. */
export const tryAgain = () => start(load);
