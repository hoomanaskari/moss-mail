//! The OS keychain: macOS Keychain, Windows Credential Manager, Linux Secret Service.
//!
//! On Linux setups with no Secret Service running, and in dev builds, [`platform_secret_store`]
//! uses an [`EncryptedFileStore`] in the app's data folder instead.

use std::path::Path;
use std::sync::Arc;

use keyring_core::{CredentialStore, Error as KeyringError};

use crate::file_store::EncryptedFileStore;
use crate::secret::{Secret, SecretRef, SecretStore, SecretStoreError};

/// Secrets in a `keyring-core` credential store. Each secret is one item, stored as JSON.
pub struct KeyringSecretStore {
    store: Arc<CredentialStore>,
}

impl KeyringSecretStore {
    /// Wraps any `keyring-core` store. Tests pass `keyring_core::mock::Store`.
    pub fn new(store: Arc<CredentialStore>) -> Self {
        Self { store }
    }

    fn entry(&self, key: &SecretRef) -> Result<keyring_core::Entry, SecretStoreError> {
        self.store
            .build(&key.service, &key.account, None)
            .map_err(unavailable)
    }
}

impl SecretStore for KeyringSecretStore {
    fn save(&self, key: &SecretRef, secret: &Secret) -> Result<(), SecretStoreError> {
        let json = serde_json::to_string(secret).expect("secrets always serialise");
        self.entry(key)?.set_password(&json).map_err(unavailable)
    }

    fn load(&self, key: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
        match self.entry(key)?.get_password() {
            Ok(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| SecretStoreError::Corrupt(e.to_string())),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(e) => Err(unavailable(e)),
        }
    }

    fn delete(&self, key: &SecretRef) -> Result<(), SecretStoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(unavailable(e)),
        }
    }
}

fn unavailable(e: KeyringError) -> SecretStoreError {
    // keyring-core errors describe the platform failure, never the password.
    SecretStoreError::Unavailable(e.to_string())
}

/// This platform's keychain.
pub fn os_keychain() -> Result<Arc<CredentialStore>, SecretStoreError> {
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new();
    #[cfg(all(unix, not(target_os = "macos")))]
    let store = zbus_secret_service_keyring_store::Store::new();
    store
        .map(|s| s as Arc<CredentialStore>)
        .map_err(unavailable)
}

/// The store the app should use: the OS keychain, or an encrypted file in `data_dir`
/// when there is no keychain (Linux without a Secret Service).
///
/// Dev builds always use the file. macOS ties keychain access to the exact binary,
/// so every `tauri dev` rebuild would ask for the login password again.
pub fn platform_secret_store(data_dir: &Path) -> Result<Box<dyn SecretStore>, SecretStoreError> {
    choose_store(cfg!(debug_assertions), cfg!(target_os = "linux"), os_keychain, data_dir)
}

fn choose_store(
    dev_build: bool,
    allow_file_fallback: bool,
    keychain: impl FnOnce() -> Result<Arc<CredentialStore>, SecretStoreError>,
    data_dir: &Path,
) -> Result<Box<dyn SecretStore>, SecretStoreError> {
    match (!dev_build).then(keychain) {
        Some(Ok(store)) => {
            let dev = data_dir.join(crate::file_store::SECRETS_FILE).exists();
            Ok(Box::new(KeychainFirst {
                keychain: KeyringSecretStore::new(store),
                dev: dev.then(|| EncryptedFileStore::open(data_dir).ok()).flatten(),
            }))
        }
        Some(Err(error)) if !allow_file_fallback => Err(error),
        _ => Ok(Box::new(EncryptedFileStore::open(data_dir)?)),
    }
}

/// The keychain, and the file a dev build kept its sign-ins in. Both builds share the data
/// folder, so a release build copies a sign-in it only finds in the file into the keychain.
/// Each file item is removed only after its Keychain copy has been read back and verified.
/// Other items stay until they are migrated. A damaged file is the same as none.
struct KeychainFirst<S = KeyringSecretStore> {
    keychain: S,
    dev: Option<EncryptedFileStore>,
}

impl<S> KeychainFirst<S> {
    fn clean_verified(&self, key: &SecretRef, secret: &Secret) {
        if let Some(file) = &self.dev {
            if file.delete_if_matches(key, secret).is_err() {
                // No account names, secrets or OS error payloads in diagnostics.
                eprintln!("Moss: a verified sign-in is in the Keychain, but its old file copy could not be removed.");
            }
        }
    }
}

impl<S: SecretStore> SecretStore for KeychainFirst<S> {
    fn save(&self, key: &SecretRef, secret: &Secret) -> Result<(), SecretStoreError> {
        self.keychain.save(key, secret)?;
        if self.keychain.load(key)?.as_ref() != Some(secret) {
            return Err(SecretStoreError::Unavailable("The Keychain did not retain the sign-in. The previous copy has been kept.".into()));
        }
        self.clean_verified(key, secret);
        Ok(())
    }

    fn load(&self, key: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
        if let Some(secret) = self.keychain.load(key)? {
            self.clean_verified(key, &secret);
            return Ok(Some(secret));
        }
        let Some(secret) = self.dev.as_ref().and_then(|f| f.load(key).ok().flatten()) else {
            return Ok(None);
        };
        self.save(key, &secret)?;
        Ok(Some(secret))
    }

    fn delete(&self, key: &SecretRef) -> Result<(), SecretStoreError> {
        // Remove the fallback first: otherwise a failed file rewrite can resurrect
        // a deleted Keychain credential on the next load.
        if let Some(file) = &self.dev {
            file.delete(key)?;
        }
        self.keychain.delete(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring_core::api::CredentialStoreApi;
    use keyring_core::mock;

    fn store() -> (Arc<mock::Store>, KeyringSecretStore) {
        let mock = mock::Store::new().unwrap();
        (mock.clone(), KeyringSecretStore::new(mock))
    }

    fn key() -> SecretRef {
        SecretRef::for_account("maya@northwind.co", "imap")
    }

    fn fail_next(mock: &mock::Store, key: &SecretRef, err: KeyringError) {
        let entry = mock.build(&key.service, &key.account, None).unwrap();
        let cred: &mock::Cred = entry.as_any().downcast_ref().unwrap();
        cred.set_error(err);
    }

    #[test]
    fn saves_loads_and_deletes_through_the_keychain() {
        let (_, store) = store();
        let secret = Secret::Password {
            password: "hunter2".into(),
        };
        assert_eq!(store.load(&key()).unwrap(), None);
        store.save(&key(), &secret).unwrap();
        assert_eq!(store.load(&key()).unwrap(), Some(secret));
        store.delete(&key()).unwrap();
        assert_eq!(store.load(&key()).unwrap(), None);
        // Deleting again is not an error.
        store.delete(&key()).unwrap();
    }

    #[test]
    fn keychain_failures_become_unavailable() {
        let (mock, store) = store();
        let locked = || KeyringError::NoStorageAccess("locked".into());
        fail_next(&mock, &key(), locked());
        assert!(matches!(
            store.save(&key(), &Secret::ApiKey { key: "k".into() }),
            Err(SecretStoreError::Unavailable(_))
        ));
        fail_next(&mock, &key(), locked());
        assert!(matches!(
            store.load(&key()),
            Err(SecretStoreError::Unavailable(_))
        ));
        fail_next(&mock, &key(), locked());
        assert!(matches!(
            store.delete(&key()),
            Err(SecretStoreError::Unavailable(_))
        ));
    }

    #[test]
    fn keys_the_keychain_rejects_are_unavailable() {
        let invalid = KeyringSecretStore::new(Arc::new(RejectingStore));
        assert!(matches!(
            invalid.load(&key()),
            Err(SecretStoreError::Unavailable(_))
        ));
    }

    #[test]
    fn damaged_items_are_reported_as_corrupt() {
        let (mock, store) = store();
        let entry = mock.build(&key().service, &key().account, None).unwrap();
        entry.set_password("not json").unwrap();
        assert!(matches!(
            store.load(&key()),
            Err(SecretStoreError::Corrupt(_))
        ));
    }

    #[test]
    fn this_platform_has_a_keychain_store() {
        // Building the store doesn't read or write the keychain.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(os_keychain().is_ok());
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = os_keychain();
    }

    #[test]
    fn uses_the_keychain_when_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let mock: Arc<CredentialStore> = mock::Store::new().unwrap();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        chosen
            .save(&key(), &Secret::ApiKey { key: "k".into() })
            .unwrap();
        assert!(
            std::fs::read_dir(dir.path()).unwrap().next().is_none(),
            "no file written"
        );
    }

    #[test]
    fn release_keychain_failure_does_not_create_a_weaker_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let result = choose_store(false, false, || Err(SecretStoreError::Unavailable("keychain unavailable".into())), dir.path());
        assert!(matches!(result, Err(SecretStoreError::Unavailable(_))));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn verified_migration_removes_only_the_migrated_file_item() {
        let dir = tempfile::tempdir().unwrap();
        let file = EncryptedFileStore::open(dir.path()).unwrap();
        let secret = Secret::ApiKey { key: "migration-fixture".into() };
        let other = SecretRef::for_account("other@example.test", "imap");
        file.save(&key(), &secret).unwrap();
        file.save(&other, &secret).unwrap();
        let (mock, keychain) = store();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), Some(secret.clone()));
        assert_eq!(keychain.load(&key()).unwrap(), Some(secret.clone()));
        assert_eq!(file.load(&key()).unwrap(), None);
        assert_eq!(file.load(&other).unwrap(), Some(secret));
    }

    #[test]
    fn falls_back_to_an_encrypted_file_without_a_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let none = || Err(SecretStoreError::Unavailable("no Secret Service".into()));
        let chosen = choose_store(false, true, none, dir.path()).unwrap();
        chosen
            .save(&key(), &Secret::ApiKey { key: "k".into() })
            .unwrap();
        assert!(dir.path().join(crate::file_store::SECRETS_FILE).exists());
        let blocked = tempfile::NamedTempFile::new().unwrap();
        assert!(
            choose_store(false, true, none, blocked.path()).is_err(),
            "data dir is a file"
        );
        let _ = platform_secret_store(dir.path()).unwrap();
    }

    #[test]
    fn a_dev_build_keeps_secrets_in_a_file_and_never_opens_the_keychain() {
        // Every rebuild is a new app to macOS, so the keychain would ask for the
        // password again after each one.
        let dir = tempfile::tempdir().unwrap();
        let chosen = choose_store(
            true,
            false,
            || panic!("a dev build opened the keychain"),
            dir.path(),
        )
        .unwrap();
        chosen
            .save(&key(), &Secret::ApiKey { key: "k".into() })
            .unwrap();
        assert!(dir.path().join(crate::file_store::SECRETS_FILE).exists());
    }

    #[test]
    fn a_release_build_carries_a_dev_build_sign_in_over_to_the_keychain() {
        // Dev and release builds share the data folder, so a release build finds the dev
        // build's sign-ins there instead of asking you to sign in again.
        let dir = tempfile::tempdir().unwrap();
        let dev = Secret::ApiKey { key: "dev".into() };
        EncryptedFileStore::open(dir.path())
            .unwrap()
            .save(&key(), &dev)
            .unwrap();
        let (mock, keychain) = store();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), Some(dev.clone()));
        assert_eq!(keychain.load(&key()).unwrap(), Some(dev), "copied into the keychain");
        // From then on the keychain has the last word.
        let newer = Secret::ApiKey { key: "newer".into() };
        chosen.save(&key(), &newer).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), Some(newer));
        // Removing it removes the dev build's copy too, so it can't come back.
        chosen.delete(&key()).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), None);
        let file = EncryptedFileStore::open(dir.path()).unwrap();
        assert_eq!(file.load(&key()).unwrap(), None);
    }

    #[test]
    fn a_damaged_dev_file_counts_as_no_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        EncryptedFileStore::open(dir.path()).unwrap();
        std::fs::write(dir.path().join(crate::file_store::SECRETS_FILE), b"x").unwrap();
        let (mock, _) = store();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), None);
        assert!(chosen.delete(&key()).is_err(), "a damaged fallback cannot confirm deletion");
        // A damaged key: the file can't even be opened.
        std::fs::write(dir.path().join(crate::file_store::KEY_FILE), b"x").unwrap();
        let (mock, _) = store();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), None);
    }

    struct UnreliableKeychain { fails_read: bool }
    impl SecretStore for UnreliableKeychain {
        fn save(&self, _: &SecretRef, _: &Secret) -> Result<(), SecretStoreError> { Ok(()) }
        fn load(&self, _: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
            if self.fails_read { Err(SecretStoreError::Unavailable("fixture read failure".into())) }
            else { Ok(None) }
        }
        fn delete(&self, _: &SecretRef) -> Result<(), SecretStoreError> { Ok(()) }
    }

    #[test]
    fn unverified_keychain_writes_keep_the_file_copy() {
        for fails_read in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let file = EncryptedFileStore::open(dir.path()).unwrap();
            let secret = Secret::ApiKey { key: "fixture".into() };
            file.save(&key(), &secret).unwrap();
            let chosen = KeychainFirst { keychain: UnreliableKeychain { fails_read }, dev: Some(file) };
            assert!(chosen.save(&key(), &secret).is_err());
            assert_eq!(chosen.dev.as_ref().unwrap().load(&key()).unwrap(), Some(secret));
            chosen.delete(&key()).unwrap();
            assert_eq!(chosen.dev.as_ref().unwrap().load(&key()).unwrap(), None);
        }
    }

    #[test]
    fn loading_a_keychain_item_preserves_a_different_dev_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let file = EncryptedFileStore::open(dir.path()).unwrap();
        let old = Secret::ApiKey { key: "old-fixture".into() };
        let new = Secret::ApiKey { key: "new-fixture".into() };
        file.save(&key(), &old).unwrap();
        let (mock, keychain) = store();
        keychain.save(&key(), &new).unwrap();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), Some(new));
        assert_eq!(file.load(&key()).unwrap(), Some(old));
    }

    #[test]
    fn cleanup_failure_does_not_discard_a_verified_keychain_save() {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::ApiKey { key: "fixture".into() };
        EncryptedFileStore::open(dir.path()).unwrap().save(&key(), &secret).unwrap();
        let (mock, _) = store();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        std::fs::write(dir.path().join(crate::file_store::SECRETS_FILE), b"damaged").unwrap();
        chosen.save(&key(), &secret).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), Some(secret));
        assert_eq!(std::fs::read(dir.path().join(crate::file_store::SECRETS_FILE)).unwrap(), b"damaged");
    }

    #[test]
    fn failed_file_deletion_is_reported_and_preserves_the_keychain_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let secret = Secret::ApiKey { key: "fixture".into() };
        let file = EncryptedFileStore::open(dir.path()).unwrap();
        file.save(&key(), &secret).unwrap();
        let (mock, keychain) = store();
        keychain.save(&key(), &secret).unwrap();
        let chosen = choose_store(false, false, || Ok(mock), dir.path()).unwrap();
        // Force the atomic replacement to fail without changing filesystem permissions.
        std::fs::create_dir(dir.path().join("secrets.bin.tmp")).unwrap();
        assert!(chosen.delete(&key()).is_err());
        assert_eq!(keychain.load(&key()).unwrap(), Some(secret.clone()));
        assert_eq!(file.load(&key()).unwrap(), Some(secret));
        std::fs::remove_dir(dir.path().join("secrets.bin.tmp")).unwrap();
        chosen.delete(&key()).unwrap();
        assert_eq!(chosen.load(&key()).unwrap(), None);
        assert_eq!(file.load(&key()).unwrap(), None);
    }

    /// A credential store that refuses every entry.
    #[derive(Debug)]
    struct RejectingStore;

    impl CredentialStoreApi for RejectingStore {
        fn vendor(&self) -> String {
            "test".into()
        }
        fn id(&self) -> String {
            "rejecting".into()
        }
        fn build(
            &self,
            _: &str,
            _: &str,
            _: Option<&std::collections::HashMap<&str, &str>>,
        ) -> keyring_core::Result<keyring_core::Entry> {
            Err(KeyringError::Invalid("service".into(), "empty".into()))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
}
