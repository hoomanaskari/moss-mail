//! The text inside attachments, read on this Mac so search and Ask Moss can use it.
//!
//! The engine fetches a file, reads its text (`moss-extract`) and keeps the text here,
//! compressed like bodies. The file itself stays on the server; its bytes are only cache.
//! Every file read gets a row, even one with no words (a photo of a beach), so each file is
//! read once. Each text is indexed by its words ([`Store::search`]) and its meaning
//! ([`Store::meaning_search`]).
//!
//! Reading is on unless you turn it off ([`Store::set_reading_attachments`]); turning it
//! off forgets every text kept.

use moss_core::Thread;
use rusqlite::{params, OptionalExtension, Transaction};

use crate::bodies::{pack, unpack, METHOD};
use crate::codec::{now_ms, parse_id};
use crate::meaning::{quantize, ToEmbed};
use crate::{Result, Store};

const READING_KEY: &str = "attachments.text";
const STARTED: &str = "started:";

/// Characters of a file's text the embedding model reads: its start, where the title and
/// the gist usually are.
const EMBED_CHARS: usize = 1_500;

/// A file waiting to be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToRead {
    pub message_id: String,
    pub index: u32,
    pub name: String,
    pub content_type: String,
    pub bytes: u64,
}

/// A file's text waiting for a vector: its row in `attachment_text`, and what the model reads
/// (the file name as the subject, the start of its text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileToEmbed {
    pub text_id: i64,
    pub email: ToEmbed,
}

/// Adds a file's words to the keyword index.
pub(crate) fn index_words(tx: &Transaction, text_id: i64, text: &str) -> Result<()> {
    tx.execute("INSERT INTO attachment_search (rowid, text) VALUES (?1, ?2)", params![text_id, text])?;
    Ok(())
}

fn write_text(tx: &Transaction, id: i64, index: u32, text: &str) -> Result<()> {
    tx.execute("DELETE FROM attachment_text WHERE message_id = ?1 AND position = ?2", params![id, index])?;
    tx.execute(
        "INSERT INTO attachment_text (message_id, position, method, text, chars, read_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, index, METHOD, pack(text), text.chars().count() as i64, now_ms()],
    )?;
    index_words(tx, tx.last_insert_rowid(), text)?;
    Ok(())
}

impl Store {
    /// Whether Moss reads the text inside attachments. On unless turned off.
    pub fn reading_attachments(&self) -> Result<bool> {
        Ok(self.setting(READING_KEY)?.as_deref() != Some("off"))
    }

    /// Turns reading attachments on or off. Off forgets every text kept, and its place in
    /// the indexes; on reads them again, as they're needed.
    pub fn set_reading_attachments(&mut self, on: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![READING_KEY, if on { "on" } else { "off" }],
        )?;
        if !on {
            tx.execute("DELETE FROM attachment_text", [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The text of attachment `index`: `None` until it's been read, and `Some("")` for a
    /// file with no words in it.
    pub fn attachment_text(&self, message_id: &str, index: u32) -> Result<Option<String>> {
        let row: Option<(String, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT method, text FROM attachment_text WHERE message_id = ?1 AND position = ?2",
                params![parse_id(message_id), index],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        // A text this Moss can't unpack is read again.
        Ok(row.and_then(|(method, packed)| unpack(&method, &packed)))
    }

    /// Commit a unique attempt before native parsing. Markers are not empty results:
    /// they mean the file has no usable extraction yet, and must not be retried blindly.
    pub fn begin_attachment_read(&mut self, message_id: &str, index: u32) -> Result<Option<String>> {
        self.delivery_transaction(|store| {
            if !store.reading_attachments()? { return Ok(None); }
            let id = parse_id(message_id);
            let method: Option<String> = store.conn.query_row("SELECT method FROM attachment_text WHERE message_id=?1 AND position=?2", params![id,index], |row| row.get(0)).optional()?;
            if method.is_some_and(|method| method.starts_with(STARTED)) || store.attachment_text(message_id, index)?.is_some() { return Ok(None); }
            let generation: String = store.conn.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
            let token = format!("{STARTED}{generation}");
            store.conn.execute("DELETE FROM attachment_text WHERE message_id=?1 AND position=?2", params![id, index])?;
            store.conn.execute("INSERT INTO attachment_text(message_id,position,method,text,chars,read_at) VALUES(?1,?2,?3,?4,0,?5)", params![id,index,token,pack(""),now_ms()])?;
            Ok(Some(token))
        })
    }

    /// Only the still-current attempt may publish text. Turning reading off removes
    /// the marker; turning it back on creates a different generation.
    pub fn finish_attachment_read(&mut self, message_id: &str, index: u32, token: &str, text: &str) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let id = parse_id(message_id);
        let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM attachment_text WHERE message_id=?1 AND position=?2 AND method=?3) AND NOT EXISTS(SELECT 1 FROM settings WHERE key=?4 AND value='off')", params![id,index,token,READING_KEY], |row| row.get(0))?;
        if !current { return Ok(false); }
        write_text(&tx, id, index, text)?;
        tx.commit()?;
        Ok(true)
    }

    /// Keeps the text read from attachment `index`, replacing any there was, and indexes its
    /// words. An empty text records that the file has none, so it isn't read again.
    pub fn keep_attachment_text(&mut self, message_id: &str, index: u32, text: &str) -> Result<()> {
        let id = parse_id(message_id);
        let tx = self.conn.transaction()?;
        write_text(&tx, id, index, text)?;
        tx.commit()?;
        Ok(())
    }

    /// Up to `limit` files to read in the background, newest first: files (not images shown
    /// in the body) people sent since `since`, in Important or Other, up to `max_bytes`, of
    /// a kind `readable` says Moss reads, and not read yet. Leaves out your own mail, deleted
    /// and blocked mail, and `skip`.
    pub fn to_read(
        &self,
        since: i64,
        max_bytes: u64,
        readable: &dyn Fn(&str, &str) -> bool,
        skip: &[(String, u32)],
        limit: usize,
    ) -> Result<Vec<ToRead>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT a.message_id, a.position, a.name, a.content_type, a.bytes FROM attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE a.inline = 0 AND a.bytes <= ?2 AND m.sent_at >= ?1 AND m.from_me = 0
               AND m.split IN ('important', 'other') AND m.deleted_from IS NULL AND m.blocked = 0
               AND NOT EXISTS (SELECT 1 FROM attachment_text t WHERE t.message_id = a.message_id AND t.position = a.position)
             ORDER BY m.sent_at DESC, a.message_id DESC, a.position",
        )?;
        let rows = stmt.query_map(params![since, max_bytes as i64], |r| {
            Ok(ToRead {
                message_id: r.get::<_, i64>(0)?.to_string(),
                index: r.get(1)?,
                name: r.get(2)?,
                content_type: r.get(3)?,
                bytes: r.get::<_, i64>(4)? as u64,
            })
        })?;
        let mut list = Vec::new();
        for file in rows {
            let file = file?;
            if readable(&file.name, &file.content_type) && !skip.iter().any(|(m, i)| *m == file.message_id && *i == file.index) {
                list.push(file);
                if list.len() == limit {
                    break;
                }
            }
        }
        Ok(list)
    }

    /// Fills in the text of each file in `thread` that's been read (see
    /// `moss_core::Attachment::text`), for Ask Moss and the writing model.
    pub fn add_file_texts(&self, thread: &mut Thread) -> Result<()> {
        for message in &mut thread.messages {
            for file in message.attachments.iter_mut().filter(|a| !a.inline) {
                file.text = self.attachment_text(&message.id, file.index)?;
            }
        }
        Ok(())
    }

    /// Up to `limit` files' texts without a vector from `model`, newest mail first. Files with
    /// no words are left out: there's nothing to mean.
    pub fn unembedded_files(&self, model: &str, limit: usize) -> Result<Vec<FileToEmbed>> {
        Ok(self.unembedded_files_after(model,limit,None)?.0)
    }

    /// Seek by mail timestamp, message identity and attachment position. A fresh
    /// pass starts at the top to include newly read or replaced attachment text.
    pub fn unembedded_files_after(&self, model: &str, limit: usize, after: Option<(i64,i64,u32)>) -> Result<(Vec<FileToEmbed>,Option<(i64,i64,u32)>)> {
        let select = "SELECT t.id,m.id,a.name,t.method,t.text,m.sent_at,t.position
            FROM messages m INDEXED BY messages_by_sent_at
            CROSS JOIN attachment_text t ON t.message_id=m.id
            CROSS JOIN attachments a ON a.message_id=t.message_id AND a.position=t.position
            WHERE t.chars>0 AND NOT EXISTS (SELECT 1 FROM attachment_embeddings e WHERE e.text_id=t.id AND e.model=?1)";
        // Separate equal-prefix ranges so SQLite seeks every cursor component.
        let sql = if after.is_some() {
            format!("{select} AND m.sent_at=?3 AND m.id=?4 AND t.position<?5
                UNION ALL {select} AND m.sent_at=?3 AND m.id<?4
                UNION ALL {select} AND m.sent_at<?3 ORDER BY 6 DESC,2 DESC,7 DESC LIMIT ?2")
        } else { format!("{select} ORDER BY m.sent_at DESC,m.id DESC,t.position DESC LIMIT ?2") };
        let mut stmt=self.conn.prepare(&sql)?;
        let read=|r: &rusqlite::Row<'_>|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Vec<u8>>(4)?,r.get::<_,i64>(5)?,r.get::<_,u32>(6)?));
        let rows=match after {
            Some((sent_at,id,position))=>stmt.query_map(params![model,limit as i64,sent_at,id,position],read)?,
            None=>stmt.query_map(params![model,limit as i64],read)?,
        };
        let mut list=Vec::new();let mut cursor=None;
        for row in rows {
            let (text_id,message_id,name,method,packed,sent_at,position)=row?;
            let text=unpack(&method,&packed).unwrap_or_default().chars().take(EMBED_CHARS).collect();
            list.push(FileToEmbed {text_id,email:ToEmbed {message_id,subject:name,text}});
            cursor=Some((sent_at,message_id,position));
        }
        Ok((list,cursor))
    }

    /// A completed model call owns only the text and name it actually read.
    pub fn set_file_embedding_if_current(&self,expected:&FileToEmbed,model:&str,vector:&[f32])->Result<bool> {
        let tx=self.conn.unchecked_transaction()?;
        let current=self.conn.query_row("SELECT t.message_id,a.name,t.method,t.text FROM attachment_text t JOIN attachments a ON a.message_id=t.message_id AND a.position=t.position WHERE t.id=?1",[expected.text_id],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Vec<u8>>(3)?))).optional()?;
        let Some((message_id,subject,method,packed))=current else {return Ok(false);};
        let text=unpack(&method,&packed).unwrap_or_default().chars().take(EMBED_CHARS).collect();
        if (ToEmbed{message_id,subject,text})!=expected.email {return Ok(false);}
        self.set_file_embedding(expected.text_id,model,vector)?;
        tx.commit()?;
        Ok(true)
    }

    /// Saves the vector of a file's text from `model`, replacing any older one.
    pub fn set_file_embedding(&self, text_id: i64, model: &str, vector: &[f32]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO attachment_embeddings (text_id, model, vector) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM attachment_text WHERE id=?1)",
            params![text_id, model, quantize(vector)],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn invitation(store: &mut Store) -> crate::Ingested {
        let raw = mail()
            .id("permit@x")
            .from("Elmford Council <permits@elmford-council.example>")
            .subject("Letter from the council")
            .body("Please see the attached letter.")
            .attach("P2026-0457.pdf", "application/pdf", "%PDF")
            .inline("logo.png", "image/png", "png")
            .build();
        ingest(store, "1:1", &raw)
    }

    fn all(_: &str, _: &str) -> bool {
        true
    }

    #[test]
    fn extraction_completion_requires_the_current_enabled_attempt_and_commits_atomically() {
        let mut store=store_with_work();let id=invitation(&mut store).message_id;
        store.set_reading_attachments(false).unwrap();
        assert_eq!(store.begin_attachment_read(&id,0).unwrap(),None);
        store.set_reading_attachments(true).unwrap();
        let token=store.begin_attachment_read(&id,0).unwrap().unwrap();
        assert!(!store.finish_attachment_read(&id,0,"wrong attempt","stale").unwrap());
        assert_eq!(store.attachment_text(&id,0).unwrap(),None);
        store.conn.execute_batch("CREATE TRIGGER refuse_text BEFORE INSERT ON attachment_text BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(store.finish_attachment_read(&id,0,&token,"current").is_err());
        assert_eq!(store.attachment_text(&id,0).unwrap(),None);
        store.conn.execute_batch("DROP TRIGGER refuse_text").unwrap();
        assert!(store.finish_attachment_read(&id,0,&token,"current").unwrap());
        assert!(!store.finish_attachment_read(&id,0,&token,"replayed").unwrap());
        assert_eq!(store.attachment_text(&id,0).unwrap().as_deref(),Some("current"));
    }

    #[test]
    fn extraction_claims_preserve_results_and_fail_closed_on_storage_errors() {
        let mut store = store_with_work();
        let id = invitation(&mut store).message_id;
        assert!(store.begin_attachment_read(&id, 0).unwrap().is_some());
        assert!(store.begin_attachment_read(&id, 0).unwrap().is_none());
        assert_eq!(store.attachment_text(&id, 0).unwrap(), None);
        store.keep_attachment_text(&id, 0, "preserved result").unwrap();
        assert!(store.begin_attachment_read(&id, 0).unwrap().is_none());
        assert_eq!(store.attachment_text(&id, 0).unwrap().as_deref(), Some("preserved result"));
        store.conn.execute("UPDATE attachment_text SET method='unsupported' WHERE message_id=?1", [&id]).unwrap();
        assert!(store.begin_attachment_read(&id, 0).unwrap().is_some());
        store.conn.execute("DELETE FROM attachment_text WHERE message_id=?1", [&id]).unwrap();
        store.conn.execute_batch("CREATE TRIGGER refuse_attempt BEFORE INSERT ON attachment_text BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(store.begin_attachment_read(&id, 0).is_err());
        assert_eq!(store.attachment_text(&id, 0).unwrap(), None);
    }

    #[test]
    fn a_files_text_is_kept_compressed_and_read_once() {
        let mut store = store_with_work();
        let id = invitation(&mut store).message_id;
        assert_eq!(store.attachment_text(&id, 0).unwrap(), None, "not read yet");
        let text = "Resident Parking Permit Notice. Date: December 12, 2026. ".repeat(50);
        store.keep_attachment_text(&id, 0, &text).unwrap();
        assert_eq!(store.attachment_text(&id, 0).unwrap().as_deref(), Some(text.as_str()));
        let (method, packed, chars): (String, Vec<u8>, i64) = store
            .conn
            .query_row("SELECT method, text, chars FROM attachment_text", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!((method.as_str(), chars as usize), ("zstd", text.len()));
        assert!(packed.len() < text.len() / 10, "{} bytes", packed.len());
        // Read again: replaced, not added.
        store.keep_attachment_text(&id, 0, "").unwrap();
        assert_eq!(store.attachment_text(&id, 0).unwrap().as_deref(), Some(""));
        let rows: i64 = store.conn.query_row("SELECT count(*) FROM attachment_text", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
        // A method this Moss doesn't know reads as not read.
        store.conn.execute("UPDATE attachment_text SET method = 'brotli'", []).unwrap();
        assert_eq!(store.attachment_text(&id, 0).unwrap(), None);
    }

    #[test]
    fn reading_is_on_until_turned_off_and_off_forgets_the_texts() {
        let mut store = store_with_work();
        let id = invitation(&mut store).message_id;
        assert!(store.reading_attachments().unwrap());
        store.keep_attachment_text(&id, 0, "permit").unwrap();
        store.set_reading_attachments(false).unwrap();
        assert!(!store.reading_attachments().unwrap());
        assert_eq!(store.attachment_text(&id, 0).unwrap(), None);
        let indexed: i64 = store.conn.query_row("SELECT count(*) FROM attachment_search WHERE attachment_search MATCH 'permit'", [], |r| r.get(0)).unwrap();
        assert_eq!(indexed, 0, "its words leave the index too");
        store.set_reading_attachments(true).unwrap();
        assert!(store.reading_attachments().unwrap());
    }

    #[test]
    fn recent_files_from_people_wait_to_be_read_newest_first() {
        let mut store = store_with_work();
        let letter = invitation(&mut store).message_id;
        let older = mail().id("old@x").at(NOV_1 - 10 * DAY).attach("old.pdf", "application/pdf", "%PDF").attach("clip.mov", "video/quicktime", "mov");
        let older = ingest(&mut store, "1:2", &older.build()).message_id;
        let ancient = mail().id("ancient@x").at(NOV_1 - 100 * DAY).attach("ancient.pdf", "application/pdf", "%PDF");
        ingest(&mut store, "1:3", &ancient.build());
        let news = mail().id("news@x").attach("issue.pdf", "application/pdf", "%PDF");
        let news = ingest(&mut store, "1:4", &news.build());
        store.conn.execute("UPDATE messages SET split = 'newsletters' WHERE id = ?1", [&news.message_id]).unwrap();
        let mine = mail().id("mine@x").from("Maya Ross <maya@northwind.co>").attach("mine.pdf", "application/pdf", "%PDF");
        ingest(&mut store, "1:5", &mine.build());

        let since = NOV_1 - 30 * DAY;
        let names = |store: &Store, readable: &dyn Fn(&str, &str) -> bool, skip: &[(String, u32)], limit| -> Vec<String> {
            store.to_read(since, 1_000, readable, skip, limit).unwrap().into_iter().map(|f| f.name).collect()
        };
        assert_eq!(names(&store, &all, &[], 10), ["P2026-0457.pdf", "old.pdf", "clip.mov"]);
        let pdfs = |name: &str, _: &str| name.ends_with(".pdf");
        assert_eq!(names(&store, &pdfs, &[], 10), ["P2026-0457.pdf", "old.pdf"]);
        assert_eq!(names(&store, &pdfs, &[], 1), ["P2026-0457.pdf"]);
        assert_eq!(names(&store, &pdfs, &[(letter.clone(), 0)], 10), ["old.pdf"]);
        let first = store.to_read(since, 1_000, &all, &[], 1).unwrap().remove(0);
        assert_eq!(first, ToRead { message_id: letter.clone(), index: 0, name: "P2026-0457.pdf".into(), content_type: "application/pdf".into(), bytes: 4 });
        // Read, too big, deleted or blocked: not waiting.
        store.keep_attachment_text(&letter, 0, "").unwrap();
        assert_eq!(names(&store, &all, &[], 10), ["old.pdf", "clip.mov"]);
        assert!(store.to_read(since, 2, &all, &[], 10).unwrap().is_empty());
        store.conn.execute("UPDATE messages SET blocked = 1 WHERE id = ?1", [&older]).unwrap();
        assert!(names(&store, &all, &[], 10).is_empty());
    }

    #[test]
    fn a_thread_gets_the_text_of_its_files_that_were_read() {
        let mut store = store_with_work();
        let ingested = invitation(&mut store);
        let id = ingested.message_id;
        let mut thread = store.thread(&ingested.thread_id, NOV_1).unwrap().unwrap();
        store.add_file_texts(&mut thread).unwrap();
        assert_eq!(thread.messages[0].attachments[0].text, None);
        store.keep_attachment_text(&id, 0, "Date: December 12, 2026").unwrap();
        store.keep_attachment_text(&id, 1, "the logo").unwrap();
        store.add_file_texts(&mut thread).unwrap();
        let files = &thread.messages[0].attachments;
        assert_eq!(files[0].text.as_deref(), Some("Date: December 12, 2026"));
        assert_eq!(files[1].text, None, "images in the body aren't files");
    }

    #[test]
    fn file_embedding_pages_seek_across_timestamp_and_message_ties() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        let mut store = store_with_work();
        let first = invitation(&mut store).message_id;
        store.keep_attachment_text(&first,0,"first").unwrap();
        store.keep_attachment_text(&first,1,"second").unwrap();
        store.conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000)
            INSERT INTO messages(account_id,thread_id,message_id,from_name,from_email,to_json,cc_json,bcc_json,subject,sent_at,snippet,read,from_me,participants,received_at,split,section)
            SELECT account_id,thread_id,printf('fileembedding%d@example.test',i),from_name,from_email,to_json,cc_json,bcc_json,subject,sent_at-(i%3),snippet,0,0,participants,received_at,split,section FROM messages,n WHERE messages.id=1;
            INSERT INTO attachments(message_id,position,name,content_type,bytes,content_id,inline) SELECT m.id,a.position,a.name,a.content_type,a.bytes,a.content_id,a.inline FROM messages m,attachments a WHERE m.id>1 AND a.message_id=1;
            INSERT INTO attachment_text(message_id,position,method,text,chars,read_at) SELECT m.id,t.position,t.method,t.text,t.chars,t.read_at FROM messages m,attachment_text t WHERE m.id>1 AND t.message_id=1;").unwrap();
        let expected: Vec<i64> = store.conn.prepare("SELECT t.id FROM attachment_text t JOIN messages m ON m.id=t.message_id ORDER BY m.sent_at DESC,m.id DESC,t.position DESC").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let ticks=Arc::new(AtomicUsize::new(0));let count=ticks.clone();
        store.conn.progress_handler(1,Some(move||{count.fetch_add(1,Ordering::Relaxed);false})).unwrap();
        let mut seen=Vec::new();let mut cursor=None;
        loop {
            ticks.store(0,Ordering::Relaxed);
            let (batch,next)=store.unembedded_files_after("test",17,cursor).unwrap();
            let steps=ticks.load(Ordering::Relaxed);
            assert!(steps<10000,"file batch must seek instead of rescanning: {steps}, after {cursor:?}");
            if batch.is_empty(){assert!(next.is_none());break;}
            for file in &batch { seen.push(file.text_id);store.set_file_embedding(file.text_id,"test",&[1.0]).unwrap(); }
            cursor=next;
        }
        assert_eq!(seen,expected);
        assert_eq!(seen.len(),2002);
        assert_eq!(store.unembedded_files_after("other",2,None).unwrap().0.len(),2);
        assert!(store.unembedded_files_after("other",0,None).unwrap().0.is_empty());
        store.conn.progress_handler(0,None::<fn()->bool>).unwrap();
    }

    #[test]
    fn replaced_attachment_text_ignores_old_vectors_without_hiding_the_new_text() {
        let mut store=store_with_work();let id=invitation(&mut store).message_id;
        store.keep_attachment_text(&id,0,"old text").unwrap();
        let old=store.unembedded_files("test",1).unwrap().remove(0);
        store.keep_attachment_text(&id,0,"new text").unwrap();
        assert!(!store.set_file_embedding_if_current(&old,"test",&[1.0]).unwrap());
        let waiting=store.unembedded_files("test",1).unwrap();
        assert_eq!(waiting.len(),1);assert_eq!(waiting[0].email.text,"new text");
        assert_eq!(waiting[0].text_id,old.text_id,"fixture exercises rowid reuse");
        let count:i64=store.conn.query_row("SELECT count(*) FROM attachment_embeddings",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
        assert!(store.set_file_embedding_if_current(&waiting[0],"test",&[1.0]).unwrap());
        assert!(store.unembedded_files("test",1).unwrap().is_empty());
        store.conn.execute("DELETE FROM attachment_text",[]).unwrap();
        assert!(!store.set_file_embedding_if_current(&waiting[0],"test",&[1.0]).unwrap());
    }

    #[test]
    fn texts_with_words_wait_for_a_vector_from_each_model() {
        let mut store = store_with_work();
        let id = invitation(&mut store).message_id;
        let other = ingest(&mut store, "1:2", &mail().id("b@x").at(NOV_1 - DAY).attach("blank.png", "image/png", "png").build()).message_id;
        store.keep_attachment_text(&other, 0, "").unwrap();
        assert!(store.unembedded_files("gemma", 10).unwrap().is_empty(), "nothing to mean");
        let long = format!("Resident Parking Permit {}", "x".repeat(3_000));
        store.keep_attachment_text(&id, 0, &long).unwrap();
        let waiting = store.unembedded_files("gemma", 10).unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].email.message_id.to_string(), id);
        assert_eq!(waiting[0].email.subject, "P2026-0457.pdf");
        assert_eq!(waiting[0].email.text.chars().count(), EMBED_CHARS);
        store.set_file_embedding(waiting[0].text_id, "gemma", &[1.0, 0.0]).unwrap();
        assert!(store.unembedded_files("gemma", 10).unwrap().is_empty());
        assert_eq!(store.unembedded_files("other", 10).unwrap().len(), 1);
        // Reading it again makes a new row, which waits for a vector again.
        store.keep_attachment_text(&id, 0, "again").unwrap();
        assert_eq!(store.unembedded_files("gemma", 10).unwrap()[0].email.text, "again");
        store.conn.execute("UPDATE attachment_text SET method = 'brotli'", []).unwrap();
        assert_eq!(store.unembedded_files("gemma", 10).unwrap()[0].email.text, "", "unreadable text means nothing");
    }

    #[test]
    fn failures_are_errors() {
        let mut store = store_with_work();
        let ingested = invitation(&mut store);
        let id = ingested.message_id;
        let mut thread = store.thread(&ingested.thread_id, NOV_1).unwrap().unwrap();
        read_only(&store);
        assert!(store.keep_attachment_text(&id, 0, "x").is_err());
        assert!(store.set_reading_attachments(false).is_err());
        assert!(store.set_file_embedding(1, "gemma", &[1.0]).is_err());
        break_schema(&store, "PRAGMA query_only = false; DROP TABLE attachment_embeddings; DROP TABLE attachment_text; DELETE FROM settings");
        assert!(store.attachment_text(&id, 0).is_err());
        assert!(store.to_read(0, 1, &all, &[], 1).is_err());
        assert!(store.unembedded_files("gemma", 1).is_err());
        assert!(store.add_file_texts(&mut thread).is_err());
        break_schema(&store, "DROP TABLE settings");
        assert!(store.reading_attachments().is_err());
    }
}
