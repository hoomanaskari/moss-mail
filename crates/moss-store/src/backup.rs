//! A verified recovery archive without application sign-in settings. Restore is separate.
use std::{path::Path, io::{Read, Seek, Write}, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use rusqlite::{Connection, backup::{Backup, StepResult}};
use sha2::{Digest, Sha256};

/// Archives cannot be used directly as live databases, including by older builds.
pub(crate) const ARCHIVE_VERSION: u32 = 2_147_483_647;
pub(crate) const ARCHIVE_ID: u32 = 0x4d4f5342;
use crate::Store;

#[derive(Debug)]
pub struct BackupInfo { pub bytes: u64, pub sha256: String }
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("The backup could not be written.")]
    Io(#[from] std::io::Error),
    #[error("The backup database could not be verified.")]
    Sqlite(#[from] rusqlite::Error),
    #[error("The backup did not pass verification.")]
    Invalid,
    #[error("The backup operation was cancelled.")]
    Cancelled,
    #[error("The backup exceeds the configured limits.")]
    Limit,
    #[error("This backup format or schema is not supported by this version of Moss.")]
    Unsupported,
    /// Written, but reading it back from where it was saved failed.
    #[error("The backup was saved, but reading it back failed, so it can't be trusted. Delete it and make a new one.")]
    Unverified,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupProgress { Preparing, Copying {copied_pages:u32, total_pages:u32}, Checking, Hashing, Publishing }
pub struct BackupControl { pub max_bytes:u64, pub cancelled:Arc<AtomicBool> }
impl Default for BackupControl {
    fn default() -> Self { Self {max_bytes:64*1024*1024*1024, cancelled:Arc::new(AtomicBool::new(false))} }
}
/// A separate read-only connection for a background archive job.
pub struct BackupSource { conn: Connection }
impl Store {
    /// Open a read-only source for a background worker, without migrations or session
    /// ownership. The application must retain control of its private data directory.
    pub fn backup_source(&self) -> Result<BackupSource, BackupError> {
        let path = self.conn.path().filter(|path| !path.is_empty()).ok_or(BackupError::Unsupported)?;
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r|r.get(0))?;
        if version != crate::schema::MIGRATIONS.len() as u32 { return Err(BackupError::Unsupported); }
        Ok(BackupSource {conn})
    }
}
impl BackupSource {
    pub fn create(&mut self, destination: &Path, control: BackupControl, progress: impl FnMut(BackupProgress)) -> Result<BackupInfo, BackupError> {
        create_archive(&self.conn, destination, control, progress)
    }
}
impl Store {
    /// Publish a verified archive without replacing an existing file. The caller must
    /// keep it private: it contains mail, drafts and unresolved recovery evidence.
    pub fn create_backup(&mut self, destination: &Path) -> Result<BackupInfo, BackupError> {
        self.create_backup_controlled(destination, BackupControl::default(), |_| {})
    }

    /// Own the connection while its read transaction is pinned. Callbacks cannot borrow
    /// this Store and accidentally write into a transaction that will be rolled back.
    pub fn create_backup_controlled(&mut self, destination: &Path, control: BackupControl, progress: impl FnMut(BackupProgress)) -> Result<BackupInfo, BackupError> {
        create_archive(&self.conn, destination, control, progress)
    }
}

fn create_archive(source: &Connection, destination: &Path, control: BackupControl, mut progress: impl FnMut(BackupProgress)) -> Result<BackupInfo, BackupError> {
    report(&control, &mut progress, BackupProgress::Preparing)?;
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    // Unscrubbed SQLite pages must stay within the application's private storage.
    // Destination folders may be watched by cloud-sync software.
    // The data folder is private, and the file is the owner's only. A crash leaves it behind
    // under this name for the next launch to clear.
    let source_parent = source.path().filter(|path| !path.is_empty()).and_then(|path| Path::new(path).parent());
    let staging = source_parent.map_or_else(std::env::temp_dir, Path::to_path_buf);
    let mut file = tempfile::Builder::new().prefix(WORK).tempfile_in(staging)?;
    let mut target = Connection::open(file.path())?;
    target.pragma_update(None, "synchronous", "FULL")?;
    target.pragma_update(None, "fullfsync", true)?;
    cancel_queries(&mut target, control.cancelled.clone())?;
    cancellation_result(copy_snapshot(source, &mut target, &control, &mut progress), &control.cancelled)?;
    report(&control, &mut progress, BackupProgress::Checking)?;
    cancellation_result(finish_archive(&target), &control.cancelled)?;
    drop(target);
    file.as_file().sync_all()?;
    report(&control, &mut progress, BackupProgress::Hashing)?;
    let info = fingerprint(file.as_file_mut(), &control.cancelled)?;
    check_size(info.bytes, control.max_bytes)?;
    report(&control, &mut progress, BackupProgress::Publishing)?;
    // Always the same name, so one a crash left in the folder (perhaps a synced one) goes
    // before the next backup there. Made new, so nothing in its place is written through.
    clear(parent, PUBLISHING);
    let mut output = tempfile::Builder::new().prefix(PARTIAL).rand_bytes(0).tempfile_in(parent)?;
    file.as_file_mut().rewind()?;
    let mut buffer = [0u8; 65_536];
    loop {
        check_cancelled(&control.cancelled)?;
        let count = file.read(&mut buffer)?;
        if count == 0 { break; }
        output.write_all(&buffer[..count])?;
    }
    output.as_file().sync_all()?;
    output.as_file_mut().rewind()?;
    let copied = fingerprint(output.as_file_mut(), &control.cancelled)?;
    if copied.bytes != info.bytes || copied.sha256 != info.sha256 { return Err(BackupError::Invalid); }
    check_cancelled(&control.cancelled)?;
    // This is the commit point. Cancellation after publication must not turn a
    // completed archive into an apparent cancellation or remove somebody's backup.
    let published = output.persist_noclobber(destination).map_err(|error|error.error)?;
    published.sync_all()?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(info)
}

/// The work file of a backup being made, in the data folder.
const WORK: &str = ".moss-backup-work-";
/// A safety copy being made, in the data folder.
const SAFETY_WORK: &str = ".moss-safety-copy-";
/// A backup being published, at its destination.
const PARTIAL: &str = ".moss-backup-partial";
/// What a backup left at its destination: the file being published, and the folder older
/// versions of Moss published it from.
const PUBLISHING: &[&str] = &[PARTIAL, ".moss-backup-publish-"];

/// Removes what a backup or a safety copy that stopped partway (a crash, a force quit) left
/// in the data folder of the database at `database`. Best effort: what can't go stays.
pub(crate) fn clear_leftovers(database: &Path) {
    let folder = database.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    clear(folder, &[WORK, SAFETY_WORK]);
}

/// Removes the regular files in `folder` whose names start with one of `prefixes`, and the
/// folders with those names holding only regular files. Links are never followed or removed.
fn clear(folder: &Path, prefixes: &[&str]) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let ours = entry.file_name().to_str().is_some_and(|name| prefixes.iter().any(|p| name.starts_with(p)));
        match entry.file_type() {
            Ok(kind) if ours && kind.is_file() => { let _ = std::fs::remove_file(entry.path()); }
            Ok(kind) if ours && kind.is_dir() => {
                clear(&entry.path(), &[""]);
                let _ = std::fs::remove_dir(entry.path());
            }
            _ => {}
        }
    }
}

/// Keeps a copy of the live database `source` at [`crate::safety_copy_path`] of `database`:
/// one consistent snapshot through SQLite's backup, WAL included, never a raw copy of the file.
/// The copy from the last update goes first, so a copy that is there is always from just before
/// this update, even when this one can't be made. Nothing is published unless all of it was copied.
pub(crate) fn keep_safety_copy(source: &Connection, database: &Path) -> Result<(), BackupError> {
    crate::forget_safety_copy(database)?;
    let destination = crate::safety_copy_path(database);
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file = tempfile::Builder::new().prefix(SAFETY_WORK).tempfile_in(parent)?;
    let mut target = Connection::open(file.path())?;
    copy_snapshot(source, &mut target, &BackupControl::default(), &mut |_| {})?;
    drop(target);
    file.as_file().sync_all()?;
    file.persist(&destination).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn copy_snapshot(source: &Connection, target: &mut Connection, control: &BackupControl, progress: &mut impl FnMut(BackupProgress)) -> Result<(), BackupError> {
    let source = source.unchecked_transaction()?;
    source.query_row("SELECT count(*) FROM sqlite_schema", [], |r|r.get::<_,i64>(0))?;
    let pages: u32 = source.query_row("PRAGMA page_count", [], |r|r.get(0))?;
    let page_size: u32 = source.query_row("PRAGMA page_size", [], |r|r.get(0))?;
    source_size(pages, page_size, control.max_bytes)?;
    let backup = Backup::new(&source, target)?;
    loop {
        check_cancelled(&control.cancelled)?;
        let step = backup.step(128)?;
        let status = backup.progress();
        report(control, progress, BackupProgress::Copying {copied_pages:(status.pagecount-status.remaining) as u32, total_pages:status.pagecount as u32})?;
        if step != StepResult::More { return require_complete(step); }
    }
}

fn check_size(bytes: u64, maximum: u64) -> Result<(), BackupError> {
    if bytes > maximum { return Err(BackupError::Limit); }
    Ok(())
}

fn source_size(pages: u32, page_size: u32, maximum: u64) -> Result<(), BackupError> {
    // SQLite's backup progress API uses signed 32-bit page counts.
    if pages > i32::MAX as u32 { return Err(BackupError::Limit); }
    check_size(u64::from(pages) * u64::from(page_size), maximum)
}

fn finish_archive(target: &Connection) -> Result<(), BackupError> {
    target.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON; DELETE FROM settings; DELETE FROM credential_cleanup;")?;
    verify_exclusions(target)?;
    let version: u32 = target.query_row("PRAGMA user_version", [], |r|r.get(0))?;
    target.execute_batch("CREATE TABLE moss_backup_manifest(format INTEGER NOT NULL, schema_version INTEGER NOT NULL);")?;
    target.execute("INSERT INTO moss_backup_manifest VALUES(1,?1)", [version])?;
    target.pragma_update(None, "application_id", ARCHIVE_ID)?;
    target.pragma_update(None, "user_version", ARCHIVE_VERSION)?;
    target.execute_batch("VACUUM")?;
    verify_database(target)
}

fn report(control: &BackupControl, progress: &mut impl FnMut(BackupProgress), event: BackupProgress) -> Result<(), BackupError> {
    check_cancelled(&control.cancelled)?;
    progress(event);
    check_cancelled(&control.cancelled)
}

pub(crate) fn check_cancelled(cancelled: &AtomicBool) -> Result<(), BackupError> {
    if cancelled.load(Ordering::Relaxed) { return Err(BackupError::Cancelled); }
    Ok(())
}

fn cancel_queries(database: &mut Connection, cancelled: Arc<AtomicBool>) -> Result<(), BackupError> {
    database.progress_handler(1000, Some(move || cancelled.load(Ordering::Relaxed)))?;
    Ok(())
}

fn cancellation_result<T>(result: Result<T, BackupError>, cancelled: &AtomicBool) -> Result<T, BackupError> {
    match result {
        Err(BackupError::Sqlite(_)) if cancelled.load(Ordering::Relaxed) => Err(BackupError::Cancelled),
        other => other,
    }
}

pub(crate) fn private_staging(parent: Option<&Path>, prefix: &str) -> Result<tempfile::TempDir, std::io::Error> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    match parent { Some(parent) => builder.tempdir_in(parent), None => builder.tempdir() }
}

fn require_complete(result: StepResult) -> Result<(), BackupError> {
    if result != StepResult::Done { return Err(BackupError::Invalid); }
    Ok(())
}

pub(crate) fn verify_exclusions(database: &Connection) -> Result<(), BackupError> {
    let retained: bool = database.query_row("SELECT EXISTS(SELECT 1 FROM settings) OR EXISTS(SELECT 1 FROM credential_cleanup)", [], |r|r.get(0))?;
    if retained { return Err(BackupError::Invalid); }
    Ok(())
}

pub(crate) fn verify_database(database: &Connection) -> Result<(), BackupError> {
    let integrity: String = database.query_row("PRAGMA integrity_check", [], |r|r.get(0))?;
    if integrity != "ok" { return Err(BackupError::Invalid); }
    let mut statement = database.prepare("PRAGMA foreign_key_check")?;
    if statement.query([])?.next()?.is_some() { return Err(BackupError::Invalid); }
    Ok(())
}

fn fingerprint(file: &mut impl Read, cancelled: &AtomicBool) -> Result<BackupInfo, BackupError> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut bytes = 0;
    loop {
        check_cancelled(cancelled)?;
        let read = file.read(&mut buffer)?;
        if read == 0 { break; }
        hash.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok(BackupInfo {bytes, sha256:format!("{:x}", hash.finalize())})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use rusqlite::Connection;

    fn assert_destination_scrubbed(path: &Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { assert_destination_scrubbed(&path); }
            else {
                let bytes = std::fs::read(&path).unwrap();
                let forbidden = b"fixture-authority-never-in-destination-51793";
                assert!(!bytes.windows(forbidden.len()).any(|part| part == forbidden), "excluded authority reached the destination");
            }
        }
    }

    /// Names in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn work_files_have_names_the_next_launch_or_backup_knows_to_clear() {
        let private = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let mut store = Store::open(private.path().join("mail.db")).unwrap();
        let mut seen = Vec::new();
        store.create_backup_controlled(&destination.path().join("backup"), BackupControl::default(), |event| {
            if event == BackupProgress::Checking { seen = names(private.path()); }
        }).unwrap();
        // What a crash then would leave: a regular file under a name of Moss's own.
        let work: Vec<&String> = seen.iter().filter(|n| !n.starts_with("mail.")).collect();
        assert_eq!(work.len(), 1, "{seen:?}");
        assert!(work[0].starts_with(".moss-backup-work-"), "{work:?}");
        assert!(std::fs::symlink_metadata(private.path().join(work[0])).is_err(), "and gone after");
        assert_eq!(names(destination.path()), ["backup"]);
    }

    #[test]
    fn a_partial_backup_a_crash_left_goes_before_the_next_backup_there() {
        let mut store = store_with_work();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".moss-backup-partial"), b"half an archive").unwrap();
        // Where an older Moss published from.
        std::fs::create_dir(root.path().join(".moss-backup-publish-x1")).unwrap();
        std::fs::write(root.path().join(".moss-backup-publish-x1/.tmpy2"), b"half an archive").unwrap();
        std::fs::write(root.path().join("notes.txt"), b"kept").unwrap();
        store.create_backup(&root.path().join("backup")).unwrap();
        assert_eq!(names(root.path()), ["backup", "notes.txt"]);
        crate::verify_backup(&root.path().join("backup"), None, crate::BackupVerification::default()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_where_the_partial_backup_goes_is_never_written_through() {
        let mut store = store_with_work();
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = elsewhere.path().join("target");
        std::fs::write(&target, b"someone else's").unwrap();
        std::os::unix::fs::symlink(&target, root.path().join(".moss-backup-partial")).unwrap();
        assert!(matches!(store.create_backup(&root.path().join("backup")), Err(BackupError::Io(_))));
        assert_eq!(std::fs::read(&target).unwrap(), b"someone else's");
        assert_eq!(names(root.path()), [".moss-backup-partial"]);
    }

    #[test]
    fn the_copy_from_an_earlier_update_goes_even_when_a_new_one_cannot_be_made() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("moss.db");
        std::fs::write(crate::safety_copy_path(&path), b"weeks old").unwrap();
        let conn = Connection::open(&path).unwrap();
        // A copy can't be read from a connection already in a transaction.
        conn.execute_batch("CREATE TABLE t(x); BEGIN; INSERT INTO t VALUES (1);").unwrap();
        assert!(keep_safety_copy(&conn, &path).is_err());
        assert!(!crate::safety_copy_path(&path).exists());
        assert_eq!(names(root.path()), ["moss.db", "moss.db-journal"]);
    }

    #[test]
    fn destination_never_receives_unscrubbed_bytes_during_creation() {
        let private = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let mut store = Store::open(private.path().join("mail.db")).unwrap();
        store.set_setting("connection:fixture", Some("fixture-authority-never-in-destination-51793")).unwrap();
        store.create_backup_controlled(&destination.path().join("backup"), BackupControl::default(), |_| {
            assert_destination_scrubbed(destination.path());
        }).unwrap();
        assert_destination_scrubbed(destination.path());
        assert_eq!(store.setting("connection:fixture").unwrap().as_deref(), Some("fixture-authority-never-in-destination-51793"));
    }

    #[test]
    fn background_source_refuses_memory_missing_and_unsupported_databases() {
        let memory = Store::open_in_memory().unwrap();
        assert!(matches!(memory.backup_source(), Err(BackupError::Unsupported)));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("live.db");
        let store = Store::open(&path).unwrap();
        store.conn.pragma_update(None, "user_version", ARCHIVE_VERSION).unwrap();
        assert!(matches!(store.backup_source(), Err(BackupError::Unsupported)));
        store.conn.pragma_update(None, "user_version", 1).unwrap();
        assert!(matches!(store.backup_source(), Err(BackupError::Unsupported)));
        assert_eq!(store.schema_version().unwrap(), 1, "no implicit migration");
        std::fs::rename(&path, root.path().join("moved.db")).unwrap();
        assert!(store.backup_source().is_err());
        assert!(!path.exists(), "must not recreate the database");
    }

    #[cfg(unix)]
    #[test]
    fn background_source_refuses_a_replaced_symlink() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("live.db");
        let store = Store::open(&path).unwrap();
        let moved = root.path().join("moved.db");
        std::fs::rename(&path, &moved).unwrap();
        let other_path = root.path().join("other.db");
        let _other = Store::open(&other_path).unwrap();
        std::os::unix::fs::symlink(&other_path, &path).unwrap();
        assert!(store.backup_source().is_err());
    }

    #[test]
    fn background_source_is_read_only_and_does_not_borrow_the_live_store() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().join("live.db")).unwrap();
        store.upsert_account(&account("work", "a@example.com")).unwrap();
        let mut source = store.backup_source().unwrap();
        assert!(source.conn.execute("DELETE FROM accounts", []).is_err());
        let output = root.path().join("archive");
        let mut wrote = false;
        source.create(&output, BackupControl::default(), |event| {
            if matches!(event, BackupProgress::Copying {..}) && !wrote {
                store.set_setting("fixture", Some("concurrent write")).unwrap();
                wrote = true;
            }
        }).unwrap();
        assert!(wrote);
        assert_eq!(store.setting("fixture").unwrap().as_deref(), Some("concurrent write"));
        crate::verify_backup(&output, None, crate::BackupVerification::default()).unwrap();
        assert_eq!(store.accounts().unwrap().len(), 1);
    }

    #[test]
    fn backup_keeps_mail_but_excludes_connection_settings_and_cleanup_authority() {
        let mut store = store_with_work();
        let message = ingest(&mut store, "one", &mail().body("Private fixture mail").build());
        store.set_setting("connection:work", Some("fixture-secret-must-not-leave-source-82973")).unwrap();
        store.set_setting("unknown-future-token", Some("fixture-future-secret-981723")).unwrap();
        store.queue_credential_cleanup(&crate::CredentialCleanup {service:"fixture-service".into(), account:"fixture-retired-key".into()}).unwrap();
        store.conn.execute_batch("CREATE TABLE observed_backup(sync INTEGER, flush INTEGER); CREATE TRIGGER observe_backup AFTER DELETE ON settings WHEN OLD.key='connection:work' BEGIN INSERT INTO observed_backup SELECT (SELECT synchronous FROM pragma_synchronous),(SELECT fullfsync FROM pragma_fullfsync); END;").unwrap();
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("mail.moss-backup");
        let info = store.create_backup(&output).unwrap();
        assert!(info.bytes > 0);
        let db = Connection::open(&output).unwrap();
        assert_eq!(db.query_row("SELECT sync,flush FROM observed_backup", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(), (2,1));
        assert_eq!(db.query_row("SELECT count(*) FROM messages", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("SELECT count(*) FROM settings", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(db.query_row("SELECT count(*) FROM credential_cleanup", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
        assert!(store.thread(&message.thread_id, NOV_1).unwrap().is_some());
        assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("fixture-secret-must-not-leave-source-82973"));
        assert!(store.credential_cleanup_pending().unwrap());
        let bytes = std::fs::read(&output).unwrap();
        for forbidden in [b"fixture-secret-must-not-leave-source-82973".as_slice(), b"fixture-future-secret-981723", b"fixture-retired-key"] {
            assert!(!bytes.windows(forbidden.len()).any(|part|part==forbidden));
        }
        assert!(Store::open(&output).is_err(), "an archive must not open as an active mail database");
        assert!(std::fs::read(&output).unwrap() == bytes, "refusing an archive must not modify its bytes");
    }
    #[test]
    fn archive_preserves_local_originals_drafts_files_and_delivery_evidence() {
        let root = tempfile::tempdir().unwrap();
        let source_path = root.path().join("source.db");
        let mut store = Store::open(&source_path).unwrap();
        let mut account = account("work", "a@example.com");
        account.provider = moss_core::ProviderKind::Resend;
        store.upsert_account(&account).unwrap();
        let raw = mail().body("Only local original").build();
        let message = ingest(&mut store, "one", &raw);
        let draft = draft("saved-draft");
        store.save_draft(&draft).unwrap();
        store.add_draft_file(&draft.id, "fixture.txt", "text/plain", b"attachment bytes", NOV_1).unwrap();
        store.prepare_delivery("held-operation", &crate::test_support::draft("held-draft"), "frozen outgoing", "held@example.test", NOV_1).unwrap();
        let output = root.path().join("mail.moss-backup");
        let info = store.create_backup(&output).unwrap();
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(info.bytes, bytes.len() as u64);
        assert_eq!(info.sha256, format!("{:x}", Sha256::digest(&bytes)));
        let db = Connection::open(&output).unwrap();
        assert_eq!(db.query_row("SELECT raw FROM message_raw WHERE message_id=?1", [&message.message_id], |r|r.get::<_,Vec<u8>>(0)).unwrap(), raw);
        assert_eq!(db.query_row("SELECT body FROM drafts WHERE id='saved-draft'", [], |r|r.get::<_,String>(0)).unwrap(), draft.body);
        assert_eq!(db.query_row("SELECT data FROM draft_files", [], |r|r.get::<_,Vec<u8>>(0)).unwrap(), b"attachment bytes");
        assert_eq!(db.query_row("SELECT state FROM deliveries WHERE id='held-operation'", [], |r|r.get::<_,String>(0)).unwrap(), "held");
        assert_eq!(db.query_row("SELECT schema_version FROM moss_backup_manifest WHERE format=1", [], |r|r.get::<_,u32>(0)).unwrap(), store.schema_version().unwrap());
        assert_eq!(db.query_row("PRAGMA application_id", [], |r|r.get::<_,u32>(0)).unwrap(), ARCHIVE_ID);
        assert_eq!(db.query_row("PRAGMA user_version", [], |r|r.get::<_,u32>(0)).unwrap(), ARCHIVE_VERSION);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().filter_map(std::result::Result::ok).filter(|e|e.file_name().to_string_lossy().starts_with(".moss-backup-")).count(), 0);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&output).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn existing_output_is_never_replaced_and_failed_backups_leave_no_staging_files() {
        let mut store = store_with_work();
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("keep.moss-backup");
        std::fs::write(&output, b"existing archive").unwrap();
        assert!(store.create_backup(&output).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"existing archive");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        assert!(store.create_backup(&root.path().join("missing/backup")).is_err());
    }

    #[test]
    fn refused_scrubbing_never_publishes_an_archive_or_changes_the_source() {
        for table in ["settings", "credential_cleanup"] {
            let mut store = store_with_work();
            store.set_setting("connection:work", Some("fixture-secret")).unwrap();
            store.queue_credential_cleanup(&crate::CredentialCleanup {service:"fixture".into(), account:"fixture".into()}).unwrap();
            store.conn.execute_batch(&format!("CREATE TRIGGER refuse_scrub BEFORE DELETE ON {table} BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
            let root = tempfile::tempdir().unwrap();
            assert!(store.create_backup(&root.path().join("backup")).is_err());
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
            assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("fixture-secret"));
            assert!(store.credential_cleanup_pending().unwrap());
        }
    }

    #[test]
    fn incomplete_copy_and_invalid_database_are_rejected() {
        for step in [StepResult::Busy, StepResult::Locked, StepResult::More] { assert!(require_complete(step).is_err()); }
        require_complete(StepResult::Done).unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=OFF; CREATE TABLE parent(id INTEGER PRIMARY KEY); CREATE TABLE child(parent INTEGER REFERENCES parent(id)); INSERT INTO child VALUES(1);").unwrap();
        assert!(verify_database(&db).is_err());
        db.execute_batch("DELETE FROM child; CREATE TABLE checked(value INTEGER CHECK(value>0)); PRAGMA ignore_check_constraints=ON; INSERT INTO checked VALUES(-1); PRAGMA ignore_check_constraints=OFF;").unwrap();
        assert!(verify_database(&db).is_err());
        db.execute_batch("DELETE FROM checked").unwrap();
        verify_database(&db).unwrap();
        let digest = fingerprint(&mut std::io::Cursor::new(b"abc"), &AtomicBool::new(false)).unwrap();
        assert_eq!(digest.bytes, 3);
        assert_eq!(digest.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn cancelled_backup_does_not_create_an_output_or_touch_the_source() {
        let mut store = store_with_work();
        store.set_setting("connection:work", Some("fixture sign-in")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let control = BackupControl::default();
        let progress = std::cell::RefCell::new(Vec::new());
        let mut record = |event| progress.borrow_mut().push(event);
        let successful = tempfile::tempdir().unwrap();
        let completed = successful.path().join("backup");
        store.create_backup_controlled(&completed, BackupControl::default(), &mut record).unwrap();
        assert!(completed.exists());
        assert!(!progress.borrow().is_empty());
        progress.borrow_mut().clear();
        control.cancelled.store(true, Ordering::Relaxed);
        let result = store.create_backup_controlled(&root.path().join("backup"), control, &mut record);
        assert!(matches!(result, Err(BackupError::Cancelled)));
        assert!(progress.borrow().is_empty());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("fixture sign-in"));
    }

    #[test]
    fn cancellation_at_each_phase_keeps_the_source_and_publishes_nothing() {
        let mut store = store_with_work();
        store.set_setting("connection:work", Some("fixture kept")).unwrap();
        for phase in [BackupProgress::Preparing, BackupProgress::Copying {copied_pages:0,total_pages:0}, BackupProgress::Checking, BackupProgress::Hashing, BackupProgress::Publishing] {
            let root = tempfile::tempdir().unwrap();
            let control = BackupControl::default(); let cancelled = control.cancelled.clone();
            let mut reached = false;
            let result = store.create_backup_controlled(&root.path().join("archive"), control, |event| {
                if std::mem::discriminant(&event) == std::mem::discriminant(&phase) {
                    reached = true; cancelled.store(true, Ordering::Relaxed);
                }
            });
            assert!(reached, "{phase:?}");
            assert!(matches!(result, Err(BackupError::Cancelled)), "{phase:?}");
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
            assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some("fixture kept"));
        }
    }

    #[test]
    fn chunked_copy_keeps_one_snapshot_while_another_connection_commits() {
        let root = tempfile::tempdir().unwrap(); let path = root.path().join("source.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "a@example.com")).unwrap();
        let old_name = store.accounts().unwrap()[0].name.clone();
        store.add_draft_file("fixture", "large.bin", "application/octet-stream", &vec![7;2_000_000], NOV_1).unwrap();
        let writer = Connection::open(&path).unwrap();
        let mut changed = false; let mut events = Vec::new();
        let output = root.path().join("snapshot");
        store.create_backup_controlled(&output, BackupControl::default(), |event| {
            events.push(event);
            if matches!(event, BackupProgress::Copying {..}) && !changed {
                writer.execute("UPDATE accounts SET name='Changed during backup' WHERE id='work'", []).unwrap();
                changed = true;
            }
        }).unwrap();
        assert!(changed);
        assert_eq!(store.accounts().unwrap()[0].name, "Changed during backup");
        let archive = Connection::open(&output).unwrap();
        assert_eq!(archive.query_row("SELECT name FROM accounts WHERE id='work'", [], |r|r.get::<_,String>(0)).unwrap(), old_name);
        let copied: Vec<_> = events.iter().filter_map(|event| if let BackupProgress::Copying {copied_pages,total_pages} = event {Some((*copied_pages,*total_pages))} else {None}).collect();
        assert!(copied.len() > 1);
        assert!(copied[0].0 <= 128);
        for window in copied.windows(2) { assert!(window[1].0 > window[0].0 && window[1].0-window[0].0 <= 128); assert_eq!(window[1].1, window[0].1); }
        let last = copied.last().unwrap(); assert_eq!(last.0,last.1);
        assert_eq!(events.first(), Some(&BackupProgress::Preparing));
        assert_eq!(&events[events.len()-3..], [BackupProgress::Checking,BackupProgress::Hashing,BackupProgress::Publishing]);
    }

    #[test]
    fn configured_size_limits_refuse_work_without_an_output() {
        let mut store = store_with_work(); let root = tempfile::tempdir().unwrap();
        let result = store.create_backup_controlled(&root.path().join("archive"), BackupControl {max_bytes:1,..Default::default()}, |_| {});
        assert!(matches!(result, Err(BackupError::Limit)));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        store.conn.execute_batch("VACUUM").unwrap();
        let source_bytes = store.conn.query_row("SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()", [], |r|r.get::<_,i64>(0)).unwrap() as u64;
        let result = store.create_backup_controlled(&root.path().join("archive"), BackupControl {max_bytes:source_bytes,..Default::default()}, |_| {});
        assert!(matches!(result, Err(BackupError::Limit)), "the manifest must not put the final archive over its budget");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        check_size(3,3).unwrap(); assert!(matches!(check_size(4,3), Err(BackupError::Limit)));
        source_size(i32::MAX as u32, 65536, u64::MAX).unwrap();
        assert!(matches!(source_size(i32::MAX as u32+1, 4096, u64::MAX), Err(BackupError::Limit)));
    }

    #[test]
    fn sqlite_work_and_hashing_observe_cancellation_and_keep_other_errors() {
        let mut database = Connection::open_in_memory().unwrap();
        let cancelled = Arc::new(AtomicBool::new(true));
        cancel_queries(&mut database, cancelled.clone()).unwrap();
        let result = database.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<100000) SELECT sum(i) FROM n").map_err(BackupError::from);
        assert!(matches!(cancellation_result(result, &cancelled), Err(BackupError::Cancelled)));
        let io: Result<(),BackupError> = Err(std::io::Error::other("fixture").into());
        assert!(matches!(cancellation_result(io, &cancelled), Err(BackupError::Io(_))));
        cancelled.store(false, Ordering::Relaxed);
        assert!(matches!(cancellation_result::<()>(Err(rusqlite::Error::InvalidQuery.into()), &cancelled), Err(BackupError::Sqlite(_))));
        struct CancellingReader(Arc<AtomicBool>, std::io::Cursor<Vec<u8>>);
        impl Read for CancellingReader {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> { self.0.store(true, Ordering::Relaxed); self.1.read(bytes) }
        }
        let mut reader = CancellingReader(cancelled.clone(), std::io::Cursor::new(b"abc".to_vec()));
        assert!(matches!(fingerprint(&mut reader, &cancelled), Err(BackupError::Cancelled)));
        assert_eq!(BackupError::Cancelled.to_string(), "The backup operation was cancelled.");
    }

}
