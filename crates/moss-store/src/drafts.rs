//! Unsent emails, saved as you type so nothing is lost, and kept in the account's Drafts
//! folder too, so they show on your phone and in webmail. Each draft remembers its copy
//! there and whether that copy is up to date; the engine pushes and pulls.

use mail_parser::MessageParser;
use moss_core::{ComposeMode, Draft, DraftFile};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};

use crate::codec::{json, name, named, parse_id, to_json};
use crate::parse::parse;
use crate::{text, Error, Result, Store};

/// The header that names the Moss draft a server copy is of.
pub const DRAFT_HEADER: &str = "X-Moss-Draft";
/// The header that says whether a server copy is a reply, reply all, forward or new email.
pub const MODE_HEADER: &str = "X-Moss-Mode";
/// The header with the signature you chose in Moss for a server copy: its id, or "none" when
/// you took it off. Missing when Moss picks, so another Mac never undoes your choice.
pub const SIGNATURE_HEADER: &str = "X-Moss-Signature";

/// Where a draft's copy in the Drafts folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCopy {
    /// Its Message-ID, without <>.
    pub message_id: String,
    /// Its id in the Drafts folder, once known.
    pub provider_id: Option<String>,
}

/// A draft as sync sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedDraft {
    pub draft: Draft,
    /// Opaque content identity. Timestamps are not versions.
    pub revision: String,
    pub copy: Option<ServerCopy>,
    /// The copy holds this version of the draft, so there's nothing to push.
    pub pushed: bool,
}

/// A message from the Drafts folder, read as a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDraft {
    /// Its id is the Moss draft it's a copy of, or "server:" and its Message-ID.
    pub draft: Draft,
    pub message_id: String,
    /// The Moss draft it's a copy of ([`DRAFT_HEADER`]), when Moss wrote it.
    pub moss_id: Option<String>,
    /// When it was saved (its Date).
    pub saved_at: i64,
    /// Its files, for the engine to keep with the draft. Pictures shown in its text aren't.
    pub files: Vec<ServerFile>,
}

/// A file on a draft from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFile {
    pub name: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

const COLUMNS: &str =
    "id, mode, thread_id, account_id, to_json, cc_json, bcc_json, subject, body, body_html, remind_if_no_reply, updated_at, signature_id, signature_chosen, from_address, files_json";

fn synced(r: &Row) -> rusqlite::Result<SyncedDraft> {
    let draft = draft(r)?;
    let copy = r
        .get::<_, Option<String>>(16)?
        .map(|message_id| Ok::<_, rusqlite::Error>(ServerCopy { message_id, provider_id: r.get(17)? }))
        .transpose()?;
    let revision: String = r.get(19)?;
    if revision.len() != 32 || !revision.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(rusqlite::Error::FromSqlConversionFailure(19,rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData,"The stored draft revision is invalid."))));
    }
    let pushed = copy.is_some() && r.get::<_, Option<String>>(18)?.as_deref() == Some(revision.as_str());
    Ok(SyncedDraft { draft, revision, copy, pushed })
}

const SYNCED: &str = "server_message_id, server_provider_id, pushed_revision, revision";

fn draft(r: &Row) -> rusqlite::Result<Draft> {
    Ok(Draft {
        id: r.get(0)?,
        revision: None,
        mode: named(r, 1)?,
        thread_id: r.get::<_, Option<i64>>(2)?.map(|id| id.to_string()),
        from_account_id: r.get(3)?,
        to: json(r, 4)?,
        cc: json(r, 5)?,
        bcc: json(r, 6)?,
        subject: r.get(7)?,
        body: r.get(8)?,
        body_html: r.get(9)?,
        remind_if_no_reply: r.get(10)?,
        updated_at: r.get(11)?,
        signature_id: r.get(12)?,
        signature_chosen: r.get(13)?,
        from_address: r.get(14)?,
        attachments: json(r, 15)?,
    })
}

impl Store {
    /// Check within the same transaction that will change content or its ownership.
    /// A missing token is never permission to overwrite or recreate an old identity.
    pub fn require_draft_revision(&self, id:&str, expected:Option<&str>)->Result<()> {
        if self.conn.is_autocommit() {
            return Err(Error::Invalid("Draft revision checks require an owning transaction.".into()));
        }
        let current=self.synced_draft(id)?;
        match (current,expected) {
            (Some(current),Some(expected)) if current.revision==expected=>Ok(()),
            (None,None)=>{
                let used:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM draft_identities WHERE id=?1)",[id],|row|row.get(0))?;
                if used {Err(Error::DraftConflict)} else {Ok(())}
            }
            _=>Err(Error::DraftConflict),
        }
    }

    /// Saves a draft, replacing any with the same id. Its server copy is kept, and is
    /// behind until it's pushed again.
    pub fn save_draft(&self, d: &Draft) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.save_draft_in_transaction(d)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn remember_draft_identity(&self, id: &str) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO draft_identities(id) VALUES (?1)", [id])?;
        self.require_recorded_draft_identity(id)
    }

    fn require_recorded_draft_identity(&self, id: &str) -> Result<()> {
        let recorded: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM draft_identities WHERE id=?1)", [id], |row| row.get(0))?;
        if !recorded { return Err(Error::Invalid("The draft identity could not be retained. No changes were saved.".into())); }
        Ok(())
    }

    /// The caller owns the transaction and must roll it back on any error.
    /// Used by attachment import and durable delivery restoration as one atomic change.
    pub(crate) fn save_draft_in_transaction(&self, d: &Draft) -> Result<()> {
        self.discard_removed_draft_files_in_transaction(d)?;
        let updates: Vec<String> =
            COLUMNS.split(", ").skip(1).map(|c| format!("{c} = excluded.{c}")).collect();
        let changed = self.conn.execute(
            &format!(
                "INSERT INTO drafts ({COLUMNS}) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
                 WHERE NOT EXISTS (SELECT 1 FROM deliveries WHERE draft_id = ?1 AND state != 'cancelled')
                   AND NOT EXISTS (SELECT 1 FROM scheduled WHERE draft_id = ?1)
                 ON CONFLICT (id) DO UPDATE SET {}",
                updates.join(", ")
            ),
            params![
                d.id,
                name(&d.mode),
                d.thread_id.as_deref().map(parse_id),
                d.from_account_id,
                to_json(&d.to),
                to_json(&d.cc),
                to_json(&d.bcc),
                d.subject,
                d.body,
                d.body_html,
                d.remind_if_no_reply,
                d.updated_at,
                d.signature_id,
                d.signature_chosen,
                d.from_address,
                to_json(&d.attachments),
            ],
        )?;
        if changed == 0 { return Err(Error::Invalid("This draft belongs to an outgoing delivery or schedule. Cancel it before editing.".into())); }
        self.require_recorded_draft_identity(&d.id)
    }

    /// A save or handoff releases only files removed from its previously saved row.
    /// The caller owns the transaction and rolls back on any error.
    pub(crate) fn discard_removed_draft_files_in_transaction(&self, d: &Draft) -> Result<()> {
        if self.conn.is_autocommit() {
            return Err(Error::Invalid("Draft changes require an owning transaction.".into()));
        }
        for file in d.attachments.iter().filter(|file| file.forwarded.is_none()) {
            if file.id.parse::<i64>().ok().is_none_or(|id| id.to_string() != file.id) {
                return Err(Error::Invalid("The attachment identifier is invalid. Attach the file again.".into()));
            }
            let owned: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM draft_files WHERE draft_id=?1 AND id=?2)", params![d.id, file.id], |row| row.get(0))?;
            if !owned { return Err(Error::Invalid("An attachment does not belong to this draft. Attach the file again.".into())); }
        }
        let previous: Vec<DraftFile> = self.conn.query_row(
            "SELECT files_json FROM drafts WHERE id = ?1", [&d.id], |row| json(row, 0),
        ).optional()?.unwrap_or_default();
        // Delete only files removed from the previously saved row. A file staged
        // concurrently has not reached that row yet and must survive an older autosave.
        let ids: Vec<i64> = previous.iter()
            .filter(|file| file.forwarded.is_none() && !d.attachments.iter().any(|kept| kept.forwarded.is_none() && kept.id == file.id))
            .map(|file| parse_id(&file.id)).collect();
        let sql = format!("DELETE FROM draft_files WHERE draft_id = ? AND id IN ({})", vec!["?"; ids.len()].join(", "));
        let values = std::iter::once(rusqlite::types::Value::from(d.id.clone())).chain(ids.into_iter().map(Into::into));
        self.conn.execute(&sql, params_from_iter(values))?;
        Ok(())
    }

    /// Keeps a file you attached to draft `draft_id`, which may not be saved yet.
    pub fn add_draft_file(
        &mut self,
        draft_id: &str,
        name: &str,
        content_type: &str,
        data: &[u8],
        now: i64,
    ) -> Result<DraftFile> {
        let tx = self.conn.unchecked_transaction()?;
        let sql = "INSERT INTO draft_files (draft_id, name, content_type, data, added_at) VALUES (?1, ?2, ?3, ?4, ?5)";
        self.conn.execute(sql, params![draft_id, name, content_type, data, now])?;
        let file = DraftFile {
            id: self.conn.last_insert_rowid().to_string(),
            name: name.into(),
            bytes: data.len() as u64,
            content_type: content_type.into(),
            forwarded: None,
        };
        tx.commit()?;
        Ok(file)
    }

    /// Counts all staged files, including ones not saved into the draft yet.
    pub fn draft_file_usage(&self, draft_id: &str) -> Result<(u64, u64)> {
        Ok(self.conn.query_row("SELECT count(*), coalesce(sum(length(data)), 0) FROM draft_files WHERE draft_id = ?1", [draft_id], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)))?)
    }

    /// Imports a batch atomically. A failed insert leaves none of the batch behind.
    pub fn add_draft_files(&mut self, draft_id: &str, files: &[(&str, &str, &[u8])], now: i64) -> Result<Vec<DraftFile>> {
        let tx = self.conn.unchecked_transaction()?;
        let added = self.insert_draft_files(draft_id, files, now)?;
        tx.commit()?;
        Ok(added)
    }

    /// Agent-supplied bytes and their owning draft are committed together.
    pub fn attach_and_save_draft(&mut self, draft: &Draft, files: &[(&str, &str, &[u8])]) -> Result<Draft> {
        let tx = self.conn.unchecked_transaction()?;
        let mut saved = draft.clone();
        saved.attachments.extend(self.insert_draft_files(&draft.id, files, draft.updated_at)?);
        self.save_draft_in_transaction(&saved)?;
        tx.commit()?;
        Ok(saved)
    }

    /// Save an edit and imported bytes with account-move bookkeeping in one commit.
    /// The caller encodes the provider-specific cleanup; the store owns its account.
    pub fn edit_draft(
        &mut self,
        draft: &Draft,
        files: &[(&str, &str, &[u8])],
        now: i64,
        cleanup: impl FnOnce(&ServerCopy) -> Result<Vec<String>>,
    ) -> Result<(Draft, Option<String>)> {
        self.delivery_transaction(|store| {
            store.require_draft_revision(&draft.id,draft.revision.as_deref())?;
            let before = store.synced_draft(&draft.id)?;
            let mut saved = draft.clone();
            saved.attachments.extend(store.insert_draft_files(&draft.id, files, now)?);
            store.save_draft_in_transaction(&saved)?;
            let mut moved = None;
            if let Some(before) = before.filter(|before| before.draft.from_account_id != saved.from_account_id) {
                store.set_server_copy(&saved.id, None, None)?;
                if let Some(copy) = before.copy {
                    store.queue_draft_cleanup_in_transaction(&before.draft.from_account_id, &cleanup(&copy)?, now)?;
                    moved = Some(before.draft.from_account_id);
                }
            }
            saved.revision=Some(store.synced_draft(&saved.id)?.ok_or(Error::DraftConflict)?.revision);
            Ok((saved, moved))
        })
    }

    pub(crate) fn insert_draft_files(&self, draft_id: &str, files: &[(&str, &str, &[u8])], now: i64) -> Result<Vec<DraftFile>> {
        let mut added = Vec::with_capacity(files.len());
        for (name, content_type, data) in files {
            self.conn.execute("INSERT INTO draft_files (draft_id, name, content_type, data, added_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![draft_id, name, content_type, data, now])?;
            added.push(DraftFile { id: self.conn.last_insert_rowid().to_string(), name: (*name).into(), bytes: data.len() as u64, content_type: (*content_type).into(), forwarded: None });
        }
        Ok(added)
    }

    /// Releases only an unreferenced staged file owned by this draft.
    pub fn discard_staged_draft_file(&self, draft_id: &str, id: &str) -> Result<bool> {
        let Ok(file_id) = id.parse::<i64>() else { return Ok(false); };
        // References use canonical ids. Reject aliases such as +1 or 01 before comparison.
        if id != file_id.to_string() { return Ok(false); }
        let tx = self.conn.unchecked_transaction()?;
        // Decode the complete reference set so malformed metadata fails closed.
        let files: Vec<DraftFile> = tx.query_row(
            "SELECT files_json FROM drafts WHERE id = ?1", [draft_id], |row| json(row, 0),
        ).optional()?.unwrap_or_default();
        if files.iter().any(|file| file.id == id) { return Ok(false); }
        let removed = tx.execute(
            "DELETE FROM draft_files WHERE draft_id = ?1 AND id = ?2
             AND NOT EXISTS (SELECT 1 FROM scheduled WHERE draft_id = ?1)
             AND NOT EXISTS (SELECT 1 FROM deliveries WHERE draft_id = ?1
                             AND state NOT IN ('complete', 'cancelled'))",
            params![draft_id, file_id],
        )? > 0;
        tx.commit()?;
        Ok(removed)
    }

    /// A file you attached: its name, content type and bytes.
    pub fn draft_file(&self, id: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        let Ok(id) = id.parse::<i64>() else { return Ok(None) };
        let sql = "SELECT name, content_type, data FROM draft_files WHERE id = ?1";
        Ok(self.conn.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?)
    }

    /// Read local bytes only within the draft that owns them.
    pub fn draft_file_for(&self, draft_id: &str, id: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        let Ok(parsed) = id.parse::<i64>() else { return Ok(None) };
        if parsed.to_string() != id { return Ok(None); }
        let id = parsed;
        Ok(self.conn.query_row("SELECT name, content_type, data FROM draft_files WHERE draft_id=?1 AND id=?2",
            params![draft_id, id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?)
    }

    /// Lets these files go: their email has gone.
    pub fn delete_draft_files(&mut self, ids: &[String]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for id in ids.iter().filter_map(|id| id.parse::<i64>().ok()) {
            tx.execute("DELETE FROM draft_files WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Lets go of every file kept for draft `draft_id`: it was discarded.
    #[cfg(test)]
    pub fn forget_draft_files(&mut self, draft_id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.ensure_draft_discardable(draft_id)?;
        tx.execute("DELETE FROM draft_files WHERE draft_id = ?1", [draft_id])?;
        tx.commit()?;
        Ok(())
    }

    fn ensure_draft_discardable(&self, draft_id: &str) -> Result<()> {
        let owned: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM scheduled WHERE draft_id = ?1)
             OR EXISTS(SELECT 1 FROM deliveries WHERE draft_id = ?1
               AND state NOT IN ('complete','cancelled'))",
            [draft_id], |row| row.get(0),
        )?;
        if owned { return Err(Error::Invalid("These files belong to an outgoing delivery or schedule. Cancel it before discarding them.".into())); }
        Ok(())
    }

    /// Discard an externally observed draft without deleting another editor's work.
    pub fn discard_editable_draft(
        &mut self, id: &str, expected_revision: Option<&str>, keep_files: bool, now: i64,
        cleanup: impl FnOnce(&ServerCopy) -> Result<Vec<String>>,
    ) -> Result<(bool, Option<String>)> {
        self.discard_draft_with_revision(id, Some(expected_revision), keep_files, now, cleanup)
    }

    /// Delete locally and retain server cleanup in the same fully durable commit.
    /// A missing editable row can still own unsaved staging, which is released only
    /// when requested and when no delivery or schedule owns it.
    pub fn discard_draft(
        &mut self,
        id: &str,
        keep_files: bool,
        now: i64,
        cleanup: impl FnOnce(&ServerCopy) -> Result<Vec<String>>,
    ) -> Result<(bool, Option<String>)> {
        self.discard_draft_with_revision(id, None, keep_files, now, cleanup)
    }

    fn discard_draft_with_revision(
        &mut self, id: &str, expected: Option<Option<&str>>, keep_files: bool, now: i64,
        cleanup: impl FnOnce(&ServerCopy) -> Result<Vec<String>>,
    ) -> Result<(bool, Option<String>)> {
        self.delivery_transaction(|store| {
            if let Some(revision) = expected { store.require_draft_revision(id, revision)?; }
            store.ensure_draft_discardable(id)?;
            let before = store.synced_draft(id)?;
            if !keep_files {
                store.conn.execute("DELETE FROM draft_files WHERE draft_id=?1", [id])?;
            }
            let Some(before) = before else { return Ok((false, None)) };
            if store.conn.execute("DELETE FROM drafts WHERE id=?1", [id])? != 1 {
                return Err(Error::Invalid("The draft could not be discarded.".into()));
            }
            let account = if let Some(copy) = before.copy {
                store.queue_draft_cleanup_in_transaction(&before.draft.from_account_id, &cleanup(&copy)?, now)?;
                Some(before.draft.from_account_id)
            } else { None };
            Ok((true, account))
        })
    }

    /// Transfer an editable row to a durable send/schedule without accepting an ignored delete.
    pub(crate) fn delete_transferred_draft(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM drafts WHERE id=?1", [id])?;
        let remains: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE id=?1)", [id], |row| row.get(0))?;
        if remains { return Err(Error::Invalid("The draft could not be transferred to its send operation. No local changes were saved.".into())); }
        Ok(())
    }

    /// Lets go of files whose draft is gone (discarded before it was saved, or sent while
    /// Moss quit), preserving scheduled and unresolved delivery files. Only safe
    /// while no composer is open. Returns how many went.
    pub fn forget_lost_draft_files(&mut self) -> Result<usize> {
        let sql = "DELETE FROM draft_files WHERE draft_id NOT IN (SELECT id FROM drafts)
                   AND draft_id NOT IN (SELECT draft_id FROM scheduled)
                   AND draft_id NOT IN (SELECT draft_id FROM deliveries
                     WHERE state NOT IN ('complete','cancelled'))";
        Ok(self.conn.execute(sql, [])?)
    }

    pub fn draft(&self, id: &str) -> Result<Option<Draft>> {
        let sql = format!("SELECT {COLUMNS} FROM drafts WHERE id = ?1");
        Ok(self.conn.query_row(&sql, [id], draft).optional()?)
    }

    /// Drafts in an account (`None` for all), most recently edited first.
    pub fn drafts(&self, account: Option<&str>) -> Result<Vec<Draft>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM drafts WHERE ?1 IS NULL OR account_id = ?1 ORDER BY updated_at DESC, id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([account], draft)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One editable row with its store-issued precondition.
    pub fn editable_draft(&self, id: &str) -> Result<Option<Draft>> {
        Ok(self.synced_draft(id)?.map(|current| Draft { revision: Some(current.revision), ..current.draft }))
    }

    /// Recover an acknowledgement only when all saved content still matches.
    /// This is a read, never permission to overwrite a newer or retired draft.
    pub fn draft_save_receipt(&self, attempted: &Draft) -> Result<Option<Draft>> {
        Ok(self.editable_draft(&attempted.id)?.filter(|current| {
            let mut content = attempted.clone();
            content.revision = current.revision.clone();
            content.updated_at = current.updated_at;
            content == *current
        }))
    }

    /// Editable drafts with the content identities required by external writers.
    pub fn editable_drafts(&self) -> Result<Vec<Draft>> {
        let sql = format!("SELECT {COLUMNS}, {SYNCED} FROM drafts ORDER BY updated_at DESC, id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let current = synced(row)?;
            let mut draft = current.draft;
            draft.revision = Some(current.revision);
            Ok(draft)
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Returns whether there was such a draft.
    pub fn delete_draft(&mut self, id: &str) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM drafts WHERE id = ?1", [id])? > 0)
    }

    /// An account's drafts with their server copies, most recently edited first.
    pub fn synced_drafts(&self, account_id: &str) -> Result<Vec<SyncedDraft>> {
        let sql = format!(
            "SELECT {COLUMNS}, {SYNCED} FROM drafts WHERE account_id = ?1 ORDER BY updated_at DESC, id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([account_id], synced)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn synced_draft(&self, id: &str) -> Result<Option<SyncedDraft>> {
        let sql = format!("SELECT {COLUMNS}, {SYNCED} FROM drafts WHERE id = ?1");
        Ok(self.conn.query_row(&sql, [id], synced).optional()?)
    }

    /// Records the draft's copy on the server, or that it has none. `pushed` is the version
    /// (the opaque content revision) the copy holds; `None` to push it again. Returns whether there was
    /// such a draft.
    pub fn set_server_copy(&self, id: &str, copy: Option<&ServerCopy>, pushed: Option<&str>) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE drafts SET server_message_id = ?2, server_provider_id = ?3, pushed_revision = ?4 WHERE id = ?1",
            params![id, copy.map(|c| &c.message_id), copy.and_then(|c| c.provider_id.as_ref()), pushed],
        )? > 0)
    }

    /// Record only the upload's observed account/copy lineage. Later edits stay pending
    /// because their revision differs from the one actually uploaded.
    pub(crate) fn record_uploaded_draft(&self, observed: &SyncedDraft, copy: &ServerCopy) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE drafts SET server_message_id=?1,server_provider_id=?2,pushed_revision=?3
             WHERE id=?4 AND account_id=?5 AND server_message_id IS ?6",
            params![copy.message_id,copy.provider_id,observed.revision,observed.draft.id,
                observed.draft.from_account_id,observed.copy.as_ref().map(|c|&c.message_id)],
        )? > 0)
    }

    /// Commit the upload receipt and its required server cleanup together.
    /// A stale acknowledgement owns only its newly uploaded copy, never the old one.
    /// Replaying an active receipt preserves later edits and provider-ID discovery.
    /// This does not close the crash window before this transaction; that needs the upload ledger.
    pub fn finish_draft_upload(
        &mut self,
        observed: &SyncedDraft,
        copy: &ServerCopy,
        now: i64,
        cleanup: impl FnOnce(&ServerCopy) -> String,
    ) -> Result<Option<(crate::ChangeId, ServerCopy)>> {
        self.delivery_transaction(|store| {
            let recorded_revision: Option<Option<String>> = store.conn.query_row(
                "SELECT pushed_revision FROM drafts WHERE id=?1 AND account_id=?2 AND server_message_id=?3",
                params![observed.draft.id, observed.draft.from_account_id, copy.message_id], |row| row.get(0),
            ).optional()?;
            if let Some(revision) = recorded_revision {
                if revision.as_deref() == Some(observed.revision.as_str()) { return Ok(None) }
                return Err(Error::UploadReceiptConflict);
            }
            let recorded = store.record_uploaded_draft(observed, copy)?;
            let target = if recorded {
                observed.copy.as_ref()
            } else { Some(copy) };
            let Some(target) = target else { return Ok(None) };
            let queued = store.queue_change(&observed.draft.from_account_id, &cleanup(target), now)?;
            Ok(Some((queued, target.clone())))
        })
    }

    /// Discover the provider id of the same copy without changing which content is pushed.
    pub fn confirm_server_copy(&self, id: &str, account: &str, copy: &ServerCopy) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE drafts SET server_provider_id=?1 WHERE id=?2 AND account_id=?3 AND server_message_id=?4",
            params![copy.provider_id,id,account,copy.message_id],
        )? > 0)
    }

    /// Clears the provider id after observing a missing copy.
    pub fn forget_server_provider_id(&self, observed: &SyncedDraft) -> Result<bool> {
        let Some(copy) = &observed.copy else { return Ok(false) };
        Ok(self.conn.execute(
            "UPDATE drafts SET server_provider_id=NULL
             WHERE id=?1 AND account_id=?2 AND server_message_id=?3 AND server_provider_id IS ?4",
            params![observed.draft.id, observed.draft.from_account_id, copy.message_id, copy.provider_id],
        )? > 0)
    }

    /// Remote hints cannot authorize moving a local draft to another account.
    /// Call before importing files, while retaining the store lock through the save.
    pub fn ensure_server_draft_account(&self, draft: &Draft) -> Result<()> {
        if !self.server_draft_account_matches(draft)? {
            return Err(Error::Invalid("A server draft conflicts with a draft in another account. Both copies have been kept.".into()));
        }
        Ok(())
    }

    /// A foreign hint is an item conflict, not permission to stop unrelated sync.
    pub fn server_draft_account_matches(&self,draft:&Draft)->Result<bool> {
        Ok(!self.conn.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE id=?1 AND account_id<>?2)",params![draft.id,draft.from_account_id],|row|row.get::<_,bool>(0))?)
    }

    /// Saves a draft as its copy on the server has it, so there's nothing to push.
    pub fn keep_server_draft(&mut self, draft: &Draft, copy: &ServerCopy) -> Result<()> {
        self.keep_server_draft_with_files(draft, copy, &[], draft.updated_at)
    }

    /// Commit remote content, imported files and copy bookkeeping together.
    pub fn keep_server_draft_with_files(
        &mut self,
        draft: &Draft,
        copy: &ServerCopy,
        files: &[ServerFile],
        now: i64,
    ) -> Result<()> {
        self.delivery_transaction(|store|store.keep_server_draft_in_transaction(draft,copy,files,now))
    }

    fn keep_server_draft_in_transaction(&self,draft:&Draft,copy:&ServerCopy,files:&[ServerFile],now:i64)->Result<()> {
        self.ensure_server_draft_account(draft)?;
        let files: Vec<_> = files.iter().map(|file| (file.name.as_str(), file.content_type.as_str(), file.data.as_slice())).collect();
        let mut saved = draft.clone();
        saved.attachments.extend(self.insert_draft_files(&saved.id, &files, now)?);
        self.save_draft_in_transaction(&saved)?;
        let revision = self.synced_draft(&saved.id)?.ok_or_else(|| Error::Invalid("The saved server draft is unavailable.".into()))?.revision;
        if !self.set_server_copy(&saved.id, Some(copy), Some(&revision))? {
            return Err(Error::Invalid("The server draft receipt was not saved.".into()));
        }
        Ok(())
    }

    /// Apply a network result only to the exact local snapshot it observed. A
    /// conflict keeps both copies for the next sync; it grants no remote deletion.
    pub fn apply_server_draft(&mut self, draft: &Draft, copy: &ServerCopy, files: &[ServerFile], now: i64, observed: Option<&SyncedDraft>) -> Result<bool> {
        self.delivery_transaction(|store| {
            if store.synced_draft(&draft.id)?.as_ref()!=observed { return Ok(false); }
            store.keep_server_draft_in_transaction(draft,copy,files,now)?;
            Ok(true)
        })
    }

    /// Keep conflicting remote content independently, without replacing local text.
    pub fn keep_conflicting_server_draft(&mut self, original_id:&str, recovered:&Draft, copy:&ServerCopy, files:&[ServerFile], now:i64)->Result<bool> {
        self.delivery_transaction(|store| {
            let known:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE account_id=?1 AND server_message_id=?2)",params![recovered.from_account_id,copy.message_id],|row|row.get(0))?;
            if known { return Ok(false); }
            if recovered.id==original_id || store.synced_draft(&recovered.id)?.is_some() {
                return Err(Error::Invalid("The recovered draft needs its own unused identity.".into()));
            }
            let original=store.synced_draft(original_id)?;
            if original.as_ref().is_some_and(|draft|draft.draft.from_account_id!=recovered.from_account_id) {
                return Err(Error::Invalid("The conflicting draft belongs to another account.".into()));
            }
            store.keep_server_draft_in_transaction(recovered,copy,files,now)?;
            // The adopted copy still names the original draft. Publish this recovery
            // under its own identity before another device removes that old copy.
            if !store.set_server_copy(&recovered.id,Some(copy),None)? {
                return Err(Error::Invalid("The recovered draft's upload state was not saved.".into()));
            }
            // Its old remote copy may have disappeared. Keep the current local text
            // pending for upload so absence handling cannot delete it on this pull.
            if let Some(original)=original {
                if !store.set_server_copy(original_id,original.copy.as_ref(),None)? {
                    return Err(Error::Invalid("The local draft's recovery state was not saved.".into()));
                }
            }
            Ok(true)
        })
    }

    /// An absent server copy can remove only the unchanged, fully pushed row.
    pub fn delete_server_draft(&mut self, observed: &SyncedDraft) -> Result<bool> {
        self.delivery_transaction(|store| {
            if !observed.pushed || store.synced_draft(&observed.draft.id)?.as_ref()!=Some(observed) { return Ok(false); }
            Ok(store.conn.execute("DELETE FROM drafts WHERE id=?1",[&observed.draft.id])?>0)
        })
    }

    /// Reads a message from the account's Drafts folder as a draft. A reply stays in its
    /// conversation when that is on this Mac; one written in another app loses its quote,
    /// which Moss adds again when it sends. `undated` is its time when it has no Date.
    /// `None` when it isn't an email.
    pub fn draft_from_server(&self, account_id: &str, raw: &[u8], undated: i64) -> Result<Option<ServerDraft>> {
        let address: String = self
            .conn
            .query_row("SELECT address FROM accounts WHERE id = ?1", [account_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::UnknownAccount(account_id.into()))?;
        let Some(parsed) = parse(raw) else {
            return Ok(None);
        };
        let message = MessageParser::default().parse(raw);
        let header = |name: &str| {
            let value = message.as_ref()?.headers().get(name)?.raw_value();
            Some(String::from_utf8_lossy(value).trim().to_string()).filter(|v| !v.is_empty())
        };
        let moss_id = header(DRAFT_HEADER);
        let mut thread_id = None;
        for r in parsed.refs.iter().rev() {
            thread_id = self
                .conn
                .query_row(
                    "SELECT thread_id FROM messages WHERE account_id = ?1 AND message_id = ?2",
                    params![account_id, r],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if thread_id.is_some() {
                break;
            }
        }
        let mode = match (thread_id, header(MODE_HEADER)) {
            (None, _) => ComposeMode::New,
            (Some(_), Some(mode)) => serde_json::from_value(mode.into())
                .ok()
                .filter(|m| *m != ComposeMode::New)
                .unwrap_or(ComposeMode::Reply),
            (Some(_), None) => ComposeMode::Reply,
        };
        let foreign_reply = moss_id.is_none() && thread_id.is_some();
        // Only HTML (as Gmail web may leave it): a line per block, so the text reads as written
        // and a signature at its end can be told apart. Otherwise the plain-text part.
        let plain = message.as_ref().is_some_and(|m| m.text_body().any(|p| p.is_content_type("text", "plain")));
        let text = match (&parsed.body_html, plain) {
            (Some(html), false) => text::html_lines(html),
            _ => parsed.body_text.clone(),
        };
        let (body, body_html) = match foreign_reply {
            true => (text::strip_quoted(&text), String::new()),
            false => (text.trim_end().to_string(), parsed.body_html.clone().unwrap_or_default()),
        };
        let from = parsed.from.email.trim().to_lowercase();
        let saved_at = parsed.sent_at.unwrap_or(undated);
        let chosen = moss_id.as_ref().and(header(SIGNATURE_HEADER));
        let draft = Draft {
        revision: None,
            id: moss_id.clone().unwrap_or_else(|| format!("server:{}", parsed.message_id)),
            mode,
            thread_id: thread_id.map(|id| id.to_string()),
            from_account_id: account_id.into(),
            to: parsed.to,
            cc: parsed.cc,
            bcc: parsed.bcc,
            subject: parsed.subject,
            body,
            body_html,
            remind_if_no_reply: false,
            updated_at: saved_at,
            signature_chosen: chosen.is_some(),
            signature_id: chosen.filter(|id| id != "none"),
            from_address: (!from.is_empty() && from != address.to_lowercase()).then_some(from),
            attachments: vec![],
        };
        let shown = |p: &mail_parser::MessagePart| {
            let html = parsed.body_html.as_deref().unwrap_or_default();
            p.content_id().is_some_and(|id| crate::parse::shown_in(html, id))
        };
        let files = message
            .as_ref()
            .map(|m| {
                crate::parse::files(m)
                    .filter(|p| !shown(p))
                    .map(|p| ServerFile {
                        name: p.attachment_name().unwrap_or("attachment").to_string(),
                        content_type: p
                            .content_type()
                            .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")))
                            .unwrap_or_else(|| "application/octet-stream".into())
                            .to_ascii_lowercase(),
                        data: p.decoded().into_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(ServerDraft { draft, message_id: parsed.message_id, moss_id, saved_at, files }))
    }
}

#[cfg(test)]
mod tests {
    use super::{ServerCopy, ServerFile, SyncedDraft};
    use crate::test_support::*;
    use moss_core::{ComposeMode, DraftFile, FileRef};

    fn discard_change(copy: &ServerCopy) -> String {
        format!("delete {}", copy.message_id)
    }

    fn bounded_discard_change(copy: &ServerCopy) -> crate::Result<Vec<String>> {
        Ok(vec![discard_change(copy)])
    }

    #[test]
    fn attachment_id_aliases_cannot_delete_the_owned_bytes() {
        let mut store = store_with_work();
        let mut owner = crate::test_support::draft("owner");
        let file = store.add_draft_file(&owner.id, "kept.txt", "text/plain", b"keep", NOV_1).unwrap();
        owner.attachments.push(file.clone());
        store.save_draft(&owner).unwrap();
        for alias in [format!("0{}", file.id), format!("+{}", file.id), format!("{}e0", file.id)] {
            let mut forged = owner.clone();
            forged.attachments[0].id = alias.clone();
            assert!(store.save_draft(&forged).is_err(), "attachment aliases must be rejected");
            assert!(store.draft_file_for(&owner.id, &alias).unwrap().is_none());
            assert_eq!(store.draft(&owner.id).unwrap(), Some(owner.clone()));
            assert_eq!(store.draft_file(&file.id).unwrap().unwrap().2, b"keep");
        }
    }

    #[test]
    fn a_draft_cannot_claim_another_drafts_local_attachment() {
        let mut store = store_with_work();
        let mut owner = crate::test_support::draft("owner");
        let file = store.add_draft_file(&owner.id, "private.txt", "text/plain", b"private fixture bytes", NOV_1).unwrap();
        owner.attachments.push(file.clone());
        store.save_draft(&owner).unwrap();
        let target = crate::test_support::draft("target");
        store.save_draft(&target).unwrap();
        let mut forged = target.clone();
        forged.attachments.push(file.clone());
        assert!(store.save_draft(&forged).is_err(), "foreign attachment ownership must be rejected");
        assert_eq!(store.draft(&target.id).unwrap(), Some(target));
        assert_eq!(store.draft(&owner.id).unwrap(), Some(owner));
        assert_eq!(store.draft_file(&file.id).unwrap().unwrap().2, b"private fixture bytes");
        assert!(store.draft_file_for("target", &file.id).unwrap().is_none());
        assert!(store.draft_file_for("owner", "not-an-id").unwrap().is_none());
        assert_eq!(store.draft_file_for("owner", &file.id).unwrap().unwrap().2, b"private fixture bytes");
    }

    #[test]
    fn pre_network_cleanup_limits_preserve_draft_copy_and_files() {
        for editing in [false, true] {
            let mut store = store_with_work();
            store.upsert_account(&account("other", "other@example.test")).unwrap();
            let original = store.attach_and_save_draft(&draft("bounded"), &[("kept.txt", "text/plain", b"kept")]).unwrap();
            store.set_server_copy(&original.id, Some(&copy("old@example.test", Some("1:1"))), None).unwrap();
            let before = store.synced_draft(&original.id).unwrap();
            let oversized = |_: &ServerCopy| Ok(vec!["x".repeat(crate::MAX_CHANGE_PAYLOAD_BYTES + 1)]);
            let result = if editing {
                let edited = moss_core::Draft { revision: before.as_ref().map(|d| d.revision.clone()), from_account_id: "other".into(), ..original.clone() };
                store.edit_draft(&edited, &[("new.txt", "text/plain", b"new")], NOV_1, oversized).map(|_| ())
            } else {
                store.discard_draft(&original.id, false, NOV_1, oversized).map(|_| ())
            };
            assert!(result.is_err());
            assert_eq!(store.synced_draft(&original.id).unwrap(), before);
            assert!(store.pending_changes("work").unwrap().is_empty());
            assert!(store.pending_changes("other").unwrap().is_empty());
            assert_eq!(store.conn.query_row("SELECT count(*) FROM draft_files", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
            assert_eq!(store.draft_file(&original.attachments[0].id).unwrap().unwrap().2, b"kept");
        }
    }

    #[test]
    fn account_move_cleanup_uses_full_durability_and_its_own_batch() {
        let mut store = store_with_work();
        store.upsert_account(&account("other", "other@example.test")).unwrap();
        let original = draft("durable-move");
        store.keep_server_draft(&original, &copy("old@example.test", Some("1:1"))).unwrap();
        store.conn.pragma_update(None, "synchronous", 1).unwrap();
        store.conn.execute_batch("CREATE TRIGGER require_durable_move BEFORE UPDATE ON drafts WHEN (SELECT synchronous FROM pragma_synchronous) != 2 OR (SELECT fullfsync FROM pragma_fullfsync) != 1 BEGIN SELECT RAISE(ABORT,'move requires full durability'); END;").unwrap();
        let edited = moss_core::Draft { revision: Some(store.synced_draft(&original.id).unwrap().unwrap().revision), from_account_id:"other".into(), ..original };
        let (_, account) = store.edit_draft(&edited, &[], NOV_1, bounded_discard_change).unwrap();
        assert_eq!(account.as_deref(), Some("work"));
        let pending = store.pending_changes("work").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(store.batch_for_change("work", pending[0].0).unwrap().unwrap().kind, crate::BatchKind::DraftCleanup);
        assert_eq!(store.conn.pragma_query_value(None, "synchronous", |r|r.get::<_,i64>(0)).unwrap(), 1);
    }

    #[test]
    fn an_unknown_delivery_state_keeps_discard_and_file_cleanup_closed() {
        let mut store = store_with_work();
        let d = store.attach_and_save_draft(&draft("unknown-owner"), &[("kept.txt", "text/plain", b"kept")]).unwrap();
        store.prepare_delivery("unknown-operation", &d, "{}", "unknown@fixture", NOV_1).unwrap();
        store.conn.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE deliveries SET state='future-state';").unwrap();
        assert!(!store.discard_staged_draft_file(&d.id, &d.attachments[0].id).unwrap());
        assert_eq!(store.forget_lost_draft_files().unwrap(), 0);
        for keep_files in [false, true] {
            assert!(store.discard_draft(&d.id, keep_files, NOV_1, bounded_discard_change).is_err());
            assert_eq!(store.draft_file(&d.attachments[0].id).unwrap().unwrap().2, b"kept");
        }
        assert!(store.pending_changes("work").unwrap().is_empty());
        assert!(store.forget_draft_files(&d.id).is_err());
        assert_eq!(store.draft_file(&d.attachments[0].id).unwrap().unwrap().2, b"kept");
    }

    #[test]
    fn discard_commits_cleanup_and_file_policy_across_reopening() {
        for keep_files in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("discard.db");
            let mut store = crate::Store::open(&path).unwrap();
            store.upsert_account(&account("work", "work@example.test")).unwrap();
            let d = store.attach_and_save_draft(&draft("discard"), &[("kept.txt", "text/plain", b"kept")]).unwrap();
            store.keep_server_draft(&d, &copy("remove@fixture", Some("1:1"))).unwrap();
            store.conn.execute_batch("CREATE TRIGGER require_durable_discard BEFORE DELETE ON drafts WHEN (SELECT synchronous FROM pragma_synchronous) != 2 OR (SELECT fullfsync FROM pragma_fullfsync) != 1 BEGIN SELECT RAISE(ABORT,'discard requires full durability'); END;").unwrap();
            let outcome = store.discard_draft(&d.id, keep_files, NOV_1, bounded_discard_change).unwrap();
            assert_eq!(outcome, (true, Some("work".into())));
            assert_eq!(store.conn.pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0)).unwrap(), 1);
            drop(store);
            let mut store = crate::Store::open(&path).unwrap();
            assert_eq!(store.draft(&d.id).unwrap(), None);
            assert_eq!(store.draft_file(&d.attachments[0].id).unwrap().is_some(), keep_files);
            assert_eq!(store.pending_changes("work").unwrap().iter().map(|(_, json)| json.as_str()).collect::<Vec<_>>(), ["delete remove@fixture"]);
            assert_eq!(store.discard_draft(&d.id, keep_files, NOV_1 + 1, bounded_discard_change).unwrap(), (false, None));
            assert_eq!(store.pending_changes("work").unwrap().len(), 1);
        }
    }

    #[test]
    fn discard_handles_local_only_rows_and_unsaved_staging() {
        for saved in [false, true] {
            for keep_files in [false, true] {
                let mut store = store_with_work();
                let d = draft("local-discard");
                let file = store.add_draft_file(&d.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
                if saved { store.save_draft(&d).unwrap(); }
                assert_eq!(store.discard_draft(&d.id, keep_files, NOV_1, bounded_discard_change).unwrap(), (saved, None));
                assert!(store.draft(&d.id).unwrap().is_none());
                assert_eq!(store.draft_file(&file.id).unwrap().is_some(), keep_files);
                assert!(store.pending_changes("work").unwrap().is_empty());
            }
        }
    }

    #[test]
    fn ignored_or_failed_discard_rolls_back_files_row_and_queue() {
        for trigger in [
            "CREATE TRIGGER refuse_discard BEFORE DELETE ON drafts BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER refuse_discard BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            "CREATE TRIGGER refuse_discard BEFORE DELETE ON draft_files BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            "CREATE TRIGGER refuse_discard BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;",
        ] {
            let mut store = store_with_work();
            let d = store.attach_and_save_draft(&draft("failed-discard"), &[("kept.txt", "text/plain", b"kept")]).unwrap();
            store.keep_server_draft(&d, &copy("kept@fixture", Some("1:1"))).unwrap();
            let before = store.synced_draft(&d.id).unwrap().unwrap();
            store.conn.execute_batch(trigger).unwrap();
            assert!(store.discard_draft(&d.id, false, NOV_1, bounded_discard_change).is_err(), "{trigger}");
            assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), before);
            assert_eq!(store.draft_file(&d.attachments[0].id).unwrap().unwrap().2, b"kept");
            assert!(store.pending_changes("work").unwrap().is_empty());
        }
    }

    #[test]
    fn remote_file_import_rolls_back_every_write_boundary() {
        for trigger in [
            "CREATE TRIGGER fail_remote BEFORE INSERT ON draft_files WHEN NEW.name='second.txt' BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            "CREATE TRIGGER fail_remote BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            "CREATE TRIGGER fail_remote BEFORE UPDATE OF server_message_id ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            "CREATE TRIGGER fail_remote BEFORE UPDATE OF server_message_id ON drafts BEGIN SELECT RAISE(IGNORE); END;",
        ] {
            let mut store = store_with_work();
            let d = store.attach_and_save_draft(&draft("remote-atomic"), &[("original.txt", "text/plain", b"original")]).unwrap();
            let before = store.synced_draft(&d.id).unwrap().unwrap();
            store.conn.execute_batch(trigger).unwrap();
            let remote = moss_core::Draft { attachments: vec![], body: "remote edit".into(), ..d.clone() };
            let files = ["first.txt", "second.txt"].map(|name| ServerFile { name: name.into(), content_type: "text/plain".into(), data: b"remote".to_vec() });
            assert!(store.keep_server_draft_with_files(&remote, &copy("remote@fixture", Some("1:3")), &files, NOV_1).is_err());
            assert_eq!(store.synced_draft(&d.id).unwrap(), Some(before));
            assert_eq!(store.draft_file_usage(&d.id).unwrap(), (1, 8));
            assert_eq!(store.draft_file(&d.attachments[0].id).unwrap().unwrap().2, b"original");
        }
    }

    #[test]
    fn remote_files_cannot_leak_into_scheduled_or_outgoing_drafts() {
        for scheduled in [true, false] {
            let mut store = store_with_work();
            let d = store.attach_and_save_draft(&draft("remote-owned"), &[("original.txt", "text/plain", b"original")]).unwrap();
            if scheduled { store.schedule_email(&d, "{}", NOV_1, NOV_1).unwrap(); }
            else { store.prepare_delivery("remote-owner", &d, "{}", "owned@fixture", NOV_1).unwrap(); }
            let files = [ServerFile { name: "remote.txt".into(), content_type: "text/plain".into(), data: b"remote".to_vec() }];
            assert!(store.keep_server_draft_with_files(&d, &copy("remote@fixture", Some("1:3")), &files, NOV_1).is_err());
            assert_eq!(store.draft_file_usage(&d.id).unwrap(), (1, 8));
            if scheduled { store.unschedule_email(&d.id, NOV_1).unwrap(); }
            else { store.cancel_delivery("remote-owner").unwrap(); }
            assert_eq!(store.draft(&d.id).unwrap(), Some(d));
        }
    }

    fn expected_revision(draft:&moss_core::Draft, revision:Option<&str>)->moss_core::Draft {
        let mut value=serde_json::to_value(draft).unwrap();
        value["revision"]=serde_json::to_value(revision).unwrap();
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn stale_user_edits_cannot_replace_newer_content_or_remove_its_imported_files() {
        let mut store=store_with_work();let original=draft("versioned-edit");
        store.save_draft(&original).unwrap();
        let before=store.synced_draft(&original.id).unwrap().unwrap();
        let mut newer=expected_revision(&original,Some(&before.revision));newer.body="Newer text".into();
        store.edit_draft(&newer,&[("new.txt","text/plain",b"new bytes")],NOV_1,bounded_discard_change).unwrap();
        let saved=store.synced_draft(&original.id).unwrap().unwrap();
        assert_ne!(before.revision,saved.revision);
        let stale=expected_revision(&original,Some(&before.revision));
        assert!(store.edit_draft(&stale,&[],NOV_1,bounded_discard_change).is_err());
        assert_eq!(store.synced_draft(&original.id).unwrap(),Some(saved));
        assert_eq!(store.draft_file_usage(&original.id).unwrap(),(1,9));
    }

    #[test]
    fn stale_discard_preserves_the_winner_and_its_files() {
        let mut store=store_with_work();let original=draft("discard-revision");
        let (first,_)=store.edit_draft(&original,&[],NOV_1,bounded_discard_change).unwrap();
        let mut edit=first.clone();edit.body="Winner".into();
        let (winner,_)=store.edit_draft(&edit,&[("winner.txt","text/plain",b"winner")],NOV_1,bounded_discard_change).unwrap();
        assert!(store.discard_editable_draft(&original.id,first.revision.as_deref(),false,NOV_1,bounded_discard_change).is_err());
        assert_eq!(store.editable_drafts().unwrap(),vec![winner.clone()]);
        assert_eq!(store.draft_file_usage(&original.id).unwrap(),(1,6));
        assert!(store.discard_editable_draft(&original.id,winner.revision.as_deref(),false,NOV_1,bounded_discard_change).unwrap().0);
        assert!(store.editable_drafts().unwrap().is_empty());
        assert_eq!(store.draft_file_usage(&original.id).unwrap(),(0,0));
    }

    #[test]
    fn editable_revision_is_not_part_of_delivery_or_schedule_content() {
        for scheduled in [false,true] {
            let mut store=store_with_work();let original=draft("revision-snapshot");
            let (mut saved,_)=store.edit_draft(&original,&[],NOV_1,bounded_discard_change).unwrap();
            if scheduled {
                store.schedule_operation("snapshot-operation",&saved,"{}",NOV_1+1000,NOV_1).unwrap();
                let snapshot=store.scheduled_email_for_operation("snapshot-operation").unwrap().unwrap();
                assert_eq!(snapshot.draft.revision,None);
                saved.revision=Some("different-expectation".into());
                assert_eq!(store.schedule_operation("snapshot-operation",&saved,"{}",NOV_1+1000,NOV_1).unwrap(),crate::ScheduleOutcome::AlreadyScheduled);
            } else {
                store.prepare_delivery("snapshot-operation",&saved,"{}","message@fixture",NOV_1).unwrap();
                assert_eq!(store.delivery("snapshot-operation").unwrap().unwrap().draft.revision,None);
                saved.revision=Some("different-expectation".into());
                assert!(!store.prepare_delivery("snapshot-operation",&saved,"{}","message@fixture",NOV_1).unwrap());
            }
        }
    }

    fn transferred_identity_survives_account_removal(scheduled: bool) {
        let mut store=store_with_work();let original=draft("transfer-identity");
        if scheduled { store.schedule_operation("identity-operation",&original,"{}",NOV_1+1000,NOV_1).unwrap(); }
        else { store.prepare_delivery("identity-operation",&original,"{}","identity@fixture",NOV_1).unwrap(); }
        let remembered:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM draft_identities WHERE id=?1)",[&original.id],|row|row.get(0)).unwrap();
        assert!(remembered);
        if scheduled { store.cancel_schedule_operation("identity-operation",&original.id,NOV_1).unwrap(); }
        else {
            store.cancel_delivery("identity-operation").unwrap();
        }
        store.delete_draft(&original.id).unwrap();
        store.remove_account("work").unwrap();
        if !scheduled { assert_eq!(store.delivery_state("identity-operation").unwrap(), Some(crate::DeliveryState::Cancelled)); }
        store.upsert_account(&account("work","work@example.test")).unwrap();
        assert!(store.edit_draft(&original,&[],NOV_1,bounded_discard_change).is_err());
    }

    #[test]
    fn delivery_transfer_remembers_previously_unsaved_identities() { transferred_identity_survives_account_removal(false); }

    #[test]
    fn schedule_transfer_remembers_previously_unsaved_identities() { transferred_identity_survives_account_removal(true); }

    #[test]
    fn delivery_receipts_retain_the_consumed_edit_precondition_separately_from_content() {
        let mut store=store_with_work();let original=draft("receipt-precondition");
        let (saved,_)=store.edit_draft(&original,&[],NOV_1,bounded_discard_change).unwrap();
        store.prepare_delivery("receipt-operation",&saved,"{}","receipt@fixture",NOV_1).unwrap();
        let receipt=store.delivery("receipt-operation").unwrap().unwrap();
        assert_eq!(receipt.draft.revision,None);
        assert_eq!(serde_json::to_value(receipt).unwrap()["sourceRevision"],serde_json::to_value(saved.revision).unwrap());
    }

    #[test]
    fn ignored_identity_recording_rolls_back_the_edit_and_imported_bytes() {
        let mut store=store_with_work();let original=draft("identity-fault");
        store.conn.execute_batch("CREATE TRIGGER ignore_identity BEFORE INSERT ON draft_identities BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(store.edit_draft(&original,&[("kept.txt","text/plain",b"private")],NOV_1,bounded_discard_change).is_err());
        assert_eq!(store.draft(&original.id).unwrap(),None);
        assert_eq!(store.draft_file_usage(&original.id).unwrap(),(0,0));
    }

    #[test]
    fn draft_revision_checks_refuse_to_run_outside_the_owning_transaction() {
        let store = store_with_work();
        assert!(matches!(store.require_draft_revision("new", None), Err(crate::Error::Invalid(_))));
        assert!(store.drafts(None).unwrap().is_empty());
    }

    #[test]
    fn save_receipts_require_every_content_field_and_fail_closed_on_read_errors() {
        let mut store = store_with_work();
        let attempted = draft("receipt-fields");
        let (ack, _) = store.edit_draft(&attempted, &[], NOV_1, bounded_discard_change).unwrap();
        let changes = serde_json::json!({
            "id": "another-id", "mode": "forward", "threadId": "123", "fromAccountId": "other",
            "to": [], "cc": [{"name":"Other","email":"other@example.test"}],
            "bcc": [{"name":"Other","email":"other@example.test"}],
            "subject": "Different", "body": "Different", "bodyHtml": "<p>Different</p>",
            "remindIfNoReply": true, "signatureId": "other", "signatureChosen": true,
            "fromAddress": "other@example.test",
            "attachments": [{"id":"999","name":"missing.txt","bytes":1,"contentType":"text/plain","forwarded":null}]
        });
        for (field, value) in changes.as_object().unwrap() {
            let mut changed = serde_json::to_value(&attempted).unwrap();
            changed[field] = value.clone();
            let changed: moss_core::Draft = serde_json::from_value(changed).unwrap();
            assert_eq!(store.draft_save_receipt(&changed).unwrap(), None, "{field}");
        }
        assert_eq!(store.editable_draft(&attempted.id).unwrap(), Some(ack));
        store.conn.execute_batch("ALTER TABLE drafts RENAME TO unavailable_drafts").unwrap();
        assert!(store.draft_save_receipt(&attempted).is_err());
    }

    #[test]
    fn lost_save_acknowledgement_is_recovered_without_rewriting_content() {
        let mut store = store_with_work();
        let mut attempted = draft("lost-save-ack");
        attempted.attachments.push(store.add_draft_file(&attempted.id, "kept.txt", "text/plain", b"kept", NOV_1).unwrap());
        assert_eq!(store.draft_save_receipt(&attempted).unwrap(), None);
        let (ack, _) = store.edit_draft(&attempted, &[], NOV_1, bounded_discard_change).unwrap();
        attempted.updated_at += 100;
        assert_eq!(store.draft_save_receipt(&attempted).unwrap(), Some(ack.clone()));
        assert_eq!(store.editable_draft(&attempted.id).unwrap(), Some(ack.clone()));
        assert_eq!(store.draft_file_usage(&attempted.id).unwrap(), (1, 4));
        let mut newer = ack.clone(); newer.body = "A different writer's edit".into();
        let (winner, _) = store.edit_draft(&newer, &[], NOV_1, bounded_discard_change).unwrap();
        assert_eq!(store.draft_save_receipt(&attempted).unwrap(), None);
        assert_eq!(store.editable_draft(&attempted.id).unwrap(), Some(winner));
        store.delete_draft(&attempted.id).unwrap();
        assert_eq!(store.draft_save_receipt(&attempted).unwrap(), None);
        assert!(store.edit_draft(&attempted, &[], NOV_1, bounded_discard_change).is_err());
    }

    #[test]
    fn absent_expectations_are_insert_only_and_cannot_resurrect_a_deleted_identity() {
        let mut store=store_with_work();let original=draft("insert-only");
        let (saved,_)=store.edit_draft(&original,&[],NOV_1,bounded_discard_change).unwrap();
        let value=serde_json::to_value(&saved).unwrap();
        assert_eq!(value["revision"].as_str().map(str::len),Some(32));
        assert!(store.edit_draft(&original,&[],NOV_1,bounded_discard_change).is_err());
        store.delete_draft(&original.id).unwrap();
        assert!(store.edit_draft(&original,&[],NOV_1,bounded_discard_change).is_err());
        assert!(store.edit_draft(&saved,&[],NOV_1,bounded_discard_change).is_err());
        assert_eq!(store.draft(&original.id).unwrap(),None);
    }

    #[test]
    fn remote_conflict_recovery_never_reuses_an_existing_or_foreign_identity() {
        let mut store=store_with_work();let original=draft("original");
        store.keep_server_draft(&original,&copy("old@fixture",Some("1:1"))).unwrap();
        let occupied=draft("occupied");store.save_draft(&occupied).unwrap();
        store.upsert_account(&account("other","other@example.test")).unwrap();
        let foreign=moss_core::Draft{from_account_id:"other".into(),..draft("foreign")};
        let before=store.drafts(None).unwrap();
        for recovered in [original.clone(),occupied,foreign] {
            assert!(store.keep_conflicting_server_draft(&original.id,&recovered,&copy("new@fixture",Some("1:2")),&[],NOV_1).is_err());
            assert_eq!(store.drafts(None).unwrap(),before);
        }
        let recovered=draft("separate-copy");
        assert!(store.keep_conflicting_server_draft("already-deleted",&recovered,&copy("new@fixture",Some("1:2")),&[],NOV_1).unwrap());
        assert_eq!(store.draft("already-deleted").unwrap(),None);
        assert_eq!(store.draft(&recovered.id).unwrap(),Some(recovered));
    }

    #[test]
    fn remote_conflict_copy_and_local_reupload_commit_together() {
        for (target,refusal) in [("original",None),("original",Some("ABORT,'fixture'")),("original",Some("IGNORE")),("recovered",Some("ABORT,'fixture'")),("recovered",Some("IGNORE"))] {
            let mut store=store_with_work();let original=draft("original");
            store.keep_server_draft(&original,&copy("old@fixture",Some("1:1"))).unwrap();
            let before=store.synced_draft(&original.id).unwrap().unwrap();
            if let Some(refusal)=refusal { store.conn.execute_batch(&format!("CREATE TRIGGER refuse_reupload BEFORE UPDATE OF pushed_revision ON drafts WHEN OLD.id='{target}' AND NEW.pushed_revision IS NULL BEGIN SELECT RAISE({refusal}); END;")).unwrap(); }
            let recovered=moss_core::Draft{id:"recovered".into(),body:"remote conflict".into(),..original.clone()};
            let files=[ServerFile{name:"remote.txt".into(),content_type:"text/plain".into(),data:b"remote bytes".to_vec()}];
            let result=store.keep_conflicting_server_draft(&original.id,&recovered,&copy("conflict@fixture",Some("1:2")),&files,NOV_1);
            if refusal.is_some() {
                assert!(result.is_err());
                assert_eq!(store.synced_draft(&original.id).unwrap(),Some(before));
                assert_eq!(store.draft(&recovered.id).unwrap(),None);
                assert_eq!(store.draft_file_usage(&recovered.id).unwrap(),(0,0));
            } else {
                assert!(result.unwrap());
                let local=store.synced_draft(&original.id).unwrap().unwrap();
                assert_eq!(local.draft,original);assert!(!local.pushed);
                assert_eq!(store.draft(&recovered.id).unwrap().unwrap().body,"remote conflict");
                assert!(!store.synced_draft(&recovered.id).unwrap().unwrap().pushed);
                assert_eq!(store.draft_file_usage(&recovered.id).unwrap(),(1,12));
                let duplicate=moss_core::Draft{id:"second-recovery".into(),..recovered};
                assert!(!store.keep_conflicting_server_draft(&original.id,&duplicate,&copy("conflict@fixture",Some("1:2")),&files,NOV_1).unwrap());
                assert_eq!(store.draft(&duplicate.id).unwrap(),None);
            }
        }
    }

    #[test]
    fn fetched_server_drafts_cannot_replace_edits_deletions_or_recreated_rows() {
        for action in ["edit","delete","recreate"] {
            let mut store=store_with_work();let original=draft("race");
            store.keep_server_draft(&original,&copy("old@fixture",Some("1:1"))).unwrap();
            let observed=store.synced_draft(&original.id).unwrap().unwrap();
            let mut newer=original.clone();newer.body="new local text".into();
            match action {
                "edit"=>store.save_draft(&newer).unwrap(),
                "delete"=>{store.delete_draft(&original.id).unwrap();},
                _=>{store.delete_draft(&original.id).unwrap();store.save_draft(&original).unwrap();},
            }
            let before=store.synced_draft(&original.id).unwrap();
            let mut remote=original.clone();remote.body="stale network text".into();
            let files=[ServerFile{name:"remote.txt".into(),content_type:"text/plain".into(),data:b"remote bytes".to_vec()}];
            assert!(!store.apply_server_draft(&remote,&copy("new@fixture",Some("1:2")),&files,NOV_1,Some(&observed)).unwrap(),"{action}");
            assert!(!store.delete_server_draft(&observed).unwrap(),"{action}");
            assert_eq!(store.synced_draft(&original.id).unwrap(),before);
            assert_eq!(store.draft_file_usage(&original.id).unwrap(),(0,0));
        }
    }

    #[test]
    fn remote_insert_is_insert_only_and_current_remote_edits_can_commit() {
        let mut store=store_with_work();let d=draft("insert");let c=copy("remote@fixture",Some("1:1"));
        assert!(store.apply_server_draft(&d,&c,&[],NOV_1,None).unwrap());
        let observed=store.synced_draft(&d.id).unwrap().unwrap();
        assert!(!store.apply_server_draft(&d,&c,&[],NOV_1,None).unwrap());
        let mut next=d.clone();next.body="updated remote".into();
        assert!(store.apply_server_draft(&next,&c,&[],NOV_1,Some(&observed)).unwrap());
        let current=store.synced_draft(&d.id).unwrap().unwrap();
        assert_eq!(current.draft.body,"updated remote");assert!(current.pushed);
        let mut local=next.clone();local.body="not uploaded".into();
        store.save_draft(&local).unwrap();
        let dirty=store.synced_draft(&d.id).unwrap().unwrap();
        assert!(!dirty.pushed);
        assert!(!store.delete_server_draft(&dirty).unwrap());
        assert_eq!(store.draft(&d.id).unwrap(),Some(local));
        store.keep_server_draft(&next,&c).unwrap();
        let current=store.synced_draft(&d.id).unwrap().unwrap();
        assert!(store.delete_server_draft(&current).unwrap());
        assert_eq!(store.draft(&d.id).unwrap(),None);
    }

    #[test]
    fn server_account_check_preserves_database_failures() {
        let store = store_with_work();
        store.conn.execute_batch("ALTER TABLE drafts RENAME TO unavailable_drafts;").unwrap();
        assert!(matches!(store.ensure_server_draft_account(&draft("unreadable")), Err(crate::Error::Sqlite(_))));
    }

    #[test]
    fn server_draft_save_cannot_move_an_existing_draft_between_accounts() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "personal@example.test")).unwrap();
        let d = draft("foreign-server-hint");
        store.keep_server_draft(&d, &copy("original@fixture", Some("1:1"))).unwrap();
        let before = store.synced_draft(&d.id).unwrap().unwrap();
        let foreign = moss_core::Draft { from_account_id: "personal".into(), body: "foreign content".into(), ..d };
        let result = store.keep_server_draft(&foreign, &copy("foreign@fixture", Some("2:9")));
        assert_eq!(store.synced_draft(&foreign.id).unwrap(), Some(before));
        assert!(matches!(result, Err(crate::Error::Invalid(_))));
    }

    #[test]
    fn losing_the_saved_server_draft_rolls_back_the_replacement() {
        let mut store = store_with_work();
        let mut d = draft("lost-server-save");
        store.keep_server_draft(&d, &copy("old@fixture", Some("1:1"))).unwrap();
        let before = store.synced_draft(&d.id).unwrap().unwrap();
        store.conn.execute_batch("CREATE TRIGGER lose_saved_draft AFTER UPDATE OF body ON drafts BEGIN DELETE FROM drafts WHERE id=NEW.id; END;").unwrap();
        d.body = "new server content".into();
        assert!(matches!(store.keep_server_draft(&d, &copy("new@fixture", Some("1:2"))), Err(crate::Error::Invalid(_))));
        assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), before);
    }

    #[test]
    fn every_draft_content_field_changes_its_revision() {
        let mut store = store_with_work();
        store.upsert_account(&account("other", "other@example.test")).unwrap();
        let thread = ingest(&mut store, "revision-thread", &mail().build()).thread_id;
        let changes = [
            ("mode", "'reply'".to_string()), ("thread_id", thread),
            ("account_id", "'other'".into()), ("to_json", "'[]'".into()),
            ("cc_json", "'[{\"name\":\"CC\",\"email\":\"cc@example.test\"}]'".into()),
            ("bcc_json", "'[{\"name\":\"BCC\",\"email\":\"bcc@example.test\"}]'".into()),
            ("subject", "'Changed subject'".into()), ("body", "'Changed text'".into()),
            ("body_html", "'<p>Changed HTML</p>'".into()), ("remind_if_no_reply", "1".into()),
            ("signature_id", "'changed-signature'".into()), ("signature_chosen", "1".into()),
            ("from_address", "'alias@example.test'".into()),
            ("files_json", "'[{\"id\":\"1\",\"name\":\"file.txt\",\"bytes\":1,\"contentType\":\"text/plain\",\"forwarded\":null}]'".into()),
        ];
        for (column, value) in changes {
            let d = draft(column);
            store.save_draft(&d).unwrap();
            let before = store.synced_draft(&d.id).unwrap().unwrap().revision;
            let sql = format!("UPDATE drafts SET {column}={value} WHERE id=?1");
            store.conn.execute(&sql, [&d.id]).unwrap();
            let after = store.synced_draft(&d.id).unwrap().unwrap().revision;
            assert_ne!(after, before, "{column}");
            store.conn.execute(&sql, [&d.id]).unwrap();
            assert_eq!(store.synced_draft(&d.id).unwrap().unwrap().revision, after, "no-op {column}");
        }
    }

    #[test]
    fn missing_server_copy_cannot_change_a_moved_or_replaced_draft() {
        let mut store = store_with_work();
        store.upsert_account(&account("other", "other@example.test")).unwrap();
        let mut d = draft("missing-copy");
        store.keep_server_draft(&d, &copy("old@fixture", Some("1:1"))).unwrap();
        let observed = store.synced_draft(&d.id).unwrap().unwrap();
        d.from_account_id = "other".into();
        store.save_draft(&d).unwrap();
        let moved = store.synced_draft(&d.id).unwrap().unwrap();
        assert!(!store.forget_server_provider_id(&observed).unwrap());
        assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), moved);
        d.from_account_id = "work".into();
        store.save_draft(&d).unwrap();
        store.keep_server_draft(&d, &copy("new@fixture", Some("1:2"))).unwrap();
        let replaced = store.synced_draft(&d.id).unwrap().unwrap();
        assert!(!store.forget_server_provider_id(&observed).unwrap());
        assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), replaced);
        store.confirm_server_copy(&d.id, "work", &copy("new@fixture", Some("2:9"))).unwrap();
        let discovered = store.synced_draft(&d.id).unwrap().unwrap();
        assert!(!store.forget_server_provider_id(&replaced).unwrap());
        assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), discovered);
        assert!(store.forget_server_provider_id(&discovered).unwrap());
        let cleared = store.synced_draft(&d.id).unwrap().unwrap();
        assert_eq!(cleared.copy, Some(copy("new@fixture", None)));
        assert_eq!(cleared.draft, d);
        assert_eq!(cleared.revision, discovered.revision);
        assert!(cleared.pushed);
        let mut absent = cleared.clone(); absent.copy = None;
        assert!(!store.forget_server_provider_id(&absent).unwrap());
        assert!(store.delete_draft(&d.id).unwrap());
        assert!(!store.forget_server_provider_id(&cleared).unwrap());
        read_only(&store);
        assert!(store.forget_server_provider_id(&cleared).is_err());
    }

    #[test]
    fn draft_revision_upgrade_preserves_content_and_legacy_upload_bookkeeping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v29-drafts.db");
        let mut old = historical_store(&path, 29);
        old.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        let mut expected = Vec::new();
        for (id, copy_id, pushed_at, pushed) in [
            ("current", Some("current@fixture"), Some(NOV_1), true),
            ("behind", Some("behind@fixture"), Some(NOV_1 - 1), false),
            ("unknown", Some("unknown@fixture"), None, false),
            ("absent", None, Some(NOV_1), false),
        ] {
            let mut d = draft(id);
            d.attachments.push(old.add_draft_file(id, "kept.txt", "text/plain", b"kept", NOV_1).unwrap());
            insert_historical_draft(&old, &d);
            old.conn.execute("UPDATE drafts SET server_message_id=?1,server_provider_id='1:1',pushed_at=?2 WHERE id=?3",
                rusqlite::params![copy_id,pushed_at,id]).unwrap();
            expected.push((d,pushed));
        }
        drop(old);
        let store = crate::Store::open(&path).unwrap();
        let mut revisions = std::collections::HashSet::new();
        for (d,pushed) in &expected {
            let current = store.synced_draft(&d.id).unwrap().unwrap();
            assert_eq!(current.draft, *d);
            assert_eq!(current.pushed, *pushed);
            assert!(revisions.insert(current.revision.clone()));
            assert_eq!(current.revision.len(), 32);
            assert_eq!(store.draft_file(&d.attachments[0].id).unwrap().unwrap().2, b"kept");
        }
        let before = store.synced_drafts("work").unwrap();
        drop(store);
        let store = crate::Store::open(&path).unwrap();
        assert_eq!(store.synced_drafts("work").unwrap(), before);
    }

    #[test]
    fn upload_receipt_and_old_copy_cleanup_commit_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.db");
        let mut store = crate::Store::open(&path).unwrap();
        store.upsert_account(&account("work", "work@example.test")).unwrap();
        let d = draft("receipt");
        store.keep_server_draft(&d, &copy("old@fixture", Some("1:1"))).unwrap();
        let observed = store.synced_draft(&d.id).unwrap().unwrap();
        let uploaded = copy("new@fixture", None);
        store.conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
        store.conn.execute_batch("CREATE TRIGGER require_durable_receipt BEFORE UPDATE ON drafts WHEN (SELECT synchronous FROM pragma_synchronous)!=2 OR (SELECT fullfsync FROM pragma_fullfsync)!=1 BEGIN SELECT RAISE(ABORT,'receipt requires full durability'); END;").unwrap();
        let (queued, target) = store.finish_draft_upload(&observed, &uploaded, NOV_1, discard_change).unwrap().unwrap();
        assert_eq!(target, observed.copy.unwrap());
        drop(store);
        let store = crate::Store::open(&path).unwrap();
        let current = store.synced_draft(&d.id).unwrap().unwrap();
        assert_eq!(current.copy, Some(uploaded));
        assert!(current.pushed);
        assert_eq!(store.pending_changes("work").unwrap(), vec![(queued, "delete old@fixture".into())]);
        assert_eq!(store.conn.pragma_query_value(None, "synchronous", |r|r.get::<_,i64>(0)).unwrap(), 1);
    }

    #[test]
    fn failed_upload_cleanup_queue_rolls_back_the_receipt() {
        for action in ["ABORT,'fixture'", "IGNORE"] {
            let mut store = store_with_work();
            let d = draft("receipt-rollback");
            store.keep_server_draft(&d, &copy("old@fixture", Some("1:1"))).unwrap();
            let observed = store.synced_draft(&d.id).unwrap().unwrap();
            store.conn.execute_batch(&format!("CREATE TRIGGER refuse_receipt_cleanup BEFORE INSERT ON outbox BEGIN SELECT RAISE({action}); END;")).unwrap();
            assert!(store.finish_draft_upload(&observed, &copy("new@fixture", None), NOV_1, discard_change).is_err(), "{action}");
            assert_eq!(store.synced_draft(&d.id).unwrap(), Some(observed));
            assert!(store.pending_changes("work").unwrap().is_empty());
        }
    }

    #[test]
    fn replayed_upload_receipt_never_queues_its_active_copy_for_deletion() {
        for previous_copy in [false, true] {
            let mut store = store_with_work();
            let mut d = draft("receipt-replay");
            if previous_copy { store.keep_server_draft(&d, &copy("old@fixture", Some("1:1"))).unwrap(); }
            else { store.save_draft(&d).unwrap(); }
            let observed = store.synced_draft(&d.id).unwrap().unwrap();
            let uploaded = copy("accepted@fixture", None);
            store.finish_draft_upload(&observed, &uploaded, NOV_1, discard_change).unwrap();
            let queued = store.pending_changes("work").unwrap();
            d.body.push_str(" newer local edit");
            store.save_draft(&d).unwrap();
            store.confirm_server_copy(&d.id, "work", &copy("accepted@fixture", Some("1:9"))).unwrap();
            let current = store.synced_draft(&d.id).unwrap();
            assert!(store.finish_draft_upload(&observed, &uploaded, NOV_1 + 1, discard_change).unwrap().is_none());
            assert_eq!(store.synced_draft(&d.id).unwrap(), current);
            assert_eq!(store.pending_changes("work").unwrap(), queued);
        }
    }

    #[test]
    fn reused_upload_message_id_cannot_acknowledge_different_content() {
        let mut store = store_with_work();
        let mut d = draft("receipt-reused-id");
        let original = copy("same@fixture", Some("1:1"));
        store.keep_server_draft(&d, &original).unwrap();
        d.body.push_str(" changed content");
        store.save_draft(&d).unwrap();
        let observed = store.synced_draft(&d.id).unwrap().unwrap();
        assert!(store.finish_draft_upload(&observed, &copy("same@fixture", Some("1:2")), NOV_1, discard_change).is_err());
        assert_eq!(store.synced_draft(&d.id).unwrap(), Some(observed));
        assert!(store.pending_changes("work").unwrap().is_empty());
    }

    #[test]
    fn draft_revisions_track_content_not_save_time_or_sync_metadata() {
        for recursive in [false, true] {
            let mut store = store_with_work();
            store.conn.pragma_update(None, "recursive_triggers", recursive).unwrap();
            let mut d = draft("revision");
            store.save_draft(&d).unwrap();
            let first = store.synced_draft(&d.id).unwrap().unwrap();
            assert_eq!(first.revision.len(), 32);
            store.save_draft(&d).unwrap();
            d.updated_at += 1;
            store.save_draft(&d).unwrap();
            assert_eq!(store.synced_draft(&d.id).unwrap().unwrap().revision, first.revision);
            let uploaded = copy("revision@fixture", None);
            assert!(store.record_uploaded_draft(&first, &uploaded).unwrap());
            let current = store.synced_draft(&d.id).unwrap().unwrap();
            assert_eq!(current.revision, first.revision);
            assert!(current.pushed);
            d.body.push_str(" edited at the same time");
            store.save_draft(&d).unwrap();
            let edited = store.synced_draft(&d.id).unwrap().unwrap();
            assert_ne!(edited.revision, first.revision);
            assert!(!edited.pushed);
            assert!(store.delete_draft(&d.id).unwrap());
            store.save_draft(&d).unwrap();
            let recreated = store.synced_draft(&d.id).unwrap().unwrap();
            assert_ne!(recreated.revision, edited.revision);
            assert_ne!(recreated.revision, first.revision);
            assert!(!recreated.pushed);
        }
    }

    #[test]
    fn upload_acknowledgements_preserve_edits_and_reject_changed_copy_lineage() {
        let mut store = store_with_work();
        let mut d = draft("upload-revision");
        store.save_draft(&d).unwrap();
        let observed = store.synced_draft(&d.id).unwrap().unwrap();
        d.body.push_str(" newer");
        store.save_draft(&d).unwrap();
        let uploaded = copy("uploaded@fixture", None);
        assert!(store.record_uploaded_draft(&observed, &uploaded).unwrap());
        let pending = store.synced_draft(&d.id).unwrap().unwrap();
        assert_eq!(pending.draft, d);
        assert!(!pending.pushed);
        assert_eq!(pending.copy, Some(uploaded.clone()));
        assert!(!store.record_uploaded_draft(&observed, &copy("stale@fixture", None)).unwrap());
        assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), pending);
        let mut wrong_account = pending.clone();
        wrong_account.draft.from_account_id = "another-account".into();
        assert!(!store.record_uploaded_draft(&wrong_account, &uploaded).unwrap());
        assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), pending);
        assert!(store.record_uploaded_draft(&pending, &copy("latest@fixture", None)).unwrap());
        assert!(store.synced_draft(&d.id).unwrap().unwrap().pushed);
        assert!(store.delete_draft(&d.id).unwrap());
        assert!(!store.record_uploaded_draft(&pending, &uploaded).unwrap());
        read_only(&store);
        assert!(store.record_uploaded_draft(&pending, &uploaded).is_err());
    }

    #[test]
    fn discovering_a_provider_id_cannot_mark_a_local_edit_as_uploaded() {
        let mut store = store_with_work();
        let mut d = draft("discovery");
        store.keep_server_draft(&d, &copy("same@fixture", None)).unwrap();
        d.body.push_str(" local edit");
        store.save_draft(&d).unwrap();
        let before = store.synced_draft(&d.id).unwrap().unwrap();
        for (id, account, message) in [(&*d.id, "other", "same@fixture"), (&*d.id, "work", "other@fixture"), ("missing", "work", "same@fixture")] {
            assert!(!store.confirm_server_copy(id, account, &copy(message, Some("1:9"))).unwrap());
            assert_eq!(store.synced_draft(&d.id).unwrap().unwrap(), before);
        }
        assert!(store.confirm_server_copy(&d.id, "work", &copy("same@fixture", Some("1:9"))).unwrap());
        let after = store.synced_draft(&d.id).unwrap().unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.draft, d);
        assert_eq!(after.copy, Some(copy("same@fixture", Some("1:9"))));
        assert!(!after.pushed);
        read_only(&store);
        assert!(store.confirm_server_copy(&d.id, "work", &copy("same@fixture", None)).is_err());
    }

    #[test]
    fn corrupt_draft_revisions_fail_closed() {
        let store = store_with_work();
        store.save_draft(&draft("corrupt-revision")).unwrap();
        store.conn.pragma_update(None, "ignore_check_constraints", true).unwrap();
        for revision in ["", "short", "0123456789abcdef0123456789abcdeG", "0123456789abcdef0123456789abcdef00", "0123456789abcdef0123456789abcdeF"] {
            store.conn.execute("UPDATE drafts SET revision=?1 WHERE id='corrupt-revision'", [revision]).unwrap();
            assert!(store.synced_draft("corrupt-revision").is_err(), "{revision}");
            assert!(store.synced_drafts("work").is_err(), "{revision}");
        }
    }

    #[test]
    fn discarding_staged_files_checks_owner_and_releases_their_quota() {
        let mut store = store_with_work();
        let file = store.add_draft_file("d1", "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
        assert!(!store.discard_staged_draft_file("another-draft", &file.id).unwrap());
        assert!(!store.discard_staged_draft_file("d1", "message:1").unwrap());
        assert_eq!(store.draft_file_usage("d1").unwrap(), (1, 6));
        assert!(store.discard_staged_draft_file("d1", &file.id).unwrap());
        assert_eq!(store.draft_file_usage("d1").unwrap(), (0, 0));
        assert!(!store.discard_staged_draft_file("d1", &file.id).unwrap());
    }

    #[test]
    fn numeric_aliases_cannot_bypass_saved_attachment_protection() {
        let mut store = store_with_work();
        let saved = store.attach_and_save_draft(&draft("aliased-file"), &[("saved.txt", "text/plain", b"saved")]).unwrap();
        let id = &saved.attachments[0].id;
        for alias in [format!("0{id}"), format!("+{id}")] {
            assert!(!store.discard_staged_draft_file(&saved.id, &alias).unwrap());
            assert!(store.draft_file(id).unwrap().is_some());
        }
    }

    #[test]
    fn a_saved_attachment_cannot_be_discarded_as_staging() {
        let mut store = store_with_work();
        let saved = store.attach_and_save_draft(&draft("saved-file"), &[("saved.txt", "text/plain", b"saved")]).unwrap();
        assert!(!store.discard_staged_draft_file(&saved.id, &saved.attachments[0].id).unwrap());
        let other = store.add_draft_file(&saved.id, "other.txt", "text/plain", b"other", NOV_1).unwrap();
        assert!(store.discard_staged_draft_file(&saved.id, &other.id).unwrap());
        assert_eq!(store.draft_file_usage(&saved.id).unwrap(), (1, 5));
    }

    #[test]
    fn pending_deliveries_protect_their_files_until_completion_or_cancellation() {
        let mut store = store_with_work();
        let d = draft("pending-file");
        let file = store.add_draft_file(&d.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
        store.prepare_delivery("pending", &d, "{}", "pending@fixture", NOV_1).unwrap();
        for state in ["held", "claimed", "uncertain", "accepted"] {
            store.conn.execute("UPDATE deliveries SET state = ?1 WHERE id = 'pending'", [state]).unwrap();
            assert!(!store.discard_staged_draft_file(&d.id, &file.id).unwrap(), "{state}");
            assert_eq!(store.draft_file_usage(&d.id).unwrap(), (1, 6));
        }
        store.complete_delivery("pending").unwrap();
        assert!(store.discard_staged_draft_file(&d.id, &file.id).unwrap());
        let cancelled = draft("cancelled-file");
        let staged = store.add_draft_file(&cancelled.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
        store.prepare_delivery("cancelled", &cancelled, "{}", "cancelled@fixture", NOV_1).unwrap();
        store.cancel_delivery("cancelled").unwrap();
        assert!(store.discard_staged_draft_file(&cancelled.id, &staged.id).unwrap());
    }

    #[test]
    fn schedules_protect_staging_until_the_draft_is_returned() {
        let mut store = store_with_work();
        let d = draft("scheduled-file");
        let file = store.add_draft_file(&d.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
        store.schedule_email(&d, "{}", NOV_1, NOV_1).unwrap();
        assert!(!store.discard_staged_draft_file(&d.id, &file.id).unwrap());
        store.unschedule_email(&d.id, NOV_1).unwrap();
        assert!(store.discard_staged_draft_file(&d.id, &file.id).unwrap());
    }

    #[test]
    fn corrupt_references_or_write_errors_fail_closed_during_staged_discard() {
        let mut store = store_with_work();
        let d = draft("corrupt-file");
        store.save_draft(&d).unwrap();
        let file = store.add_draft_file(&d.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
        for corrupt in ["not json", "{}", "[{\"id\":1}]"] {
            store.conn.execute("UPDATE drafts SET files_json = ?2 WHERE id = ?1", rusqlite::params![d.id, corrupt]).unwrap();
            assert!(store.discard_staged_draft_file(&d.id, &file.id).is_err());
            assert_eq!(store.draft_file_usage(&d.id).unwrap(), (1, 6));
        }
        store.conn.execute("UPDATE drafts SET files_json = '[]' WHERE id = ?1", [&d.id]).unwrap();
        read_only(&store);
        assert!(store.discard_staged_draft_file(&d.id, &file.id).is_err());
        assert!(store.draft_file(&file.id).unwrap().is_some());
    }

    #[test]
    fn an_untyped_server_attachment_has_a_safe_binary_content_type() {
        let store = store_with_work();
        let raw = b"From: Maya <maya@northwind.co>\r\nTo: Daniel <daniel@northwind.co>\r\nMessage-ID: <untyped@fixture>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nDraft text\r\n--b\r\nContent-Disposition: attachment; filename=unknown.bin\r\nContent-Transfer-Encoding: base64\r\n\r\nAAEC\r\n--b--\r\n";
        let server = store.draft_from_server("work", raw, NOV_1).unwrap().unwrap();
        assert_eq!(server.files, vec![ServerFile { name: "unknown.bin".into(), content_type: "application/octet-stream".into(), data: vec![0, 1, 2] }]);
    }

    #[test]
    fn the_transaction_internal_writer_refuses_autocommit() {
        let store = store_with_work();
        let draft = draft("unguarded-save");
        assert!(store.save_draft_in_transaction(&draft).is_err());
        assert_eq!(store.draft(&draft.id).unwrap(), None);
    }

    #[test]
    fn a_server_replacement_releases_only_previously_saved_files() {
        let mut store = store_with_work();
        let old = store.attach_and_save_draft(&draft("server-files"), &[("old.txt", "text/plain", b"old")]).unwrap();
        let staged = store.add_draft_file(&old.id, "new.txt", "text/plain", b"new", NOV_1).unwrap();
        let server = moss_core::Draft { attachments: vec![], body: "Remote edit".into(), ..old.clone() };
        store.keep_server_draft(&server, &copy("remote@fixture", Some("1"))).unwrap();
        assert_eq!(store.draft_file(&old.attachments[0].id).unwrap(), None);
        assert_eq!(store.draft_file(&staged.id).unwrap().unwrap().2, b"new");
        assert!(store.synced_draft(&old.id).unwrap().unwrap().pushed);
    }

    #[test]
    fn cancelling_a_delivery_keeps_files_staged_during_the_hold() {
        let mut store = store_with_work();
        let saved = store.attach_and_save_draft(&draft("held-files"), &[("saved.txt", "text/plain", b"saved")]).unwrap();
        store.prepare_delivery("held-operation", &saved, "outgoing", "held@fixture", NOV_1).unwrap();
        let staged = store.add_draft_file(&saved.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
        assert!(store.cancel_delivery("held-operation").unwrap());
        assert_eq!(store.draft(&saved.id).unwrap(), Some(saved.clone()));
        assert_eq!(store.draft_file(&saved.attachments[0].id).unwrap().unwrap().2, b"saved");
        assert_eq!(store.draft_file(&staged.id).unwrap().unwrap().2, b"staged");
    }

    #[test]
    fn an_autosave_does_not_delete_a_file_staged_after_its_snapshot() {
        let mut store = store_with_work();
        let old = draft("staging-race");
        store.save_draft(&old).unwrap();
        let staged = store.add_draft_file(&old.id, "new.txt", "text/plain", b"new attachment", NOV_1).unwrap();
        let edited = moss_core::Draft { body: "Text typed before file selection completed".into(), ..old };
        store.save_draft(&edited).unwrap();
        assert_eq!(store.draft_file(&staged.id).unwrap().unwrap().2, b"new attachment");
        assert_eq!(store.draft(&edited.id).unwrap(), Some(edited.clone()));
        let attached = moss_core::Draft { attachments: vec![staged.clone()], ..edited.clone() };
        store.save_draft(&attached).unwrap();
        // Deliberate removal after the file was saved still releases its bytes.
        store.save_draft(&edited).unwrap();
        assert_eq!(store.draft_file(&staged.id).unwrap(), None);
    }

    #[test]
    fn failed_file_cleanup_does_not_save_a_partial_draft() {
        let mut store = store_with_work();
        let saved = store.attach_and_save_draft(&draft("atomic-save"), &[("notes.txt", "text/plain", b"important")]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER fail_file_cleanup BEFORE DELETE ON draft_files BEGIN SELECT RAISE(ABORT, 'fixture cleanup failure'); END;").unwrap();
        let edited = moss_core::Draft { body: "Edited text".into(), attachments: vec![], ..saved.clone() };
        assert!(store.save_draft(&edited).is_err());
        assert_eq!(store.draft(&saved.id).unwrap(), Some(saved.clone()));
        assert_eq!(store.draft_file(&saved.attachments[0].id).unwrap().unwrap().2, b"important");
    }

    #[test]
    fn failed_server_copy_update_does_not_replace_the_local_draft() {
        let mut store = store_with_work();
        let saved = draft("atomic-server-save");
        store.save_draft(&saved).unwrap();
        store.conn.execute_batch("CREATE TRIGGER fail_server_copy BEFORE UPDATE OF server_message_id ON drafts BEGIN SELECT RAISE(ABORT, 'fixture copy failure'); END;").unwrap();
        let edited = moss_core::Draft { body: "Server text".into(), ..saved.clone() };
        assert!(store.keep_server_draft(&edited, &copy("remote@fixture", Some("1"))).is_err());
        assert_eq!(store.draft(&saved.id).unwrap(), Some(saved));
    }

    #[test]
    fn drafts_are_saved_replaced_listed_and_deleted() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        let mut reply = draft("d1");
        reply.mode = ComposeMode::Reply;
        reply.thread_id = Some(thread.clone());
        reply.signature_id = Some("work-short".into());
        reply.signature_chosen = true;
        reply.from_address = Some("hi@rossstudio.art".into());
        store.save_draft(&reply).unwrap();
        let personal = draft("d2");
        let personal = moss_core::Draft {
            from_account_id: "personal".into(),
            updated_at: NOV_1 + 1,
            ..personal
        };
        store.save_draft(&personal).unwrap();
        assert_eq!(store.draft("d1").unwrap(), Some(reply.clone()));
        assert_eq!(store.draft("nope").unwrap(), None);
        assert_eq!(store.drafts(None).unwrap(), vec![personal.clone(), reply.clone()]);
        assert_eq!(store.drafts(Some("work")).unwrap(), vec![reply.clone()]);

        reply.body = "Signed.".into();
        store.save_draft(&reply).unwrap();
        assert_eq!(store.draft("d1").unwrap().unwrap().body, "Signed.");
        assert_eq!(store.drafts(None).unwrap().len(), 2);

        assert!(store.delete_draft("d1").unwrap());
        assert!(!store.delete_draft("d1").unwrap());
        assert_eq!(store.drafts(None).unwrap(), vec![personal]);
    }

    #[test]
    fn a_file_that_went_never_lends_its_id_to_the_next_one() {
        // Its thumbnail is kept by id, so a new file with that id would show the old one.
        let mut store = store_with_work();
        let first = store.add_draft_file("d1", "a.pdf", "application/pdf", b"%PDF", NOV_1).unwrap();
        store.forget_draft_files("d1").unwrap();
        let next = store.add_draft_file("d2", "b.pdf", "application/pdf", b"%PDF-2", NOV_1).unwrap();
        assert_ne!(next.id, first.id);
        assert_eq!(store.draft_file(&first.id).unwrap(), None);
    }

    fn copy(id: &str, uid: Option<&str>) -> ServerCopy {
        ServerCopy { message_id: id.into(), provider_id: uid.map(String::from) }
    }

    #[test]
    fn a_draft_remembers_its_server_copy_and_whether_that_copy_is_up_to_date() {
        let store = store_with_work();
        let d = draft("d1");
        store.save_draft(&d).unwrap();
        let synced = store.synced_drafts("work").unwrap();
        assert_eq!(synced, vec![SyncedDraft { draft: d.clone(), revision: synced[0].revision.clone(), copy: None, pushed: false }]);
        assert_eq!(store.synced_draft("d1").unwrap(), Some(synced[0].clone()));
        assert_eq!(store.synced_draft("nope").unwrap(), None);
        assert!(store.synced_drafts("personal").unwrap().is_empty());

        assert!(store.set_server_copy("d1", Some(&copy("c1@x", Some("7:1"))), Some(&synced[0].revision)).unwrap());
        let up = store.synced_draft("d1").unwrap().unwrap();
        assert_eq!((up.copy, up.pushed), (Some(copy("c1@x", Some("7:1"))), true));

        // Saving again keeps the copy, which is now behind.
        let edited = moss_core::Draft { body: "Friday at 1?".into(), updated_at: d.updated_at + 1, ..d.clone() };
        store.save_draft(&edited).unwrap();
        let behind = store.synced_draft("d1").unwrap().unwrap();
        assert_eq!((behind.draft, behind.copy, behind.pushed), (edited, Some(copy("c1@x", Some("7:1"))), false));

        assert!(store.set_server_copy("d1", None, None).unwrap());
        assert_eq!(store.synced_draft("d1").unwrap().unwrap().copy, None);
        assert!(!store.set_server_copy("nope", None, None).unwrap());
        read_only(&store);
        assert!(store.set_server_copy("d1", None, None).is_err());
    }

    #[test]
    fn a_draft_from_the_server_is_kept_with_its_copy_up_to_date() {
        let mut store = store_with_work();
        let d = draft("d1");
        store.keep_server_draft(&d, &copy("c1@x", Some("7:1"))).unwrap();
        let kept = store.synced_draft("d1").unwrap().unwrap();
        assert_eq!((kept.draft, kept.copy, kept.pushed), (d, Some(copy("c1@x", Some("7:1"))), true));
    }

    /// A draft as another mail app keeps it in Drafts.
    fn on_server() -> crate::test_support::Mail {
        mail()
            .from("Maya Ross <maya@northwind.co>")
            .to("Daniel Okafor <daniel@northwind.co>")
            .id("dr1@phone")
            .subject("Lunch?")
            .body("Friday?\n\nSent from my phone")
    }

    #[test]
    fn a_draft_started_elsewhere_reads_as_a_new_email() {
        let store = store_with_work();
        let read = store.draft_from_server("work", &on_server().build(), 5).unwrap().unwrap();
        assert_eq!(read.message_id, "dr1@phone");
        assert_eq!(read.moss_id, None);
        assert_eq!(read.draft.id, "server:dr1@phone");
        assert_eq!(read.draft.mode, ComposeMode::New);
        assert_eq!(read.draft.thread_id, None);
        assert_eq!(read.draft.from_account_id, "work");
        assert_eq!(read.draft.from_address, None);
        assert_eq!(read.draft.to[0].email, "daniel@northwind.co");
        assert_eq!(read.draft.subject, "Lunch?");
        assert_eq!(read.draft.body, "Friday?\n\nSent from my phone");
        // No HTML of its own: the engine makes it from the text.
        assert_eq!(read.draft.body_html, "");
        assert_eq!(read.draft.updated_at, NOV_1);
        assert_eq!(read.saved_at, NOV_1);
        assert_eq!((read.draft.signature_id, read.draft.signature_chosen, read.draft.remind_if_no_reply), (None, false, false));
        // Without a date, it's as new as the caller says.
        let undated = store.draft_from_server("work", &on_server().without("Date").build(), 5).unwrap().unwrap();
        assert_eq!((undated.saved_at, undated.draft.updated_at), (5, 5));
        // From another of your addresses, it goes from that one.
        let other = on_server().from("Maya <HI@rossstudio.art>").build();
        let other = store.draft_from_server("work", &other, 5).unwrap().unwrap();
        assert_eq!(other.draft.from_address.as_deref(), Some("hi@rossstudio.art"));
        assert_eq!(store.draft_from_server("work", b"not an email", 5).unwrap(), None);
        assert!(matches!(store.draft_from_server("nobody", &on_server().build(), 5), Err(crate::Error::UnknownAccount(_))));
    }

    #[test]
    fn a_reply_started_elsewhere_stays_in_its_conversation_without_its_quote() {
        let mut store = store_with_work();
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        let reply = on_server()
            .subject("Re: Q4 budget")
            .reply_to(&["q4@northwind.co"])
            .body("Signed.\n\nOn Sun, 1 Nov 2026, Daniel wrote:\n> Can you sign off the Q4 budget?")
            .html("<p>Signed.</p><blockquote>Can you sign off the Q4 budget?</blockquote>");
        let read = store.draft_from_server("work", &reply.build(), 5).unwrap().unwrap();
        assert_eq!(read.draft.mode, ComposeMode::Reply);
        assert_eq!(read.draft.thread_id, Some(thread.clone()));
        // Moss adds the quote when it sends, as for any reply.
        assert_eq!(read.draft.body, "Signed.");
        assert_eq!(read.draft.body_html, "");
        // Replying to mail this Mac hasn't got: a new email, as it is.
        let elsewhere = reply.clone().reply_to(&["old@northwind.co"]).build();
        let read = store.draft_from_server("work", &elsewhere, 5).unwrap().unwrap();
        assert_eq!((read.draft.mode, read.draft.thread_id), (ComposeMode::New, None));
        assert!(read.draft.body.contains("> Can you sign off"));
        assert!(read.draft.body_html.contains("<blockquote>"));
    }

    #[test]
    fn a_draft_from_the_server_brings_its_files_but_not_the_pictures_in_its_text() {
        let store = store_with_work();
        let raw = on_server()
            .html("<p>Friday?</p><img src=\"cid:logo.png\">")
            .attach("plan.pdf", "application/pdf", "PDF-DATA")
            .inline("logo.png", "image/png", "PNG")
            .build();
        let read = store.draft_from_server("work", &raw, 5).unwrap().unwrap();
        assert_eq!(read.files, vec![ServerFile { name: "plan.pdf".into(), content_type: "application/pdf".into(), data: b"PDF-DATA".to_vec() }]);
        assert!(read.draft.attachments.is_empty(), "the engine keeps them, then lists them");
        assert!(store.draft_from_server("work", &on_server().build(), 5).unwrap().unwrap().files.is_empty());
    }

    #[test]
    fn a_draft_moss_wrote_reads_back_as_it_was() {
        let mut store = store_with_work();
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        let ours = on_server()
            .reply_to(&["q4@northwind.co"])
            .header("X-Moss-Draft", "d7")
            .header("X-Moss-Mode", "reply-all")
            .body("> not a quote, just how I write")
            .html("<p>&gt; not a quote, just how I write</p>");
        let read = store.draft_from_server("work", &ours.build(), 5).unwrap().unwrap();
        assert_eq!(read.moss_id.as_deref(), Some("d7"));
        assert_eq!(read.draft.id, "d7");
        assert_eq!((read.draft.mode, read.draft.thread_id), (ComposeMode::ReplyAll, Some(thread)));
        assert_eq!(read.draft.body, "> not a quote, just how I write");
        assert_eq!(read.draft.body_html, "<p>&gt; not a quote, just how I write</p>");
        // A forward of mail that isn't here, or a mode this Moss doesn't know, falls back.
        let lost = ours.clone().reply_to(&["old@northwind.co"]).header("X-Moss-Mode", "forward").build();
        assert_eq!(store.draft_from_server("work", &lost, 5).unwrap().unwrap().draft.mode, ComposeMode::New);
        let odd = ours.clone().header("X-Moss-Mode", "shout").build();
        assert_eq!(store.draft_from_server("work", &odd, 5).unwrap().unwrap().draft.mode, ComposeMode::Reply);
        let new = ours.header("X-Moss-Mode", "new").build();
        assert_eq!(store.draft_from_server("work", &new, 5).unwrap().unwrap().draft.mode, ComposeMode::Reply);
    }

    #[test]
    fn a_draft_with_only_html_reads_a_line_for_each_block() {
        let mut store = store_with_work();
        let html = "<div dir=\"ltr\">Friday?<div><br></div><div>-- </div><div><b>Maya</b> Ross</div><div>Design lead</div></div>";
        let only = |extra: &str| {
            format!(
                "From: Maya Ross <maya@northwind.co>\r\nTo: daniel@northwind.co\r\nSubject: Lunch?\r\nMessage-ID: <h1@web>\r\n{extra}\
                 MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n"
            )
            .into_bytes()
        };
        let read = store.draft_from_server("work", &only(""), 5).unwrap().unwrap();
        assert_eq!(read.draft.body, "Friday?\n\n--\nMaya Ross\nDesign lead");
        assert_eq!(read.draft.body_html.trim_end(), html);
        // A reply in a conversation here loses its quote and what follows "--", as with plain text.
        let thread = ingest(&mut store, "1", &mail().build()).thread_id;
        let reply = only("In-Reply-To: <q4@northwind.co>\r\n");
        let read = store.draft_from_server("work", &reply, 5).unwrap().unwrap();
        assert_eq!((read.draft.thread_id, read.draft.body.as_str()), (Some(thread), "Friday?"));
        // With a plain-text part too, that part is the text, as written.
        let both = on_server().body("Friday?\n\nSent from my phone").html(html).build();
        assert_eq!(store.draft_from_server("work", &both, 5).unwrap().unwrap().draft.body, "Friday?\n\nSent from my phone");
    }

    #[test]
    fn a_signature_you_chose_in_moss_reads_back_with_its_draft() {
        let store = store_with_work();
        let signature = |raw: Vec<u8>| {
            let d = store.draft_from_server("work", &raw, 5).unwrap().unwrap().draft;
            (d.signature_id, d.signature_chosen)
        };
        let ours = on_server().header("X-Moss-Draft", "d7");
        assert_eq!(signature(ours.clone().header("X-Moss-Signature", "studio").build()), (Some("studio".into()), true));
        // You took it off.
        assert_eq!(signature(ours.clone().header("X-Moss-Signature", "none").build()), (None, true));
        // Not chosen: Moss picks again.
        assert_eq!(signature(ours.build()), (None, false));
        // Only Moss writes it: on a draft from another app it means nothing.
        assert_eq!(signature(on_server().header("X-Moss-Signature", "studio").build()), (None, false));
    }

    fn file(name: &str) -> (String, String, Vec<u8>) {
        (name.into(), "application/pdf".into(), b"%PDF-1.7".to_vec())
    }

    #[test]
    fn a_drafts_files_are_kept_with_it_until_it_lets_them_go() {
        let mut store = store_with_work();
        let (name, kind, data) = file("plan.pdf");
        let plan = store.add_draft_file("d1", &name, &kind, &data, NOV_1).unwrap();
        assert_eq!(
            plan,
            DraftFile { id: plan.id.clone(), name: "plan.pdf".into(), bytes: 8, content_type: kind.clone(), forwarded: None }
        );
        let notes = store.add_draft_file("d1", "notes.txt", "text/plain", b"hi", NOV_1).unwrap();
        assert_ne!(plan.id, notes.id);
        assert_eq!(store.draft_file(&plan.id).unwrap(), Some((name, kind, data)));
        assert_eq!(store.draft_file("999").unwrap(), None);
        assert_eq!(store.draft_file("not a number").unwrap(), None);

        // Saved with both, the draft lists them; saved without one, that one goes.
        let forwarded = DraftFile {
            id: "9:1".into(),
            name: "deck.key".into(),
            bytes: 10,
            content_type: String::new(),
            forwarded: Some(FileRef { message_id: "9".into(), index: 1 }),
        };
        let mut d = draft("d1");
        d.attachments = vec![plan.clone(), notes.clone(), forwarded];
        store.save_draft(&d).unwrap();
        assert_eq!(store.draft("d1").unwrap().unwrap().attachments, d.attachments);
        d.attachments.remove(1);
        store.save_draft(&d).unwrap();
        assert_eq!(store.draft("d1").unwrap().unwrap().attachments, d.attachments);
        assert_eq!(store.draft_file(&notes.id).unwrap(), None);
        assert!(store.draft_file(&plan.id).unwrap().is_some());

        // Deleting the draft keeps its files, for a send that is still waiting to go.
        assert!(store.delete_draft("d1").unwrap());
        assert!(store.draft_file(&plan.id).unwrap().is_some());
        store.delete_draft_files(&[plan.id.clone(), "x".into()]).unwrap();
        assert_eq!(store.draft_file(&plan.id).unwrap(), None);
        let other = store.add_draft_file("d2", "a.txt", "text/plain", b"a", NOV_1).unwrap();
        store.forget_draft_files("d2").unwrap();
        assert_eq!(store.draft_file(&other.id).unwrap(), None);
    }

    #[test]
    fn handoffs_release_removed_saved_files_but_preserve_kept_and_staged_files() {
        for scheduled in [true, false] {
            let mut store = store_with_work();
            let mut d = store.attach_and_save_draft(&draft("handoff-files"), &[
                ("removed.txt", "text/plain", b"removed"),
                ("kept.txt", "text/plain", b"kept"),
            ]).unwrap();
            let removed = d.attachments.remove(0);
            let staged = store.add_draft_file(&d.id, "staged.txt", "text/plain", b"staged", NOV_1).unwrap();
            if scheduled { store.schedule_email(&d, "{}", NOV_1, NOV_1).unwrap(); }
            else { store.prepare_delivery("handoff", &d, "{}", "handoff@fixture", NOV_1).unwrap(); }
            assert!(store.draft_file(&removed.id).unwrap().is_none(), "scheduled={scheduled}");
            assert!(store.draft_file(&d.attachments[0].id).unwrap().is_some());
            assert!(store.draft_file(&staged.id).unwrap().is_some());
            if scheduled { store.unschedule_email(&d.id, NOV_1).unwrap(); }
            else { store.cancel_delivery("handoff").unwrap(); }
            assert_eq!(store.draft(&d.id).unwrap().unwrap().attachments, d.attachments);
            assert!(store.draft_file(&removed.id).unwrap().is_none());
        }
    }

    #[test]
    fn failed_handoff_file_cleanup_rolls_back_ownership_and_keeps_the_saved_draft() {
        for scheduled in [true, false] {
            let mut store = store_with_work();
            let saved = store.attach_and_save_draft(&draft("failed-handoff"), &[("keep.txt", "text/plain", b"keep")]).unwrap();
            let mut edited = saved.clone();
            edited.attachments.clear();
            store.conn.execute_batch("CREATE TRIGGER refuse_file_cleanup BEFORE DELETE ON draft_files BEGIN SELECT RAISE(ABORT, 'fixture cleanup failure'); END;").unwrap();
            let failed = if scheduled { store.schedule_email(&edited, "{}", NOV_1, NOV_1) }
                else { store.prepare_delivery("failed-handoff", &edited, "{}", "failure@fixture", NOV_1).map(|_| ()) };
            assert!(failed.is_err());
            assert_eq!(store.draft(&saved.id).unwrap(), Some(saved.clone()));
            assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_some());
            assert!(store.delivery("failed-handoff").unwrap().is_none());
            assert!(store.scheduled_email(&saved.id).unwrap().is_none());
        }
    }

    #[test]
    fn unreadable_file_ownership_fails_closed() {
        let mut store = store_with_work();
        let file = store.add_draft_file("unreadable-owner", "keep.txt", "text/plain", b"keep", NOV_1).unwrap();
        store.conn.execute_batch("DROP TABLE scheduled").unwrap();
        assert!(store.forget_draft_files("unreadable-owner").is_err());
        assert_eq!(store.draft_file(&file.id).unwrap().unwrap().2, b"keep");
    }

    #[test]
    fn scheduled_ownership_refuses_stale_saves_until_unscheduled() {
        let mut store = store_with_work();
        let saved = store.attach_and_save_draft(&draft("scheduled-owner"), &[("keep.txt", "text/plain", b"keep")]).unwrap();
        store.schedule_email(&saved, "{}", NOV_1, NOV_1).unwrap();
        let mut stale = saved.clone();
        stale.body = "A stale writer".into();
        stale.attachments.clear();
        assert!(store.save_draft(&stale).is_err());
        assert!(store.draft(&saved.id).unwrap().is_none());
        assert_eq!(store.scheduled_email(&saved.id).unwrap().unwrap().draft, saved);
        assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_some());
        store.unschedule_email(&saved.id, NOV_1).unwrap();
        store.save_draft(&stale).unwrap();
        assert_eq!(store.draft(&saved.id).unwrap(), Some(stale));
    }

    #[test]
    fn discarding_files_refuses_scheduled_or_unresolved_delivery_ownership() {
        let mut store = store_with_work();
        let saved = store.attach_and_save_draft(&draft("discard-owner"), &[("keep.txt", "text/plain", b"keep")]).unwrap();
        store.schedule_email(&saved, "{}", NOV_1, NOV_1).unwrap();
        assert!(store.forget_draft_files(&saved.id).is_err());
        assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_some());
        store.unschedule_email(&saved.id, NOV_1).unwrap();
        store.prepare_delivery("discard-owner", &saved, "{}", "owner@fixture", NOV_1).unwrap();
        for state in ["held", "claimed", "uncertain", "accepted"] {
            store.conn.execute("UPDATE deliveries SET state = ?1 WHERE id = 'discard-owner'", [state]).unwrap();
            assert!(store.forget_draft_files(&saved.id).is_err(), "{state}");
            assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_some());
        }
        store.complete_delivery("discard-owner").unwrap();
        store.forget_draft_files(&saved.id).unwrap();
        assert!(store.draft_file(&saved.attachments[0].id).unwrap().is_none());
    }

    #[test]
    fn startup_cleanup_preserves_unresolved_delivery_files() {
        let mut store = store_with_work();
        let mut d = draft("recoverable-file");
        let file = store.add_draft_file(&d.id, "receipt.txt", "text/plain", b"receipt", NOV_1).unwrap();
        d.attachments = vec![file.clone()];
        store.prepare_delivery("recoverable", &d, "{}", "recoverable@fixture", NOV_1).unwrap();
        for state in ["held", "claimed", "uncertain", "accepted"] {
            store.conn.execute("UPDATE deliveries SET state = ?1 WHERE id = 'recoverable'", [state]).unwrap();
            assert_eq!(store.forget_lost_draft_files().unwrap(), 0, "{state}");
            assert_eq!(store.draft_file(&file.id).unwrap().unwrap().2, b"receipt");
        }
        store.complete_delivery("recoverable").unwrap();
        assert_eq!(store.forget_lost_draft_files().unwrap(), 1);
        assert!(store.draft_file(&file.id).unwrap().is_none());
    }

    #[test]
    fn files_of_drafts_that_are_gone_are_let_go() {
        let mut store = store_with_work();
        let kept = store.add_draft_file("d1", "a.pdf", "application/pdf", b"a", NOV_1).unwrap();
        let lost = store.add_draft_file("gone", "b.pdf", "application/pdf", b"b", NOV_1).unwrap();
        let mut d = draft("d1");
        d.attachments = vec![kept.clone()];
        store.save_draft(&d).unwrap();
        // One set to send later is out of Drafts, but its files go when it does.
        let later = store.add_draft_file("later", "c.pdf", "application/pdf", b"c", NOV_1).unwrap();
        let mut scheduled = draft("later");
        scheduled.attachments = vec![later.clone()];
        let email = serde_json::json!({});
        store.schedule_email(&scheduled, &email.to_string(), NOV_1, NOV_1).unwrap();
        assert_eq!(store.forget_lost_draft_files().unwrap(), 1);
        assert!(store.draft_file(&later.id).unwrap().is_some());
        assert!(store.draft_file(&kept.id).unwrap().is_some());
        assert_eq!(store.draft_file(&lost.id).unwrap(), None);
    }

    #[test]
    fn a_draft_must_point_at_a_real_thread() {
        let store = store_with_work();
        let mut lost = draft("d1");
        lost.thread_id = Some("999".into());
        assert!(store.save_draft(&lost).is_err());
    }
}
