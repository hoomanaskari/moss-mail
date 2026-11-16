//! What Ask Moss reads beyond search: files people sent, by sender, name, kind and date, and
//! how much mail is on this device.

use moss_core::{Attachment, FileHit, FileKind, FileQuery, Person};
use rusqlite::{named_params, OptionalExtension, Row};

use crate::{Result, Store};

/// One row of [`Store::find_files`].
fn hit(r: &Row) -> rusqlite::Result<FileHit> {
    let bytes = r.get::<_, i64>(9)? as u64;
    Ok(FileHit {
        thread_id: r.get::<_, i64>(0)?.to_string(),
        message_id: r.get::<_, i64>(1)?.to_string(),
        subject: r.get(2)?,
        from: Person { name: r.get(3)?, email: r.get(4)? },
        sent_at: r.get(5)?,
        file: Attachment {
            index: r.get(6)?,
            name: r.get(7)?,
            size: moss_core::format_size(bytes),
            bytes,
            content_type: r.get(8)?,
            inline: false,
            text: None,
        },
    })
}

impl Store {
    /// Emails on this device, leaving out deleted and blocked mail: "2,431 emails on this Mac".
    pub fn mail_count(&self) -> Result<u64> {
        let sql = "SELECT count(*) FROM messages WHERE deleted_from IS NULL AND blocked = 0";
        Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
    }

    /// Attachment `index` of a message, with where it came from. `None` if there's no such file.
    pub fn file(&self, message_id: &str, index: u32) -> Result<Option<FileHit>> {
        let sql = "SELECT m.thread_id, m.id, m.subject, m.from_name, m.from_email, m.sent_at,
                          a.position, a.name, a.content_type, a.bytes
                   FROM attachments a JOIN messages m ON m.id = a.message_id
                   WHERE a.message_id = ?1 AND a.position = ?2";
        Ok(self.conn.query_row(sql, rusqlite::params![crate::codec::parse_id(message_id), index], hit).optional()?)
    }

    /// Files that match `query`, newest first. Leaves out images shown in the body, and deleted
    /// or blocked mail.
    pub fn find_files(&self, query: &FileQuery) -> Result<Vec<FileHit>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT m.thread_id, m.id, m.subject, m.from_name, m.from_email, m.sent_at,
                    a.position, a.name, a.content_type, a.bytes
             FROM attachments a JOIN messages m ON m.id = a.message_id
             WHERE a.inline = 0 AND m.deleted_from IS NULL AND m.blocked = 0
               AND (:from IS NULL OR instr(lower(m.from_name), lower(:from)) > 0 OR instr(lower(m.from_email), lower(:from)) > 0)
               AND (:name IS NULL OR instr(lower(a.name), lower(:name)) > 0)
               AND (:after IS NULL OR m.sent_at >= :after)
               AND (:before IS NULL OR m.sent_at < :before)
             ORDER BY m.sent_at DESC, a.message_id DESC, a.position",
        )?;
        let params = named_params! {":from": query.from, ":name": query.name, ":after": query.after, ":before": query.before};
        let rows = stmt.query_map(params, hit)?;
        let mut hits = Vec::new();
        for hit in rows {
            let hit = hit?;
            if query.kind.is_some_and(|k| FileKind::of(&hit.file.name, &hit.file.content_type) != k) {
                continue;
            }
            hits.push(hit);
            if hits.len() == query.limit {
                break;
            }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn names(store: &Store, query: FileQuery) -> Vec<String> {
        store
            .find_files(&FileQuery { limit: 10, ..query })
            .unwrap()
            .into_iter()
            .map(|f| f.file.name)
            .collect()
    }

    fn files() -> Store {
        let mut store = store_with_work();
        let budget = mail().id("a@x").at(NOV_1).attach("budget-q4.xlsx", "application/octet-stream", "1,2").inline(
            "logo.png",
            "image/png",
            "png",
        );
        ingest(&mut store, "1:1", &budget.build());
        let contract = mail()
            .id("b@x")
            .at(NOV_1 + 2 * DAY)
            .from("Legal Team <legal@lawfirm.example>")
            .subject("Signed")
            .attach("Contract-Signed.pdf", "application/pdf", "%PDF");
        ingest(&mut store, "1:2", &contract.build());
        store
    }

    #[test]
    fn finds_files_by_sender_name_kind_and_date_newest_first() {
        let store = files();
        assert_eq!(names(&store, FileQuery::default()), ["Contract-Signed.pdf", "budget-q4.xlsx"]);
        let from = |s: &str| FileQuery { from: Some(s.into()), ..Default::default() };
        assert_eq!(names(&store, from("LEGAL")), ["Contract-Signed.pdf"]);
        assert_eq!(names(&store, from("daniel@northwind")), ["budget-q4.xlsx"]);
        let name = FileQuery { name: Some("contract".into()), ..Default::default() };
        assert_eq!(names(&store, name), ["Contract-Signed.pdf"]);
        let kind = FileQuery { kind: Some(FileKind::Spreadsheet), ..Default::default() };
        assert_eq!(names(&store, kind), ["budget-q4.xlsx"]);
        let after = FileQuery { after: Some(NOV_1 + DAY), ..Default::default() };
        assert_eq!(names(&store, after), ["Contract-Signed.pdf"]);
        let before = FileQuery { before: Some(NOV_1 + DAY), ..Default::default() };
        assert_eq!(names(&store, before), ["budget-q4.xlsx"]);
        assert_eq!(names(&store, FileQuery { from: Some("nobody".into()), ..Default::default() }), Vec::<String>::new());
    }

    #[test]
    fn says_where_each_file_came_from() {
        let store = files();
        let hit = store.find_files(&FileQuery { limit: 1, ..Default::default() }).unwrap().remove(0);
        assert_eq!(hit.subject, "Signed");
        assert_eq!(hit.from, Person { name: "Legal Team".into(), email: "legal@lawfirm.example".into() });
        assert_eq!(hit.sent_at, NOV_1 + 2 * DAY);
        assert_eq!((hit.file.index, hit.file.content_type.as_str(), hit.file.inline), (0, "application/pdf", false));
        assert_eq!(store.thread(&hit.thread_id, NOV_1).unwrap().unwrap().messages[0].id, hit.message_id);
    }

    #[test]
    fn one_file_is_found_by_its_message_and_index() {
        let store = files();
        let newest = store.find_files(&FileQuery { limit: 1, ..Default::default() }).unwrap().remove(0);
        assert_eq!(store.file(&newest.message_id, 0).unwrap(), Some(newest.clone()));
        assert_eq!(store.file(&newest.message_id, 5).unwrap(), None);
        assert_eq!(store.file("nope", 0).unwrap(), None);
        break_schema(&store, "DROP TABLE attachments");
        assert!(store.file(&newest.message_id, 0).is_err());
    }

    #[test]
    fn the_limit_counts_files_of_the_kind_asked_for() {
        let store = files();
        let one = FileQuery { kind: Some(FileKind::Spreadsheet), limit: 1, ..Default::default() };
        assert_eq!(store.find_files(&one).unwrap().len(), 1);
        assert_eq!(store.find_files(&FileQuery { limit: 1, ..Default::default() }).unwrap().len(), 1);
    }

    #[test]
    fn leaves_out_deleted_and_blocked_mail() {
        let mut store = files();
        let thread = store.find_files(&FileQuery { limit: 1, ..Default::default() }).unwrap()[0].thread_id.clone();
        store.delete_thread(&thread).unwrap();
        assert_eq!(names(&store, FileQuery::default()), ["budget-q4.xlsx"]);
        store.restore_thread(&thread).unwrap();
        store.conn.execute("UPDATE messages SET blocked = 1", []).unwrap();
        assert_eq!(names(&store, FileQuery::default()), Vec::<String>::new());
    }

    #[test]
    fn a_broken_database_is_an_error() {
        let store = files();
        break_schema(&store, "DROP TABLE attachments");
        assert!(store.find_files(&FileQuery::default()).is_err());
        break_schema(&store, "ALTER TABLE messages RENAME TO gone");
        assert!(store.mail_count().is_err());
    }

    #[test]
    fn counts_the_mail_on_this_device_but_not_deleted_or_blocked_mail() {
        let store = files();
        assert_eq!(store.mail_count().unwrap(), 2);
        let thread = store.find_files(&FileQuery { limit: 1, ..Default::default() }).unwrap()[0].thread_id.clone();
        store.delete_thread(&thread).unwrap();
        assert_eq!(store.mail_count().unwrap(), 1);
        store.conn.execute("UPDATE messages SET blocked = 1", []).unwrap();
        assert_eq!(store.mail_count().unwrap(), 0);
    }
}
