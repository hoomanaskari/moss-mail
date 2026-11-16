/**
 * Emails you sent that Moss stopped before they went: it quit or crashed while they waited for
 * Undo, so they are back in Drafts, unsent. Moss says so once after launch, with a way to
 * Drafts. The engine keeps each until the window says it was shown.
 */
import { mail } from "../services/mail";
import { toast, useStore } from "./store";

/** Said already in this window, in case the engine asks again before it hears so. */
const said = new Set<string>();

export function resetInterruptedSends() {
  said.clear();
}

const quoted = (subject: string) => `“${subject.trim() || "No subject"}”`;

export async function sayInterruptedSends(): Promise<void> {
  const fresh = (await mail().interruptedSends()).filter((send) => !said.has(send.id));
  if (!fresh.length) return;
  for (const send of fresh) said.add(send.id);
  const text = fresh.length === 1
    ? `Moss stopped before sending ${quoted(fresh[0].subject)}. It is back in Drafts.`
    : `Moss stopped before sending ${fresh.length} emails. They are back in Drafts.`;
  useStore.setState({
    toast: { ...toast(text), undo: false, stays: true, show: { label: "Show drafts", run: () => useStore.getState().openDrafts() } },
  });
  await mail().dismissInterruptedSends(fresh.map((send) => send.id));
}
