import { Button } from "../design-system/atoms";
import { refreshCredentialCleanup, useCredentialCleanup } from "../services/credentialCleanup";
import styles from "./CredentialCleanupNotice.module.css";

export function CredentialCleanupNotice() {
  const { pending, busy, unavailable } = useCredentialCleanup();
  if (!pending) return null;
  return <aside role="status" aria-label="Credential cleanup" className={styles.notice}>
    <strong>{unavailable ? "Cannot check credential cleanup." : "Credential cleanup is pending."}</strong>
    <p>Removed accounts stay removed. A saved sign-in may remain on this Mac until cleanup succeeds.</p>
    <Button variant="secondary" disabled={busy} onClick={() => void refreshCredentialCleanup(true)}>{busy ? "Checking…" : "Retry credential cleanup"}</Button>
  </aside>;
}
