//! Files people sent: what kind each one is, and a search over them (for Ask Moss).

use serde::{Deserialize, Serialize};

use crate::{Attachment, Person};

/// What kind of file an attachment is, from its type and name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    Pdf,
    Image,
    Document,
    Spreadsheet,
    Slides,
    Archive,
    Other,
}

impl FileKind {
    pub const ALL: [FileKind; 7] = [
        FileKind::Pdf,
        FileKind::Image,
        FileKind::Document,
        FileKind::Spreadsheet,
        FileKind::Slides,
        FileKind::Archive,
        FileKind::Other,
    ];

    /// The kind of a file called `name` with MIME type `content_type`. The name's extension
    /// wins, since mail often says `application/octet-stream` for everything.
    pub fn of(name: &str, content_type: &str) -> FileKind {
        let name = name.to_ascii_lowercase();
        let by_name = name.rsplit_once('.').and_then(|(_, ext)| match ext {
            "pdf" => Some(FileKind::Pdf),
            "png" | "jpg" | "jpeg" | "gif" | "heic" | "webp" | "tif" | "tiff" | "bmp" | "svg" => Some(FileKind::Image),
            "doc" | "docx" | "pages" | "odt" | "rtf" | "txt" | "md" => Some(FileKind::Document),
            "xls" | "xlsx" | "numbers" | "ods" | "csv" | "tsv" => Some(FileKind::Spreadsheet),
            "ppt" | "pptx" | "key" | "odp" => Some(FileKind::Slides),
            "zip" | "rar" | "7z" | "gz" | "tgz" | "tar" => Some(FileKind::Archive),
            _ => None,
        });
        let t = content_type.to_ascii_lowercase();
        by_name.unwrap_or(if t == "application/pdf" {
            FileKind::Pdf
        } else if t.starts_with("image/") {
            FileKind::Image
        } else if t.contains("spreadsheet") || t.contains("excel") {
            FileKind::Spreadsheet
        } else if t.contains("presentation") || t.contains("powerpoint") {
            FileKind::Slides
        } else if t.contains("zip") {
            FileKind::Archive
        } else if t == "text/plain" || t.contains("word") {
            FileKind::Document
        } else {
            FileKind::Other
        })
    }

    /// "pdf", "image"…
    pub fn name(self) -> &'static str {
        match self {
            FileKind::Pdf => "pdf",
            FileKind::Image => "image",
            FileKind::Document => "document",
            FileKind::Spreadsheet => "spreadsheet",
            FileKind::Slides => "slides",
            FileKind::Archive => "archive",
            FileKind::Other => "other",
        }
    }

    /// The kind called `name`, as [`FileKind::name`] spells it, in any case.
    pub fn parse(name: &str) -> Option<FileKind> {
        let name = name.trim().to_ascii_lowercase();
        FileKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Which files to find. Every field left out matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileQuery {
    /// Part of the sender's name or address.
    pub from: Option<String>,
    /// Part of the file name.
    pub name: Option<String>,
    pub kind: Option<FileKind>,
    /// Sent at or after (ms).
    pub after: Option<i64>,
    /// Sent before (ms).
    pub before: Option<i64>,
    /// At most this many; 0 for all.
    pub limit: usize,
}

/// A file someone sent, and the email it came in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub from: Person,
    pub sent_at: i64,
    pub file: Attachment,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_says_what_kind_of_file_it_is_before_the_type_does() {
        let kind = |name, t| FileKind::of(name, t).name();
        assert_eq!(kind("contract.PDF", "application/octet-stream"), "pdf");
        assert_eq!(kind("scan", "application/pdf"), "pdf");
        assert_eq!(kind("photo.heic", "application/octet-stream"), "image");
        assert_eq!(kind("logo", "image/png"), "image");
        assert_eq!(kind("notes.docx", ""), "document");
        assert_eq!(kind("notes.pages", ""), "document");
        assert_eq!(kind("readme", "text/plain"), "document");
        assert_eq!(kind("letter", "application/msword"), "document");
        assert_eq!(kind("budget.xlsx", ""), "spreadsheet");
        assert_eq!(kind("data.csv", ""), "spreadsheet");
        assert_eq!(kind("sheet", "application/vnd.ms-excel"), "spreadsheet");
        assert_eq!(kind("deck.key", ""), "slides");
        assert_eq!(kind("deck", "application/vnd.ms-powerpoint"), "slides");
        assert_eq!(kind("deck", "application/vnd.openxmlformats-officedocument.presentationml.presentation"), "slides");
        assert_eq!(kind("q.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"), "spreadsheet");
        assert_eq!(kind("all.zip", ""), "archive");
        assert_eq!(kind("bundle", "application/zip"), "archive");
        assert_eq!(kind("invite.ics", "text/calendar"), "other");
        assert_eq!(kind("thing", "application/octet-stream"), "other");
    }

    #[test]
    fn kinds_are_read_back_by_name() {
        for kind in FileKind::ALL {
            assert_eq!(FileKind::parse(kind.name()), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.name());
        }
        assert_eq!(FileKind::parse(" PDF "), Some(FileKind::Pdf));
        assert_eq!(FileKind::parse("video"), None);
    }
}
