//! Email bodies and file bytes kept on this device, and how much space they take.
//!
//! Bodies live in their own table, compressed, so lists and search never read them. Each
//! one records how it was compressed ([`METHOD`]), so that can change later. A body is
//! either kept for offline use, by the [`Offline`] setting, or it's cache: mail someone
//! opened that the setting wouldn't keep. Cache goes oldest first once it's over its limit
//! ([`Store::trim_cache`]). File bytes (inline images, opened attachments) work the same way.

use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::codec::now_ms;
use crate::{Result, Store};

/// How bodies are compressed now. Rows say which method they used.
pub const METHOD: &str = "zstd";
/// Level 12 compresses mail close to the maximum, many times faster than level 19.
const LEVEL: i32 = 12;
/// "Recent only" keeps this much mail for offline use.
pub const RECENT_MS: i64 = 90 * 86_400_000;
/// The cache limit until someone picks another.
pub const DEFAULT_CACHE_LIMIT: u64 = 500 * 1_000_000;

const OFFLINE_KEY: &str = "storage.offline";
const CACHE_LIMIT_KEY: &str = "storage.cacheLimit";

/// What Moss keeps on this device so mail opens offline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Offline {
    /// The text of mail from the last 90 days.
    Recent,
    /// The text of all mail, compressed. No files.
    #[default]
    Text,
    /// All mail with its files.
    Everything,
}

impl Offline {
    /// Whether a message sent at `sent_at` keeps its body for offline use.
    pub fn keeps_text(self, sent_at: i64, now: i64) -> bool {
        self != Offline::Recent || sent_at >= now - RECENT_MS
    }

    pub fn keeps_files(self) -> bool {
        self == Offline::Everything
    }
}

/// Space used on this device, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Everything that lists, finds and sorts mail: details, the keyword and meaning
    /// indexes, summaries.
    pub index: u64,
    /// Bodies and files kept by the offline setting, and originals only this device has.
    pub offline: u64,
    /// Bodies and files kept because someone opened them.
    pub cache: u64,
}

/// A body as stored: plain text, and HTML when the email has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub text: String,
    pub html: Option<String>,
}

pub(crate) fn pack(text: &str) -> Vec<u8> {
    zstd::encode_all(text.as_bytes(), LEVEL).expect("compressing in memory")
}

/// Unpacks a column compressed with `method`. A method this Moss doesn't know, or bytes
/// that don't unpack, read as missing: the body is fetched from the server again.
pub(crate) fn unpack(method: &str, packed: &[u8]) -> Option<String> {
    match method {
        METHOD => zstd::decode_all(packed).ok().and_then(|b| String::from_utf8(b).ok()),
        _ => None,
    }
}

/// Stores a body, replacing any there was. `offline` says whether the offline setting
/// keeps it; otherwise it's cache.
pub(crate) fn keep(tx: &Transaction, id: i64, body: &Body, offline: bool) -> Result<()> {
    let text = pack(&body.text);
    let html = body.html.as_deref().map(pack);
    let bytes = text.len() + html.as_ref().map_or(0, Vec::len);
    tx.execute(
        "INSERT OR REPLACE INTO bodies (message_id, method, text, html, bytes, offline, used_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, METHOD, text, html, bytes as i64, offline, now_ms()],
    )?;
    Ok(())
}

/// Stores a file's bytes, as offline or cache.
pub(crate) fn keep_file(tx: &Transaction, id: i64, position: i64, data: &[u8], offline: bool) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO attachment_data (message_id, position, data, offline, used_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, position, data, offline, now_ms()],
    )?;
    Ok(())
}

impl Store {
    /// What the offline setting is.
    pub fn offline(&self) -> Result<Offline> {
        let saved = self.setting(OFFLINE_KEY)?;
        Ok(saved.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    /// Changes the offline setting. Bodies it no longer keeps become cache; bodies it keeps
    /// now stop being cache. Mail it would now keep that isn't here comes when it's opened.
    pub fn set_offline(&mut self, offline: Offline, now: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![OFFLINE_KEY, serde_json::to_string(&offline)?],
        )?;
        let since = if offline == Offline::Recent { now - RECENT_MS } else { i64::MIN };
        tx.execute(
            "UPDATE bodies SET offline = (SELECT m.sent_at >= ?1 FROM messages m WHERE m.id = bodies.message_id)",
            [since],
        )?;
        tx.execute("UPDATE attachment_data SET offline = ?1", [offline.keeps_files()])?;
        tx.commit()?;
        Ok(())
    }

    /// The most the cache may hold, in bytes.
    pub fn cache_limit(&self) -> Result<u64> {
        let saved = self.setting(CACHE_LIMIT_KEY)?;
        Ok(saved.and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_CACHE_LIMIT))
    }

    pub fn set_cache_limit(&mut self, bytes: u64) -> Result<()> {
        self.set_setting(CACHE_LIMIT_KEY, Some(&bytes.to_string()))
    }

    /// A message's body, when it's on this device. Reading it counts as using it, so cache
    /// that's read stays longest.
    pub fn body(&self, message_id: i64) -> Result<Option<Body>> {
        let body = self.read_body(message_id)?;
        if body.is_some() {
            self.conn.execute("UPDATE bodies SET used_at = ?2 WHERE message_id = ?1", params![message_id, now_ms()])?;
        }
        Ok(body)
    }

    /// Search previews must not write cache bookkeeping or retain every matching body.
    pub(crate) fn read_body(&self, message_id: i64) -> Result<Option<Body>> {
        let row: Option<(String, Vec<u8>, Option<Vec<u8>>)> = self
            .conn
            .query_row(
                "SELECT method, text, html FROM bodies WHERE message_id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((method, text, html)) = row else {
            return Ok(None);
        };
        let Some(text) = unpack(&method, &text) else {
            return Ok(None);
        };
        let html = html.and_then(|h| unpack(&method, &h));
        Ok(Some(Body { text, html }))
    }

    /// Removes cache, oldest first, until it holds at most `limit` bytes. Bodies and files
    /// the offline setting keeps are never removed. Returns the bytes freed.
    pub fn trim_cache(&mut self, limit: u64) -> Result<u64> {
        let tx = self.conn.transaction()?;
        let rows: Vec<(bool, i64, i64, i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT 1, message_id, 0, bytes, used_at FROM bodies WHERE offline = 0
                 UNION ALL
                 SELECT 0, message_id, position, length(data), used_at FROM attachment_data WHERE offline = 0
                 ORDER BY 5 DESC, 2 DESC",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut kept = 0u64;
        let mut freed = 0u64;
        for (body, id, position, bytes, _) in rows {
            let bytes = bytes as u64;
            if kept + bytes <= limit {
                kept += bytes;
                continue;
            }
            if body {
                tx.execute("DELETE FROM bodies WHERE message_id = ?1", [id])?;
            } else {
                tx.execute(
                    "DELETE FROM attachment_data WHERE message_id = ?1 AND position = ?2",
                    [id, position],
                )?;
            }
            freed += bytes;
        }
        tx.commit()?;
        Ok(freed)
    }

    /// Removes all cache. The index, offline mail and settings stay.
    pub fn clear_cache(&mut self) -> Result<u64> {
        self.trim_cache(0)
    }

    /// Space used by each kind of data. The index is the rest of the file: messages,
    /// threads, the keyword and meaning indexes, details and summaries.
    pub fn usage(&self) -> Result<Usage> {
        let sum = |sql: &str| -> Result<u64> {
            Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
        };
        let offline = sum("SELECT coalesce(sum(bytes), 0) FROM bodies WHERE offline = 1")?
            + sum("SELECT coalesce(sum(length(data)), 0) FROM attachment_data WHERE offline = 1")?
            + sum("SELECT coalesce(sum(length(raw)), 0) FROM message_raw")?;
        let cache = sum("SELECT coalesce(sum(bytes), 0) FROM bodies WHERE offline = 0")?
            + sum("SELECT coalesce(sum(length(data)), 0) FROM attachment_data WHERE offline = 0")?;
        let pages = sum("SELECT page_count - freelist_count FROM pragma_page_count, pragma_freelist_count")?;
        let file = pages * sum("SELECT page_size FROM pragma_page_size")?;
        Ok(Usage { index: file.saturating_sub(offline + cache), offline, cache })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn now() -> i64 {
        now_ms()
    }

    fn add(store: &mut Store, provider_id: &str, sent_at: i64) -> i64 {
        let raw = mail().id(&format!("{provider_id}@x")).at(sent_at).body("Can you sign off the budget?").build();
        ingest(store, provider_id, &raw).message_id.parse().unwrap()
    }

    fn offline_flags(store: &Store) -> Vec<bool> {
        let mut stmt = store.conn.prepare("SELECT offline FROM bodies ORDER BY message_id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn bodies_are_compressed_and_say_how() {
        let mut store = store_with_work();
        let text = "Can you sign off the Q4 budget? ".repeat(200);
        let raw = mail().body(&text).html(&format!("<p>{text}</p>")).build();
        let id: i64 = ingest(&mut store, "1:1", &raw).message_id.parse().unwrap();
        let (method, bytes): (String, i64) = store
            .conn
            .query_row("SELECT method, bytes FROM bodies WHERE message_id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(method, "zstd");
        assert!(bytes < 600, "{bytes} bytes for {} of text and HTML", text.len() * 2);
        let body = store.body(id).unwrap().unwrap();
        assert_eq!(body.text.trim(), text.trim());
        assert_eq!(body.html.unwrap(), format!("<p>{text}</p>"));
    }

    #[test]
    fn a_body_in_an_unknown_method_or_broken_reads_as_missing() {
        let mut store = store_with_work();
        let id = add(&mut store, "1:1", now());
        store.conn.execute("UPDATE bodies SET method = 'brotli'", []).unwrap();
        assert_eq!(store.body(id).unwrap(), None);
        store.conn.execute("UPDATE bodies SET method = 'zstd', text = x'00'", []).unwrap();
        assert_eq!(store.body(id).unwrap(), None);
        assert_eq!(store.body(999).unwrap(), None);
        assert_eq!(unpack("zstd", &pack("hi")).as_deref(), Some("hi"));
    }

    #[test]
    fn the_offline_setting_decides_which_bodies_are_kept_and_files_never_by_default() {
        let n = now();
        assert!(Offline::Text.keeps_text(0, n));
        assert!(Offline::Recent.keeps_text(n - RECENT_MS, n));
        assert!(!Offline::Recent.keeps_text(n - RECENT_MS - 1, n));
        assert!(!Offline::Text.keeps_files() && !Offline::Recent.keeps_files());
        assert!(Offline::Everything.keeps_files());

        let mut store = store_with_work();
        assert_eq!(store.offline().unwrap(), Offline::Text);
        add(&mut store, "1:1", n - 200 * DAY);
        store.set_offline(Offline::Recent, n).unwrap();
        assert_eq!(store.offline().unwrap(), Offline::Recent);
        // Old mail from now on comes without its body; what was here becomes cache.
        add(&mut store, "1:2", n - 100 * DAY);
        add(&mut store, "1:3", n - DAY);
        assert_eq!(offline_flags(&store), [false, true]);
        store.set_offline(Offline::Everything, n).unwrap();
        assert_eq!(offline_flags(&store), [true, true]);
    }

    #[test]
    fn a_saved_setting_this_moss_cant_read_is_the_default() {
        let mut store = store_with_work();
        store.set_setting(OFFLINE_KEY, Some("\"floppy\"")).unwrap();
        store.set_setting(CACHE_LIMIT_KEY, Some("lots")).unwrap();
        assert_eq!(store.offline().unwrap(), Offline::Text);
        assert_eq!(store.cache_limit().unwrap(), DEFAULT_CACHE_LIMIT);
        store.set_cache_limit(250_000_000).unwrap();
        assert_eq!(store.cache_limit().unwrap(), 250_000_000);
    }

    #[test]
    fn the_cache_loses_its_oldest_first_and_offline_mail_stays() {
        let mut store = store_with_work();
        let n = now();
        let (a, b, c) = (add(&mut store, "1:1", n), add(&mut store, "1:2", n), add(&mut store, "1:3", n));
        let file = vec![7u8; 1000];
        {
            let tx = store.conn.transaction().unwrap();
            tx.execute("UPDATE bodies SET offline = 0, used_at = message_id", []).unwrap();
            tx.execute(
                "INSERT INTO attachments (message_id, position, name, content_type, bytes, inline) VALUES (?1, 0, 'a', 'x/y', 1000, 0)",
                [c],
            )
            .unwrap();
            keep_file(&tx, c, 0, &file, false).unwrap();
            tx.execute("UPDATE attachment_data SET used_at = 0", []).unwrap();
            tx.execute("UPDATE bodies SET offline = 1 WHERE message_id = ?1", [c]).unwrap();
            tx.commit().unwrap();
        }
        let body = |id| store.conn.query_row("SELECT bytes FROM bodies WHERE message_id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap() as u64;
        let (size_a, size_b) = (body(a), body(b));
        assert_eq!(store.usage().unwrap().cache, size_a + size_b + 1000);
        // Reading a counts as using it, so b goes first after the file.
        store.body(a).unwrap();
        assert_eq!(store.trim_cache(size_a + size_b).unwrap(), 1000);
        assert_eq!(store.trim_cache(size_a).unwrap(), size_b);
        assert!(store.body(a).unwrap().is_some() && store.body(b).unwrap().is_none());
        assert_eq!(store.clear_cache().unwrap(), size_a);
        assert_eq!(store.usage().unwrap().cache, 0);
        assert!(store.body(c).unwrap().is_some());
    }

    #[test]
    fn usage_counts_offline_mail_cache_and_the_index_apart() {
        let mut store = store_with_work();
        let before = store.usage().unwrap();
        assert_eq!((before.offline, before.cache), (0, 0));
        let id = add(&mut store, "1:1", now());
        let used = store.usage().unwrap();
        let bytes: i64 = store.conn.query_row("SELECT bytes FROM bodies WHERE message_id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(used.offline, bytes as u64);
        assert!(used.index > 0);
    }

    #[test]
    fn failed_writes_are_errors() {
        let mut store = store_with_work();
        add(&mut store, "1:1", now());
        read_only(&store);
        assert!(store.set_offline(Offline::Recent, now()).is_err());
        assert!(store.trim_cache(0).is_ok(), "nothing to remove is not a write");
        assert!(store.set_cache_limit(1).is_err());
        store.conn.pragma_update(None, "query_only", false).unwrap();
        store.conn.execute("UPDATE bodies SET offline = 0", []).unwrap();
        read_only(&store);
        assert!(store.trim_cache(0).is_err());
        assert!(store.body(1).is_err());
    }
}
