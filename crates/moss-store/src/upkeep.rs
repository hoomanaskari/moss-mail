//! Keeping the database small and its indexes whole: giving freed space back to the disk,
//! and rebuilding the keyword index, details and people from the mail that's here.

use moss_core::Person;
use rusqlite::{params, OptionalExtension};

use crate::bodies::{self, Body};
use crate::codec::{json, now_ms};
use crate::ingest::PREVIEW;
use crate::parse::parse;
use crate::{attachment_text, details, people, text, Result, Store};

/// `PRAGMA auto_vacuum` value for incremental vacuum.
pub(crate) const INCREMENTAL: i64 = 2;

/// Pages one step of tidying the keyword index writes: about 2 MB, a few milliseconds.
pub const INDEX_PAGES: u32 = 500;
/// Free pages one step gives back to the disk: 4 MB at 4 KB a page.
pub const FREE_PAGES: u32 = 1000;

/// While the index is being rebuilt: the last message id done.
const REBUILD_KEY: &str = "index.rebuiltThrough";

impl Store {
    /// Gives the space freed by deleted mail back to the disk, after tidying the keyword
    /// index. On a database made before
    /// incremental vacuum that takes one full `VACUUM`, which switches it to incremental
    /// vacuum; after that, free pages are released without rewriting anything. Returns
    /// whether the full `VACUUM` ran. [`Self::tidy_index_step`], [`Self::free_space_step`] and
    /// [`Self::empty_wal`] do the same in steps, so the database can be used in between.
    pub fn compact(&self) -> Result<bool> {
        let mut first = true;
        while self.tidy_index_step(INDEX_PAGES, first)? {
            first = false;
        }
        let full = self.auto_vacuum()? != INCREMENTAL;
        while self.free_space_step(FREE_PAGES)? {}
        self.empty_wal()?;
        Ok(full)
    }

    fn auto_vacuum(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?)
    }

    /// Merges the keyword index's pieces, writing about `pages` pages: deleted mail leaves its
    /// words there until they're merged. `first` starts a new merge of everything; the steps
    /// after it carry that one on. Returns whether there was anything to merge: call again
    /// (not `first`) until there isn't. Run to the end, it does what FTS5's `optimize` does.
    pub fn tidy_index_step(&self, pages: u32, first: bool) -> Result<bool> {
        let before = self.conn.total_changes();
        let pages = if first { -i64::from(pages) } else { i64::from(pages) };
        self.conn.execute("INSERT INTO message_search (message_search, rank) VALUES ('merge', ?1)", [pages])?;
        // FTS5 counts two changes or more when a merge did some work.
        Ok(self.conn.total_changes() - before >= 2)
    }

    /// Gives up to `pages` free pages back to the disk. Returns whether more are free. A
    /// database made before incremental vacuum is rewritten once instead, which switches it
    /// over; if that fails, the database is as it was.
    pub fn free_space_step(&self, pages: u32) -> Result<bool> {
        if self.auto_vacuum()? != INCREMENTAL {
            // Takes effect with the VACUUM.
            self.conn.pragma_update(None, "auto_vacuum", INCREMENTAL)?;
            self.conn.execute_batch("VACUUM")?;
            return Ok(false);
        }
        // Frees one page per row, so step it to the end.
        let mut stmt = self.conn.prepare(&format!("PRAGMA incremental_vacuum({pages})"))?;
        let mut rows = stmt.query([])?;
        while rows.next()?.is_some() {}
        let free: i64 = self.conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
        Ok(free > 0)
    }

    /// Empties the WAL file, so the disk gets back the space a vacuum went through it with.
    pub fn empty_wal(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    /// Starts rebuilding the keyword index (of emails and the files in them), details, people
    /// and previews from the mail on this device. Search finds less until
    /// [`Store::rebuild_step`] has been through it all.
    pub fn start_rebuild(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM message_search; DELETE FROM attachment_search; DELETE FROM details; DELETE FROM people;")?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, '0') ON CONFLICT (key) DO UPDATE SET value = '0'",
            [REBUILD_KEY],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Whether a rebuild is under way.
    pub fn rebuilding(&self) -> Result<bool> {
        Ok(self.setting(REBUILD_KEY)?.is_some())
    }

    /// Rebuilds up to `limit` more messages, oldest first, in one short transaction.
    /// Text comes from the body kept here, or the original (Resend mail, whose body is
    /// kept again from it); mail with neither is found by its subject and people until
    /// it's opened. Returns how many it did; 0 once the rebuild is over.
    pub fn rebuild_step(&mut self, limit: usize) -> Result<usize> {
        let Some(through) = self.setting(REBUILD_KEY)?.and_then(|s| s.parse::<i64>().ok()) else {
            return Ok(0);
        };
        let offline = self.offline()?;
        let tx = self.conn.transaction()?;
        let batch: Vec<(i64, String, String, String, Vec<Person>, bool, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT id, account_id, subject, participants, from_name, from_email, from_me, sent_at, to_json,
                        cc_json, bcc_json
                 FROM messages WHERE id > ?1 ORDER BY id LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![through, limit as i64], |r| {
                let mut to: Vec<Person> = json(r, 8)?;
                to.extend(json::<Vec<Person>>(r, 9)?);
                to.extend(json::<Vec<Person>>(r, 10)?);
                let from = Person { name: r.get(4)?, email: r.get(5)? };
                let from_me: bool = r.get(6)?;
                let who = if from_me { to } else { vec![from] };
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, who, from_me, r.get(7)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (id, account, subject, participants, who, from_me, sent_at) in &batch {
            let kept: Option<(String, Vec<u8>)> = tx
                .query_row("SELECT method, text FROM bodies WHERE message_id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let mut body = kept.and_then(|(method, packed)| bodies::unpack(&method, &packed));
            if body.is_none() {
                let raw: Option<Vec<u8>> =
                    tx.query_row("SELECT raw FROM message_raw WHERE message_id = ?1", [id], |r| r.get(0)).optional()?;
                if let Some(parsed) = raw.as_deref().and_then(parse) {
                    let whole = Body { text: parsed.body_text.clone(), html: parsed.body_html.clone() };
                    bodies::keep(&tx, *id, &whole, offline.keeps_text(*sent_at, now_ms()))?;
                    body = Some(parsed.body_text);
                }
            }
            let stripped = body.as_deref().map(text::strip_quoted).unwrap_or_default();
            if body.is_some() {
                // So a preview made by an older Moss is made again by this one.
                tx.execute("UPDATE messages SET snippet = ?2 WHERE id = ?1", params![id, text::snippet(&stripped, PREVIEW)])?;
            }
            tx.execute(
                "INSERT INTO message_search (rowid, subject, participants, body) VALUES (?1, ?2, ?3, ?4)",
                params![id, subject, participants, stripped],
            )?;
            details::find(&tx, *id, subject, &stripped)?;
            let files: Vec<(i64, String, Vec<u8>)> = {
                let mut stmt = tx.prepare_cached("SELECT id, method, text FROM attachment_text WHERE message_id = ?1")?;
                let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for (text_id, method, packed) in files {
                attachment_text::index_words(&tx, text_id, &bodies::unpack(&method, &packed).unwrap_or_default())?;
            }
            let (from, to) = if *from_me { (0, 1) } else { (1, 0) };
            for person in who {
                people::bump(&tx, account, person, from, to, *sent_at)?;
            }
        }
        match batch.last() {
            Some(last) if batch.len() == limit => {
                tx.execute("UPDATE settings SET value = ?2 WHERE key = ?1", params![REBUILD_KEY, last.0.to_string()])?;
            }
            _ => {
                tx.execute("DELETE FROM settings WHERE key = ?1", [REBUILD_KEY])?;
            }
        }
        tx.commit()?;
        Ok(batch.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::ProviderKind;

    fn found(store: &Store, query: &str) -> usize {
        store.search(query, None, 10, NOV_1).unwrap().len()
    }

    #[test]
    fn rebuilding_reads_the_index_details_and_people_back_a_batch_at_a_time() {
        let mut store = store_with_work();
        for n in 0..3 {
            let raw = mail().id(&format!("{n}@x")).subject(&format!("Order {n}")).body("Total $40, order #AB123").build();
            ingest(&mut store, &format!("1:{n}"), &raw);
        }
        assert_eq!(store.rebuild_step(10).unwrap(), 0, "nothing to do until it starts");
        // A preview made by an older Moss is made again.
        store.conn.execute("UPDATE messages SET snippet = '<body></body>…'", []).unwrap();
        store.start_rebuild().unwrap();
        assert!(store.rebuilding().unwrap());
        assert_eq!(found(&store, "total"), 0);
        assert_eq!(store.rebuild_step(2).unwrap(), 2);
        assert!(store.rebuilding().unwrap());
        assert_eq!(store.rebuild_step(2).unwrap(), 1);
        assert!(!store.rebuilding().unwrap());
        assert_eq!(found(&store, "total"), 3);
        let details: i64 = store.conn.query_row("SELECT count(*) FROM details", [], |r| r.get(0)).unwrap();
        assert_eq!(details, 6);
        assert_eq!(store.contact("work", "daniel@northwind.co").unwrap().unwrap().from_them, 3);
        assert_eq!(snippets(&store), ["Total $40, order #AB123"; 3]);
    }

    fn snippets(store: &Store) -> Vec<String> {
        let mut stmt = store.conn.prepare("SELECT snippet FROM messages ORDER BY id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn resend_mail_gets_its_body_back_from_the_original_and_mail_without_either_keeps_its_subject() {
        let mut store = store_with_work();
        let resend = moss_core::Account { provider: ProviderKind::Resend, ..account("re", "maya@moss.email") };
        store.upsert_account(&resend).unwrap();
        let raw = mail().id("r@x").subject("Invoice").body("Pay by Friday").build();
        let r = store.ingest(&raw, &moss_core::MessageSource { account_id: "re".into(), ..source("re_1") }).unwrap();
        let w = ingest(&mut store, "1:1", &mail().id("w@x").subject("Lunch").body("Thai?").build());
        store.conn.execute_batch("DELETE FROM bodies").unwrap();
        // Mail sent from here, by Maya to Daniel.
        let sent = mail().id("s@x").from("maya@northwind.co").to("Daniel <daniel@northwind.co>").build();
        store.ingest(&sent, &moss_core::MessageSource { mailbox: "Sent".into(), ..source("2:1") }).unwrap();
        store.start_rebuild().unwrap();
        while store.rebuild_step(1).unwrap() > 0 {}
        assert!(store.body(r.message_id.parse().unwrap()).unwrap().is_some());
        assert!(store.body(w.message_id.parse().unwrap()).unwrap().is_none());
        assert_eq!(found(&store, "friday"), 1);
        assert_eq!(found(&store, "lunch"), 1);
        assert_eq!(found(&store, "thai"), 0);
        assert_eq!(snippets(&store)[1], "Thai?", "with no body here, the preview stays");
        assert_eq!(store.contact("work", "daniel@northwind.co").unwrap().unwrap().to_them, 1);
    }

    #[test]
    fn rebuilding_indexes_the_text_inside_files_again() {
        let mut store = store_with_work();
        let raw = mail().id("a@x").attach("letter.pdf", "application/pdf", "%PDF").attach("blank.png", "image/png", "png");
        let id = ingest(&mut store, "1:1", &raw.build()).message_id;
        store.keep_attachment_text(&id, 0, "parking permit").unwrap();
        store.keep_attachment_text(&id, 1, "").unwrap();
        store.start_rebuild().unwrap();
        assert_eq!(found(&store, "permit"), 0);
        while store.rebuild_step(5).unwrap() > 0 {}
        assert_eq!(found(&store, "permit"), 1);
        assert_eq!(store.search("permit", None, 10, NOV_1).unwrap()[0].file.as_ref().map(|f| f.name.as_str()), Some("letter.pdf"));
    }

    #[test]
    fn failed_rebuilds_are_errors() {
        let mut store = store_with_work();
        ingest(&mut store, "1:1", &mail().build());
        store.start_rebuild().unwrap();
        read_only(&store);
        assert!(store.start_rebuild().is_err());
        assert!(store.rebuild_step(5).is_err());
    }

    /// A thread with a 4 MB file kept, so it takes room in the file. Returns its id.
    fn big(store: &mut Store, n: u64) -> i64 {
        let mut seed = n;
        let noise: Vec<u8> = (0..4_000_000)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (seed >> 56) as u8
            })
            .collect();
        let raw = mail().id(&format!("big{n}@x")).attach("big.bin", "application/octet-stream", "x").build();
        let got = ingest(store, &format!("1:{n}"), &raw);
        store.keep_attachment(&got.message_id, 0, &noise).unwrap();
        got.thread_id.parse().unwrap()
    }

    fn file_len(path: &std::path::Path) -> u64 {
        std::fs::metadata(path).unwrap().len()
    }

    #[test]
    fn compacting_vacuums_an_old_database_once_then_frees_space_as_it_goes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        // A database made before incremental vacuum.
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            crate::migrate(&mut conn, crate::schema::MIGRATIONS).unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        let mode = |s: &Store| -> i64 { s.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap() };
        assert_eq!(mode(&store), 0);
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        // Schema growth is not unreclaimed message storage. Allow only a small
        // amount of message/index metadata above this database's empty baseline.
        let compact_limit = file_len(&path) + 64 * 1024;
        let thread = big(&mut store, 1);
        store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let before = file_len(&path);
        assert!(before > 4_000_000, "{before}");

        store.conn.execute("DELETE FROM threads WHERE id = ?1", [thread]).unwrap();
        assert!(store.compact().unwrap());
        assert_eq!(mode(&store), 2);
        assert!(file_len(&path) < compact_limit, "{}", file_len(&path));
        assert_eq!(file_len(&path.with_extension("db-wal")), 0);

        // From now on, freed pages go back without a full rewrite.
        let thread = big(&mut store, 2);
        store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        assert!(file_len(&path) > 4_000_000);
        store.conn.execute("DELETE FROM threads WHERE id = ?1", [thread]).unwrap();
        assert!(!store.compact().unwrap());
        assert!(file_len(&path) < compact_limit, "{}", file_len(&path));
    }

    fn free_pages(store: &Store) -> i64 {
        store.conn.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn the_keyword_index_is_tidied_a_bounded_step_at_a_time_until_nothing_is_left() {
        let mut store = store_with_work();
        // Each email in its own write leaves the index in many small pieces, and some deleted.
        store.conn.execute("INSERT INTO message_search (message_search, rank) VALUES ('automerge', 0)", []).unwrap();
        for n in 0..40 {
            let words: Vec<String> = (0..1500).map(|i| format!("w{n}x{i}")).collect();
            let raw = mail().id(&format!("{n}@x")).subject(&format!("Order {n}")).body(&words.join(" ")).build();
            ingest(&mut store, &format!("1:{n}"), &raw);
        }
        store.conn.execute("DELETE FROM messages WHERE subject = 'Order 7'", []).unwrap();
        let mut steps = 0;
        let mut first = true;
        while store.tidy_index_step(1, first).unwrap() {
            first = false;
            steps += 1;
            assert!(steps < 1000, "it ends");
        }
        assert!(steps > 1, "one page at a time takes more than one step: {steps}");
        assert!(!store.tidy_index_step(1, true).unwrap(), "once tidy, there is nothing to do");
        assert_eq!(found(&store, "order"), 10, "search finds what it did");
        assert_eq!(found(&store, "7"), 0);
        read_only(&store);
        assert!(store.tidy_index_step(1, true).is_err());
    }

    #[test]
    fn freed_space_goes_back_to_the_disk_a_few_pages_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        let mut store = Store::open(&path).unwrap();
        store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
        let thread = big(&mut store, 1);
        store.conn.execute("DELETE FROM threads WHERE id = ?1", [thread]).unwrap();
        let free = free_pages(&store);
        assert!(free > 100, "{free}");
        assert!(store.free_space_step(100).unwrap());
        assert_eq!(free_pages(&store), free - 100);
        while store.free_space_step(100).unwrap() {}
        assert_eq!(free_pages(&store), 0);
        assert!(!store.free_space_step(100).unwrap(), "nothing left to free");
        store.empty_wal().unwrap();
        assert_eq!(file_len(&path.with_extension("db-wal")), 0);
        read_only(&store);
        assert!(store.free_space_step(100).is_err());
    }

    #[test]
    fn an_old_database_is_vacuumed_once_in_its_first_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            crate::migrate(&mut conn, crate::schema::MIGRATIONS).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert!(!store.free_space_step(100).unwrap(), "one VACUUM does it all");
        let mode: i64 = store.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, INCREMENTAL);
        let store = store_with_work();
        read_only(&store);
        assert!(store.empty_wal().is_ok(), "a checkpoint writes nothing new");
    }

    #[test]
    fn compacting_a_read_only_database_fails_cleanly() {
        let store = store_with_work();
        read_only(&store);
        assert!(store.compact().is_err());
    }
}
