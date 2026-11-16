//! Writes RFC 822 / MIME messages: outgoing mail, and Resend mail that comes
//! without its original bytes.

use base64::Engine as _;
use chrono::{DateTime, Utc};

use crate::{Address, Attachment, Error, OutgoingMessage, Result};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// An outgoing message, ready to send and to file.
pub(crate) struct Built {
    pub message_id: String,
    /// What goes over SMTP: no Bcc header.
    pub wire: Vec<u8>,
    /// The copy for Sent: the same message with its Bcc header.
    pub archive: Vec<u8>,
}

/// The body of a message: text and/or HTML, plus attachments.
pub(crate) struct Body<'a> {
    pub text: Option<&'a str>,
    pub html: Option<&'a str>,
    pub attachments: &'a [Attachment],
}

pub(crate) fn build(
    msg: &OutgoingMessage,
    date: DateTime<Utc>,
    boundary: &mut dyn FnMut() -> String,
) -> Result<Built> {
    if msg.to.is_empty() && msg.cc.is_empty() && msg.bcc.is_empty() {
        return Err(Error::InvalidMessage("a message needs a recipient".into()));
    }
    build_draft(msg, date, boundary)
}

/// Like [`build`], for a draft's copy in the Drafts folder: it may have no one to send
/// to yet.
pub(crate) fn build_draft(
    msg: &OutgoingMessage,
    date: DateTime<Utc>,
    boundary: &mut dyn FnMut() -> String,
) -> Result<Built> {
    if msg.from.email.trim().is_empty() {
        return Err(Error::InvalidMessage("a message needs a sender".into()));
    }
    no_line_breaks(&msg.subject)?;
    let message_id = match &msg.message_id {
        Some(id) => {
            no_line_breaks(id)?;
            angle(id)
        }
        None => new_message_id(&msg.from.email),
    };

    let mut headers = vec![
        ("Date".to_string(), msg.date.unwrap_or(date).to_rfc2822()),
        ("From".to_string(), format_address(&msg.from)?),
    ];
    if !msg.reply_to.is_empty() {
        headers.push(("Reply-To".into(), format_addresses(&msg.reply_to)?));
    }
    if !msg.to.is_empty() {
        headers.push(("To".into(), format_addresses(&msg.to)?));
    }
    if !msg.cc.is_empty() {
        headers.push(("Cc".into(), format_addresses(&msg.cc)?));
    }
    let bcc_at = headers.len();
    headers.push(("Subject".into(), encode_header(&msg.subject)));
    headers.push(("Message-ID".into(), message_id.clone()));
    if let Some(parent) = &msg.in_reply_to {
        no_line_breaks(parent)?;
        headers.push(("In-Reply-To".into(), angle(parent)));
    }
    if !msg.references.is_empty() {
        for id in &msg.references {
            no_line_breaks(id)?;
        }
        let ids: Vec<String> = msg.references.iter().map(|id| angle(id)).collect();
        headers.push(("References".into(), ids.join(" ")));
    }
    for (name, value) in &msg.headers {
        let token = name.len() > 2
            && name.get(..2).is_some_and(|head| head.eq_ignore_ascii_case("x-"))
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !token {
            return Err(Error::InvalidMessage(format!("not a header Moss writes: {name:?}")));
        }
        no_line_breaks(value)?;
        headers.push((name.clone(), encode_header(value)));
    }
    for a in &msg.attachments {
        no_line_breaks(&a.filename)?;
        no_line_breaks(&a.content_type)?;
        no_line_breaks(a.content_id.as_deref().unwrap_or(""))?;
    }

    let body = write_body(
        &Body {
            text: msg.text.as_deref(),
            html: msg.html.as_deref(),
            attachments: &msg.attachments,
        },
        boundary,
    );
    let mut wire = write_headers(&headers);
    wire.extend_from_slice(&body);
    let archive = if msg.bcc.is_empty() {
        wire.clone()
    } else {
        headers.insert(bcc_at, ("Bcc".into(), format_addresses(&msg.bcc)?));
        let mut archive = write_headers(&headers);
        archive.extend_from_slice(&body);
        archive
    };
    Ok(Built {
        message_id,
        wire,
        archive,
    })
}

/// Writes headers then the MIME body. Header values are written as given
/// (call `encode_header` first for free text).
pub(crate) fn write_message(
    headers: &[(String, String)],
    body: &Body<'_>,
    boundary: &mut dyn FnMut() -> String,
) -> Vec<u8> {
    let mut out = write_headers(headers);
    out.extend_from_slice(&write_body(body, boundary));
    out
}

pub(crate) fn new_boundary() -> String {
    format!("=_moss_{}", uuid::Uuid::new_v4().simple())
}

fn new_message_id(email: &str) -> String {
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.trim())
        .filter(|d| !d.is_empty());
    format!(
        "<{}@{}>",
        uuid::Uuid::new_v4().simple(),
        domain.unwrap_or("moss.local")
    )
}

/// `<id>` with angle brackets, however it was given.
pub(crate) fn angle(id: &str) -> String {
    format!(
        "<{}>",
        id.trim().trim_start_matches('<').trim_end_matches('>')
    )
}

pub(crate) fn format_address(a: &Address) -> Result<String> {
    let email = a.email.trim();
    if email.is_empty()
        || email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>,;".contains(c))
    {
        return Err(Error::InvalidMessage(format!(
            "not an email address: {email:?}"
        )));
    }
    let name = a.name.as_deref().map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Ok(email.to_string());
    }
    no_line_breaks(name)?;
    let display = if !name.is_ascii() {
        encode_header(name)
    } else if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " !#$%&'*+-/=?^_`{|}~".contains(c))
    {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    };
    Ok(format!("{display} <{email}>"))
}

pub(crate) fn format_addresses(list: &[Address]) -> Result<String> {
    let all: Result<Vec<String>> = list.iter().map(format_address).collect();
    Ok(all?.join(", "))
}

/// Free text for a header: as is when ASCII, RFC 2047 encoded words otherwise.
pub(crate) fn encode_header(value: &str) -> String {
    if value.is_ascii() {
        return value.to_string();
    }
    // 45 bytes of UTF-8 make 60 base64 characters: a 72-character word.
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in value.chars() {
        if chunk.len() + c.len_utf8() > 45 {
            words.push(encoded_word(&chunk));
            chunk.clear();
        }
        chunk.push(c);
    }
    words.push(encoded_word(&chunk));
    words.join(" ")
}

fn encoded_word(chunk: &str) -> String {
    format!("=?UTF-8?B?{}?=", B64.encode(chunk))
}

fn no_line_breaks(value: &str) -> Result<()> {
    if value.contains(['\r', '\n']) {
        return Err(Error::InvalidMessage(format!(
            "line break in a header: {value:?}"
        )));
    }
    Ok(())
}

fn write_headers(headers: &[(String, String)]) -> Vec<u8> {
    headers
        .iter()
        .map(|(name, value)| fold(name, value))
        .collect::<String>()
        .into_bytes()
}

/// `Name: value`, folded at spaces to keep lines within 78 characters.
fn fold(name: &str, value: &str) -> String {
    let mut out = format!("{name}:");
    let mut line = out.len();
    for (i, word) in value.split(' ').enumerate() {
        if i > 0 && line + 1 + word.len() > 78 {
            out.push_str("\r\n ");
            line = 1;
        } else {
            out.push(' ');
            line += 1;
        }
        out.push_str(word);
        line += word.len();
    }
    out.push_str("\r\n");
    out
}

enum Part {
    Leaf {
        headers: Vec<String>,
        content: Vec<u8>,
    },
    Multi {
        kind: &'static str,
        parts: Vec<Part>,
    },
}

fn write_body(body: &Body<'_>, boundary: &mut dyn FnMut() -> String) -> Vec<u8> {
    let text = body.text.map(|t| text_part("plain", t));
    let html = body.html.map(|h| text_part("html", h));
    let mut main = match (text, html) {
        (Some(t), Some(h)) => Part::Multi {
            kind: "alternative",
            parts: vec![t, h],
        },
        (Some(part), None) | (None, Some(part)) => part,
        (None, None) => text_part("plain", ""),
    };
    let (inline, files): (Vec<&Attachment>, Vec<&Attachment>) = body
        .attachments
        .iter()
        .partition(|a| a.content_id.is_some());
    if !inline.is_empty() {
        let mut parts = vec![main];
        parts.extend(inline.into_iter().map(attachment_part));
        main = Part::Multi {
            kind: "related",
            parts,
        };
    }
    if !files.is_empty() {
        let mut parts = vec![main];
        parts.extend(files.into_iter().map(attachment_part));
        main = Part::Multi {
            kind: "mixed",
            parts,
        };
    }
    let mut out = b"MIME-Version: 1.0\r\n".to_vec();
    write_part(&mut out, &main, boundary);
    out
}

fn write_part(out: &mut Vec<u8>, part: &Part, boundary: &mut dyn FnMut() -> String) {
    match part {
        Part::Leaf { headers, content } => {
            for h in headers {
                out.extend_from_slice(h.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(content);
        }
        Part::Multi { kind, parts } => {
            let b = boundary();
            out.extend_from_slice(
                format!("Content-Type: multipart/{kind}; boundary=\"{b}\"\r\n\r\n").as_bytes(),
            );
            for p in parts {
                out.extend_from_slice(format!("--{b}\r\n").as_bytes());
                write_part(out, p, boundary);
            }
            out.extend_from_slice(format!("--{b}--\r\n").as_bytes());
        }
    }
}

fn text_part(subtype: &str, text: &str) -> Part {
    let (normalised, lone_cr) = crlf(text);
    let seven_bit = text.is_ascii() && !lone_cr && normalised.split("\r\n").all(|l| l.len() <= 998);
    let (encoding, content) = if seven_bit {
        let mut content = normalised.into_bytes();
        if !content.ends_with(b"\r\n") {
            content.extend_from_slice(b"\r\n");
        }
        ("7bit", content)
    } else {
        ("base64", base64_lines(normalised.as_bytes()))
    };
    Part::Leaf {
        headers: vec![
            format!("Content-Type: text/{subtype}; charset=utf-8"),
            format!("Content-Transfer-Encoding: {encoding}"),
        ],
        content,
    }
}

/// Line endings as CRLF. Also says whether there was a CR on its own.
fn crlf(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len());
    let mut lone_cr = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                out.push_str("\r\n");
            }
            '\r' => {
                lone_cr = true;
                out.push('\r');
            }
            '\n' => out.push_str("\r\n"),
            c => out.push(c),
        }
    }
    (out, lone_cr)
}

fn attachment_part(a: &Attachment) -> Part {
    let content_type = if a.content_type.is_empty() {
        "application/octet-stream"
    } else {
        &a.content_type
    };
    let disposition = if a.content_id.is_some() {
        "inline"
    } else {
        "attachment"
    };
    let mut headers = vec![
        format!(
            "Content-Type: {content_type}; {}",
            param("name", &a.filename)
        ),
        format!(
            "Content-Disposition: {disposition}; {}",
            param("filename", &a.filename)
        ),
    ];
    if let Some(cid) = &a.content_id {
        headers.push(format!("Content-ID: {}", angle(cid)));
    }
    headers.push("Content-Transfer-Encoding: base64".into());
    Part::Leaf {
        headers,
        content: base64_lines(&a.data),
    }
}

/// A MIME parameter, RFC 2231 encoded when it isn't plain ASCII.
fn param(name: &str, value: &str) -> String {
    if value.is_ascii() && !value.contains(['"', '\\']) {
        return format!("{name}=\"{value}\"");
    }
    let mut encoded = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("{name}*=utf-8''{encoded}")
}

fn base64_lines(data: &[u8]) -> Vec<u8> {
    let encoded = B64.encode(data);
    let mut out = Vec::with_capacity(encoded.len() + encoded.len() / 38 + 2);
    for line in encoded.as_bytes().chunks(76) {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    if out.is_empty() {
        out.extend_from_slice(b"\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn counter() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("b{n}")
        }
    }

    fn date() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 1, 9, 30, 0).unwrap()
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn reply() -> OutgoingMessage {
        OutgoingMessage {
            from: Address::named("Ada Lovelace", "ada@moss.dev"),
            to: vec![Address::new("bob@example.com")],
            cc: vec![Address::named("Carol", "carol@example.com")],
            bcc: vec![Address::new("dan@example.com")],
            reply_to: vec![Address::new("replies@moss.dev")],
            subject: "Re: Plans".into(),
            text: Some("Sounds good.\nSee you then.".into()),
            in_reply_to: Some("abc@example.com".into()),
            references: vec!["<root@example.com>".into(), "abc@example.com".into()],
            message_id: Some("<fixed@moss.dev>".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_persisted_submission_keeps_its_date_and_complete_payload() {
        let mut message = reply();
        message.date = Some(date());
        message.attachments.push(Attachment { filename: "note.bin".into(), content_type: "application/octet-stream".into(), content_id: None, data: vec![0, 255, 1] });
        let json = serde_json::to_vec(&message).unwrap();
        let encoded: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(encoded["attachments"][0]["data"], "AP8B", "binary payloads use bounded base64 expansion");
        let restored: OutgoingMessage = serde_json::from_slice(&json).unwrap();
        assert_eq!(restored, message);
        let first = build(&message, date(), &mut counter()).unwrap();
        let later = build(&restored, date() + chrono::Duration::days(1), &mut counter()).unwrap();
        assert_eq!(first.archive, later.archive, "a recovered submission must not acquire a new Date");
        assert_eq!(first.message_id, later.message_id);
    }

    #[test]
    fn a_plain_reply_has_threading_headers_and_hides_bcc_on_the_wire() {
        let built = build(&reply(), date(), &mut counter()).unwrap();
        assert_eq!(built.message_id, "<fixed@moss.dev>");
        assert_eq!(
            text(&built.wire),
            "Date: Sun, 1 Nov 2026 09:30:00 +0000\r\n\
             From: Ada Lovelace <ada@moss.dev>\r\n\
             Reply-To: replies@moss.dev\r\n\
             To: bob@example.com\r\n\
             Cc: Carol <carol@example.com>\r\n\
             Subject: Re: Plans\r\n\
             Message-ID: <fixed@moss.dev>\r\n\
             In-Reply-To: <abc@example.com>\r\n\
             References: <root@example.com> <abc@example.com>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Transfer-Encoding: 7bit\r\n\
             \r\n\
             Sounds good.\r\nSee you then.\r\n"
        );
        let archive = text(&built.archive);
        assert!(archive.contains("Cc: Carol <carol@example.com>\r\nBcc: dan@example.com\r\n"));
        assert!(!text(&built.wire).contains("dan@"));
    }

    #[test]
    fn a_new_message_gets_a_message_id_on_the_senders_domain() {
        let msg = OutgoingMessage {
            message_id: None,
            ..reply()
        };
        let built = build(&msg, date(), &mut counter()).unwrap();
        assert!(built.message_id.starts_with('<'));
        assert!(built.message_id.ends_with("@moss.dev>"));
        let odd = OutgoingMessage {
            from: Address::new("nodomain"),
            ..msg
        };
        assert!(build(&odd, date(), &mut counter())
            .unwrap()
            .message_id
            .ends_with("@moss.local>"));
    }

    #[test]
    fn text_and_html_become_multipart_alternative() {
        let msg = OutgoingMessage {
            html: Some("<p>Hi</p>".into()),
            text: Some("Hi".into()),
            ..reply()
        };
        let wire = text(&build(&msg, date(), &mut counter()).unwrap().wire);
        let body = wire.split_once("MIME-Version: 1.0\r\n").unwrap().1;
        assert_eq!(
            body,
            "Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Transfer-Encoding: 7bit\r\n\
             \r\n\
             Hi\r\n\
             --b1\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Transfer-Encoding: 7bit\r\n\
             \r\n\
             <p>Hi</p>\r\n\
             --b1--\r\n"
        );
    }

    #[test]
    fn inline_images_go_in_multipart_related_and_files_in_multipart_mixed() {
        let msg = OutgoingMessage {
            text: None,
            html: Some("<img src=\"cid:logo\">".into()),
            attachments: vec![
                Attachment {
                    filename: "logo.png".into(),
                    content_type: "image/png".into(),
                    content_id: Some("logo".into()),
                    data: vec![1, 2, 3],
                },
                Attachment {
                    filename: "résumé.pdf".into(),
                    content_type: "application/pdf".into(),
                    content_id: None,
                    data: b"%PDF".to_vec(),
                },
            ],
            ..reply()
        };
        let wire = text(&build(&msg, date(), &mut counter()).unwrap().wire);
        let body = wire.split_once("MIME-Version: 1.0\r\n").unwrap().1;
        assert_eq!(
            body,
            "Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: multipart/related; boundary=\"b2\"\r\n\
             \r\n\
             --b2\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Transfer-Encoding: 7bit\r\n\
             \r\n\
             <img src=\"cid:logo\">\r\n\
             --b2\r\n\
             Content-Type: image/png; name=\"logo.png\"\r\n\
             Content-Disposition: inline; filename=\"logo.png\"\r\n\
             Content-ID: <logo>\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             AQID\r\n\
             --b2--\r\n\
             --b1\r\n\
             Content-Type: application/pdf; name*=utf-8''r%C3%A9sum%C3%A9.pdf\r\n\
             Content-Disposition: attachment; filename*=utf-8''r%C3%A9sum%C3%A9.pdf\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             JVBERg==\r\n\
             --b1--\r\n"
        );
    }

    #[test]
    fn a_message_with_no_body_gets_an_empty_text_part() {
        let msg = OutgoingMessage {
            text: None,
            ..reply()
        };
        let wire = text(&build(&msg, date(), &mut counter()).unwrap().wire);
        assert!(wire.ends_with("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\n\r\n"));
    }

    #[test]
    fn non_ascii_or_long_lines_are_base64_wrapped_at_76() {
        let long = "é".repeat(60);
        let msg = OutgoingMessage {
            text: Some(long.clone()),
            ..reply()
        };
        let wire = text(&build(&msg, date(), &mut counter()).unwrap().wire);
        let body = wire
            .split_once("Content-Transfer-Encoding: base64\r\n\r\n")
            .unwrap()
            .1;
        let lines: Vec<&str> = body.trim_end().split("\r\n").collect();
        assert_eq!(lines[0].len(), 76);
        assert_eq!(B64.decode(lines.concat()).unwrap(), long.as_bytes());

        let wide = "a".repeat(1200);
        let msg = OutgoingMessage {
            text: Some(wide),
            ..reply()
        };
        assert!(text(&build(&msg, date(), &mut counter()).unwrap().wire).contains("base64"));
    }

    #[test]
    fn bare_cr_in_ascii_text_forces_base64() {
        let msg = OutgoingMessage {
            text: Some("a\rb".into()),
            ..reply()
        };
        assert!(text(&build(&msg, date(), &mut counter()).unwrap().wire).contains("base64"));
    }

    #[test]
    fn crlf_text_is_kept_as_is() {
        let msg = OutgoingMessage {
            text: Some("a\r\nb\r\n".into()),
            ..reply()
        };
        assert!(text(&build(&msg, date(), &mut counter()).unwrap().wire)
            .ends_with("\r\n\r\na\r\nb\r\n"));
    }

    #[test]
    fn non_ascii_subjects_and_names_are_encoded_words() {
        assert_eq!(encode_header("Hello"), "Hello");
        assert_eq!(encode_header("Café"), "=?UTF-8?B?Q2Fmw6k=?=");
        let long = encode_header(&"é".repeat(40));
        let words: Vec<&str> = long.split(' ').collect();
        assert_eq!(words.len(), 2);
        assert!(words
            .iter()
            .all(|w| w.len() <= 75 && w.starts_with("=?UTF-8?B?")));
        assert_eq!(
            format_address(&Address::named("Zoë", "z@x.io")).unwrap(),
            "=?UTF-8?B?Wm/Dqw==?= <z@x.io>"
        );
    }

    #[test]
    fn names_with_specials_are_quoted() {
        assert_eq!(
            format_address(&Address::named("Lovelace, Ada \"A\"", "a@x.io")).unwrap(),
            "\"Lovelace, Ada \\\"A\\\"\" <a@x.io>"
        );
        assert_eq!(
            format_address(&Address::named("", "a@x.io")).unwrap(),
            "a@x.io"
        );
        assert_eq!(
            format_addresses(&[Address::new("a@x.io"), Address::new("b@x.io")]).unwrap(),
            "a@x.io, b@x.io"
        );
    }

    #[test]
    fn long_headers_are_folded() {
        let refs: Vec<String> = (0..6)
            .map(|i| format!("<message-{i}@long.example.com>"))
            .collect();
        let msg = OutgoingMessage {
            references: refs,
            ..reply()
        };
        let wire = text(&build(&msg, date(), &mut counter()).unwrap().wire);
        let header = wire
            .split("References: ")
            .nth(1)
            .unwrap()
            .split("\r\nMIME")
            .next()
            .unwrap();
        assert!(header.contains("\r\n <message-"));
        for line in header.split("\r\n") {
            assert!(line.len() <= 78, "{line}");
        }
    }

    #[test]
    fn header_injection_is_refused() {
        let bad_subject = OutgoingMessage {
            subject: "Hi\r\nBcc: x@evil.io".into(),
            ..reply()
        };
        assert!(matches!(
            build(&bad_subject, date(), &mut counter()),
            Err(Error::InvalidMessage(_))
        ));
        let bad_name = OutgoingMessage {
            from: Address::named("A\nB", "a@x.io"),
            ..reply()
        };
        assert!(build(&bad_name, date(), &mut counter()).is_err());
        let bad_email = OutgoingMessage {
            to: vec![Address::new("a@x.io>, b@y.io")],
            ..reply()
        };
        assert!(build(&bad_email, date(), &mut counter()).is_err());
        let bad_ref = OutgoingMessage {
            in_reply_to: Some("a\r\nb".into()),
            ..reply()
        };
        assert!(build(&bad_ref, date(), &mut counter()).is_err());
        let bad_file = OutgoingMessage {
            attachments: vec![Attachment {
                filename: "a\r\n.txt".into(),
                ..Default::default()
            }],
            ..reply()
        };
        assert!(build(&bad_file, date(), &mut counter()).is_err());
        let bad_type = OutgoingMessage {
            attachments: vec![Attachment {
                filename: "a".into(),
                content_type: "x\r\ny".into(),
                ..Default::default()
            }],
            ..reply()
        };
        assert!(build(&bad_type, date(), &mut counter()).is_err());
    }

    #[test]
    fn a_message_needs_a_sender_and_a_recipient() {
        let no_from = OutgoingMessage {
            from: Address::default(),
            ..reply()
        };
        assert!(
            matches!(build(&no_from, date(), &mut counter()), Err(Error::InvalidMessage(m)) if m.contains("sender"))
        );
        let nobody = OutgoingMessage {
            to: vec![],
            cc: vec![],
            bcc: vec![],
            ..reply()
        };
        assert!(
            matches!(build(&nobody, date(), &mut counter()), Err(Error::InvalidMessage(m)) if m.contains("recipient"))
        );
        let bcc_only = OutgoingMessage {
            to: vec![],
            cc: vec![],
            ..reply()
        };
        assert!(!text(&build(&bcc_only, date(), &mut counter()).unwrap().wire).contains("\r\nTo:"));
    }

    #[test]
    fn attachments_without_a_type_are_octet_streams() {
        let msg = OutgoingMessage {
            attachments: vec![Attachment {
                filename: "a.bin".into(),
                data: vec![0],
                ..Default::default()
            }],
            ..reply()
        };
        assert!(text(&build(&msg, date(), &mut counter()).unwrap().wire)
            .contains("Content-Type: application/octet-stream; name=\"a.bin\""));
    }

    #[test]
    fn an_empty_attachment_is_a_blank_line() {
        let empty = Attachment {
            filename: "empty.txt".into(),
            ..Default::default()
        };
        let msg = OutgoingMessage {
            attachments: vec![empty],
            ..reply()
        };
        let wire = text(&build(&msg, date(), &mut counter()).unwrap().wire);
        assert!(wire.contains("Content-Transfer-Encoding: base64\r\n\r\n\r\n--b1--"));
    }

    #[test]
    fn message_ids_get_angle_brackets_once() {
        assert_eq!(angle("a@b"), "<a@b>");
        assert_eq!(angle(" <a@b> "), "<a@b>");
    }

    #[test]
    fn boundaries_are_unique_and_safe() {
        let (a, b) = (new_boundary(), new_boundary());
        assert_ne!(a, b);
        assert!(a.starts_with("=_moss_") && a.chars().all(|c| c.is_ascii_graphic()));
    }

    #[test]
    fn write_message_writes_headers_as_given() {
        let headers = vec![("X-Test".to_string(), "1".to_string())];
        let body = Body {
            text: Some("hi"),
            html: None,
            attachments: &[],
        };
        let out = text(&write_message(&headers, &body, &mut counter()));
        assert!(out.starts_with("X-Test: 1\r\nMIME-Version: 1.0\r\n"));
    }

    #[test]
    fn a_draft_needs_no_recipient_and_carries_moss_headers() {
        let draft = OutgoingMessage {
            to: vec![],
            cc: vec![],
            bcc: vec![],
            headers: vec![("X-Moss-Draft".into(), "d1".into()), ("X-Moss-Mode".into(), "reply".into())],
            ..reply()
        };
        let built = build_draft(&draft, date(), &mut counter()).unwrap();
        let raw = text(&built.archive);
        assert!(!raw.contains("\r\nTo:") && !raw.contains("Cc:"), "{raw}");
        assert!(raw.contains("References: <root@example.com> <abc@example.com>\r\nX-Moss-Draft: d1\r\nX-Moss-Mode: reply\r\n"), "{raw}");
        // Sending still needs someone to send to; extra headers go out too.
        assert!(matches!(build(&draft, date(), &mut counter()), Err(Error::InvalidMessage(m)) if m.contains("recipient")));
        let sent = OutgoingMessage { to: vec![Address::new("bob@example.com")], ..draft.clone() };
        assert!(text(&build(&sent, date(), &mut counter()).unwrap().wire).contains("X-Moss-Draft: d1\r\n"));
        // A draft still needs a sender.
        let no_from = OutgoingMessage { from: Address::new(""), ..draft.clone() };
        assert!(build_draft(&no_from, date(), &mut counter()).is_err());
    }

    #[test]
    fn extra_headers_cant_smuggle_in_other_headers() {
        let with = |name: &str, value: &str| OutgoingMessage {
            headers: vec![(name.into(), value.into())],
            ..reply()
        };
        for (name, value) in [("X-Moss-Draft", "d1\r\nBcc: eve@x.io"), ("X-Moss Draft", "d1"), ("", "d1"), ("X:Moss", "d1"), ("Xé-Moss", "d1")] {
            assert!(
                matches!(build(&with(name, value), date(), &mut counter()), Err(Error::InvalidMessage(_))),
                "{name:?}: {value:?}"
            );
        }
        // Free text is encoded like any header.
        let wire = text(&build(&with("X-Moss-Note", "café"), date(), &mut counter()).unwrap().wire);
        assert!(wire.contains("X-Moss-Note: =?UTF-8?B?"), "{wire}");
    }
}
