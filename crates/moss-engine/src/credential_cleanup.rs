//! Retired Keychain references are retried only while credential writers are excluded.
use moss_accounts::{AccountConnection, SecretRef, KEYCHAIN_SERVICE};
use moss_store::CredentialCleanup;
use serde::Serialize;
use crate::{connection_key, Engine, Error, Result, Shared};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CredentialCleanupStatus {
    pub pending: bool,
}

pub(crate) fn reference(connection: &AccountConnection) -> Result<CredentialCleanup> {
    let legacy = SecretRef::for_account(&connection.email, connection.service.slug());
    let generated = connection.secret.account.strip_prefix(&format!("{} [", legacy.account))
        .and_then(|s| s.strip_suffix(']'))
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()));
    if connection.secret.service != KEYCHAIN_SERVICE || (connection.secret.account != legacy.account && !generated) {
        return Err(Error::Invalid("This account's credential reference cannot be retired safely.".into()));
    }
    Ok(CredentialCleanup {service:connection.secret.service.clone(),account:connection.secret.account.clone()})
}

impl Engine {
    pub fn credential_cleanup_status(&self) -> Result<CredentialCleanupStatus> {
        Ok(CredentialCleanupStatus {pending:self.shared.store().credential_cleanup_pending()?})
    }

    pub fn retry_credential_cleanup(&self) -> Result<CredentialCleanupStatus> {
        let _credentials = self.shared.credential_lifecycle.try_write()
            .map_err(|_| Error::Invalid("A sign-in is still in use. Try credential cleanup again when it finishes.".into()))?;
        Ok(self.shared.clean_credentials())
    }
}

impl Shared {
    /// Caller holds the credential write gate. Keychain prompts must not hold the
    /// database mutex: unrelated reads and recovery remain available while it waits.
    pub(crate) fn clean_credentials(&self) -> CredentialCleanupStatus {
        let attempt = (|| -> Result<()> {
            let mut store = self.store();
            let pending = store.next_credential_cleanup_batch()?;
            // Corrupt account settings stop cleanup instead of guessing what is unused.
            let mut used = Vec::new();
            for account in store.accounts()? {
                if let Some(json) = store.setting(&connection_key(&account.id))? {
                    let connection: AccountConnection = serde_json::from_str(&json)
                        .map_err(|_| Error::Invalid("Saved credential references cannot be read.".into()))?;
                    used.push(connection.secret);
                }
            }
            drop(store);
            for reference in pending {
                if reference.service != KEYCHAIN_SERVICE { continue; }
                let key = SecretRef {service:reference.service.clone(),account:reference.account.clone()};
                if used.contains(&key) {
                    // Reusing a reference supersedes its old retirement intent.
                    // A later replacement or removal queues a fresh intent atomically.
                    self.store().finish_credential_cleanup(&reference)?;
                    continue;
                }
                if self.secrets.delete(&key).is_ok() && matches!(self.secrets.load(&key), Ok(None)) {
                    self.store().finish_credential_cleanup(&reference)?;
                }
            }
            Ok(())
        })();
        // After removal committed, an unavailable Keychain or database must not make
        // the renderer believe that the account still exists. The durable intent remains.
        CredentialCleanupStatus {pending:attempt.is_err() || self.store().credential_cleanup_pending().unwrap_or(true)}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moss_accounts::{AuthMethod, MailService};
    fn connection(key: SecretRef) -> AccountConnection {
        AccountConnection {email:"a@example.com".into(),service:MailService::Imap,kind:moss_core::ProviderKind::Imap,servers:None,auth:AuthMethod::Password,secret:key}
    }
    #[test]
    fn retirement_accepts_only_the_accounts_own_legacy_or_generated_reference() {
        let legacy=SecretRef::for_account("a@example.com","imap");
        assert_eq!(reference(&connection(legacy.clone())).unwrap().account,legacy.account);
        let generated=SecretRef {account:format!("{} [0123456789abcdef0123456789abcdef]",legacy.account),..legacy.clone()};
        assert_eq!(reference(&connection(generated.clone())).unwrap().account,generated.account);
        for bad in [
            SecretRef {service:"another.application".into(),..legacy.clone()},
            SecretRef::for_account("different@example.com","imap"),
            SecretRef {account:format!("{} [short]",legacy.account),..legacy.clone()},
            SecretRef {account:format!("{} [zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz]",legacy.account),..legacy.clone()},
            SecretRef {account:format!("{} [0123456789abcdef0123456789abcdef",legacy.account),..legacy},
        ] {assert!(reference(&connection(bad)).is_err());}
    }
}
