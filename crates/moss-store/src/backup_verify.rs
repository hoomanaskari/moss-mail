//! Verify a private copy of an archive. Verification never activates stored work.
use std::{path::Path, io::{Read, Write}, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use rusqlite::{Connection, OpenFlags, config::DbConfig, limits::Limit};
use sha2::{Digest, Sha256};
use crate::{BackupError, BackupInfo};
use crate::backup::check_cancelled;

pub struct BackupVerification {
    pub max_bytes: u64,
    pub max_steps: u64,
    pub max_value_bytes: i32,
    pub cancelled: Arc<AtomicBool>,
}
impl Default for BackupVerification {
    fn default() -> Self { Self {max_bytes:64*1024*1024*1024,max_steps:100_000_000,max_value_bytes:128*1024*1024,cancelled:Arc::new(AtomicBool::new(false))} }
}
#[derive(Debug)]
pub struct VerifiedBackup {
    info: BackupInfo,
    schema_version: u32,
    _file: tempfile::NamedTempFile,
    _directory: tempfile::TempDir,
}
impl VerifiedBackup {
    pub fn info(&self) -> &BackupInfo { &self.info }
    pub fn schema_version(&self) -> u32 { self.schema_version }
}
pub fn verify_backup(source: &Path, expected_sha256: Option<&str>, options: BackupVerification) -> Result<VerifiedBackup, BackupError> {
    check_cancelled(&options.cancelled)?;
    if options.max_steps == 0 || options.max_value_bytes <= 0 { return Err(BackupError::Limit); }
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut input = open.open(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() { return Err(BackupError::Invalid); }
    if metadata.len() > options.max_bytes { return Err(BackupError::Limit); }
    let directory = crate::backup::private_staging(None, "moss-verify-")?;
    let mut file = tempfile::NamedTempFile::new_in(directory.path())?;
    let info = copy_and_hash(&mut input, file.as_file_mut(), &options)?;
    if expected_sha256.is_some_and(|expected| expected != info.sha256) { return Err(BackupError::Invalid); }
    let mut database = Connection::open_with_flags(file.path(), OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    database.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    database.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    database.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
    database.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false)?;
    for (limit, value) in [(Limit::SQLITE_LIMIT_LENGTH, options.max_value_bytes), (Limit::SQLITE_LIMIT_SQL_LENGTH, 262_144),
        (Limit::SQLITE_LIMIT_SCHEMA, 1024), (Limit::SQLITE_LIMIT_VDBE_OP, 1_000_000), (Limit::SQLITE_LIMIT_ATTACHED, 0)] {
        database.set_limit(limit, value)?;
    }
    let exhausted = Arc::new(AtomicBool::new(false));
    let mut budget = Budget {remaining:options.max_steps, cancelled:options.cancelled.clone(), exhausted:exhausted.clone()};
    database.progress_handler(1000, Some(move || budget.stop()))?;
    let result = inspect(&database);
    let schema_version = inspection_result(result, &options.cancelled, &exhausted)?;
    check_cancelled(&options.cancelled)?;
    drop(database);
    Ok(VerifiedBackup {info, schema_version, _file:file, _directory:directory})
}

fn inspection_result(result: Result<u32, BackupError>, cancelled: &AtomicBool, exhausted: &AtomicBool) -> Result<u32, BackupError> {
    match result {
        Err(_) if cancelled.load(Ordering::Relaxed) => Err(BackupError::Cancelled),
        Err(_) if exhausted.load(Ordering::Relaxed) => Err(BackupError::Limit),
        other => other,
    }
}


fn copy_and_hash(input: &mut impl Read, output: &mut impl Write, options: &BackupVerification) -> Result<BackupInfo, BackupError> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut bytes = 0u64;
    loop {
        check_cancelled(&options.cancelled)?;
        let read = input.read(&mut buffer)?;
        if read == 0 { break; }
        bytes = bytes.checked_add(read as u64).ok_or(BackupError::Limit)?;
        if bytes > options.max_bytes { return Err(BackupError::Limit); }
        output.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
    }
    Ok(BackupInfo {bytes, sha256:format!("{:x}", hash.finalize())})
}

struct Budget { remaining:u64, cancelled:Arc<AtomicBool>, exhausted:Arc<AtomicBool> }
impl Budget {
    fn stop(&mut self) -> bool {
        if self.cancelled.load(Ordering::Relaxed) { return true; }
        if self.remaining < 1000 { self.exhausted.store(true, Ordering::Relaxed); return true; }
        self.remaining -= 1000;
        false
    }
}

type SchemaObject = (String, String, String, Option<String>);
fn schema(database: &Connection) -> Result<Vec<SchemaObject>, rusqlite::Error> {
    let mut statement = database.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY name")?;
    let rows = statement.query_map([], |r|Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    rows.collect()
}
fn inspect(database: &Connection) -> Result<u32, BackupError> {
    database.execute_batch("PRAGMA query_only=ON; PRAGMA mmap_size=0; PRAGMA cell_size_check=ON;")?;
    let application: u32 = database.query_row("PRAGMA application_id", [], |r|r.get(0))?;
    let version: u32 = database.query_row("PRAGMA user_version", [], |r|r.get(0))?;
    if application != crate::backup::ARCHIVE_ID || version != crate::backup::ARCHIVE_VERSION { return Err(BackupError::Unsupported); }
    let expected = crate::Store::open_in_memory().map_err(|_|BackupError::Invalid)?;
    expected.conn.execute_batch("CREATE TABLE moss_backup_manifest(format INTEGER NOT NULL, schema_version INTEGER NOT NULL); ANALYZE;")?;
    let expected_schema = schema(&expected.conn)?;
    let actual_schema = schema(database)?;
    // SQLite statistics are optional. Their definitions must still be canonical.
    if actual_schema.iter().any(|object| !expected_schema.contains(object))
        || expected_schema.iter().filter(|object| !matches!(object.1.as_str(), "sqlite_stat1" | "sqlite_stat4")).any(|object| !actual_schema.contains(object)) {
        return Err(BackupError::Unsupported);
    }
    let mut manifest = database.prepare("SELECT format,schema_version FROM moss_backup_manifest LIMIT 2")?;
    let entries = manifest.query_map([], |r|Ok((r.get::<_,u32>(0)?, r.get::<_,u32>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    let supported = crate::schema::MIGRATIONS.len() as u32;
    if entries.as_slice() != [(1,supported)] { return Err(BackupError::Unsupported); }
    crate::backup::verify_exclusions(database)?;
    crate::backup::verify_database(database)?;
    Ok(supported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Store, test_support::*};

    #[test]
    fn verifies_a_private_copy_without_changing_the_original_or_activating_it() {
        let mut store = store_with_work();
        ingest(&mut store, "fixture", &mail().build());
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.moss-backup");
        let created = store.create_backup(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let verified = verify_backup(&path, Some(&created.sha256), BackupVerification::default()).unwrap();
        assert_eq!(verified.info().sha256, created.sha256);
        assert_eq!(verified.info().bytes, created.bytes);
        assert_eq!(verified.schema_version(), store.schema_version().unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(Store::open(verified._file.path()).is_err());
        assert_ne!(verified._file.path(), path);
        std::fs::write(&path, b"changed after verification").unwrap();
        assert_eq!(std::fs::read(verified._file.path()).unwrap(), before);
        let staged = verified._file.path().to_owned();
        drop(verified);
        assert!(!staged.exists());
    }
    fn archive() -> (tempfile::TempDir, std::path::PathBuf) {
        let mut store = store_with_work();
        ingest(&mut store, "fixture", &mail().build());
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.moss-backup");
        store.create_backup(&path).unwrap();
        (root,path)
    }

    #[test]
    fn rejects_wrong_checksums_identifiers_formats_and_schema_changes() {
        let (_root,path) = archive();
        assert!(matches!(verify_backup(&path, Some(&"0".repeat(64)), BackupVerification::default()), Err(BackupError::Invalid)));
        for sql in ["PRAGMA application_id=0", "PRAGMA user_version=35", "UPDATE moss_backup_manifest SET format=2",
            "UPDATE moss_backup_manifest SET schema_version=0", "UPDATE moss_backup_manifest SET schema_version=2147483647",
            "DELETE FROM moss_backup_manifest", "INSERT INTO moss_backup_manifest SELECT * FROM moss_backup_manifest",
            "CREATE TRIGGER unexpected AFTER INSERT ON messages BEGIN SELECT 1; END", "DROP INDEX messages_open",
            "DROP TABLE moss_backup_manifest; CREATE VIEW moss_backup_manifest AS SELECT 1 AS format,35 AS schema_version"] {
            let (_root,path) = archive();
            Connection::open(&path).unwrap().execute_batch(sql).unwrap();
            assert!(matches!(verify_backup(&path, None, BackupVerification::default()), Err(BackupError::Unsupported)), "{sql}");
        }
    }

    #[test]
    fn rejects_retained_authority_corrupt_bytes_and_broken_foreign_keys() {
        for sql in ["INSERT INTO settings VALUES('connection:work','fixture')", "INSERT INTO credential_cleanup VALUES('fixture','fixture')",
            "PRAGMA foreign_keys=OFF; UPDATE messages SET account_id='missing'"] {
            let (_root,path) = archive();
            Connection::open(&path).unwrap().execute_batch(sql).unwrap();
            assert!(matches!(verify_backup(&path, None, BackupVerification::default()), Err(BackupError::Invalid)), "{sql}");
        }
        let (_root,path) = archive();
        std::fs::write(&path, b"not a SQLite archive").unwrap();
        assert!(verify_backup(&path, None, BackupVerification::default()).is_err());
    }

    #[test]
    fn accepts_canonical_optional_statistics_and_keeps_the_copy_private() {
        let mut store = store_with_work();
        store.conn.execute_batch("ANALYZE").unwrap();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("backup");
        store.create_backup(&path).unwrap();
        let verified = verify_backup(&path, None, BackupVerification::default()).unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(verified._file.path()).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(verified._directory.path()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert_eq!(verified.schema_version(), store.schema_version().unwrap());
    }

    #[test]
    fn enforces_file_sql_and_value_limits_and_cancellation() {
        let (_root,path) = archive();
        for options in [BackupVerification {max_bytes:1,..Default::default()}, BackupVerification {max_steps:1,..Default::default()},
            BackupVerification {max_steps:0,..Default::default()}, BackupVerification {max_value_bytes:0,..Default::default()}] {
            assert!(matches!(verify_backup(&path, None, options), Err(BackupError::Limit)));
        }
        assert!(verify_backup(&path, None, BackupVerification {max_value_bytes:1,..Default::default()}).is_err());
        let options = BackupVerification::default(); options.cancelled.store(true, Ordering::Relaxed);
        assert!(matches!(verify_backup(&path.with_extension("missing"), None, options), Err(BackupError::Cancelled)));
        assert!(verify_backup(&path.with_extension("missing"), None, BackupVerification::default()).is_err());
    }

    #[test]
    fn copy_limits_and_cancellation_are_checked_while_streaming() {
        let mut output = Vec::new();
        let limited = BackupVerification {max_bytes:2,..Default::default()};
        assert!(matches!(copy_and_hash(&mut std::io::Cursor::new(b"abc"), &mut output, &limited), Err(BackupError::Limit)));
        assert!(output.is_empty());
        let info = copy_and_hash(&mut std::io::Cursor::new(b"abc"), &mut output, &BackupVerification::default()).unwrap();
        assert_eq!(output, b"abc");
        assert_eq!(info.bytes, 3);
        assert_eq!(info.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        struct CancellingReader(Arc<AtomicBool>, std::io::Cursor<Vec<u8>>);
        impl Read for CancellingReader {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> { self.0.store(true, Ordering::Relaxed); self.1.read(bytes) }
        }
        let options = BackupVerification::default();
        let mut reader = CancellingReader(options.cancelled.clone(), std::io::Cursor::new(b"abc".to_vec()));
        assert!(matches!(copy_and_hash(&mut reader, &mut Vec::new(), &options), Err(BackupError::Cancelled)));
    }

    #[test]
    fn query_budget_and_interruption_errors_preserve_the_reason() {
        let cancelled = Arc::new(AtomicBool::new(false)); let exhausted = Arc::new(AtomicBool::new(false));
        let mut budget = Budget {remaining:1000, cancelled:cancelled.clone(), exhausted:exhausted.clone()};
        assert!(!budget.stop()); assert!(budget.stop()); assert!(exhausted.load(Ordering::Relaxed));
        assert!(matches!(inspection_result(Err(BackupError::Invalid), &cancelled, &exhausted), Err(BackupError::Limit)));
        cancelled.store(true, Ordering::Relaxed); assert!(budget.stop());
        assert!(matches!(inspection_result(Err(BackupError::Invalid), &cancelled, &exhausted), Err(BackupError::Cancelled)));
        assert_eq!(inspection_result(Ok(35), &cancelled, &exhausted).unwrap(), 35);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_directories_and_fifos_without_reading_their_contents() {
        use std::os::unix::{fs::symlink, ffi::OsStrExt};
        let (root,path) = archive();
        let link = root.path().join("link"); symlink(&path, &link).unwrap();
        assert!(verify_backup(&link, None, BackupVerification::default()).is_err());
        assert!(matches!(verify_backup(root.path(), None, BackupVerification::default()), Err(BackupError::Invalid)));
        let fifo = root.path().join("pipe");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe {libc::mkfifo(name.as_ptr(), 0o600)}, 0);
        assert!(matches!(verify_backup(&fifo, None, BackupVerification::default()), Err(BackupError::Invalid)));
    }

    #[cfg(unix)]
    #[test]
    fn backup_creation_and_verification_staging_directories_start_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        for parent in [Some(root.path()), None] {
            let staging = crate::backup::private_staging(parent, "moss-private-fixture-").unwrap();
            assert_eq!(std::fs::metadata(staging.path()).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

}
