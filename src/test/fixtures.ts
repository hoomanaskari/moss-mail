import { resetCredentialCleanup } from "../services/credentialCleanup";
import { useRemoteImages } from "../state/remoteImages";
import type { Account, Message, Person, Thread } from "../data/types";
import type { SavedDraft } from "../state/drafts";
import { resetServerChanges } from "../services/serverChanges";
import { useScheduled } from "../state/scheduled";
import { useSignatures } from "../state/signatures";
import { useStartup } from "../state/startup";
import { useStore } from "../state/store";
import { useTrash } from "../state/trash";
import { trashThreads as demoTrash } from "../data/demo";
import { pristineUpdates, useUpdates } from "../state/updates";
import { demoUpdates, setUpdates } from "../services/updates";
import { installedMoss, useAppIdentity } from "../services/appIdentity";

export const person = (name: string, email: string, extra: Partial<Person> = {}): Person => ({
  name,
  email,
  initials: name
    .split(" ")
    .map((w) => w[0])
    .join(""),
  ...extra,
});

export const maya = person("Maya Ross", "maya@northwind.co");
export const lena = person("Lena Vogt", "lena@northwind.co", { short: "Lena" });
export const jonas = person("Jonas Weber", "jonas@northwind.co", { short: "Jonas" });

export const work: Account = {
  id: "work",
  name: "Work",
  letter: "W",
  address: maya.email,
  senderName: "Maya Ross",
  color: "work",
  orgDomain: "northwind.co",
};
export const personal: Account = {
  id: "personal",
  name: "Personal",
  letter: "P",
  address: "maya.ross@fastmail.com",
  senderName: "Maya Ross",
  color: "personal",
};

export function message(overrides: Partial<Message> = {}): Message {
  return {
    id: "m1",
    from: lena,
    to: [maya],
    cc: [],
    bcc: [],
    time: "9:12 AM",
    sentFull: "Sunday, 1 November, 9:12 AM",
    paragraphs: ["Hello"],
    attachments: [],
    ...overrides,
  };
}

/** An unsent email, as saved: a new one from work unless `overrides` say otherwise. */
export function draft(id: string, overrides: Partial<SavedDraft> = {}): SavedDraft {
  return {
    id,
    mode: "new",
    fromAccountId: "work",
    to: [],
    cc: [],
    bcc: [],
    subject: "",
    body: "",
    bodyHtml: "",
    fieldsOpen: true,
    showCc: false,
    showBcc: false,
    remindIfNoReply: false,
    signatureId: null,
    updatedAt: Date.now(),
    ...overrides,
  };
}

export function thread(id: string, overrides: Partial<Thread> = {}): Thread {
  return {
    id,
    accountId: "work",
    split: "important",
    section: "reply",
    from: lena,
    subject: `Subject ${id}`,
    time: "9:12 AM",
    messages: [message({ id: `${id}-1` })],
    ...overrides,
  };
}

const pristine = useStore.getState();
const pristineSignatures = useSignatures.getState();
const pristineScheduled = useScheduled.getState();

/** Resets the store to its first-launch state, with the given accounts and threads instead of the demo data. */
export function resetStore(threads: Thread[], accounts: Account[] = [work, personal]) {
  resetServerChanges();
  resetCredentialCleanup();
  useRemoteImages.setState({ policy: "familiar", saved: true });
  useSignatures.setState(pristineSignatures, true);
  useScheduled.setState(pristineScheduled, true);
  useUpdates.setState(pristineUpdates);
  setUpdates(demoUpdates());
  useAppIdentity.setState(installedMoss, true);
  useStartup.setState({ failed: undefined, trying: false });
  useTrash.setState({ threads: [], cursorId: undefined }, true);
  useStore.setState(
    { ...pristine, accounts, threads, doneThreads: [], blockedThreads: [], cursorId: threads[0]?.id, themeChoice: "system", theme: "dark", readingSize: "default" },
    true,
  );
}

/** Resets the store to first launch with the demo data, for tests of whole screens. */
export function resetDemoStore() {
  resetServerChanges();
  resetCredentialCleanup();
  useRemoteImages.setState({ policy: "familiar", saved: true });
  useSignatures.setState(pristineSignatures, true);
  useScheduled.setState(pristineScheduled, true);
  useUpdates.setState(pristineUpdates);
  setUpdates(demoUpdates());
  useAppIdentity.setState(installedMoss, true);
  useStartup.setState({ failed: undefined, trying: false });
  useTrash.setState({ threads: demoTrash, cursorId: undefined }, true);
  useStore.setState({ ...pristine, themeChoice: "system", theme: "dark", readingSize: "default" }, true);
}
