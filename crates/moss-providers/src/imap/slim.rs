//! Syncing big messages without their attachments.
//!
//! Sync first asks for each message's size and structure (BODYSTRUCTURE). Messages up to
//! [`WHOLE_BELOW`] bytes, which is most mail, then come whole in one command. For a bigger
//! one, another command fetches its header and only the parts Moss shows (text,
//! HTML, calendar invites, small inline images), and [`rebuild`] puts the message back
//! together with the same structure, so part numbers still match the server's. Each part
//! left out keeps its MIME headers, an empty body and an [`OMITTED`] header with its size;
//! its bytes are fetched from the server if someone opens it.

use async_imap::imap_proto::{
    BodyContentCommon, BodyContentSinglePart, BodyParams, BodyStructure, ContentEncoding,
};

/// Messages up to this size come whole.
pub(crate) const WHOLE_BELOW: u32 = 128 * 1024;

/// On a part left out of a rebuilt message: its decoded size in bytes.
pub const OMITTED: &str = "X-Moss-Omitted";

/// Inline images up to this size are fetched, so HTML mail shows them offline. The store
/// keeps images up to the same size (`moss_store`'s `INLINE_KEEP_MAX`).
const INLINE_IMAGE_MAX: u32 = 64 * 1024;

/// The part numbers to fetch to rebuild a message, or `None` when it should come whole
/// (an attached email, or a structure Moss can't put back together).
pub(crate) fn wanted(structure: &BodyStructure) -> Option<Vec<Vec<u32>>> {
    let mut out = Vec::new();
    collect(structure, &mut Vec::new(), &mut out)?;
    Some(out)
}

fn collect(s: &BodyStructure, path: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) -> Option<()> {
    match s {
        BodyStructure::Multipart { common, bodies, .. } => {
            boundary(common)?;
            for (i, body) in bodies.iter().enumerate() {
                path.push(i as u32 + 1);
                collect(body, path, out)?;
                path.pop();
            }
        }
        BodyStructure::Message { .. } => return None,
        BodyStructure::Text { common, other, .. } | BodyStructure::Basic { common, other, .. } => {
            if shown(common, other) {
                out.push(if path.is_empty() { vec![1] } else { path.clone() });
            }
        }
    }
    Some(())
}

/// A part Moss shows: text that isn't a file, or a small inline image.
fn shown(common: &BodyContentCommon, other: &BodyContentSinglePart) -> bool {
    let file = common.disposition.as_ref().is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
    let ty = &common.ty.ty;
    if ty.eq_ignore_ascii_case("text") {
        return !file;
    }
    ty.eq_ignore_ascii_case("image") && !file && other.id.is_some() && decoded_size(other) <= INLINE_IMAGE_MAX
}

fn boundary<'a>(common: &'a BodyContentCommon) -> Option<&'a str> {
    let params = common.ty.params.as_ref()?;
    params.iter().find(|(k, _)| k.eq_ignore_ascii_case("boundary")).map(|(_, v)| v.as_ref())
}

/// About how big a part is once decoded: base64 takes 78 bytes a line for 57.
fn decoded_size(other: &BodyContentSinglePart) -> u32 {
    match other.transfer_encoding {
        ContentEncoding::Base64 => (other.octets as u64 * 57 / 78) as u32,
        _ => other.octets,
    }
}

/// The message rebuilt from its header block and the parts fetched (by part number).
/// `None` if a wanted part is missing.
pub(crate) fn rebuild(
    header: &[u8],
    structure: &BodyStructure,
    parts: &dyn Fn(&[u32]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let mut out = crate::parts::header_block(header);
    match structure {
        BodyStructure::Multipart { .. } => write_children(&mut out, structure, &mut Vec::new(), parts)?,
        BodyStructure::Text { common, other, .. } | BodyStructure::Basic { common, other, .. } => {
            if shown(common, other) {
                out.extend(parts(&[1])?);
            } else {
                // The part's headers are the message's: its size goes before the blank line.
                out.truncate(out.len() - 2);
                out.extend(format!("{OMITTED}: {}\r\n\r\n", decoded_size(other)).bytes());
            }
        }
        BodyStructure::Message { .. } => return None,
    }
    Some(out)
}

/// The parts of a multipart, between its boundaries.
fn write_children(
    out: &mut Vec<u8>,
    s: &BodyStructure,
    path: &mut Vec<u32>,
    parts: &dyn Fn(&[u32]) -> Option<Vec<u8>>,
) -> Option<()> {
    let BodyStructure::Multipart { common, bodies, .. } = s else { return None };
    let boundary = boundary(common)?;
    for (i, body) in bodies.iter().enumerate() {
        path.push(i as u32 + 1);
        out.extend(format!("--{boundary}\r\n").bytes());
        match body {
            BodyStructure::Multipart { common, .. } => {
                let ty = &common.ty;
                out.extend(format!("Content-Type: {}/{}{}\r\n\r\n", ty.ty, ty.subtype, params(&ty.params)).bytes());
                write_children(out, body, path, parts)?;
            }
            BodyStructure::Text { common, other, .. } | BodyStructure::Basic { common, other, .. } => {
                out.extend(part_headers(common, other).bytes());
                if shown(common, other) {
                    out.extend(b"\r\n");
                    out.extend(parts(path)?);
                } else {
                    out.extend(format!("{OMITTED}: {}\r\n\r\n", decoded_size(other)).bytes());
                }
            }
            BodyStructure::Message { .. } => return None,
        }
        out.extend(b"\r\n");
        path.pop();
    }
    out.extend(format!("--{boundary}--\r\n").bytes());
    Some(())
}

/// A leaf part's MIME headers, from what BODYSTRUCTURE says about it.
fn part_headers(common: &BodyContentCommon, other: &BodyContentSinglePart) -> String {
    let ty = &common.ty;
    let mut out = format!("Content-Type: {}/{}{}\r\n", ty.ty, ty.subtype, params(&ty.params));
    let encoding = match &other.transfer_encoding {
        ContentEncoding::SevenBit => "7bit",
        ContentEncoding::EightBit => "8bit",
        ContentEncoding::Binary => "binary",
        ContentEncoding::Base64 => "base64",
        ContentEncoding::QuotedPrintable => "quoted-printable",
        ContentEncoding::Other(other) => other,
    };
    out += &format!("Content-Transfer-Encoding: {encoding}\r\n");
    if let Some(id) = &other.id {
        out += &format!("Content-ID: {id}\r\n");
    }
    if let Some(d) = &common.disposition {
        out += &format!("Content-Disposition: {}{}\r\n", d.ty, params(&d.params));
    }
    out
}

/// `; name="value"` for each parameter. Values come as IMAP quoted them, `\"` and all, which
/// is how MIME quotes them too. RFC 2231 ones (`name*=utf-8''…`) go unquoted.
fn params(params: &BodyParams) -> String {
    let mut out = String::new();
    for (k, v) in params.iter().flatten() {
        if k.ends_with('*') {
            out += &format!("; {k}={v}");
        } else {
            out += &format!("; {k}=\"{v}\"");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_imap::imap_proto::{AttributeValue, Response};
    use mail_parser::MessageParser;

    /// The BODYSTRUCTURE in `text`, as a server would send it.
    fn structure(text: &str) -> BodyStructure<'static> {
        let line = format!("* 1 FETCH (BODYSTRUCTURE {text})\r\n");
        let (_, response) = Response::parse(line.as_bytes()).unwrap();
        let Response::Fetch(_, mut attributes) = response else { panic!("not a fetch") };
        let Some(AttributeValue::BodyStructure(s)) = attributes.pop() else { panic!("no structure") };
        s.into_owned()
    }

    pub(crate) const MIXED: &str = r#"((("text" "plain" ("charset" "utf-8") NIL NIL "7bit" 12 1 NIL NIL NIL NIL)("text" "html" ("charset" "utf-8") NIL NIL "quoted-printable" 20 1 NIL NIL NIL NIL) "alternative" ("boundary" "a") NIL NIL NIL)("image" "png" NIL "<logo>" NIL "base64" 8 NIL ("inline" NIL) NIL NIL)("image" "jpeg" ("name" "big.jpg") "<photo>" NIL "base64" 400000 NIL ("inline" NIL) NIL NIL)("application" "pdf" ("name" "Q4 \"final\".pdf") NIL NIL "base64" 780000 NIL ("attachment" ("filename" "Q4 \"final\".pdf")) NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;

    #[test]
    fn wants_the_text_html_and_small_inline_images() {
        assert_eq!(wanted(&structure(MIXED)), Some(vec![vec![1, 1], vec![1, 2], vec![2]]));
        let single = r#"("text" "plain" ("charset" "utf-8") NIL NIL "7bit" 12 1 NIL NIL NIL NIL)"#;
        assert_eq!(wanted(&structure(single)), Some(vec![vec![1]]));
        let whole_pdf = r#"("application" "pdf" NIL NIL NIL "base64" 780000 NIL ("attachment" NIL) NIL NIL)"#;
        assert_eq!(wanted(&structure(whole_pdf)), Some(vec![]));
        let invite = r#"(("text" "plain" NIL NIL NIL "7bit" 12 1 NIL NIL NIL NIL)("text" "calendar" ("method" "REQUEST") NIL NIL "7bit" 900 20 NIL NIL NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        assert_eq!(wanted(&structure(invite)), Some(vec![vec![1], vec![2]]));
        // A text file sent as an attachment stays on the server.
        let text_file = r#"(("text" "plain" NIL NIL NIL "7bit" 12 1 NIL NIL NIL NIL)("text" "plain" ("name" "log.txt") NIL NIL "7bit" 900000 20 NIL ("attachment" ("filename" "log.txt")) NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        assert_eq!(wanted(&structure(text_file)), Some(vec![vec![1]]));
    }

    #[test]
    fn some_messages_come_whole() {
        let forwarded = r#"(("text" "plain" NIL NIL NIL "7bit" 12 1 NIL NIL NIL NIL)("message" "rfc822" NIL NIL NIL "7bit" 300 ("Mon, 2 Nov 2026 09:00:00 +0000" "Hi" NIL NIL NIL NIL NIL NIL NIL NIL) ("text" "plain" NIL NIL NIL "7bit" 2 1 NIL NIL NIL NIL) 8 NIL NIL NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        assert_eq!(wanted(&structure(forwarded)), None);
        let no_boundary = r#"(("text" "plain" NIL NIL NIL "7bit" 12 1 NIL NIL NIL NIL) "mixed" NIL NIL NIL NIL)"#;
        assert_eq!(wanted(&structure(no_boundary)), None);
        // Nor can they be rebuilt, if asked.
        assert_eq!(rebuild(HEADER, &structure(forwarded), &parts), None);
        let alone = r#"("message" "rfc822" NIL NIL NIL "7bit" 300 ("Mon, 2 Nov 2026 09:00:00 +0000" "Hi" NIL NIL NIL NIL NIL NIL NIL NIL) ("text" "plain" NIL NIL NIL "7bit" 2 1 NIL NIL NIL NIL) 8 NIL NIL NIL NIL)"#;
        assert_eq!(wanted(&structure(alone)), None);
        assert_eq!(rebuild(HEADER, &structure(alone), &parts), None);
    }

    fn parts(path: &[u32]) -> Option<Vec<u8>> {
        let fetched: [(&[u32], &[u8]); 4] = [
            (&[1, 1], b"Hello there"),
            (&[1, 2], b"<p>Hello =3D there</p>"),
            (&[2], b"UE5H"),
            (&[1], b"Just text"),
        ];
        fetched.iter().find(|(p, _)| *p == path).map(|(_, bytes)| bytes.to_vec())
    }

    const HEADER: &[u8] = b"From: a@x.co\r\nSubject: Q4\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n";

    #[test]
    fn rebuilds_nested_parts_as_greenmail_describes_them() {
        let text = r#"(((("text" "plain" ("charset" "utf-8") NIL NIL "7BIT" 67 2 NIL NIL NIL)("text" "html" ("charset" "utf-8") NIL NIL "7BIT" 103 2 NIL NIL NIL) "alternative" ("boundary" "alt") NIL NIL)("IMAGE" "PNG" NIL "<logo@moss.test>" NIL "base64" 4246 NIL ("inline" ("filename" "logo.png")) NIL) "related" ("boundary" "rel") NIL NIL)("APPLICATION" "PDF" ("name" "Q4.pdf") NIL NIL "base64" 410661 NIL ("attachment" ("filename" "Q4.pdf")) NIL) "mixed" ("boundary" "mix") NIL NIL)"#;
        let s = structure(text);
        assert_eq!(wanted(&s), Some(vec![vec![1, 1, 1], vec![1, 1, 2], vec![1, 2]]));
        let fetched = |path: &[u32]| -> Option<Vec<u8>> {
            Some(match path {
                [1, 1, 1] => b"Deck attached.".to_vec(),
                [1, 1, 2] => b"<p>Deck attached.</p>".to_vec(),
                _ => b"UE5H".to_vec(),
            })
        };
        // GreenMail leaves the blank line after the header out of BODY[HEADER].
        let header = b"Subject: Q4 deck\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n";
        let raw = rebuild(header, &s, &fetched).unwrap();
        let message = MessageParser::default().parse(&raw).unwrap();
        assert_eq!(message.body_text(0).as_deref(), Some("Deck attached."), "{}", String::from_utf8_lossy(&raw));
        let names: Vec<_> = message.attachments().map(|a| a.attachment_name().unwrap_or("?").to_string()).collect();
        assert_eq!(names, ["logo.png", "Q4.pdf"], "{}", String::from_utf8_lossy(&raw));
    }

    #[test]
    fn rebuilds_the_message_with_the_same_parts_in_the_same_places() {
        let raw = rebuild(HEADER, &structure(MIXED), &parts).unwrap();
        let message = MessageParser::default().parse(&raw).unwrap();
        assert_eq!(message.subject(), Some("Q4"));
        assert_eq!(message.body_text(0).as_deref(), Some("Hello there"));
        assert_eq!(message.body_html(0).as_deref(), Some("<p>Hello = there</p>"));
        let root = message.root().root_part();
        let child = |n: usize| root.children().nth(n).unwrap();
        assert_eq!(child(1).decoded().as_ref(), b"PNG");
        assert_eq!(child(1).content_id(), Some("logo"));
        // Left out, but still described: name, type, where it is and how big.
        let pdf = child(3);
        assert_eq!(pdf.attachment_name(), Some("Q4 \"final\".pdf"));
        assert!(pdf.is_content_type("application", "pdf"));
        assert!(pdf.decoded().is_empty());
        let omitted = |p: &mail_parser::MessagePart| {
            p.headers().get(OMITTED).map(|h| String::from_utf8_lossy(h.raw_value()).trim().to_string())
        };
        assert_eq!(omitted(&pdf).as_deref(), Some("570000"));
        assert_eq!(omitted(&child(2)).as_deref(), Some("292307"));
        assert_eq!(omitted(&child(1)), None);
    }

    #[test]
    fn a_single_part_message_keeps_its_header() {
        let single = r#"("text" "plain" ("charset" "utf-8") NIL NIL "7bit" 9 1 NIL NIL NIL NIL)"#;
        let header = b"Subject: Hi\r\nContent-Type: text/plain\r\n\r\n";
        let raw = rebuild(header, &structure(single), &parts).unwrap();
        assert_eq!(raw, b"Subject: Hi\r\nContent-Type: text/plain\r\n\r\nJust text");
        // A message that is one big attachment: just the header, saying what's left out.
        let pdf = r#"("application" "pdf" NIL NIL NIL "base64" 780 NIL ("attachment" NIL) NIL NIL)"#;
        let raw = rebuild(b"Subject: Hi\r\n\r\n", &structure(pdf), &parts).unwrap();
        assert_eq!(raw, b"Subject: Hi\r\nX-Moss-Omitted: 570\r\n\r\n");
    }

    #[test]
    fn files_left_out_keep_their_names_and_encodings() {
        let files = r#"(("text" "plain" NIL NIL NIL "8bit" 9 1 NIL NIL NIL NIL)("application" "pdf" NIL NIL NIL "binary" 200000 NIL ("attachment" ("filename*" "utf-8''r%C3%A9sum%C3%A9.pdf")) NIL NIL)("application" "zip" NIL NIL NIL "x-uuencode" 300000 NIL ("attachment" ("filename" "a.zip")) NIL NIL)("application" "octet-stream" NIL NIL NIL "7bit" 300000 NIL NIL NIL NIL) "mixed" ("boundary" "m") NIL NIL NIL)"#;
        let s = structure(files);
        assert_eq!(wanted(&s), Some(vec![vec![1]]));
        let raw = rebuild(HEADER, &s, &parts_of_files).unwrap();
        let text = String::from_utf8(raw.clone()).unwrap();
        for header in [
            "Content-Transfer-Encoding: 8bit",
            "Content-Transfer-Encoding: binary",
            "Content-Transfer-Encoding: x-uuencode",
            "Content-Transfer-Encoding: 7bit",
            "X-Moss-Omitted: 200000",
        ] {
            assert!(text.contains(header), "{header} in {text}");
        }
        let message = MessageParser::default().parse(&raw).unwrap();
        let pdf = message.root().root_part().children().nth(1).unwrap();
        assert_eq!(pdf.attachment_name(), Some("résumé.pdf"));
    }

    fn parts_of_files(path: &[u32]) -> Option<Vec<u8>> {
        (path == [1]).then(|| b"Hi there".to_vec())
    }

    #[test]
    fn a_missing_part_means_it_comes_whole() {
        assert_eq!(rebuild(HEADER, &structure(MIXED), &|_| None), None);
    }
}
