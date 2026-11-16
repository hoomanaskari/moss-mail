//! What each message means, as a vector from the embedding model, so search can find mail
//! by meaning and not only by its words. Vectors are cut to [`DIMS`] numbers (the model is
//! trained so the first ones carry the most) and stored as one byte each: 256 bytes a
//! message. Each records the model that made it, so a new model makes them again. The text
//! inside files gets vectors too (see `attachment_text.rs`), found the same way.

use rusqlite::{named_params, params, OptionalExtension};

use moss_core::SearchHit;

use crate::search::Found;
use crate::{text, Result, Store};

/// A pinned read-only snapshot. Only semantic search is exposed to its owner.
pub struct MeaningReader { store: Store }

impl Store {
    /// In-memory previews have no independent connection; disk profiles use a reader.
    pub fn meaning_reader(&self) -> Result<Option<MeaningReader>> {
        let Some(path) = self.conn.path().filter(|path| !path.is_empty()) else { return Ok(None); };
        let conn = rusqlite::Connection::open_with_flags(path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW)
            .map_err(|_| crate::Error::Invalid("The search database could not be opened. Reopen Moss before searching.".into()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("BEGIN DEFERRED")?;
        // Reading the header pins this WAL snapshot before the writer lock is released.
        let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != crate::schema::MIGRATIONS.len() as u32 {
            return Err(crate::Error::Invalid("The search database schema changed. Reopen Moss before searching.".into()));
        }
        Ok(Some(MeaningReader { store: Store { conn, delivery_session: None } }))
    }
}

impl MeaningReader {
    pub fn search(&self, model: &str, query: &[f32], account: Option<&str>, least: f64, limit: usize, now: i64) -> Result<Vec<SearchHit>> {
        self.store.meaning_search(model, query, account, least, limit, now)
    }
}

/// Numbers kept of each vector.
pub const DIMS: usize = 256;
/// Characters of a message's text the model reads.
const TEXT_CHARS: usize = 1_500;

/// A message to make a vector for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToEmbed {
    pub message_id: i64,
    pub subject: String,
    /// The new text, or the preview when the body isn't here.
    pub text: String,
}

/// A vector cut to [`DIMS`], made unit length, and stored as signed bytes.
pub fn quantize(vector: &[f32]) -> Vec<u8> {
    let cut = &vector[..vector.len().min(DIMS)];
    let length = cut.iter().map(|x| x * x).sum::<f32>().sqrt().max(f32::MIN_POSITIVE);
    cut.iter().map(|x| ((x / length) * 127.0).round().clamp(-127.0, 127.0) as i8 as u8).collect()
}

/// How alike two stored vectors are, from -1 to 1.
pub fn similarity(a: &[u8], b: &[u8]) -> f64 {
    let dot: i64 = a.iter().zip(b).map(|(x, y)| (*x as i8 as i64) * (*y as i8 as i64)).sum();
    dot as f64 / (127.0 * 127.0)
}

impl Store {
    /// Up to `limit` messages without a vector from `model`, newest first.
    pub fn unembedded(&self, model: &str, limit: usize) -> Result<Vec<ToEmbed>> {
        Ok(self.unembedded_after(model, limit, None)?.0)
    }

    /// Continue a backfill strictly before its last persisted message. A new wake
    /// starts without a cursor so newer mail and changed model identities are seen.
    pub fn unembedded_after(&self, model: &str, limit: usize, after: Option<(i64, i64)>) -> Result<(Vec<ToEmbed>, Option<(i64, i64)>)> {
        let select = "SELECT m.id, m.subject, m.snippet, m.sent_at FROM messages m
             WHERE NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.message_id = m.id AND e.model = ?1)";
        // Split the equal-timestamp tail from older timestamps. SQLite otherwise
        // seeks only the first tuple column when the second is the rowid alias.
        let sql = if after.is_some() {
            format!("{select} AND m.sent_at=?3 AND m.id<?4 UNION ALL {select} AND m.sent_at<?3 ORDER BY 4 DESC, 1 DESC LIMIT ?2")
        } else {
            format!("{select} ORDER BY m.sent_at DESC, m.id DESC LIMIT ?2")
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let read = |r: &rusqlite::Row<'_>| Ok((ToEmbed {message_id:r.get(0)?,subject:r.get(1)?,text:r.get(2)?},r.get::<_,i64>(3)?));
        let rows = match after {
            Some((sent_at,id)) => stmt.query_map(params![model,limit as i64,sent_at,id],read)?,
            None => stmt.query_map(params![model,limit as i64],read)?,
        };
        let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let cursor = rows.last().map(|(message,sent_at)| (*sent_at,message.message_id));
        let mut list: Vec<ToEmbed> = rows.into_iter().map(|(message,_)|message).collect();
        for m in &mut list { self.populate_embedding_text(m)?; }
        Ok((list,cursor))
    }

    fn populate_embedding_text(&self,message:&mut ToEmbed)->Result<()> {
        if let Some(body)=self.body(message.message_id)? {
            message.text=text::strip_quoted(&body.text).chars().take(TEXT_CHARS).collect();
        }
        Ok(())
    }

    /// Saves a message's vector from `model`, replacing any older one.
    pub fn set_embedding(&self, message_id: i64, model: &str, vector: &[f32]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO embeddings (message_id, model, vector) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM messages WHERE id=?1)",
            params![message_id, model, quantize(vector)],
        )?;
        Ok(())
    }

    /// Publish only if the model's input still represents this message.
    pub fn set_embedding_if_current(&self, expected:&ToEmbed, model:&str, vector:&[f32])->Result<bool> {
        let tx=self.conn.unchecked_transaction()?;
        let current=self.conn.query_row("SELECT id,subject,snippet FROM messages WHERE id=?1",[expected.message_id],|row|Ok(ToEmbed{message_id:row.get(0)?,subject:row.get(1)?,text:row.get(2)?})).optional()?;
        let Some(mut current)=current else {return Ok(false);};
        self.populate_embedding_text(&mut current)?;
        if current!=*expected {return Ok(false);}
        self.set_embedding(expected.message_id,model,vector)?;
        tx.commit()?;
        Ok(true)
    }

    /// How many messages have a vector from `model`, of how many.
    pub fn embedded(&self, model: &str) -> Result<(u64, u64)> {
        Ok(self.conn.query_row(
            "SELECT (SELECT count(*) FROM embeddings WHERE model = ?1), (SELECT count(*) FROM messages)",
            [model],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?)
    }

    /// Threads whose messages, or the files in them, mean most nearly what `query` (a vector
    /// from `model`) does, at least `least` alike, best first, one hit per thread. Done threads
    /// are included.
    pub fn meaning_search(
        &self,
        model: &str,
        query: &[f32],
        account: Option<&str>,
        least: f64,
        limit: usize,
        now: i64,
    ) -> Result<Vec<SearchHit>> {
        let query = quantize(query);
        let mut stmt = self.conn.prepare(
            "SELECT e.message_id, m.thread_id, e.vector, NULL FROM embeddings e JOIN messages m ON m.id = e.message_id
             WHERE e.model = :model AND (:account IS NULL OR m.account_id = :account)
               AND m.deleted_from IS NULL
             UNION ALL
             SELECT t.message_id, m.thread_id, e.vector, t.position FROM attachment_embeddings e
                 JOIN attachment_text t ON t.id = e.text_id JOIN messages m ON m.id = t.message_id
             WHERE e.model = :model AND (:account IS NULL OR m.account_id = :account)
               AND m.deleted_from IS NULL",
        )?;
        let rows = stmt.query_map(named_params! {":model": model, ":account": account}, |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, Option<u32>>(3)?))
        })?;
        // Best message (or file) per thread.
        let mut best: std::collections::HashMap<i64, Found> = std::collections::HashMap::new();
        for row in rows {
            let (message_id, thread_id, vector, file) = row?;
            let alike = similarity(&query, &vector);
            if alike < least {
                continue;
            }
            let found = Found { message_id, thread_id, rank: -alike, file };
            let entry = best.entry(thread_id).or_insert(found);
            if found.rank < entry.rank {
                *entry = found;
            }
        }
        let mut found: Vec<Found> = best.into_values().collect();
        found.sort_by(|a, b| a.rank.total_cmp(&b.rank).then(b.message_id.cmp(&a.message_id)));
        found.truncate(limit);
        self.hits(&found, &[], now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn axis(i: usize) -> Vec<f32> {
        let mut v = vec![0.0; 768];
        v[i] = 2.0;
        v
    }

    #[test]
    fn semantic_reader_keeps_one_snapshot_while_the_writer_reuses_mail_identities() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path().join("moss.db")).unwrap();
        store.upsert_account(&account("work", "sender@example.test")).unwrap();
        let old = ingest(&mut store, "old", &mail().id("old@example.test").subject("Old conversation").body("Original text").attach("old.txt", "text/plain", "old file").build());
        store.set_embedding(old.message_id.parse().unwrap(), "fixture", &axis(1)).unwrap();
        store.keep_attachment_text(&old.message_id, 0, "Original attachment text").unwrap();
        let old_file = store.unembedded_files("fixture", 1).unwrap().remove(0);
        store.set_file_embedding(old_file.text_id, "fixture", &axis(2)).unwrap();
        store.conn.execute("UPDATE bodies SET used_at=7", []).unwrap();
        let reader = store.meaning_reader().unwrap().unwrap();
        assert!(reader.store.conn.execute("UPDATE messages SET subject='must not write'", []).is_err());
        store.claim_delivery_session().unwrap();
        assert!(store.remove_account("work").unwrap());
        store.upsert_account(&account("work", "replacement@example.test")).unwrap();
        let new = ingest(&mut store, "new", &mail().id("new@example.test").subject("Replacement conversation").body("Replacement text").attach("new.txt", "text/plain", "new file").build());
        assert_eq!((old.thread_id, old.message_id), (new.thread_id.clone(), new.message_id.clone()));
        store.set_embedding(new.message_id.parse().unwrap(), "fixture", &axis(1)).unwrap();
        store.keep_attachment_text(&new.message_id, 0, "Replacement attachment text").unwrap();
        let new_file = store.unembedded_files("fixture", 1).unwrap().remove(0);
        assert_eq!(old_file.text_id, new_file.text_id);
        store.set_file_embedding(new_file.text_id, "fixture", &axis(2)).unwrap();
        let files = reader.search("fixture", &axis(2), Some("work"), 0.3, 10, NOV_1).unwrap();
        assert_eq!(files[0].file.as_ref().unwrap().name, "old.txt");
        assert_eq!(files[0].snippet.iter().map(|part|part.text.as_str()).collect::<String>(), "Original attachment text");
        let hits = reader.search("fixture", &axis(1), Some("work"), 0.3, 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread.subject, "Old conversation", "ranking and hydration must share the original read snapshot");
        assert_eq!(hits[0].snippet.iter().map(|part| part.text.as_str()).collect::<String>(), "Original text");
        assert_eq!(reader.store.conn.query_row("SELECT used_at FROM bodies", [], |row|row.get::<_,i64>(0)).unwrap(), 7);
        assert!(reader.search("fixture", &axis(1), Some("absent"), 0.3, 10, NOV_1).unwrap().is_empty());
        assert_eq!(store.meaning_search("fixture", &axis(1), None, 0.3, 10, NOV_1).unwrap()[0].thread.subject, "Replacement conversation");
        drop(reader);
        assert_eq!(store.meaning_reader().unwrap().unwrap().search("fixture", &axis(1), None, 0.3, 10, NOV_1).unwrap()[0].thread.subject, "Replacement conversation");
    }

    #[test]
    fn semantic_readers_refuse_changed_schema_and_missing_databases() {
        assert!(Store::open_in_memory().unwrap().meaning_reader().unwrap().is_none());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("moss.db");
        let store = Store::open(&path).unwrap();
        store.conn.pragma_update(None,"user_version",0).unwrap();
        assert!(store.meaning_reader().is_err());
        store.conn.pragma_update(None,"user_version",crate::schema::MIGRATIONS.len() as u32).unwrap();
        store.conn.execute_batch("DROP TABLE embeddings").unwrap();
        assert!(store.meaning_reader().unwrap().unwrap().search("fixture",&axis(1),None,0.3,10,NOV_1).is_err());
        std::fs::remove_file(&path).unwrap();
        let error = match store.meaning_reader() { Err(error) => error, Ok(_) => panic!("missing database must fail") };
        assert_eq!(error.to_string(), "The search database could not be opened. Reopen Moss before searching.");
        assert!(!path.exists(), "a search must not create an empty replacement database");
    }

    #[cfg(unix)]
    #[test]
    fn semantic_reader_does_not_follow_a_replaced_database_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("moss.db");
        let moved = directory.path().join("moved.db");
        let store = Store::open(&path).unwrap();
        store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        std::fs::rename(&path,&moved).unwrap();
        std::os::unix::fs::symlink(&moved,&path).unwrap();
        assert!(store.meaning_reader().is_err());
    }

    #[test]
    fn embedding_backfill_pages_seek_past_completed_messages() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        let mut store = store_with_work();
        ingest(&mut store, "first", &mail().build());
        store.conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<4000)
            INSERT INTO messages(account_id,thread_id,message_id,from_name,from_email,to_json,cc_json,bcc_json,subject,sent_at,snippet,read,from_me,participants,received_at,split,section)
            SELECT account_id,thread_id,printf('embedding%d@example.test',i),from_name,from_email,to_json,cc_json,bcc_json,subject,sent_at,snippet,0,0,participants,received_at,split,section FROM messages,n WHERE messages.id=1;").unwrap();
        let ticks = Arc::new(AtomicUsize::new(0));
        let count = ticks.clone();
        store.conn.progress_handler(1, Some(move || { count.fetch_add(1, Ordering::Relaxed); false })).unwrap();
        let mut cursor = None;
        let mut seen = Vec::new();
        loop {
            ticks.store(0, Ordering::Relaxed);
            let (batch, next) = store.unembedded_after("test", 32, cursor).unwrap();
            let steps = ticks.load(Ordering::Relaxed);
            assert!(steps < 10000, "Each indexed page should seek, not rescan the inbox: {steps} VM steps");
            if batch.is_empty() { assert!(next.is_none()); break; }
            assert!(next.is_some());
            for message in &batch {
                seen.push(message.message_id);
                store.set_embedding(message.message_id, "test", &[1.0]).unwrap();
            }
            cursor = next;
        }
        assert_eq!(seen, (1..=4001).rev().collect::<Vec<_>>(), "equal timestamps must neither skip nor repeat messages");
        assert!(store.unembedded_after("test", 32, None).unwrap().0.is_empty());
        assert_eq!(store.unembedded_after("different-model", 2, None).unwrap().0.len(), 2);
        assert!(store.unembedded_after("different-model", 0, None).unwrap().0.is_empty());
        store.conn.progress_handler(0, None::<fn() -> bool>).unwrap();
    }

    #[test]
    fn a_reused_message_id_cannot_receive_an_older_contents_vector() {
        let mut store=store_with_work();let first=ingest(&mut store,"first",&mail().id("first@example.test").subject("Old content").build());
        let expected=store.unembedded("test",1).unwrap().remove(0);
        store.conn.execute("DELETE FROM messages WHERE id=?1",[&first.message_id]).unwrap();
        let second=ingest(&mut store,"second",&mail().id("second@example.test").subject("New content").build());
        assert_eq!(first.message_id,second.message_id,"fixture exercises rowid reuse");
        assert!(!store.set_embedding_if_current(&expected,"test",&[1.0]).unwrap());
        assert_eq!(store.embedded("test").unwrap(),(0,1));
        let current=store.unembedded("test",1).unwrap().remove(0);
        assert!(store.set_embedding_if_current(&current,"test",&[1.0]).unwrap());
        assert_eq!(store.embedded("test").unwrap(),(1,1));
        store.conn.execute("DELETE FROM messages",[]).unwrap();
        assert!(!store.set_embedding_if_current(&current,"test",&[1.0]).unwrap());
    }

    #[test]
    fn deleted_messages_do_not_abort_late_embedding_results_or_create_orphans() {
        let mut store=store_with_work();let mail=ingest(&mut store,"gone",&mail().build());
        let id=mail.message_id.parse::<i64>().unwrap();
        store.conn.execute("DELETE FROM messages WHERE id=?1",[id]).unwrap();
        store.set_embedding(id,"test",&[1.0]).unwrap();
        assert_eq!(store.embedded("test").unwrap(),(0,0));
    }

    #[test]
    fn vectors_are_cut_made_unit_length_and_stored_a_byte_a_number() {
        let q = quantize(&axis(3));
        assert_eq!(q.len(), DIMS);
        assert_eq!(q[3] as i8, 127);
        assert!((similarity(&q, &q) - 1.0).abs() < 1e-9);
        assert_eq!(similarity(&q, &quantize(&axis(4))), 0.0);
        let opposite: Vec<f32> = axis(3).iter().map(|x| -x).collect();
        assert!((similarity(&q, &quantize(&opposite)) + 1.0).abs() < 1e-9);
        assert_eq!(quantize(&[0.0; 4]), [0, 0, 0, 0], "an empty vector stays empty");
    }

    #[test]
    fn messages_wait_for_a_vector_from_each_model_newest_first() {
        let mut store = store_with_work();
        let old = ingest(&mut store, "1:1", &mail().id("a@x").at(NOV_1 - DAY).subject("Flight").body("Lisbon on Friday").build());
        let new = ingest(&mut store, "1:2", &mail().id("b@x").at(NOV_1).subject("Lunch").build());
        let waiting = store.unembedded("gemma", 10).unwrap();
        let ids: Vec<String> = waiting.iter().map(|m| m.message_id.to_string()).collect();
        assert_eq!(ids, [new.message_id.clone(), old.message_id.clone()]);
        assert_eq!((waiting[1].subject.as_str(), waiting[1].text.as_str()), ("Flight", "Lisbon on Friday"));
        store.set_embedding(waiting[0].message_id, "gemma", &axis(1)).unwrap();
        assert_eq!(store.unembedded("gemma", 10).unwrap().len(), 1);
        assert_eq!(store.unembedded("other", 10).unwrap().len(), 2);
        assert_eq!(store.embedded("gemma").unwrap(), (1, 2));
        // Without the body, the preview stands in.
        store.conn.execute_batch("DELETE FROM bodies").unwrap();
        assert_eq!(store.unembedded("gemma", 10).unwrap()[0].text, "Lisbon on Friday");
    }

    #[test]
    fn search_by_meaning_finds_the_nearest_thread_in_an_account() {
        let mut store = store_with_work();
        store.upsert_account(&account("home", "maya@home.org")).unwrap();
        let flight = ingest(&mut store, "1:1", &mail().id("a@x").subject("Flight").build());
        let lunch = ingest(&mut store, "1:2", &mail().id("b@x").subject("Lunch").build());
        let reply = ingest(&mut store, "1:3", &mail().id("c@x").subject("Re: Lunch").reply_to(&["b@x"]).build());
        let home = store
            .ingest(&mail().id("d@x").subject("Trip").build(), &moss_core::MessageSource { account_id: "home".into(), ..source("9:1") })
            .unwrap();
        let mut near = axis(1);
        near[2] = 1.0;
        for (m, v) in [(&flight, axis(1)), (&lunch, axis(2)), (&reply, near.clone()), (&home, axis(1))] {
            store.set_embedding(m.message_id.parse().unwrap(), "gemma", &v).unwrap();
        }
        let hits = store.meaning_search("gemma", &axis(1), Some("work"), 0.3, 10, NOV_1).unwrap();
        let threads: Vec<&str> = hits.iter().map(|h| h.thread.id.as_str()).collect();
        assert_eq!(threads, [flight.thread_id.as_str(), lunch.thread_id.as_str()]);
        assert_eq!(hits[1].message_id, reply.message_id, "the thread's nearest message");
        assert!(hits[0].rank < hits[1].rank);
        assert_eq!(store.meaning_search("gemma", &axis(1), None, 0.3, 1, NOV_1).unwrap().len(), 1);
        assert!(store.meaning_search("gemma", &axis(9), None, 0.3, 10, NOV_1).unwrap().is_empty());
        assert!(store.meaning_search("other", &axis(1), None, 0.3, 10, NOV_1).unwrap().is_empty());
        store.delete_thread(&flight.thread_id).unwrap();
        let hits = store.meaning_search("gemma", &axis(1), Some("work"), 0.3, 10, NOV_1).unwrap();
        let threads: Vec<&str> = hits.iter().map(|h| h.thread.id.as_str()).collect();
        assert_eq!(threads, [lunch.thread_id.as_str()], "deleted mail isn't found");
    }

    #[test]
    fn a_thread_is_found_by_its_nearest_message_even_when_an_earlier_one_matches_too() {
        let mut store = store_with_work();
        let lunch = ingest(&mut store, "1:1", &mail().id("a@x").subject("Lunch").build());
        let reply = ingest(&mut store, "1:2", &mail().id("b@x").subject("Re: Lunch").reply_to(&["a@x"]).build());
        // Both near enough to the search; the reply is nearer.
        let mut weaker = axis(1);
        weaker[2] = 4.0;
        let mut nearer = axis(1);
        nearer[2] = 1.0;
        store.set_embedding(lunch.message_id.parse().unwrap(), "gemma", &weaker).unwrap();
        store.set_embedding(reply.message_id.parse().unwrap(), "gemma", &nearer).unwrap();
        let hits = store.meaning_search("gemma", &axis(1), None, 0.3, 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1, "one hit per thread");
        assert_eq!(hits[0].message_id, reply.message_id);
    }

    #[test]
    fn the_text_inside_files_is_found_by_meaning_and_the_hit_names_the_file() {
        let mut store = store_with_work();
        let letter = mail().id("a@x").subject("Letter from the council").attach("appointment.pdf", "application/pdf", "%PDF");
        let letter = ingest(&mut store, "1:1", &letter.build());
        store.set_embedding(letter.message_id.parse().unwrap(), "gemma", &axis(2)).unwrap();
        store.keep_attachment_text(&letter.message_id, 0, "Resident parking permit").unwrap();
        let file = store.unembedded_files("gemma", 1).unwrap().remove(0);
        store.set_file_embedding(file.text_id, "gemma", &axis(1)).unwrap();

        let hits = store.meaning_search("gemma", &axis(1), None, 0.3, 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].thread.id.as_str(), hits[0].message_id.as_str()), (letter.thread_id.as_str(), letter.message_id.as_str()));
        assert_eq!(hits[0].file.as_ref().map(|f| f.name.as_str()), Some("appointment.pdf"));
        // Nearer the email itself: the hit is the email.
        let hits = store.meaning_search("gemma", &axis(2), None, 0.3, 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file.is_none(), "an email hit must not expose an attachment result");
        assert!(store.meaning_search("gemma", &axis(1), Some("home"), 0.3, 10, NOV_1).unwrap().is_empty());
        assert!(store.meaning_search("other", &axis(1), None, 0.3, 10, NOV_1).unwrap().is_empty());
        store.delete_thread(&letter.thread_id).unwrap();
        assert!(store.meaning_search("gemma", &axis(1), None, 0.3, 10, NOV_1).unwrap().is_empty());
    }

    #[test]
    fn failures_are_errors() {
        let store = store_with_work();
        read_only(&store);
        assert!(store.set_embedding(1, "gemma", &axis(1)).is_err());
        break_schema(&store, "PRAGMA query_only = false; DROP TABLE embeddings");
        assert!(store.unembedded("gemma", 1).is_err());
        assert!(store.embedded("gemma").is_err());
        assert!(store.meaning_search("gemma", &axis(1), None, 0.0, 1, NOV_1).is_err());
    }
}
