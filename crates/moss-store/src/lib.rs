//! Moss's local mail database: one SQLite file per user, the single source of truth.
//!
//! Providers hand over raw RFC 822 bytes and [`MessageSource`] metadata; [`Store::ingest`]
//! parses, threads and indexes them idempotently. The UI reads threads, counts and search
//! results back, and changes per-message state (read, done, snoozed, split, section, labels).

use std::path::Path;

use rusqlite::Connection;

pub use moss_core::MessageSource;

mod accounts;
mod authres;
mod backup;
mod backup_verify;
mod credential_cleanup;
mod attachment_text;
mod addresses;
mod bodies;
mod classify;
mod codec;
mod content;
mod conversations;
mod decisions;
mod details;
mod drafts;
mod local_draft_recovery;
mod delivery;
mod delivery_compaction;
mod delivery_attention;
mod forwards;
mod files;
mod images;
mod ingest;
mod meaning;
mod moves;
mod outbox;
mod change_recovery;
mod change_batches;
mod parse;
mod people;
mod query;
mod schema;
mod schedule_operations;
mod session;
mod search;
mod senders;
mod signatures;
mod state;
mod summaries;
mod summary_cache;
mod summary_rules;
mod sync;
mod upkeep;
mod voice;
#[cfg(test)]
mod test_support;
pub mod text;

pub use backup::{BackupSource, BackupInfo, BackupError, BackupControl, BackupProgress};
pub use backup_verify::{verify_backup, BackupVerification, VerifiedBackup};
pub use credential_cleanup::CredentialCleanup;
pub use delivery::{Delivery, DeliveryState, DeliveryRepair, DeliveryReceipt, InterruptedSend};
pub use delivery_compaction::delivery_fingerprint;
pub use delivery_attention::{DeliveryAttentionCursor, DeliveryAttentionEntry, DeliveryAttentionPage, DeliveryReviewCopy, DeliveryResolution, DeliveryResolutionOutcome};
pub use schedule_operations::ScheduleOutcome;
pub use outbox::ChangeId;
pub use state::{ThreadChange, ThreadTargets};
pub use change_batches::{BatchId, BatchKind, BatchOutcome, BatchPart, ChangeBatch, UnclaimedOutcome};
pub use change_recovery::{ChangeHead, ChangeState, ChangeFailure, QueuedChange, MAX_CHANGE_PAYLOAD_BYTES, MAX_CHANGE_PARTS};
pub use attachment_text::{FileToEmbed, ToRead};
pub use bodies::{Body, Offline, Usage, DEFAULT_CACHE_LIMIT};
pub use decisions::{Decision, Source, Undecided, DEADLINE, EXPECTS_REPLY, NEEDS_REPLY, SPLIT};
pub use details::Kind as DetailKind;
pub use people::Contact;
pub use content::{AttachmentSource, ServerCopies};
pub use local_draft_recovery::{LocalDraftRecovery, LocalDraftRecoveryEntry};
pub use drafts::{ServerCopy, ServerDraft, ServerFile, SyncedDraft, DRAFT_HEADER, MODE_HEADER, SIGNATURE_HEADER};
pub use ingest::Ingested;
pub use meaning::{MeaningReader, ToEmbed, DIMS as MEANING_DIMS};
pub use summaries::ToSummarize;
pub use senders::BLOCKED_FOLDER;
pub use upkeep::{FREE_PAGES, INDEX_PAGES};
pub use sync::{Location, ReplyHeaders};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("not an email message")]
    NotEmail,
    #[error("no account with id {0}")]
    UnknownAccount(String),
    #[error("no {kind} with id {id}")]
    NotFound { kind: &'static str, id: String },
    #[error("{0} is a public mail service, so it can't be blocked or accepted as a whole")]
    PublicDomain(String),
    #[error("{0} isn't an email address")]
    InvalidAddress(String),
    #[error("{0} is this account's own address")]
    PrimaryAddress(String),
    /// Something you typed that Moss can't use, saying why.
    #[error("{0}")]
    Invalid(String),
    #[error("The upload receipt reuses an identity for different content. Both copies have been kept.")]
    UploadReceiptConflict,
    #[error("This draft changed or was removed elsewhere. Your edit must be kept separately before continuing.")]
    DraftConflict,
    #[error("The saved delivery draft cannot be read. Its data has been retained.")]
    UnreadableDeliveryDraft,
    #[error("This local recovery has already been resolved.")]
    LocalDraftRecoveryResolved { adopted_id: Option<String> },
    #[error("This request key was used for a different request.")]
    RequestConflict,
    #[error("the database is at schema version {found}, newer than this Moss knows ({supported})")]
    TooNew { found: u32, supported: u32 },
    /// Another Moss holds this database's session lock.
    #[error("Another Moss is using this mail. Quit it first.")]
    InUse,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub struct Store {
    conn: Connection,
    // Fields drop in order: close the database before releasing recovery ownership.
    delivery_session: Option<session::Session>,
}

impl Store {
    /// Opens (or creates) the database at `path` in WAL mode and brings its schema up to date.
    /// An update (or a new database) only goes ahead while no other Moss has the mail open:
    /// [`Error::InUse`] otherwise, and nothing is changed. Before it updates a database an older
    /// Moss left, it keeps a copy of it beside it ([`safety_copy_path`]), after removing any copy
    /// from an earlier update, so a copy there is always from just before the latest update; if
    /// the copy can't be made (no room, say), it updates anyway.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path)?;
        let found: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let supported = schema::MIGRATIONS.len() as u32;
        if found > supported { return Err(Error::TooNew {found, supported}); }
        let updating = found < supported;
        // Held while updating, so a Moss running on this mail never has it changed under it.
        let _owner = updating.then(|| session::SessionLock::take(path)).transpose()?;
        if updating {
            backup::clear_leftovers(path);
        }
        if (1..supported).contains(&found) {
            let _ = backup::keep_safety_copy(&conn, path);
        }
        incremental_if_new(&conn)?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        // With WAL, NORMAL still survives a crash; only a power cut can lose the last commits.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let mut store = Self::init(conn)?;
        if updating {
            store.delivery_session = Some(session::Session::Updated);
        }
        Ok(store)
    }

    /// A throwaway database, for tests and previews.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        incremental_if_new(&conn)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let found = migrate(&mut conn, schema::MIGRATIONS)?;
        let mut store = Self { conn, delivery_session: None };
        // Mail kept through the lean-storage version (Resend's) is read back into the index;
        // mail from before V12 gets previews without what its HTML left behind.
        if (1..12).contains(&found) {
            store.start_rebuild()?;
        }
        Ok(store)
    }

    /// Deletes the copy of this database from before its last update, if there is one: for when
    /// an account is removed, so its mail doesn't stay in the copy. See [`forget_safety_copy`].
    pub fn forget_safety_copy(&self) -> std::io::Result<()> {
        match self.conn.path().filter(|path| !path.is_empty()) {
            Some(path) => forget_safety_copy(Path::new(path)),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
}

/// Where Moss keeps a copy of the database at `database` from just before it last updated it,
/// so a failed update can be undone by hand: "moss.db" has "moss.db.before-update" beside it.
pub fn safety_copy_path(database: &Path) -> std::path::PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(".before-update");
    name.into()
}

/// Deletes the copy of the database at `database` from before its last update, if there is
/// one. Moss does this once a launch opens the mail with nothing to update, and the app does it
/// when an account is removed, so that account's mail doesn't stay in the copy.
pub fn forget_safety_copy(database: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(safety_copy_path(database)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        done => done,
    }
}

/// Whether the database at `database` is one this Moss has still to finish updating: from an
/// older Moss, or left partway by an update that failed. A file that isn't a database, or isn't
/// there, is not.
pub fn update_unfinished(database: &Path) -> bool {
    let version = Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|conn| conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0)));
    version.is_ok_and(|found| (1..schema::MIGRATIONS.len() as u32).contains(&found))
}

/// New databases free space incrementally. That can only be chosen before anything is
/// written, even the switch to WAL; older databases switch in [`Store::compact`].
fn incremental_if_new(conn: &Connection) -> Result<()> {
    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 {
        conn.pragma_update(None, "auto_vacuum", upkeep::INCREMENTAL)?;
    }
    Ok(())
}

/// Applies the migrations a database hasn't had yet, each in its own transaction. Returns
/// the version it was at.
fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<u32> {
    let found: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let supported = migrations.len() as u32;
    if found > supported {
        return Err(Error::TooNew { found, supported });
    }
    for (version, sql) in migrations.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version as u32 + 1)?;
        tx.commit()?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> u32 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_new_database_is_at_the_latest_version_in_wal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, schema::MIGRATIONS.len());
        let mode: String = store.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        let fk: bool = store.conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert!(fk);
        drop(store);
        let again = Store::open(&path).unwrap();
        assert_eq!(again.schema_version().unwrap() as usize, schema::MIGRATIONS.len());
    }

    #[test]
    fn frozen_v24_schema_matches_the_unchanged_migration_prefix() {
        fn schema(connection: &Connection) -> Vec<(String,String,String,Option<String>)> {
            connection.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").unwrap()
                .query_map([], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap()
                .collect::<rusqlite::Result<_>>().unwrap()
        }
        let frozen = Connection::open_in_memory().unwrap();
        let fixture = include_str!("../tests/fixtures/schema-v24.sql");
        let (_, frozen_sql) = fixture.split_once('\n').unwrap();
        assert_eq!(frozen_sql.strip_suffix("\nPRAGMA user_version=24;\n").unwrap(), schema::MIGRATIONS[..24].join("\n"),
            "the frozen prefix also pins data-moving statements, not only the resulting schema");
        frozen.execute_batch(fixture).unwrap();
        let mut current = Connection::open_in_memory().unwrap();
        migrate(&mut current, &schema::MIGRATIONS[..24]).unwrap();
        assert_eq!(schema(&frozen), schema(&current), "change schema in a new migration, never edit the existing V24 prefix");
    }

    #[test]
    fn upgrading_the_v24_profile_preserves_mail_drafts_files_and_schedules() {
        use crate::test_support::{draft, NOV_1};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("moss.db");
        let mut scheduled = serde_json::to_value(draft("scheduled-v24")).unwrap();
        scheduled.as_object_mut().unwrap().remove("revision");
        scheduled["to"] = serde_json::json!([{ "name": "Example recipient", "email": "recipient@example.test" }]);
        let outgoing = "{\"legacy\":\"frozen outgoing content\"}";
        {
            let connection = Connection::open(&path).unwrap();
            connection.pragma_update(None, "auto_vacuum", "INCREMENTAL").unwrap();
            connection.pragma_update(None, "journal_mode", "WAL").unwrap();
            connection.execute_batch(include_str!("../tests/fixtures/schema-v24.sql")).unwrap();
            assert_eq!(version(&connection), 24);
            connection.execute_batch("INSERT INTO accounts VALUES ('work','Work','W','sender@example.test','Example sender','work','imap',NULL,NULL,0);
                INSERT INTO threads VALUES (1,'work','Upgrade mail','upgrade mail','',1,NULL);
                INSERT INTO messages (id,account_id,thread_id,message_id,subject,from_name,from_email,to_json,cc_json,bcc_json,participants,sent_at,received_at,snippet,from_me,split,section)
                    VALUES (1,'work',1,'upgrade@example.test','Upgrade mail','Example recipient','recipient@example.test','[]','[]','[]','Example recipient',1,1,'Retained preview',0,'important','info');
                INSERT INTO drafts (id,account_id,thread_id,mode,to_json,cc_json,bcc_json,subject,body,body_html,remind_if_no_reply,updated_at)
                    VALUES ('draft-v24','work',1,'reply','[]','[]','[]','Upgrade draft','Retained unsent text','<p>Retained unsent text</p>',0,5);
                INSERT INTO draft_files (id,draft_id,name,content_type,data,added_at)
                    VALUES (7,'draft-v24','notes.txt','text/plain',x'6e6f746573',5);
                ").unwrap();
            connection.execute("INSERT INTO settings VALUES ('connection:work',?1)", [r#"{"fixture":"reference-only"}"#]).unwrap();
            connection.execute("INSERT INTO signatures VALUES ('signature-v24','Work',?1,0,5)", [r#"["Example sender","Example studio"]"#]).unwrap();
            connection.execute_batch("INSERT INTO account_signatures VALUES ('work','signature-v24','signature-v24');
                INSERT INTO sender_rules VALUES ('work','recipient@example.test','accept',5)").unwrap();
            let body = zstd::encode_all("Retained offline mail body".as_bytes(), 12).unwrap();
            connection.execute("INSERT INTO bodies VALUES (1,'zstd',?1,NULL,?2,1,5)", rusqlite::params![body, body.len() as i64]).unwrap();
            connection.execute("UPDATE drafts SET files_json=?1 WHERE id='draft-v24'", [serde_json::json!([{ "id":"7", "name":"notes.txt", "contentType":"text/plain", "bytes":5 }]).to_string()]).unwrap();
            connection.execute("INSERT INTO scheduled (draft_id,account_id,draft,outgoing,send_at,next_try,tries,error) VALUES ('scheduled-v24','work',?1,?2,?3,?4,2,'offline')",
                rusqlite::params![scheduled.to_string(), outgoing, NOV_1, NOV_1+60_000]).unwrap();
        }
        let mut operation = None;
        let mut revision = None;
        for _ in 0..2 {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.schema_version().unwrap() as usize, schema::MIGRATIONS.len());
            let mail = store.thread("1", NOV_1).unwrap().unwrap();
            assert_eq!(mail.messages[0].subject, "Upgrade mail");
            assert_eq!(mail.messages[0].body_text, "Retained offline mail body");
            assert!(!mail.messages[0].headers_only);
            assert_eq!(store.setting("connection:work").unwrap().as_deref(), Some(r#"{"fixture":"reference-only"}"#));
            let signatures = store.signatures().unwrap();
            assert_eq!(signatures.signatures[0].lines, ["Example sender", "Example studio"]);
            assert_eq!(signatures.by_account["work"].new_emails.as_deref(), Some("signature-v24"));
            assert_eq!(store.sender_status("work", "recipient@example.test").unwrap(), moss_core::SenderStatus::Known);
            let kept = store.editable_draft("draft-v24").unwrap().unwrap();
            assert_eq!(kept.body, "Retained unsent text");
            assert_eq!(kept.thread_id.as_deref(), Some("1"));
            assert_eq!(kept.attachments.len(), 1);
            assert_eq!(store.draft_file("7").unwrap().unwrap().2, b"notes");
            let token = kept.revision.unwrap();
            assert_eq!(token.len(), 32);
            if let Some(previous) = revision.replace(token.clone()) { assert_eq!(previous, token); }
            let rows = store.scheduled_emails().unwrap();
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(row.draft.id, "scheduled-v24");
            assert_eq!(row.draft.body, scheduled["body"]);
            assert_eq!((row.send_at, row.next_try, row.tries), (NOV_1, NOV_1+60_000, 2));
            assert_eq!(row.error.as_deref(), Some("offline"));
            assert!(!row.operation_id.is_empty());
            if let Some(previous) = operation.replace(row.operation_id.clone()) { assert_eq!(previous, row.operation_id); }
            assert_eq!(store.due_email(&row.draft.id, NOV_1+60_000).unwrap().unwrap().1, outgoing);
            assert!(store.pending_deliveries().unwrap().is_empty(), "migration must not submit scheduled mail");
            assert_eq!(store.conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_,String>(0)).unwrap(), "ok");
            assert!(store.conn.prepare("PRAGMA foreign_key_check").unwrap().query([]).unwrap().next().unwrap().is_none());
        }
    }

    #[test]
    fn v42_preserves_every_delivery_field_without_decoding_recovery_payloads() {
        fn records(connection: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
            connection.prepare("SELECT * FROM deliveries ORDER BY id").unwrap().query_map([], |row| {
                (0..18).map(|index| row.get(index)).collect::<rusqlite::Result<Vec<_>>>()
            }).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("moss.db");
        let before = {
            let mut connection = Connection::open(&path).unwrap();
            migrate(&mut connection, &schema::MIGRATIONS[..41]).unwrap();
            connection.execute_batch("INSERT INTO accounts VALUES ('work','Work','W','sender@example.test','Example sender','work','imap',NULL,NULL,0)").unwrap();
            for state in ["held", "claimed", "accepted", "uncertain", "complete", "cancelled"] {
                connection.execute("INSERT INTO deliveries(id,draft_id,account_id,draft,outgoing,message_id,state,raw,source,thread_id,error,created_at,reminder_applied,snooze_revision,source_revision,interrupted_from,repair_next,repair_attempt)
                    VALUES (?1,?1,'work','unreadable draft','unreadable outgoing','mail@example.test',?1,x'010203','retained source','9','retained reason',7,1,8,'revision','claimed',12345,4)", [state]).unwrap();
            }
            records(&connection)
        };
        for _ in 0..2 {
            let store = Store::open(&path).unwrap();
            assert_eq!(records(&store.conn), before);
            assert_eq!(store.conn.query_row("SELECT count(*) FROM deliveries WHERE account_retired=0", [], |row|row.get::<_,i64>(0)).unwrap(), 6);
            assert_eq!(store.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name IN ('deliveries_pending','deliveries_own_draft','delivery_repair_due')", [], |row|row.get::<_,i64>(0)).unwrap(), 3);
            assert!(store.conn.prepare("PRAGMA foreign_key_check").unwrap().query([]).unwrap().next().unwrap().is_none());
        }
    }

    /// Who mail you sent went to, as `sent_to` keeps it: (message, address, domain).
    fn sent_to(store: &Store) -> Vec<(i64, String, Option<String>)> {
        store.conn.prepare("SELECT message_id, email, domain FROM sent_to ORDER BY message_id, email").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn v46_keeps_who_you_wrote_to_in_step_with_your_mail() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("moss.db");
        {
            let mut connection = Connection::open(&path).unwrap();
            migrate(&mut connection, &schema::MIGRATIONS[..45]).unwrap();
            connection.execute_batch(r#"INSERT INTO accounts VALUES ('work','Work','W','maya@example.test','Maya','work','imap',NULL,NULL,0);
                INSERT INTO threads VALUES (1,'work','Plans','plans','',1,NULL);
                INSERT INTO messages (id,account_id,thread_id,message_id,subject,from_name,from_email,to_json,cc_json,bcc_json,participants,sent_at,received_at,snippet,from_me,split,section)
                    VALUES (1,'work',1,'a@example.test','Plans','Maya','maya@example.test','[{"name":"Ann","email":"Ann@Acme.example"},"stray",{"name":"No address"}]',
                                '[{"name":"Bo","email":"bo@two@acme.example"},{"name":"Cy","email":"cy"}]','not json','Ann',1,1,'',1,'important','info'),
                           (2,'work',1,'b@example.test','Re: Plans','Ann','ann@acme.example','[{"name":"Maya","email":"maya@example.test"}]','[]','[]','Ann',2,2,'',0,'important','reply');"#).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let acme = || Some("acme.example".to_string());
        assert_eq!(sent_to(&store), [(1, "ann@acme.example".into(), acme()), (1, "bo@two@acme.example".into(), acme()), (1, "cy".into(), None)],
            "mail you sent before the update is there, its broken parts left out");
        store.conn.execute_batch(r#"INSERT INTO messages (id,account_id,thread_id,message_id,subject,from_name,from_email,to_json,cc_json,bcc_json,participants,sent_at,received_at,snippet,from_me,split,section)
            VALUES (3,'work',1,'c@example.test','Re: Plans','Maya','maya@example.test','[]','[]','[{"name":"Di","email":"di@b.example"}]','',3,3,'',1,'important','info');
            INSERT INTO messages (id,account_id,thread_id,message_id,subject,from_name,from_email,to_json,cc_json,bcc_json,participants,sent_at,received_at,snippet,from_me,split,section)
            VALUES (4,'work',1,'d@example.test','Re: Plans','Ed','ed@b.example','[{"name":"Fay","email":"fay@b.example"}]','[]','[]','',4,4,'',0,'important','info');"#).unwrap();
        assert_eq!(sent_to(&store)[3..], [(3, "di@b.example".into(), Some("b.example".into()))], "new mail you send, Bcc too; not mail others sent");
        // Mail from an address you added becomes yours; the recipients of an edited message change.
        store.conn.execute_batch(r#"UPDATE messages SET from_me = 1 WHERE id = 4;
            UPDATE messages SET to_json = '[{"name":"Gus","email":"gus@c.example"}]', cc_json = '[]' WHERE id = 1;"#).unwrap();
        assert_eq!(sent_to(&store), [(1, "gus@c.example".into(), Some("c.example".into())), (3, "di@b.example".into(), Some("b.example".into())),
            (4, "fay@b.example".into(), Some("b.example".into()))]);
        store.conn.execute("UPDATE messages SET from_me = 0 WHERE id = 3", []).unwrap();
        store.conn.execute("DELETE FROM messages WHERE id = 4", []).unwrap();
        assert_eq!(sent_to(&store), [(1, "gus@c.example".into(), Some("c.example".into()))]);
        store.conn.execute("DELETE FROM threads", []).unwrap();
        assert!(sent_to(&store).is_empty());
        // Keeping it in step reads only the message that changed.
        let plan = crate::test_support::plan(&store, "SELECT * FROM sent_to_source WHERE message_id = ?1");
        assert!(plan.contains("SEARCH m USING INTEGER PRIMARY KEY") && !crate::test_support::scans(&plan, "m"), "{plan}");
    }

    #[test]
    fn v47_counts_only_your_own_copies_of_older_mail_as_confirmed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("moss.db");
        {
            let mut connection = Connection::open(&path).unwrap();
            migrate(&mut connection, &schema::MIGRATIONS[..46]).unwrap();
            connection.execute_batch(r#"INSERT INTO accounts VALUES ('work','Work','W','maya@example.test','Maya','work','gmail',NULL,NULL,0);
                INSERT INTO threads VALUES (1,'work','Plans','plans','',1,NULL);
                INSERT INTO mailboxes VALUES (1,'work','INBOX','inbox'),(2,'work','Sent','sent');
                INSERT INTO messages (id,account_id,thread_id,message_id,subject,from_name,from_email,to_json,cc_json,bcc_json,participants,sent_at,received_at,snippet,from_me,split,section)
                    VALUES (1,'work',1,'a@example.test','Plans','Maya','maya@example.test','[]','[]','[]','',1,1,'',1,'important','info'),
                           (2,'work',1,'b@example.test','Re: Plans','Ann','ann@acme.example','[]','[]','[]','',2,2,'',0,'important','reply'),
                           (3,'work',1,'c@example.test','Re: Plans','Maya','maya@example.test','[]','[]','[]','',3,3,'',1,'important','info'),
                           (4,'work',1,'d@example.test','Re: Plans','Maya','maya@example.test','[]','[]','[]','',4,4,'',1,'important','info');
                INSERT INTO message_locations VALUES (1,2,'7'),(2,1,'8'),(3,1,'local:<c@example.test>'),(4,1,'9');"#).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let confirmed: Vec<(i64, bool)> = store.conn.prepare("SELECT id, sender_verified FROM messages ORDER BY id").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        // Your Sent folder and Moss's own copy of what it sent are yours; the rest, even from
        // your address, can't be checked any more: those headers weren't kept.
        assert_eq!(confirmed, [(1, true), (2, false), (3, true), (4, false)]);
        assert!(store.conn.execute("UPDATE messages SET sender_verified = 2 WHERE id = 1", []).is_err());
    }

    #[test]
    fn files_attached_before_v24_stay_and_their_ids_are_not_given_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate(&mut conn, &schema::MIGRATIONS[..23]).unwrap();
            conn.execute_batch(
                "INSERT INTO draft_files VALUES (1, 'd1', 'a.pdf', 'application/pdf', x'25504446', 5),
                                                (2, 'd1', 'b.png', 'image/png', x'89', 6);
                 DELETE FROM draft_files WHERE id = 2;",
            )
            .unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.draft_file("1").unwrap(), Some(("a.pdf".into(), "application/pdf".into(), b"%PDF".to_vec())));
        store.forget_draft_files("d1").unwrap();
        let next = store.add_draft_file("d2", "c.pdf", "application/pdf", b"%PDF", 7).unwrap();
        assert_ne!(next.id, "1");
    }

    #[test]
    fn mail_from_before_lean_storage_is_fetched_again_and_resend_mail_read_back_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate(&mut conn, &schema::MIGRATIONS[..7]).unwrap();
            let raw = b"From: Dana <dana@acme.co>\r\nSubject: Invoice\r\nMessage-ID: <r@x>\r\n\r\nPay by Friday\r\n";
            conn.execute_batch(
                "INSERT INTO accounts VALUES ('re', 'Resend', 'R', 'maya@moss.email', 'Maya', 'work', 'resend', NULL, NULL, 0);
                 INSERT INTO accounts VALUES ('w', 'Work', 'W', 'maya@northwind.co', 'Maya', 'work', 'imap', NULL, NULL, 1);
                 INSERT INTO threads VALUES (1, 're', 'Invoice', 'invoice', '', 1), (2, 'w', 'Lunch', 'lunch', '', 1);",
            )
            .unwrap();
            for (id, account, thread) in [(1, "re", 1), (2, "w", 2)] {
                conn.execute(
                    "INSERT INTO messages (id, account_id, thread_id, message_id, subject, from_name, from_email, to_json,
                         cc_json, bcc_json, participants, sent_at, received_at, body_text, body_stripped, snippet, from_me,
                         split, section)
                     VALUES (?1, ?2, ?3, ?1, 'x', 'Dana', 'dana@acme.co', '[]', '[]', '[]', 'Dana', 1, 1, 'Pay', 'Pay',
                         'Pay', 0, 'important', 'reply')",
                    rusqlite::params![id, account, thread],
                )
                .unwrap();
            }
            conn.execute("INSERT INTO message_raw VALUES (1, ?1)", [&raw[..]]).unwrap();
            conn.execute_batch("INSERT INTO sync_checkpoints VALUES ('w', 'INBOX', 1, 2, NULL, NULL, 0)").unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        let count = |s: &Store, sql: &str| s.conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count(&store, "SELECT count(*) FROM messages"), 1, "the server has the rest");
        assert_eq!(count(&store, "SELECT count(*) FROM sync_checkpoints"), 0, "so sync starts again");
        assert!(store.rebuilding().unwrap());
        while store.rebuild_step(10).unwrap() > 0 {}
        assert!(store.body(1).unwrap().is_some());
        assert_eq!(store.search("friday", None, 10, 0).unwrap().len(), 1);
    }

    #[test]
    fn previews_made_before_they_left_out_html_leftovers_are_made_again_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        migrate(&mut Connection::open(&path).unwrap(), &schema::MIGRATIONS[..11]).unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.rebuilding().unwrap());
        store.conn.execute("DELETE FROM settings", []).unwrap();
        drop(store);
        assert!(!Store::open(&path).unwrap().rebuilding().unwrap(), "once is enough");
        assert!(!Store::open_in_memory().unwrap().rebuilding().unwrap(), "a new database has nothing to redo");
    }

    #[test]
    fn drafts_kept_before_they_went_to_the_server_go_up_on_the_next_sync() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            // Up to the migration before the one that gave drafts their server copy (V21).
            let v21 = schema::MIGRATIONS.iter().position(|m| m.contains("server_message_id")).unwrap();
            migrate(&mut conn, &schema::MIGRATIONS[..v21]).unwrap();
            conn.execute_batch(
                "INSERT INTO accounts VALUES ('w', 'Work', 'W', 'maya@northwind.co', 'Maya', 'work', 'imap', NULL, NULL, 0);
                 INSERT INTO drafts (id, account_id, mode, to_json, cc_json, bcc_json, subject, body, body_html,
                     remind_if_no_reply, updated_at)
                 VALUES ('d1', 'w', 'new', '[]', '[]', '[]', 'Lunch?', 'Friday?', '', 0, 5);",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let kept = store.synced_drafts("w").unwrap();
        assert_eq!((kept.len(), kept[0].copy.clone(), kept[0].pushed), (1, None, false));
    }

    #[test]
    fn new_databases_free_space_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("moss.db")).unwrap();
        let mode: i64 = store.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 2);
    }

    #[test]
    fn in_memory_databases_get_the_schema_too() {
        let store = Store::open_in_memory().unwrap();
        let tables: u32 = store
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('messages', 'message_search')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 2);
    }

    #[test]
    fn migrations_run_once_and_in_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &["CREATE TABLE a (x)"]).unwrap();
        assert_eq!(version(&conn), 1);
        migrate(&mut conn, &["CREATE TABLE a (x)", "CREATE TABLE b (x)"]).unwrap();
        assert_eq!(version(&conn), 2);
        migrate(&mut conn, &["CREATE TABLE a (x)", "CREATE TABLE b (x)"]).unwrap();
        assert_eq!(version(&conn), 2);
    }

    #[test]
    fn a_failed_migration_leaves_the_version_alone() {
        let mut conn = Connection::open_in_memory().unwrap();
        let err =
            migrate(&mut conn, &["CREATE TABLE a (x)", "CREATE TABLE b (x); NOT SQL"]).unwrap_err();
        assert!(matches!(err, Error::Sqlite(_)));
        assert_eq!(version(&conn), 1);
        let b: u32 = conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'b'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(b, 0);
    }

    #[test]
    fn refuses_a_database_from_a_newer_moss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 99).unwrap();
        let err = Store::open(&path).err().unwrap();
        assert!(matches!(err, Error::TooNew { found: 99, .. }));
        assert!(err.to_string().contains("version 99"));
        let mut connection = Connection::open(&path).unwrap();
        assert!(matches!(migrate(&mut connection, schema::MIGRATIONS), Err(Error::TooNew { found: 99, .. })));
        assert_eq!(version(&connection), 99);
    }

    #[test]
    fn decisions_keep_the_model_s_needs_reply_answers_and_ask_the_rest_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate(&mut conn, &schema::MIGRATIONS[..21]).unwrap();
            conn.execute_batch(
                "INSERT INTO accounts VALUES ('w', 'Work', 'W', 'maya@northwind.co', 'Maya', 'work', 'imap', NULL, NULL, 0);
                 INSERT INTO threads VALUES (1, 'w', 'Lunch', 'lunch', '', 1);",
            )
            .unwrap();
            for id in [1, 2] {
                conn.execute(
                    "INSERT INTO messages (id, account_id, thread_id, message_id, subject, from_name, from_email, to_json,
                         cc_json, bcc_json, participants, sent_at, received_at, snippet, from_me, split, section)
                     VALUES (?1, 'w', 1, ?1, 'x', 'Dana', 'dana@acme.co', '[]', '[]', '[]', 'Dana', 1, 1, 'x', 0,
                         'important', 'info')",
                    [id],
                )
                .unwrap();
            }
            conn.execute_batch(
                "INSERT INTO ai_decisions VALUES (1, 'needsReply', 0.1, 1, 5), (2, 'needsReply', 0.6, 0, 5),
                     (1, 'deadline', 0.9, 0, 5), (1, 'category:work', 0.7, 0, 5);",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let mut stmt = store
            .conn
            .prepare("SELECT message_id, question, yes, source, acted, value FROM ai_decisions ORDER BY message_id")
            .unwrap();
        let rows: Vec<(i64, String, f64, String, bool, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                (1, "needsReply".into(), 0.1, "model".into(), true, Some("info".into())),
                (2, "needsReply".into(), 0.6, "model".into(), false, None),
            ]
        );
    }

    /// A database an older Moss left at `version`, in WAL mode, with a row that is still only in
    /// its WAL file while the returned connection stays open: a raw file copy would miss it.
    fn older_database(path: &Path, version: usize) -> Connection {
        let mut conn = Connection::open(path).unwrap();
        migrate(&mut conn, &schema::MIGRATIONS[..version]).unwrap();
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(())).unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch("CREATE TABLE fixture (x); INSERT INTO fixture VALUES ('kept');").unwrap();
        conn
    }

    fn copied(path: &Path) -> (u32, String) {
        let copy = Connection::open(safety_copy_path(path)).unwrap();
        (version(&copy), copy.query_row("SELECT x FROM fixture", [], |r| r.get(0)).unwrap())
    }

    #[test]
    fn a_database_is_copied_beside_itself_before_it_is_updated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        assert_eq!(safety_copy_path(&path), dir.path().join("moss.db.before-update"));
        // A copy from an earlier update is replaced.
        std::fs::write(safety_copy_path(&path), b"an older copy").unwrap();
        let older = older_database(&path, 30);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, schema::MIGRATIONS.len());
        assert_eq!(copied(&path), (30, "kept".into()), "the copy is the database as it was, WAL and all");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(safety_copy_path(&path)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "mail stays private");
        }
        drop((store, older));
        // Nothing to update, nothing copied.
        std::fs::remove_file(safety_copy_path(&path)).unwrap();
        drop(Store::open(&path).unwrap());
        assert!(!safety_copy_path(&path).exists());
        // Nor for a new database.
        drop(Store::open(dir.path().join("new.db")).unwrap());
        assert!(!safety_copy_path(&dir.path().join("new.db")).exists());
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(!names.iter().any(|n| n.starts_with(".moss-")), "no work files left: {names:?}");
    }

    #[test]
    fn a_copy_that_cannot_be_made_does_not_stop_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        // Something in the way, as a full disk would be.
        std::fs::create_dir(safety_copy_path(&path)).unwrap();
        let _older = older_database(&path, 30);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, schema::MIGRATIONS.len());
        assert!(safety_copy_path(&path).is_dir(), "left as it was");
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(!names.iter().any(|n| n.starts_with(".moss-")), "no half copy left: {names:?}");
    }

    #[test]
    fn a_second_moss_does_not_update_mail_another_moss_has_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let older = older_database(&path, 30);
        let running = session::SessionLock::take(&path).unwrap();
        assert!(matches!(Store::open(&path), Err(Error::InUse)));
        assert_eq!(version(&older), 30, "nothing updated under the running Moss");
        assert!(!safety_copy_path(&path).exists(), "nor a copy made");
        drop(running);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, schema::MIGRATIONS.len());
        // An up-to-date mail file opens beside a running Moss: its own session claim refuses.
        drop(store);
        let mut running = Store::open(&path).unwrap();
        running.claim_delivery_session().unwrap();
        assert!(matches!(Store::open(&path).unwrap().claim_delivery_session(), Err(Error::InUse)));
    }

    #[test]
    fn a_lock_left_by_a_moss_that_stopped_does_not_hold_up_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let _older = older_database(&path, 30);
        // The lock file stays after a crash; the OS has let go of the lock in it.
        drop(session::SessionLock::take(&path).unwrap());
        assert!(path.with_extension("session.lock").exists());
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, schema::MIGRATIONS.len());
        assert_eq!(copied(&path), (30, "kept".into()));
    }

    #[test]
    fn a_new_database_is_only_made_with_the_folder_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        std::fs::File::create(&path).unwrap();
        let running = session::SessionLock::take(&path).unwrap();
        assert!(matches!(Store::open(&path), Err(Error::InUse)));
        assert_eq!(version(&Connection::open(&path).unwrap()), 0);
        drop(running);
        assert_eq!(Store::open(&path).unwrap().schema_version().unwrap() as usize, schema::MIGRATIONS.len());
        // A lock that can't be made fails closed.
        let other = dir.path().join("other.db");
        std::fs::create_dir(other.with_extension("session.lock")).unwrap();
        assert!(matches!(Store::open(&other), Err(Error::Invalid(_))));
    }

    #[test]
    fn the_copy_from_before_an_update_goes_once_a_launch_needs_no_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let older = older_database(&path, 30);
        // The launch that updated keeps it, through the session claim too.
        let mut updated = Store::open(&path).unwrap();
        updated.claim_delivery_session().unwrap();
        drop((updated, older));
        assert_eq!(copied(&path), (30, "kept".into()));
        // A later launch that opens the mail with nothing to update has no need of it.
        let mut later = Store::open(&path).unwrap();
        assert!(safety_copy_path(&path).exists(), "only once it owns the folder");
        later.claim_delivery_session().unwrap();
        assert!(!safety_copy_path(&path).exists());
        drop(later);
        // Nor does an update that couldn't get the folder take a copy away.
        let older = dir.path().join("older.db");
        let _conn = older_database(&older, 30);
        std::fs::write(safety_copy_path(&older), b"from an earlier update").unwrap();
        let running = session::SessionLock::take(&older).unwrap();
        assert!(Store::open(&older).is_err());
        assert_eq!(std::fs::read(safety_copy_path(&older)).unwrap(), b"from an earlier update");
        drop(running);
    }

    #[test]
    fn forgetting_the_copy_removes_only_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        drop(Store::open(&path).unwrap());
        forget_safety_copy(&path).unwrap();
        std::fs::write(safety_copy_path(&path), b"old mail").unwrap();
        forget_safety_copy(&path).unwrap();
        assert!(!safety_copy_path(&path).exists());
        assert!(path.exists());
        std::fs::create_dir(safety_copy_path(&path)).unwrap();
        assert!(forget_safety_copy(&path).is_err(), "a folder there isn't Moss's copy");
        std::fs::remove_dir(safety_copy_path(&path)).unwrap();
        // A store forgets its own, as removing an account does.
        let store = Store::open(&path).unwrap();
        std::fs::write(safety_copy_path(&path), b"a removed account's mail").unwrap();
        store.forget_safety_copy().unwrap();
        assert!(!safety_copy_path(&path).exists());
        Store::open_in_memory().unwrap().forget_safety_copy().unwrap();
    }

    #[test]
    fn an_update_left_unfinished_is_told_from_a_damaged_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        assert!(!update_unfinished(&path), "no file");
        assert!(!path.exists(), "and none made");
        std::fs::write(&path, b"not a database, just some bytes that are long enough to look like a file").unwrap();
        assert!(!update_unfinished(&path));
        std::fs::remove_file(&path).unwrap();
        let older = older_database(&path, 30);
        assert!(update_unfinished(&path));
        drop(older);
        drop(Store::open(&path).unwrap());
        assert!(!update_unfinished(&path), "up to date");
        let new = dir.path().join("new.db");
        drop(Connection::open(&new).unwrap());
        assert!(!update_unfinished(&new), "empty");
    }

    #[test]
    fn a_failed_update_is_an_error_and_the_copy_from_before_it_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let last = schema::MIGRATIONS.len() - 1;
        let older = older_database(&path, last);
        // The last update can't run: the table it makes is already there.
        older.execute_batch("CREATE TABLE recipient_picks (x)").unwrap();
        assert!(matches!(Store::open(&path), Err(Error::Sqlite(_))));
        assert_eq!(version(&older), last as u32, "the update that failed changed nothing");
        assert_eq!(copied(&path), (last as u32, "kept".into()));
    }
}
