//! What's kept of a message beyond its parsed fields, and where to fetch the rest.
//!
//! The server is the archive. For mail it holds, Moss keeps the headers, the details of
//! each attachment and where on the server its bytes are, and the body as the offline
//! setting says (see `bodies.rs`). File bytes are fetched from the server when someone
//! opens one, and kept as cache.
//!
//! The original is kept whole only where no server has a copy: Resend accounts (Resend
//! deletes received mail after about 30 days) and mail sent from here until the server's
//! Sent copy turns up.

use mail_parser::MessageParser;
use rusqlite::{params, OptionalExtension};

use crate::bodies;
use crate::codec::{now_ms, parse_id};
use crate::parse::{files, shown_in};
use crate::sync::Location;
use crate::{Result, Store};

/// Where to fetch an attachment's bytes from: the account, the IMAP section of the part
/// and the server copies of its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentSource {
    pub account_id: String,
    /// IMAP body section, like "2" or "1.2".
    pub part: String,
    pub locations: Vec<Location>,
}

/// `accounts.provider` for Resend, whose server deletes received mail after about 30 days.
pub(crate) const RESEND: &str = "resend";

/// A message's copies on the server, to fetch it (or part of it) from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCopies {
    pub account_id: String,
    /// The body is on this device. When it isn't, opening the message fetches it
    /// ([`Store::fill_in`]).
    pub has_body: bool,
    pub locations: Vec<Location>,
}

/// Whether an account's server keeps its mail.
pub(crate) fn server_keeps(provider: &str) -> bool {
    provider != RESEND
}

/// A location the server knows about, rather than a copy only this device has.
pub(crate) fn on_server(provider_id: &str) -> bool {
    !provider_id.starts_with("local:")
}

impl Store {
    /// The message exactly as the provider sent it, when this device keeps it (see the
    /// module notes). Otherwise the server has it.
    pub fn raw_message(&self, message_id: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT raw FROM message_raw WHERE message_id = ?1",
                [parse_id(message_id)],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The decoded bytes of attachment `index` (see `Attachment::index`), when they're on
    /// this device. `None` means: fetch them from the server ([`Store::attachment_source`]).
    pub fn attachment_content(&self, message_id: &str, index: u32) -> Result<Option<Vec<u8>>> {
        let id = parse_id(message_id);
        let kept: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT data FROM attachment_data WHERE message_id = ?1 AND position = ?2",
                params![id, index],
                |r| r.get(0),
            )
            .optional()?;
        if kept.is_some() {
            self.conn.execute(
                "UPDATE attachment_data SET used_at = ?3 WHERE message_id = ?1 AND position = ?2",
                params![id, index, now_ms()],
            )?;
            return Ok(kept);
        }
        let Some(raw) = self.raw_message(message_id)? else {
            return Ok(None);
        };
        let content = MessageParser::default()
            .parse(&raw)
            .and_then(|m| files(&m).nth(index as usize).map(|part| part.decoded().into_owned()));
        Ok(content)
    }

    /// Keeps the bytes of attachment `index`, fetched from the server to open it, so it
    /// opens at once next time: as cache, or offline when the setting keeps files.
    pub fn keep_attachment(&mut self, message_id: &str, index: u32, data: &[u8]) -> Result<()> {
        let offline = self.offline()?.keeps_files();
        let tx = self.conn.transaction()?;
        bodies::keep_file(&tx, parse_id(message_id), index.into(), data, offline)?;
        tx.commit()?;
        Ok(())
    }

    /// The inline images (with a Content-ID) of a message whose bytes aren't on this device,
    /// by attachment index, for when the message is opened. Inline means sent that way or
    /// shown by the HTML (see [`shown_in`]).
    pub fn missing_images(&self, message_id: &str) -> Result<Vec<u32>> {
        let id = parse_id(message_id);
        let html = self.body(id)?.and_then(|b| b.html).unwrap_or_default();
        let mut stmt = self.conn.prepare(
            "SELECT a.position, a.inline, a.content_id FROM attachments a WHERE a.message_id = ?1 AND a.content_id IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM attachment_data d WHERE d.message_id = a.message_id AND d.position = a.position)
             ORDER BY a.position",
        )?;
        let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get::<_, bool>(1)?, r.get::<_, String>(2)?)))?;
        let mut missing = Vec::new();
        for row in rows {
            let (position, inline, cid) = row?;
            if inline || shown_in(&html, &cid) {
                missing.push(position);
            }
        }
        Ok(missing)
    }

    /// Where attachment `index` can be fetched from the server. `None` for an unknown
    /// attachment, or one Moss doesn't know the section of.
    pub fn attachment_source(&self, message_id: &str, index: u32) -> Result<Option<AttachmentSource>> {
        let id = parse_id(message_id);
        let found: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT m.account_id, a.part FROM attachments a JOIN messages m ON m.id = a.message_id
                 WHERE a.message_id = ?1 AND a.position = ?2 AND a.part IS NOT NULL",
                params![id, index],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((account_id, part)) = found else {
            return Ok(None);
        };
        Ok(Some(AttachmentSource { account_id, part, locations: self.server_locations(id)? }))
    }

    /// The file name of attachment `index`, as the sender gave it.
    pub fn attachment_name(&self, message_id: &str, index: u32) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT name FROM attachments WHERE message_id = ?1 AND position = ?2",
                params![parse_id(message_id), index],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Attachment `index` of a message: its name, type and size. `None` if there's no such file.
    pub fn attachment(&self, message_id: &str, index: u32) -> Result<Option<moss_core::Attachment>> {
        Ok(self
            .conn
            .query_row(
                "SELECT name, content_type, bytes, inline FROM attachments WHERE message_id = ?1 AND position = ?2",
                params![parse_id(message_id), index],
                |r| {
                    let bytes = r.get::<_, i64>(2)? as u64;
                    Ok(moss_core::Attachment {
                        index,
                        name: r.get(0)?,
                        size: moss_core::format_size(bytes),
                        bytes,
                        content_type: r.get(1)?,
                        inline: r.get(3)?,
                        text: None,
                    })
                },
            )
            .optional()?)
    }

    /// Where on the server a message is. `None` for a message that isn't stored.
    pub fn server_copies(&self, message_id: &str) -> Result<Option<ServerCopies>> {
        let id = parse_id(message_id);
        let found: Option<(String, bool)> = self
            .conn
            .query_row(
                "SELECT account_id, EXISTS (SELECT 1 FROM bodies WHERE message_id = id) FROM messages WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((account_id, has_body)) = found else {
            return Ok(None);
        };
        Ok(Some(ServerCopies { account_id, has_body, locations: self.server_locations(id)? }))
    }

    /// The server copies of a message, to fetch it (or part of it) from.
    pub(crate) fn server_locations(&self, id: i64) -> Result<Vec<Location>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT b.name, l.provider_id FROM message_locations l JOIN mailboxes b ON b.id = l.mailbox_id
             WHERE l.message_id = ?1 AND l.provider_id NOT LIKE 'local:%' ORDER BY b.id, l.provider_id",
        )?;
        let rows = stmt.query_map([id], |r| Ok(Location { mailbox: r.get(0)?, provider_id: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::INLINE_KEEP_MAX;
    use crate::codec::name;
    use crate::test_support::*;
    use moss_core::{MessageSource, ProviderKind};

    #[test]
    fn resend_is_named_as_the_accounts_table_stores_it() {
        assert_eq!(name(&ProviderKind::Resend), RESEND);
    }

    fn with_files() -> Vec<u8> {
        mail()
            .html("<img src=\"cid:logo.png\">")
            .attach("Q4.pdf", "application/pdf", "PDFDATA")
            .inline("logo.png", "image/png", "PNG")
            .inline("photo.png", "image/png", &"P".repeat(INLINE_KEEP_MAX + 1))
            .build()
    }

    fn at(mailbox: &str, provider_id: &str) -> MessageSource {
        MessageSource { mailbox: mailbox.into(), ..source(provider_id) }
    }

    #[test]
    fn server_mail_keeps_attachment_details_but_no_file_bytes() {
        let mut store = store_with_work();
        let got = ingest(&mut store, "1:7", &with_files());
        assert_eq!(store.raw_message(&got.message_id).unwrap(), None);
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        let names: Vec<&str> =
            thread.messages[0].attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Q4.pdf", "logo.png", "photo.png"]);
        assert_eq!(thread.messages[0].attachments[0].bytes, 7);
        for index in 0..3 {
            assert_eq!(store.attachment_content(&got.message_id, index).unwrap(), None);
        }
        assert_eq!(store.attachment_content("999", 0).unwrap(), None);
        assert_eq!(store.raw_message("x").unwrap(), None);
    }

    #[test]
    fn with_everything_offline_small_inline_images_are_kept_too() {
        let mut store = store_with_work();
        store.set_offline(crate::Offline::Everything, NOV_1).unwrap();
        let got = ingest(&mut store, "1:7", &with_files());
        assert_eq!(store.attachment_content(&got.message_id, 1).unwrap().as_deref(), Some(&b"PNG"[..]));
        assert_eq!(store.usage().unwrap().cache, 0);
    }

    #[test]
    fn an_opened_attachment_is_kept_as_cache() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1:7", &with_files()).message_id;
        store.keep_attachment(&id, 0, b"PDFDATA").unwrap();
        assert_eq!(store.attachment_content(&id, 0).unwrap().as_deref(), Some(&b"PDFDATA"[..]));
        assert_eq!(store.usage().unwrap().cache, 7);
        read_only(&store);
        assert!(store.keep_attachment(&id, 0, b"PDFDATA").is_err());
        assert!(store.attachment_content(&id, 0).is_err(), "using it is a write");
    }

    #[test]
    fn attachments_say_where_on_the_server_they_are() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1:7", &with_files()).message_id;
        store.ingest(&with_files(), &at("Archive", "3:1")).unwrap();
        store.ingest(&with_files(), &at("Sent", "local:<q4@northwind.co>")).unwrap();
        let pdf = store.attachment_source(&id, 0).unwrap().unwrap();
        assert_eq!(pdf.account_id, "work");
        assert_eq!(pdf.part, "2");
        let at = |m: &str, p: &str| Location { mailbox: m.into(), provider_id: p.into() };
        assert_eq!(pdf.locations, [at("INBOX", "1:7"), at("Archive", "3:1")]);
        assert_eq!(store.attachment_source(&id, 1).unwrap().unwrap().part, "3");
        assert_eq!(store.attachment_source(&id, 9).unwrap(), None);
        assert_eq!(store.attachment_source("999", 0).unwrap(), None);
        let copies = store.server_copies(&id).unwrap().unwrap();
        assert_eq!((copies.account_id.as_str(), copies.has_body), ("work", true));
        assert_eq!(copies.locations, pdf.locations);
        assert_eq!(store.server_copies("999").unwrap(), None);
        assert_eq!(store.attachment_name(&id, 0).unwrap().as_deref(), Some("Q4.pdf"));
        assert_eq!(store.attachment_name(&id, 9).unwrap(), None);
    }

    #[test]
    fn an_attachment_has_a_name_type_and_size() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1:7", &with_files()).message_id;
        let pdf = store.attachment(&id, 0).unwrap().unwrap();
        assert_eq!((pdf.index, pdf.name.as_str(), pdf.content_type.as_str()), (0, "Q4.pdf", "application/pdf"));
        assert_eq!((pdf.bytes, pdf.size.as_str(), pdf.inline), (7, "7 B", false));
        assert_eq!(store.attachment(&id, 9).unwrap(), None);
        assert_eq!(store.attachment("999", 0).unwrap(), None);
    }

    #[test]
    fn a_body_fetched_on_open_fills_in_a_headers_only_message_with_its_images() {
        let mut store = store_with_work();
        let html = "<img src=\"cid:logo.png\"><img src=\"cid:photo.png\"><img src=\"cid:gone\">";
        let full = mail()
            .id("r@x")
            .html(html)
            .inline("logo.png", "image/png", "PNG")
            .inline("photo.png", "image/png", &"P".repeat(INLINE_KEEP_MAX + 1))
            .build();
        let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let headers = MessageSource { headers_only: true, ..at("Sent", "5:1") };
        let got = store.ingest(&full[..end], &headers).unwrap();
        let before = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(before.messages[0].headers_only);
        assert_eq!(store.server_copies(&got.message_id).unwrap().unwrap().has_body, false);
        assert!(store.fill_in(&got.message_id, &full).unwrap());
        assert!(!store.fill_in(&got.message_id, &full).unwrap(), "only once");
        assert!(!store.fill_in("999", &full).unwrap());
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(!thread.messages[0].headers_only);
        // Small inline images came with it, so they show; the big one is fetched.
        assert_eq!(
            thread.messages[0].body_html.as_deref(),
            Some("<img src=\"data:image/png;base64,UE5H\"><img src=\"cid:photo.png\"><img src=\"cid:gone\">")
        );
    }

    #[test]
    fn images_the_html_shows_are_missing_until_fetched_once_opened() {
        let mut store = store_with_work();
        let raw = mail().html("<img src=\"cid:logo.png\">").inline("logo.png", "image/png", "PNG").build();
        let got = ingest(&mut store, "1:7", &raw);
        let message = |store: &Store| store.thread(&got.thread_id, NOV_1).unwrap().unwrap().messages.remove(0);
        assert!(message(&store).images_missing);
        assert_eq!(store.missing_images(&got.message_id).unwrap(), [0]);
        store.keep_attachment(&got.message_id, 0, b"PNG").unwrap();
        assert!(!message(&store).images_missing);
        assert!(store.missing_images(&got.message_id).unwrap().is_empty());
        assert_eq!(message(&store).body_html.as_deref(), Some("<img src=\"data:image/png;base64,UE5H\">"));
        // A file that isn't shown in the HTML isn't missing.
        let pdf = ingest(&mut store, "1:8", &mail().id("p@x").attach("Q4.pdf", "application/pdf", "PDF").build());
        assert!(!store.thread(&pdf.thread_id, NOV_1).unwrap().unwrap().messages[0].images_missing);
    }

    #[test]
    fn images_the_html_shows_are_part_of_the_body_even_if_stored_as_files() {
        // Stored before Moss looked at the HTML: the logos it shows were listed as files.
        let mut store = store_with_work();
        let raw = mail()
            .html("<img src=\"cid:logo.png\">")
            .attach("logo.png", "image/png", "PNG")
            .attach("Q4.pdf", "application/pdf", "PDF")
            .build();
        let got = ingest(&mut store, "1:7", &raw);
        store.conn.execute("UPDATE attachments SET inline = 0", []).unwrap();
        store.conn.execute("DELETE FROM attachment_data", []).unwrap();
        let m = store.thread(&got.thread_id, NOV_1).unwrap().unwrap().messages.remove(0);
        let inline: Vec<bool> = m.attachments.iter().map(|a| a.inline).collect();
        assert_eq!(inline, [true, false]);
        assert!(m.images_missing);
        assert_eq!(store.missing_images(&got.message_id).unwrap(), [0]);
    }

    #[test]
    fn a_headers_only_message_filled_in_is_kept_offline_with_its_files_when_the_setting_keeps_them() {
        let mut store = store_with_work();
        store.set_offline(crate::Offline::Everything, crate::codec::now_ms()).unwrap();
        let full = mail().id("r@x").html("<img src=\"cid:logo.png\">").inline("logo.png", "image/png", "PNG").build();
        let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let got = store.ingest(&full[..end], &MessageSource { headers_only: true, ..at("Sent", "5:1") }).unwrap();
        assert_eq!(store.usage().unwrap().offline, 0);
        assert!(store.fill_in(&got.message_id, &full).unwrap());
        let usage = store.usage().unwrap();
        assert!(usage.offline > 0, "{usage:?}");
        assert_eq!(usage.cache, 0, "kept as offline mail, not as cache");
        assert!(store.missing_images(&got.message_id).unwrap().is_empty(), "its image came with it");
    }

    #[test]
    fn an_old_headers_only_message_filled_in_is_cache_when_the_setting_keeps_recent_mail() {
        let mut store = store_with_work();
        store.set_offline(crate::Offline::Recent, crate::codec::now_ms()).unwrap();
        let full = mail().id("old@x").at(NOV_1 - 400 * DAY).html("<p>Old</p>").build();
        let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let got = store.ingest(&full[..end], &MessageSource { headers_only: true, ..at("Sent", "5:2") }).unwrap();
        assert!(store.fill_in(&got.message_id, &full).unwrap());
        let usage = store.usage().unwrap();
        assert_eq!(usage.offline, 0, "too old to keep offline: {usage:?}");
        assert!(usage.cache > 0, "{usage:?}");
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.messages[0].body_html.as_deref(), Some("<p>Old</p>"));
    }

    #[test]
    fn a_body_the_offline_setting_doesnt_keep_is_cache_once_opened() {
        let mut store = store_with_work();
        store.set_offline(crate::Offline::Recent, crate::codec::now_ms()).unwrap();
        let old = mail().id("old@x").at(NOV_1 - 400 * DAY).html("<p>Old</p>").build();
        let got = ingest(&mut store, "1:1", &old);
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(thread.messages[0].headers_only, "not here, so the UI fetches it");
        assert_eq!(thread.messages[0].paragraphs, ["Hi Maya, Can you sign off the Q4 budget? Daniel"]);
        assert!(store.fill_in(&got.message_id, &old).unwrap());
        assert!(store.usage().unwrap().cache > 0);
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.messages[0].body_html.as_deref(), Some("<p>Old</p>"));
    }

    #[test]
    fn a_damaged_compressed_body_reads_as_not_here() {
        let mut store = store_with_work();
        let got = ingest(&mut store, "1:7", &mail().html("<p>Hi</p>").build());
        break_schema(&store, "UPDATE bodies SET text = x'00ff', html = x'00ff'");
        let thread = store.thread(&got.thread_id, NOV_1).unwrap().unwrap();
        assert!(thread.messages[0].headers_only);
        assert_eq!(thread.messages[0].body_html, None);
    }

    #[test]
    fn a_broken_image_table_is_an_error_when_reading_a_thread() {
        let mut store = store_with_work();
        let got = ingest(&mut store, "1:7", &with_files());
        break_schema(&store, "ALTER TABLE attachment_data RENAME COLUMN data TO bytes");
        assert!(store.thread(&got.thread_id, NOV_1).is_err());
        break_schema(&store, "ALTER TABLE attachments RENAME COLUMN content_id TO cid");
        assert!(store.missing_images(&got.message_id).is_err());
    }

    #[test]
    fn a_broken_locations_table_is_an_error() {
        let mut store = store_with_work();
        let id = ingest(&mut store, "1:7", &with_files()).message_id;
        break_schema(&store, "ALTER TABLE message_locations RENAME COLUMN provider_id TO uid");
        assert!(store.server_copies(&id).is_err());
        assert!(store.attachment_source(&id, 0).is_err());
    }

    #[test]
    fn a_resend_account_keeps_the_whole_message() {
        let mut store = Store::open_in_memory().unwrap();
        let resend = moss_core::Account {
            provider: ProviderKind::Resend,
            ..account("work", "maya@northwind.co")
        };
        store.upsert_account(&resend).unwrap();
        let id = ingest(&mut store, "re_1", &with_files()).message_id;
        assert_eq!(store.raw_message(&id).unwrap(), Some(with_files()));
        assert_eq!(store.attachment_content(&id, 0).unwrap().as_deref(), Some(&b"PDFDATA"[..]));
        assert_eq!(store.attachment_content(&id, 2).unwrap().map(|b| b.len()), Some(INLINE_KEEP_MAX + 1));
        // Another mailbox of the same account changes nothing: Resend still forgets it.
        store.ingest(&with_files(), &at("SENT", "re_2")).unwrap();
        assert!(store.raw_message(&id).unwrap().is_some());
    }

    #[test]
    fn mail_sent_from_here_is_kept_until_the_server_has_a_copy() {
        let mut store = store_with_work();
        let sent = mail().from("maya@northwind.co").attach("Q4.pdf", "application/pdf", "PDFDATA");
        let id = store.ingest(&sent.build(), &at("Sent", "local:<q4@northwind.co>")).unwrap().message_id;
        assert_eq!(store.attachment_content(&id, 0).unwrap().as_deref(), Some(&b"PDFDATA"[..]));
        assert_eq!(store.attachment_source(&id, 0).unwrap().unwrap().locations, []);
        store.ingest(&sent.build(), &at("Sent", "4:12")).unwrap();
        assert_eq!(store.raw_message(&id).unwrap(), None);
        assert_eq!(store.attachment_content(&id, 0).unwrap(), None);
        assert_eq!(store.attachment_source(&id, 0).unwrap().unwrap().part, "2");
    }
}
