//! What Moss says when it can't open your mail at launch, instead of quietly not appearing.
//! The dialog itself is shown by `run`; this decides what it says.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Why Moss couldn't start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The mail was last opened by a newer Moss.
    Newer,
    /// Another Moss has this mail open.
    InUse,
    /// The mail database at `database` couldn't be opened or updated. `unfinished` when it is
    /// still to be updated for this Moss: an update failed, perhaps partway. `copy` is the copy
    /// kept from just before Moss last updated it, when there is one.
    Mail {
        database: PathBuf,
        unfinished: bool,
        copy: Option<PathBuf>,
    },
    /// The Keychain is locked, or Moss isn't allowed to use it.
    Keychain,
    /// Anything else, in its own words: a data folder that can't be made private.
    Other(String),
}

/// Opens the passwords with `secrets`, then the mail in `data`, as Moss does at launch.
pub fn open(
    data: &Path,
    secrets: impl FnOnce(&Path) -> Result<Box<dyn moss_accounts::SecretStore>, Box<dyn Error>>,
    events: Arc<dyn moss_engine::Events>,
    roots: moss_providers::TlsRoots,
) -> Result<(Arc<dyn moss_accounts::SecretStore>, moss_engine::Engine), Failure> {
    let secrets: Arc<dyn moss_accounts::SecretStore> =
        Arc::from(secrets(data).map_err(|error| Failure::of_secrets(error.as_ref()))?);
    crate::leftovers::forget_other_ais(data, secrets.as_ref());
    let engine = crate::engine(data, secrets.clone(), events, roots)
        .map_err(|error| Failure::of_mail(error.as_ref(), &data.join(crate::DATABASE)))?;
    Ok((secrets, engine))
}

impl Failure {
    /// Why getting at the passwords failed: the Keychain, or the data folder they are kept by.
    pub fn of_secrets(error: &(dyn Error + 'static)) -> Self {
        match error.downcast_ref::<moss_accounts::SecretStoreError>() {
            Some(_) => Self::Keychain,
            None => Self::Other(error.to_string()),
        }
    }

    /// Why opening the mail database at `database` failed.
    pub fn of_mail(error: &(dyn Error + 'static), database: &Path) -> Self {
        match error.downcast_ref::<moss_store::Error>() {
            Some(moss_store::Error::TooNew { .. }) => Self::Newer,
            Some(moss_store::Error::InUse) => Self::InUse,
            Some(moss_store::Error::Invalid(why)) => Self::Other(why.clone()),
            Some(_) => {
                // Moss removes an older copy before it makes a new one, so one that is there
                // is from just before the latest update.
                let copy = moss_store::safety_copy_path(database);
                Self::Mail {
                    database: database.to_path_buf(),
                    unfinished: moss_store::update_unfinished(database),
                    copy: copy.is_file().then_some(copy),
                }
            }
            None => Self::Other(error.to_string()),
        }
    }

    /// The dialog's title and text.
    pub fn message(&self) -> (&'static str, String) {
        match self {
            Self::Newer => (
                "Update Moss",
                "This mail was last opened by a newer Moss. Update Moss to open it.".into(),
            ),
            Self::InUse => (
                "Moss can’t open your mail",
                "Another Moss is using this mail. Quit it first.".into(),
            ),
            Self::Mail { database, unfinished, copy } => {
                let mut text = match unfinished {
                    true => "Moss couldn’t finish updating your mail file for this version of Moss. \
                        Part of the update may already be in it.",
                    false => "Your mail file may be damaged. Moss couldn’t open it.",
                }
                .to_string();
                if let Some(copy) = copy {
                    text += &format!(
                        "\n\nA copy from just before the last update is at:\n{}",
                        copy.display()
                    );
                }
                text += &format!("\n\nYour mail file is at:\n{}", database.display());
                ("Moss can’t open your mail", text)
            }
            Self::Keychain => (
                "Moss can’t use the Keychain",
                "Moss keeps your passwords in the Keychain and couldn’t open it. \
                 Unlock the Keychain, or allow Moss to use it, then open Moss again."
                    .into(),
            ),
            Self::Other(why) => ("Moss can’t start", why.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::tests::Quiet;

    /// Passwords no one has saved yet, as the Keychain would hand them over.
    fn keychain(_: &Path) -> Result<Box<dyn moss_accounts::SecretStore>, Box<dyn Error>> {
        Ok(Box::new(moss_accounts::MemorySecretStore::new()))
    }

    /// Opens the mail in `data` as Moss does at launch, and says why it couldn't.
    fn failure(data: &Path) -> Failure {
        open(data, keychain, Arc::new(Quiet), crate::TlsRoots::Platform)
            .err()
            .unwrap()
    }

    fn data() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        (dir, data)
    }

    #[test]
    fn mail_from_a_newer_moss_asks_for_an_update() {
        let (_dir, data) = data();
        rusqlite::Connection::open(data.join(crate::DATABASE))
            .unwrap()
            .pragma_update(None, "user_version", 9999)
            .unwrap();
        let failure = failure(&data);
        assert_eq!(failure, Failure::Newer);
        assert_eq!(
            failure.message(),
            (
                "Update Moss",
                "This mail was last opened by a newer Moss. Update Moss to open it.".into()
            )
        );
    }

    #[test]
    fn a_damaged_mail_file_says_so_and_where_it_is() {
        let (_dir, data) = data();
        let database = data.join(crate::DATABASE);
        std::fs::write(
            &database,
            b"not a database, just some bytes that are long enough to look like a file",
        )
        .unwrap();
        // Something there that isn't a copy Moss made is not offered as one.
        std::fs::create_dir(moss_store::safety_copy_path(&database)).unwrap();
        let failure = failure(&data);
        assert_eq!(
            failure,
            Failure::Mail {
                database: database.clone(),
                unfinished: false,
                copy: None
            }
        );
        let (title, text) = failure.message();
        assert_eq!(title, "Moss can’t open your mail");
        assert!(text.starts_with("Your mail file may be damaged. Moss couldn’t open it."), "{text}");
        assert!(!text.contains("not changed"), "{text}");
        assert!(text.contains(&database.display().to_string()), "{text}");
        assert!(!text.contains("copy"), "{text}");
    }

    #[test]
    fn a_failed_update_says_it_may_be_partly_done_and_points_to_the_copy_from_just_before_it() {
        let (_dir, data) = data();
        let database = data.join(crate::DATABASE);
        let copy = moss_store::safety_copy_path(&database);
        // A copy from an earlier update, which this one replaces.
        std::fs::write(&copy, b"weeks old").unwrap();
        {
            // From an older Moss, with something in the way of the next update.
            let conn = rusqlite::Connection::open(&database).unwrap();
            conn.execute_batch("CREATE TABLE settings (x); PRAGMA user_version = 1;").unwrap();
        }
        let failure = failure(&data);
        assert_eq!(
            failure,
            Failure::Mail {
                database: database.clone(),
                unfinished: true,
                copy: Some(copy.clone())
            }
        );
        assert_ne!(std::fs::read(&copy).unwrap(), b"weeks old");
        let (_, text) = failure.message();
        assert!(
            text.starts_with("Moss couldn’t finish updating your mail file for this version of Moss. Part of the update may already be in it."),
            "{text}"
        );
        assert!(!text.contains("not changed"), "{text}");
        assert!(
            text.contains(&format!(
                "A copy from just before the last update is at:\n{}",
                copy.display()
            )),
            "{text}"
        );
        assert!(text.ends_with(&format!("Your mail file is at:\n{}", database.display())), "{text}");
    }

    #[test]
    fn mail_another_moss_is_using_says_so() {
        let (_dir, data) = data();
        let mut first = moss_store::Store::open(data.join(crate::DATABASE)).unwrap();
        first.claim_delivery_session().unwrap();
        let failure = failure(&data);
        assert_eq!(failure, Failure::InUse);
        assert_eq!(
            failure.message(),
            ("Moss can’t open your mail", "Another Moss is using this mail. Quit it first.".into())
        );
    }

    #[test]
    fn mail_another_moss_has_open_is_not_updated_under_it() {
        let (_dir, data) = data();
        let database = data.join(crate::DATABASE);
        rusqlite::Connection::open(&database).unwrap().pragma_update(None, "user_version", 1).unwrap();
        // The other Moss's lock on this mail.
        let lock = std::fs::File::create(data.join("moss.session.lock")).unwrap();
        lock.try_lock().unwrap();
        assert_eq!(failure(&data), Failure::InUse);
        let version: u32 = rusqlite::Connection::open(&database).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 1);
        drop(lock);
    }

    #[test]
    fn a_data_folder_moss_cannot_use_says_why() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the folder should be.
        let data = dir.path().join("data");
        std::fs::write(&data, b"").unwrap();
        assert!(matches!(failure(&data), Failure::Other(why) if !why.is_empty()));
    }

    #[test]
    fn mail_that_opens_starts_moss() {
        let (_dir, data) = data();
        let (secrets, engine) =
            open(&data, keychain, Arc::new(Quiet), crate::TlsRoots::Platform).unwrap();
        assert!(data.join(crate::DATABASE).exists());
        assert!(engine.accounts().unwrap().is_empty());
        let nothing = moss_accounts::SecretRef {
            service: "fixture".into(),
            account: "fixture".into(),
        };
        assert!(
            secrets.load(&nothing).unwrap().is_none(),
            "the passwords it opened come with it"
        );
    }

    #[test]
    fn a_keychain_moss_cannot_use_stops_the_start_before_the_mail_is_opened() {
        let (_dir, data) = data();
        let locked = |_: &Path| -> Result<Box<dyn moss_accounts::SecretStore>, Box<dyn Error>> {
            Err(Box::new(moss_accounts::SecretStoreError::Unavailable(
                "locked".into(),
            )))
        };
        let failure = open(&data, locked, Arc::new(Quiet), crate::TlsRoots::Platform)
            .err()
            .unwrap();
        assert_eq!(failure, Failure::Keychain);
        assert!(!data.join(crate::DATABASE).exists());
    }

    #[test]
    fn a_keychain_moss_cannot_use_asks_to_unlock_or_allow_it() {
        let refused =
            moss_accounts::SecretStoreError::Unavailable("User interaction is not allowed.".into());
        let boxed: Box<dyn Error> = Box::new(refused);
        let failure = Failure::of_secrets(boxed.as_ref());
        assert_eq!(failure, Failure::Keychain);
        let (title, text) = failure.message();
        assert_eq!(title, "Moss can’t use the Keychain");
        assert!(
            text.contains("Unlock the Keychain, or allow Moss to use it"),
            "{text}"
        );
        // The folder the passwords file would go in is not the Keychain.
        let folder: Box<dyn Error> = Box::new(std::io::Error::other(
            "Moss could not make its data directory private.",
        ));
        assert_eq!(
            Failure::of_secrets(folder.as_ref()),
            Failure::Other("Moss could not make its data directory private.".into())
        );
    }

    #[test]
    fn nothing_it_says_has_an_em_dash() {
        let failures = [
            Failure::Newer,
            Failure::InUse,
            Failure::Mail {
                database: "/m/moss.db".into(),
                unfinished: true,
                copy: Some("/m/moss.db.before-update".into()),
            },
            Failure::Mail {
                database: "/m/moss.db".into(),
                unfinished: false,
                copy: None,
            },
            Failure::Keychain,
            Failure::Other("x".into()),
        ];
        for failure in failures {
            let (title, text) = failure.message();
            assert!(!title.is_empty() && !text.is_empty(), "{failure:?}");
            assert!(!title.contains('—') && !text.contains('—'), "{failure:?}");
        }
    }
}
