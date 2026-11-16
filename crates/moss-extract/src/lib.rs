//! Reads the text inside attachments, on this Mac, so search and Ask Moss can use it.
//!
//! PDFs and pictures go to the operating system ([`Native`]: PDFKit and Vision on macOS);
//! Word, Excel and PowerPoint files are read from their XML here ([`office`]); text files,
//! CSV and calendar invitations are text already. Nothing leaves the Mac.

#[cfg(target_os = "macos")]
pub mod mac;
pub mod office;

/// What only the operating system can read. [`mac::Mac`] on macOS; tests pass a fake.
pub trait Native: Send + Sync {
    /// The text layer of a PDF. `None` if it isn't a PDF this Mac can open.
    fn pdf_text(&self, bytes: &[u8]) -> Option<String>;
    /// The words on a PDF's pages, read from how they look: for scans.
    fn pdf_ocr(&self, bytes: &[u8]) -> Option<String>;
    /// The words in a picture: a photo of a letter, a screenshot.
    fn image_ocr(&self, bytes: &[u8]) -> Option<String>;
}

/// The kinds of file Moss reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    /// A picture: read by text recognition.
    Image,
    Word,
    Sheet,
    Slides,
    /// Text already: .txt, .csv, .ics and the like.
    Text,
}

/// Most characters kept of one file: about a long report (30 pages). Ask Moss reads less.
pub const MAX_CHARS: usize = 100_000;
/// Matches the engine's attachment-reading limit, before invoking native parsers.
pub const MAX_INPUT_BYTES: usize = 25 * 1024 * 1024;

/// A PDF with fewer letters than this in its text layer is a scan: its pages are read by
/// how they look.
const SCAN_LETTERS: usize = 40;

const IMAGES: &[&str] = &["jpg", "jpeg", "png", "heic", "heif", "tif", "tiff", "bmp", "gif", "webp"];
const TEXTS: &[&str] = &["txt", "text", "csv", "tsv", "ics", "vcs", "md", "markdown", "log", "json", "xml", "vcf"];

/// What kind of file this is, by its extension, or its content type when the name has none.
/// `None` for files Moss doesn't read (videos, archives, old .doc files…).
pub fn kind(name: &str, content_type: &str) -> Option<Kind> {
    let extension = name.rfind('.').filter(|&dot| dot > 0 && dot < name.len() - 1).map(|dot| name[dot + 1..].to_ascii_lowercase());
    if let Some(ext) = extension {
        return match ext.as_str() {
            "pdf" => Some(Kind::Pdf),
            "docx" => Some(Kind::Word),
            "xlsx" => Some(Kind::Sheet),
            "pptx" => Some(Kind::Slides),
            e if IMAGES.contains(&e) => Some(Kind::Image),
            e if TEXTS.contains(&e) => Some(Kind::Text),
            _ => None,
        };
    }
    let essence = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    match essence.as_str() {
        "application/pdf" => Some(Kind::Pdf),
        "text/calendar" => Some(Kind::Text),
        e if e.starts_with("image/") && e != "image/svg+xml" => Some(Kind::Image),
        e if e.starts_with("text/") && e != "text/html" => Some(Kind::Text),
        _ => None,
    }
}

/// Whether Moss reads files like this one.
pub fn readable(name: &str, content_type: &str) -> bool {
    kind(name, content_type).is_some()
}

/// The text in a file, tidied: no blank runs, at most [`MAX_CHARS`]. `None` when Moss
/// doesn't read this kind of file, or the file is broken. An empty string when it has no
/// words (a photo of a beach).
pub fn extract(bytes: &[u8], name: &str, content_type: &str, native: &dyn Native) -> Option<String> {
    if bytes.len() > MAX_INPUT_BYTES { return None; }
    let text = match kind(name, content_type)? {
        Kind::Pdf => {
            let layer = native.pdf_text(bytes)?;
            if letters(&layer) >= SCAN_LETTERS {
                layer
            } else {
                native.pdf_ocr(bytes).filter(|scan| letters(scan) > letters(&layer)).unwrap_or(layer)
            }
        }
        Kind::Image => native.image_ocr(bytes)?,
        Kind::Word => office::word(bytes)?,
        Kind::Sheet => office::sheet(bytes)?,
        Kind::Slides => office::slides(bytes)?,
        Kind::Text => String::from_utf8_lossy(bytes).into_owned(),
    };
    Some(tidy(&text))
}

/// Collects native pages with one aggregate character budget. No later page is asked for
/// once the budget is full; temporary native objects can be released between callbacks.
#[cfg(any(target_os = "macos", test))]
fn limited_pages(pages: usize, mut read: impl FnMut(usize) -> Option<String>) -> String {
    let mut out = String::new();
    let mut used = 0;
    for index in 0..pages {
        if used == MAX_CHARS { break; }
        let Some(text) = read(index).filter(|text| !text.is_empty()) else { continue; };
        let separator = if out.is_empty() { "" } else { "\n\n" };
        for ch in separator.chars().chain(text.chars()).take(MAX_CHARS - used) {
            out.push(ch);
            used += 1;
        }
    }
    out
}

fn letters(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphanumeric()).count()
}

/// Lines trimmed, runs of blank lines made one, without the marks PDFs leave where a picture
/// was (U+FFFC), cut to [`MAX_CHARS`].
pub fn tidy(text: &str) -> String {
    let text = text.replace('\u{fffc}', "");
    let mut out = String::new();
    let mut blank = false;
    for line in text.lines().map(|l| l.trim_end_matches(['\r', ' ', '\t']).trim_start_matches(['\u{feff}'])) {
        if line.trim().is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank { "\n\n" } else { "\n" });
        }
        blank = false;
        out.push_str(line);
    }
    match out.char_indices().nth(MAX_CHARS) {
        Some((at, _)) => out[..at].to_string(),
        None => out,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;

    /// A one-page PDF with each of `lines` written on it in Helvetica.
    pub fn pdf(lines: &[&str]) -> Vec<u8> {
        let mut stream = String::from("BT /F1 24 Tf 72 720 Td 30 TL\n");
        for line in lines {
            stream.push_str(&format!("({}) Tj T*\n", line.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)")));
        }
        stream.push_str("ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{object}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        out
    }

    /// A zip of `files` (name, contents), like an Office file.
    pub fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, contents) in files {
            out.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(contents.as_bytes()).unwrap();
        }
        out.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Says what it was asked to read, and gives back what it was told to.
    #[derive(Default)]
    struct Fake {
        layer: Option<String>,
        scan: Option<String>,
        photo: Option<String>,
        asked: Mutex<Vec<&'static str>>,
    }

    impl Native for Fake {
        fn pdf_text(&self, _: &[u8]) -> Option<String> {
            self.asked.lock().unwrap().push("pdf_text");
            self.layer.clone()
        }
        fn pdf_ocr(&self, _: &[u8]) -> Option<String> {
            self.asked.lock().unwrap().push("pdf_ocr");
            self.scan.clone()
        }
        fn image_ocr(&self, _: &[u8]) -> Option<String> {
            self.asked.lock().unwrap().push("image_ocr");
            self.photo.clone()
        }
    }

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn page_output_budget_stops_work_and_preserves_unicode() {
        let mut calls = 0;
        let result = limited_pages(20, |_| { calls += 1; Some("é".repeat(MAX_CHARS + 1)) });
        assert_eq!(calls, 1);
        assert_eq!(result.chars().count(), MAX_CHARS);
        assert!(result.chars().all(|c| c == 'é'));
        assert_eq!(limited_pages(4, |i| [None, Some(""), Some("A"), Some("B")][i].map(String::from)), "A\n\nB");
        let result = limited_pages(2, |i| Some(if i == 0 { "x".repeat(MAX_CHARS - 1) } else { "must not fit".into() }));
        assert_eq!(result.chars().count(), MAX_CHARS);
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn oversized_inputs_never_reach_native_parsers() {
        let fake = Fake::default();
        let bytes = vec![b'x'; 25 * 1024 * 1024 + 1];
        for name in ["large.pdf", "large.png", "large.txt"] {
            assert!(extract(&bytes, name, "", &fake).is_none());
        }
        assert!(fake.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn knows_files_by_extension_then_by_type() {
        assert_eq!(kind("Invitation.PDF", ""), Some(Kind::Pdf));
        assert_eq!(kind("scan.heic", "application/octet-stream"), Some(Kind::Image));
        assert_eq!(kind("report.docx", ""), Some(Kind::Word));
        assert_eq!(kind("budget.xlsx", ""), Some(Kind::Sheet));
        assert_eq!(kind("deck.pptx", ""), Some(Kind::Slides));
        assert_eq!(kind("invite.ics", ""), Some(Kind::Text));
        assert_eq!(kind("notes.csv", ""), Some(Kind::Text));
        assert_eq!(kind("old.doc", "application/msword"), None);
        assert_eq!(kind("clip.mov", "video/quicktime"), None);
        // No extension: the content type says.
        assert_eq!(kind("P2026-0457", "application/pdf; name=x"), Some(Kind::Pdf));
        assert_eq!(kind("invite", "text/calendar"), Some(Kind::Text));
        assert_eq!(kind("photo", "image/jpeg"), Some(Kind::Image));
        assert_eq!(kind("readme", "text/plain"), Some(Kind::Text));
        assert_eq!(kind("logo", "image/svg+xml"), None);
        assert_eq!(kind("page", "text/html"), None);
        assert_eq!(kind("blob", "application/octet-stream"), None);
        assert_eq!(kind(".pdf", "application/pdf"), Some(Kind::Pdf), "a name that's all extension has none");
        assert_eq!(kind("trailing.", ""), None);
        assert!(readable("a.pdf", "") && !readable("a.zip", ""));
    }

    #[test]
    fn a_pdf_with_text_is_read_from_its_text_layer() {
        let fake = Fake { layer: some("Resident Parking Permit Renewal on December 12, 2026"), ..Default::default() };
        assert_eq!(extract(b"%PDF", "invite.pdf", "", &fake).unwrap(), "Resident Parking Permit Renewal on December 12, 2026");
        assert_eq!(*fake.asked.lock().unwrap(), ["pdf_text"]);
    }

    #[test]
    fn a_scanned_pdf_is_read_by_how_it_looks() {
        let fake = Fake { layer: some("  \n 1 "), scan: some("Notice of hearing\nRoom 4B"), ..Default::default() };
        assert_eq!(extract(b"%PDF", "scan.pdf", "", &fake).unwrap(), "Notice of hearing\nRoom 4B");
        assert_eq!(*fake.asked.lock().unwrap(), ["pdf_text", "pdf_ocr"]);
        // Reading it finds less than the text layer had: the layer stays.
        let fake = Fake { layer: some("Page 1"), scan: some("P1"), ..Default::default() };
        assert_eq!(extract(b"%PDF", "scan.pdf", "", &fake).unwrap(), "Page 1");
        let fake = Fake { layer: some(""), ..Default::default() };
        assert_eq!(extract(b"%PDF", "scan.pdf", "", &fake).unwrap(), "");
        // Not a PDF after all.
        assert_eq!(extract(b"junk", "scan.pdf", "", &Fake::default()), None);
    }

    #[test]
    fn pictures_are_read_by_text_recognition() {
        let fake = Fake { photo: some("PARKING FINE\n$75"), ..Default::default() };
        assert_eq!(extract(b"jpg", "IMG_0042.jpg", "", &fake).unwrap(), "PARKING FINE\n$75");
        assert_eq!(*fake.asked.lock().unwrap(), ["image_ocr"]);
        assert_eq!(extract(b"jpg", "IMG_0042.jpg", "", &Fake::default()), None);
    }

    #[test]
    fn text_files_are_text_already() {
        let ics = "BEGIN:VCALENDAR\r\nSUMMARY:Appointment\r\n\r\n\r\nDTSTART:20261212T150000Z\r\nEND:VCALENDAR\r\n";
        assert_eq!(
            extract(ics.as_bytes(), "invite.ics", "", &Fake::default()).unwrap(),
            "BEGIN:VCALENDAR\nSUMMARY:Appointment\n\nDTSTART:20261212T150000Z\nEND:VCALENDAR"
        );
        assert_eq!(extract(b"a,b\xff", "x.csv", "", &Fake::default()).unwrap(), "a,b\u{fffd}");
        assert_eq!(extract(b"x", "x.zip", "", &Fake::default()), None);
    }

    #[test]
    fn office_files_are_read_from_their_xml() {
        let docx = testing::zip(&[("word/document.xml", r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Offer letter</w:t></w:r></w:p></w:body></w:document>"#)]);
        assert_eq!(extract(&docx, "offer.docx", "", &Fake::default()).unwrap(), "Offer letter");
        let xlsx = testing::zip(&[("xl/worksheets/sheet1.xml", r#"<worksheet><sheetData><row><c t="inlineStr"><is><t>Q4</t></is></c></row></sheetData></worksheet>"#)]);
        assert_eq!(extract(&xlsx, "budget.xlsx", "", &Fake::default()).unwrap(), "Q4");
        let pptx = testing::zip(&[("ppt/slides/slide1.xml", r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Roadmap</a:t></a:r></a:p></p:sld>"#)]);
        assert_eq!(extract(&pptx, "deck.pptx", "", &Fake::default()).unwrap(), "Roadmap");
        assert_eq!(extract(b"not a zip", "offer.docx", "", &Fake::default()), None);
    }

    #[test]
    fn tidying_keeps_lines_drops_blank_runs_and_cuts_long_text() {
        assert_eq!(tidy("\u{feff}Dear Maya,  \r\n\n\n\t\nYour appointment\t\nis on Friday.\n\n"), "Dear Maya,\n\nYour appointment\nis on Friday.");
        assert_eq!(tidy(""), "");
        assert_eq!(tidy("Checklist\n\u{fffc}\nDate: 2026-11-05"), "Checklist\n\nDate: 2026-11-05", "a picture's mark goes");
        let long = "é".repeat(MAX_CHARS + 10);
        assert_eq!(tidy(&long).chars().count(), MAX_CHARS);
    }
}
