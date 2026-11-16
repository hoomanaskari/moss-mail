//! Exclusive ownership of delivery recovery for a database.

use std::path::Path;

use crate::{Error, Result, Store};

/// The engine owns the lock, even if a spawn briefly duplicates the file description.
/// Store declares this after its connection so the connection closes before unlocking.
pub(crate) struct SessionLock(std::fs::File);

/// Where a store is with the session.
pub(crate) enum Session {
    /// Opening it updated (or made) the database. Nothing is held yet.
    Updated,
    /// It holds the lock, until it is dropped.
    Held { _lock: SessionLock },
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl SessionLock {
    /// Takes the lock for the database at `database`, which must exist: [`Error::InUse`] while
    /// another Moss holds it. The OS lets go of a Moss's lock when it stops, crashes included,
    /// so a lock file left behind never blocks the next one.
    pub(crate) fn take(database: &Path) -> Result<Self> {
        let path = std::fs::canonicalize(database)
            .map_err(|_| Error::Invalid("Moss cannot locate its mail database for recovery.".into()))?
            .with_extension("session.lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(path)
            .map_err(|_| Error::Invalid("Moss cannot lock this mail data folder.".into()))?;
        // Busy is another Moss; any other failure (which this Mac has no way to bring about in a
        // test) fails closed as before.
        file.try_lock().map_err(|why| if matches!(why, std::fs::TryLockError::WouldBlock) { Error::InUse } else { Error::Invalid("Moss cannot lock this mail data folder.".into()) })?;
        Ok(Self(file))
    }
}

impl Store {
    /// Retain the OS lock for this store's lifetime. A process crash releases it.
    /// Other read connections remain possible, but another engine cannot recover mail.
    /// Owning the folder from here on, it clears what an earlier Moss left in it: work files
    /// from a backup or update that stopped partway, and the copy from before the last update
    /// once a launch has opened the mail with nothing to update.
    pub fn claim_delivery_session(&mut self) -> Result<()> {
        let updated = match self.delivery_session {
            Some(Session::Held { .. }) => return Ok(()),
            Some(Session::Updated) => true,
            None => false,
        };
        let Some(path) = self.conn.path().filter(|path| !path.is_empty()).map(std::path::PathBuf::from) else { return Ok(()); };
        self.delivery_session = Some(Session::Held { _lock: SessionLock::take(&path)? });
        crate::backup::clear_leftovers(&path);
        if !updated {
            let _ = crate::forget_safety_copy(&path);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn only_one_engine_can_own_recovery_until_its_store_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let mut first = Store::open(&path).unwrap();
        let mut second = Store::open(&path).unwrap();
        first.claim_delivery_session().unwrap();
        first.claim_delivery_session().unwrap();
        assert!(matches!(second.claim_delivery_session(), Err(crate::Error::InUse)));
        assert_eq!(crate::Error::InUse.to_string(), "Another Moss is using this mail. Quit it first.");
        drop(first);
        second.claim_delivery_session().unwrap();
        let mut third = Store::open(&path).unwrap();
        assert!(third.claim_delivery_session().is_err());
        drop(second);
        third.claim_delivery_session().unwrap();
    }

    #[test]
    fn a_database_whose_file_is_gone_cannot_own_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let mut store = Store::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(store.claim_delivery_session(), Err(crate::Error::Invalid(_))));
    }

    #[test]
    fn a_duplicated_descriptor_does_not_extend_engine_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let mut owner = Store::open(&path).unwrap();
        owner.claim_delivery_session().unwrap();
        // A process launch can briefly inherit the same open-file description before exec.
        let Some(super::Session::Held { _lock: lock }) = &owner.delivery_session else { panic!("held") };
        let inherited = lock.0.try_clone().unwrap();
        let mut next = Store::open(&path).unwrap();
        assert!(next.claim_delivery_session().is_err());
        drop(owner);
        next.claim_delivery_session().unwrap();
        drop(inherited);
        let mut third = Store::open(&path).unwrap();
        assert!(third.claim_delivery_session().is_err(), "closing an old descriptor must not release the new owner's lock");
    }

    #[test]
    fn ownership_is_enforced_across_processes() {
        const PATH: &str = "MOSS_TEST_DELIVERY_LOCK_PATH";
        const BUSY: &str = "MOSS_TEST_DELIVERY_LOCK_BUSY";
        if let Some(path) = std::env::var_os(PATH) {
            let mut store = Store::open(path).unwrap();
            assert_eq!(store.claim_delivery_session().is_err(), std::env::var_os(BUSY).is_some());
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let mut owner = Store::open(&path).unwrap();
        owner.claim_delivery_session().unwrap();
        let child = |busy: bool| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "session::tests::ownership_is_enforced_across_processes", "--nocapture"])
                .env(PATH, &path).env_remove(BUSY);
            if busy { command.env(BUSY, "1"); }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
        };
        child(true);
        drop(owner);
        child(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path.with_extension("session.lock")).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn a_lock_that_cannot_be_created_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        drop(Store::open(&path).unwrap());
        let mut store = Store::open(&path).unwrap();
        std::fs::remove_file(path.with_extension("session.lock")).unwrap();
        std::fs::create_dir(path.with_extension("session.lock")).unwrap();
        assert!(store.claim_delivery_session().is_err());
        let mut memory = Store::open_in_memory().unwrap();
        assert!(memory.claim_delivery_session().is_ok());
    }

    /// Names in `dir`, sorted.
    fn names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn owning_the_folder_clears_work_files_a_stopped_backup_or_update_left() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let path = data.join("moss.db");
        drop(Store::open(&path).unwrap());
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, b"not Moss's").unwrap();
        // A whole copy of the mail, as a backup or an update stopped by a crash leaves it.
        std::fs::write(data.join(".moss-backup-work-a1"), b"unscrubbed mail").unwrap();
        std::fs::write(data.join(".moss-safety-copy-b2"), b"half a copy").unwrap();
        // The folder an older Moss kept its backup work file in.
        std::fs::create_dir(data.join(".moss-backup-work-c3")).unwrap();
        std::fs::write(data.join(".moss-backup-work-c3/.tmpd4"), b"unscrubbed mail").unwrap();
        // Only Moss's own files go: never what a link points at, nor other names.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, data.join(".moss-safety-copy-link")).unwrap();
            std::fs::create_dir(data.join(".moss-backup-work-e5")).unwrap();
            std::os::unix::fs::symlink(&outside, data.join(".moss-backup-work-e5/inside")).unwrap();
            std::os::unix::fs::symlink(dir.path(), data.join(".moss-backup-work-f6")).unwrap();
        }
        std::fs::write(data.join(".moss-something-else"), b"kept").unwrap();
        std::fs::write(data.join("notes.txt"), b"kept").unwrap();
        let mut store = Store::open(&path).unwrap();
        store.claim_delivery_session().unwrap();
        let mut kept = vec![".moss-something-else", "moss.db", "moss.db-shm", "moss.db-wal", "moss.session.lock", "notes.txt"];
        #[cfg(unix)]
        kept.extend([".moss-backup-work-e5", ".moss-backup-work-f6", ".moss-safety-copy-link"]);
        kept.sort();
        assert_eq!(names(&data), kept);
        assert_eq!(std::fs::read(&outside).unwrap(), b"not Moss's");
        assert_eq!(names(dir.path()), ["data", "outside.txt"]);
        #[cfg(unix)]
        assert_eq!(names(&data.join(".moss-backup-work-e5")), ["inside"]);
    }
}
