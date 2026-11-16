import { create } from "zustand";
import { mail, type CredentialCleanupStatus } from "./mail";

export const useCredentialCleanup = create(() => ({ pending: false, busy: false, unavailable: false }));
let revision = 0;
export function resetCredentialCleanup() {
  revision++;
  useCredentialCleanup.setState({ pending: false, busy: false, unavailable: false });
}
export function showCredentialCleanup(status: CredentialCleanupStatus) {
  revision++;
  const valid = typeof status?.pending === "boolean";
  useCredentialCleanup.setState({ pending: valid ? status.pending : true, busy: false, unavailable: !valid });
}
export async function refreshCredentialCleanup(retry = false) {
  if (retry && useCredentialCleanup.getState().busy) return;
  const request = ++revision;
  useCredentialCleanup.setState({ busy: true });
  try {
    const result = await (retry ? mail().retryCredentialCleanup() : mail().credentialCleanupStatus());
    if (request !== revision) return;
    showCredentialCleanup(result);
  } catch {
    if (request !== revision) return;
    useCredentialCleanup.setState({ pending: true, busy: false, unavailable: true });
  }
}
