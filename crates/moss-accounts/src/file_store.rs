//! Encrypted file fallback for Linux setups with no Secret Service.
//!
//! Secrets are sealed with XChaCha20-Poly1305 under a random key kept in a separate
//! file that only the user can read (mode 0600). This keeps secrets out of plain-text
//! config, backups that copy one file and casual `grep`s; it can't protect against
//! someone who can already read the user's files, which is why the OS keychain is
//! always preferred.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

use crate::secret::{Secret, SecretRef, SecretStore, SecretStoreError};

pub(crate) const SECRETS_FILE: &str = "secrets.bin";
const LOCK_FILE: &str = "secrets.lock";
pub(crate) const KEY_FILE: &str = "secrets.key";
const NONCE_LEN: usize = 24;

#[derive(Serialize, Deserialize)]
struct Item {
    key: SecretRef,
    secret: Secret,
}

/// Secrets in an encrypted file. See the module docs for what it does and doesn't protect.
pub struct EncryptedFileStore {
    path: PathBuf,
    cipher: XChaCha20Poly1305,
    lock: Mutex<()>,
}

impl EncryptedFileStore {
    /// Opens the store in `dir`, creating the folder and a new key on first use.
    pub fn open(dir: &Path) -> Result<Self, SecretStoreError> {
        fs::create_dir_all(dir).map_err(io)?;
        let _file_lock = lock_file(&dir.join(LOCK_FILE))?;
        #[cfg(unix)]
        {
            for name in [KEY_FILE, SECRETS_FILE] {
                let path = dir.join(name);
                restrict_existing_file(&path)?;
                restrict_existing_file(&temporary_path(&path))?;
            }
            restrict_existing_file(&dir.join("secrets.tmp"))?;
        }
        let key = read_or_create_key(&dir.join(KEY_FILE))?;
        Ok(Self {
            path: dir.join(SECRETS_FILE),
            cipher: XChaCha20Poly1305::new(Key::from_slice(&key)),
            lock: Mutex::new(()),
        })
    }

    /// Deletes only the exact sign-in just verified in the Keychain. A different dev
    /// sign-in may be newer, so it must survive. The comparison and rewrite share a lock.
    pub(crate) fn delete_if_matches(
        &self,
        key: &SecretRef,
        secret: &Secret,
    ) -> Result<(), SecretStoreError> {
        let _guard = self.lock.lock().unwrap();
        let _file_lock = lock_file(&self.path.with_file_name(LOCK_FILE))?;
        let mut items = self.read()?;
        let before = items.len();
        items.retain(|item| &item.key != key || &item.secret != secret);
        if items.len() != before {
            self.write(&items)?;
        }
        Ok(())
    }

    fn read(&self) -> Result<Vec<Item>, SecretStoreError> {
        let sealed = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io(e)),
        };
        if sealed.len() < NONCE_LEN {
            return Err(SecretStoreError::Corrupt("file is truncated".into()));
        }
        let (nonce, body) = sealed.split_at(NONCE_LEN);
        let plain = self
            .cipher
            .decrypt(XNonce::from_slice(nonce), body)
            .map_err(|_| SecretStoreError::Corrupt("file doesn't decrypt with this key".into()))?;
        serde_json::from_slice(&plain).map_err(|e| SecretStoreError::Corrupt(e.to_string()))
    }

    fn delete_with(
        &self,
        key: &SecretRef,
        flush: impl Fn(&fs::File) -> std::io::Result<()>,
    ) -> Result<(), SecretStoreError> {
        let _guard = self.lock.lock().unwrap();
        let _file_lock = lock_file(&self.path.with_file_name(LOCK_FILE))?;
        let mut items = self.read()?;
        items.retain(|item| &item.key != key);
        // A previous rename may be visible even though its directory flush failed.
        // Persist the absence again before authorizing deletion of another credential copy.
        self.write_with(&items, flush)
    }

    fn write(&self, items: &[Item]) -> Result<(), SecretStoreError> {
        self.write_with(items, fs::File::sync_all)
    }

    fn write_with(
        &self,
        items: &[Item],
        flush: impl Fn(&fs::File) -> std::io::Result<()>,
    ) -> Result<(), SecretStoreError> {
        let plain = serde_json::to_vec(items).expect("secrets always serialise");
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| SecretStoreError::Unavailable(e.to_string()))?;
        let body = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), plain.as_slice())
            .expect("encryption can't fail");
        write_private_with(&self.path, &[nonce.as_slice(), &body].concat(), flush)
    }
}

impl SecretStore for EncryptedFileStore {
    fn save(&self, key: &SecretRef, secret: &Secret) -> Result<(), SecretStoreError> {
        let _guard = self.lock.lock().unwrap();
        let _file_lock = lock_file(&self.path.with_file_name(LOCK_FILE))?;
        let mut items = self.read()?;
        items.retain(|item| &item.key != key);
        items.push(Item {
            key: key.clone(),
            secret: secret.clone(),
        });
        self.write(&items)
    }

    fn load(&self, key: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
        let _guard = self.lock.lock().unwrap();
        let _file_lock = lock_file(&self.path.with_file_name(LOCK_FILE))?;
        Ok(self
            .read()?
            .into_iter()
            .find(|item| &item.key == key)
            .map(|item| item.secret))
    }

    fn delete(&self, key: &SecretRef) -> Result<(), SecretStoreError> {
        self.delete_with(key, fs::File::sync_all)
    }
}

/// A separate file remains stable while the encrypted file is atomically replaced.
/// Fresh descriptors make this work across independently opened stores and processes.
fn lock_file(path: &Path) -> Result<fs::File, SecretStoreError> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path).map_err(io)?;
    file.lock().map_err(io)?;
    Ok(file)
}

#[cfg(unix)]
fn restrict_existing_file(path: &Path) -> Result<(), SecretStoreError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io(error)),
    };
    if metadata.is_file() && metadata.permissions().mode() & 0o777 != 0o600 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io)?;
        fs::File::open(path).map_err(io)?.sync_all().map_err(io)?;
    }
    Ok(())
}

fn read_or_create_key(path: &Path) -> Result<[u8; 32], SecretStoreError> {
    match fs::read(path) {
        Ok(bytes) => bytes.try_into().map_err(|_| {
            SecretStoreError::Corrupt(format!("{} is not a 32-byte key", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match fs::symlink_metadata(path.with_file_name(SECRETS_FILE)) {
                Ok(_) => return Err(SecretStoreError::Corrupt(
                    "credential key missing; restore the matching key before opening this store"
                        .into(),
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io(error)),
            }
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).map_err(|e| SecretStoreError::Unavailable(e.to_string()))?;
            write_private(path, &key)?;
            Ok(key)
        }
        Err(e) => Err(io(e)),
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

/// Atomically replaces a private file. Callers hold the directory's secret-store lock.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SecretStoreError> {
    write_private_with(path, bytes, fs::File::sync_all)
}

fn write_private_with(
    path: &Path,
    bytes: &[u8],
    flush: impl Fn(&fs::File) -> std::io::Result<()>,
) -> Result<(), SecretStoreError> {
    let tmp = temporary_path(path);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&tmp).map_err(io)?;
    // Creation mode alone does not repair a temporary file left by an older build.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(io)?;
    }
    file.write_all(bytes).map_err(io)?;
    flush(&file).map_err(io)?;
    drop(file);
    fs::rename(&tmp, path).map_err(io)?;
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let directory = fs::File::open(parent).map_err(io)?;
        flush(&directory).map_err(io)?;
    }
    Ok(())
}

fn io(e: std::io::Error) -> SecretStoreError {
    SecretStoreError::Unavailable(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> SecretRef {
        SecretRef::for_account(name, "imap")
    }

    fn password(p: &str) -> Secret {
        Secret::Password { password: p.into() }
    }

    #[test]
    fn saves_loads_replaces_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(&dir.path().join("nested")).unwrap();
        assert_eq!(store.load(&key("a@x.io")).unwrap(), None);
        store.save(&key("a@x.io"), &password("one")).unwrap();
        store.save(&key("b@x.io"), &password("two")).unwrap();
        store.save(&key("a@x.io"), &password("three")).unwrap();
        assert_eq!(store.load(&key("a@x.io")).unwrap(), Some(password("three")));
        store.delete(&key("a@x.io")).unwrap();
        store.delete(&key("missing@x.io")).unwrap();
        assert_eq!(store.load(&key("a@x.io")).unwrap(), None);
        assert_eq!(store.load(&key("b@x.io")).unwrap(), Some(password("two")));
    }

    #[test]
    fn separate_instances_do_not_overwrite_each_others_updates() {
        let dir = tempfile::tempdir().unwrap();
        let first = EncryptedFileStore::open(dir.path()).unwrap();
        let second = EncryptedFileStore::open(dir.path()).unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for (label, store) in [("a", &first), ("b", &second)] {
                let barrier = &barrier;
                scope.spawn(move || {
                    let results: Vec<_> = (0..30)
                        .map(|n| {
                            barrier.wait();
                            store.save(&key(&format!("{label}{n}")), &password("fixture"))
                        })
                        .collect();
                    assert!(
                        results.iter().all(Result::is_ok),
                        "concurrent writes failed: {results:?}"
                    );
                });
            }
        });
        for label in ["a", "b"] {
            for n in 0..30 {
                assert_eq!(
                    first.load(&key(&format!("{label}{n}"))).unwrap(),
                    Some(password("fixture"))
                );
            }
        }
    }

    #[test]
    fn reopening_uses_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        EncryptedFileStore::open(dir.path())
            .unwrap()
            .save(&key("a@x.io"), &password("kept"))
            .unwrap();
        let again = EncryptedFileStore::open(dir.path()).unwrap();
        assert_eq!(again.load(&key("a@x.io")).unwrap(), Some(password("kept")));
    }

    #[test]
    fn the_file_holds_no_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        store
            .save(&key("maya@northwind.co"), &password("hunter2"))
            .unwrap();
        let raw = fs::read(dir.path().join(SECRETS_FILE)).unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("maya"));
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        EncryptedFileStore::open(dir.path())
            .unwrap()
            .save(&key("a@x.io"), &password("p"))
            .unwrap();
        for name in [KEY_FILE, SECRETS_FILE] {
            let mode = fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn reopening_repairs_permissions_from_a_restored_copy() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        store
            .save(&key("fixture@example.test"), &password("kept"))
            .unwrap();
        drop(store);
        for name in [KEY_FILE, SECRETS_FILE] {
            fs::set_permissions(dir.path().join(name), fs::Permissions::from_mode(0o644)).unwrap();
        }
        let reopened = EncryptedFileStore::open(dir.path()).unwrap();
        assert_eq!(
            reopened.load(&key("fixture@example.test")).unwrap(),
            Some(password("kept"))
        );
        for name in [KEY_FILE, SECRETS_FILE] {
            assert_eq!(
                fs::metadata(dir.path().join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600,
                "{name}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_uninspectable_secret_file_prevents_opening_or_creating_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SECRETS_FILE);
        std::os::unix::fs::symlink(SECRETS_FILE, &path).unwrap();
        assert!(EncryptedFileStore::open(dir.path()).is_err());
        assert!(!dir.path().join(KEY_FILE).exists());
    }

    #[test]
    fn a_missing_key_never_replaces_the_key_for_existing_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        store
            .save(&key("fixture@example.test"), &password("recoverable"))
            .unwrap();
        drop(store);
        let key_path = dir.path().join(KEY_FILE);
        let original_key = fs::read(&key_path).unwrap();
        let original_ciphertext = fs::read(dir.path().join(SECRETS_FILE)).unwrap();
        fs::remove_file(&key_path).unwrap();
        assert!(matches!(
            EncryptedFileStore::open(dir.path()),
            Err(SecretStoreError::Corrupt(_))
        ));
        assert!(!key_path.exists());
        assert_eq!(
            fs::read(dir.path().join(SECRETS_FILE)).unwrap(),
            original_ciphertext
        );
        fs::write(&key_path, original_key).unwrap();
        let recovered = EncryptedFileStore::open(dir.path()).unwrap();
        assert_eq!(
            recovered.load(&key("fixture@example.test")).unwrap(),
            Some(password("recoverable"))
        );
    }

    #[test]
    fn key_and_ciphertext_writes_leave_legacy_temporary_data_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("secrets.tmp");
        fs::write(&legacy, b"legacy ciphertext pending").unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy ciphertext pending");
        store
            .save(&key("fixture@example.test"), &password("new credential"))
            .unwrap();
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy ciphertext pending");
        assert_eq!(
            store.load(&key("fixture@example.test")).unwrap(),
            Some(password("new credential"))
        );
    }

    #[test]
    fn a_different_key_cannot_read_the_file() {
        let dir = tempfile::tempdir().unwrap();
        EncryptedFileStore::open(dir.path())
            .unwrap()
            .save(&key("a@x.io"), &password("p"))
            .unwrap();
        fs::write(dir.path().join(KEY_FILE), [7u8; 32]).unwrap();
        let other = EncryptedFileStore::open(dir.path()).unwrap();
        assert!(matches!(
            other.load(&key("a@x.io")),
            Err(SecretStoreError::Corrupt(_))
        ));
    }

    #[test]
    fn damaged_files_are_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        fs::write(dir.path().join(SECRETS_FILE), b"short").unwrap();
        assert!(matches!(
            store.load(&key("a@x.io")),
            Err(SecretStoreError::Corrupt(_))
        ));

        // Properly sealed, but not a list of items.
        let nonce = [1u8; NONCE_LEN];
        let body = store
            .cipher
            .encrypt(XNonce::from_slice(&nonce), b"{}".as_slice())
            .unwrap();
        fs::write(
            dir.path().join(SECRETS_FILE),
            [nonce.as_slice(), &body].concat(),
        )
        .unwrap();
        assert!(matches!(
            store.save(&key("a@x.io"), &password("p")),
            Err(SecretStoreError::Corrupt(_))
        ));
        assert!(matches!(
            store.delete(&key("a@x.io")),
            Err(SecretStoreError::Corrupt(_))
        ));

        fs::write(dir.path().join(KEY_FILE), b"too short").unwrap();
        assert!(matches!(
            EncryptedFileStore::open(dir.path()),
            Err(SecretStoreError::Corrupt(_))
        ));
    }

    #[test]
    fn unreadable_paths_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        // The key path is a folder, so it can't be read as a file.
        fs::create_dir(dir.path().join(KEY_FILE)).unwrap();
        assert!(matches!(
            EncryptedFileStore::open(dir.path()),
            Err(SecretStoreError::Unavailable(_))
        ));

        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        fs::create_dir(dir.path().join(SECRETS_FILE)).unwrap();
        assert!(matches!(
            store.load(&key("a@x.io")),
            Err(SecretStoreError::Unavailable(_))
        ));

        // The temporary file can't be created, so saving fails before touching anything.
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        fs::create_dir(dir.path().join(format!("{SECRETS_FILE}.tmp"))).unwrap();
        assert!(matches!(
            store.save(&key("a@x.io"), &password("p")),
            Err(SecretStoreError::Unavailable(_))
        ));
    }

    #[test]
    fn a_relative_data_directory_uses_the_current_folder() {
        const CHILD: &str = "MOSS_RELATIVE_CREDENTIAL_TEST";
        if std::env::var_os(CHILD).is_some() {
            let store = EncryptedFileStore::open(Path::new("")).unwrap();
            store
                .save(&key("fixture@example.test"), &password("kept"))
                .unwrap();
            assert_eq!(
                store.load(&key("fixture@example.test")).unwrap(),
                Some(password("kept"))
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "file_store::tests::a_relative_data_directory_uses_the_current_folder",
            ])
            .env(CHILD, "1")
            .current_dir(dir.path())
            .output()
            .unwrap();
        let output = String::from_utf8_lossy(&result.stdout);
        assert!(result.status.success(), "{output}");
        assert!(dir.path().join(KEY_FILE).is_file());
        assert!(dir.path().join(SECRETS_FILE).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn deletion_retries_do_not_acknowledge_an_unflushed_absence() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        let account = key("fixture@example.test");
        store
            .save(&account, &password("kept until durable deletion"))
            .unwrap();
        let fail_directory = |file: &fs::File| {
            if file.metadata()?.is_dir() {
                Err(std::io::Error::other("fixture directory flush failure"))
            } else {
                file.sync_all()
            }
        };
        assert!(store.delete_with(&account, fail_directory).is_err());
        assert_eq!(
            store.load(&account).unwrap(),
            None,
            "the rename is visible despite its unconfirmed durability"
        );
        assert!(
            store.delete_with(&account, fail_directory).is_err(),
            "visible absence must not turn a retry into a false durability acknowledgement"
        );
        store.delete(&account).unwrap();
        assert_eq!(store.load(&account).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn unflushed_credentials_are_never_acknowledged_as_saved() {
        for fail_on_directory in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(SECRETS_FILE);
            fs::write(&path, b"old credential data").unwrap();
            let result = write_private_with(&path, b"new credential data", |file| {
                if file.metadata()?.is_dir() == fail_on_directory {
                    Err(std::io::Error::other("fixture flush failure"))
                } else {
                    file.sync_all()
                }
            });
            assert!(matches!(result, Err(SecretStoreError::Unavailable(_))));
            let expected = if fail_on_directory {
                b"new credential data"
            } else {
                b"old credential data"
            };
            assert_eq!(
                fs::read(&path).unwrap(),
                expected,
                "replacement must wait for the file flush"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn replacing_a_secret_does_not_preserve_public_temporary_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        let path = dir.path().join(SECRETS_FILE);
        let temp = temporary_path(&path);
        fs::write(&temp, b"stale temporary data").unwrap();
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o644)).unwrap();
        store
            .save(&key("fixture@example.test"), &password("credential data"))
            .unwrap();
        assert_eq!(
            store.load(&key("fixture@example.test")).unwrap(),
            Some(password("credential data"))
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!temp.exists());
    }

    #[test]
    fn a_failed_rename_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        // A non-empty folder where the file should go can't be replaced by a rename.
        fs::create_dir_all(dir.path().join(SECRETS_FILE).join("x")).unwrap();
        let err = store.write(&[]).unwrap_err();
        assert!(matches!(err, SecretStoreError::Unavailable(_)));
    }
}
