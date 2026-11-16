//! Raw RFC 822 bytes → the fields Moss stores.

use mail_parser::{AddressList, Message, MessageParser, MessagePart, PartRole};
use moss_core::Person;
use sha2::{Digest, Sha256};

use crate::text;

pub struct ParsedAttachment {
    pub name: String,
    pub content_type: String,
    pub bytes: u64,
    pub content_id: Option<String>,
    pub inline: bool,
    /// IMAP body section ("2", "1.2"): where the server has its bytes.
    pub part: String,
    /// The bytes, for the small inline images kept on this device. See [`INLINE_KEEP_MAX`].
    pub kept: Option<Vec<u8>>,
}

/// Inline images up to this size are kept so HTML mail renders offline and without a
/// round trip per image: logos, signature images and newsletter icons are a few KB to a
/// few tens of KB. Anything bigger (a pasted photo) is fetched when the message is opened.
pub const INLINE_KEEP_MAX: usize = 64 * 1024;
/// On a part sync left on the server: its decoded size (see moss-providers' `imap/slim.rs`).
const OMITTED: &str = "X-Moss-Omitted";

/// At most this much inline image data is kept per message.
pub const INLINE_KEEP_BUDGET: usize = 256 * 1024;

pub struct Parsed {
    /// Message-ID without <>, or "sha256:…" of the raw bytes when there is none.
    pub message_id: String,
    /// In-Reply-To and References, without this message's own id.
    pub refs: Vec<String>,
    pub subject: String,
    pub from: Person,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    pub sent_at: Option<i64>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<ParsedAttachment>,
    pub has_calendar: bool,
    /// Sent to a mailing list (List-Id / List-Unsubscribe).
    pub is_list: bool,
    /// Sent by a machine (Auto-Submitted, Precedence: bulk).
    pub is_automated: bool,
    /// Every `Authentication-Results` value as written, top first: see `authres.rs`.
    pub authentication_results: Vec<String>,
    /// One From header naming one address: only then is there one sender to confirm.
    pub single_from: bool,
}

fn people(list: Option<AddressList>) -> Vec<Person> {
    list.map(|list| {
        list.mailboxes()
            .filter_map(|m| {
                let email = m.address()?.to_string();
                let name =
                    m.name().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&email).to_string();
                Some(Person { name, email })
            })
            .collect()
    })
    .unwrap_or_default()
}

/// The files in a message, in order: attachments and images shown in the body.
/// `Attachment::index` counts along this list.
pub fn files<'m>(message: &'m Message<'m>) -> impl Iterator<Item = MessagePart<'m>> + 'm {
    message.root().parts().filter(|p| p.is_attachment() || p.role() == PartRole::MediaBody)
}

/// The IMAP body section of a part (RFC 3501 6.4.5): its position among its parent's
/// children at each level, like "1.2". A message that is a single part is section "1".
pub fn section(part: &MessagePart) -> String {
    let mut path = Vec::new();
    let mut child = part.clone();
    while let Some(parent) = child.parent() {
        let position = parent.children().position(|c| c.id() == child.id()).unwrap_or(0);
        path.push((position + 1).to_string());
        child = parent;
    }
    if path.is_empty() {
        return "1".into();
    }
    path.reverse();
    path.join(".")
}

/// Whether `html` shows the file with this Content-ID (`<img src="cid:…">`). Such a file is
/// part of the body, however the sender marked it: some send their logos as attachments.
pub(crate) fn shown_in(html: &str, content_id: &str) -> bool {
    cid_matches(html, &format!("cid:{content_id}")).next().is_some()
}

/// Literal Content-ID URI matches ending at an HTML/CSS delimiter or end of input.
/// A shorter identifier must not consume the prefix of another identifier.
pub(crate) fn cid_matches<'a>(html: &'a str, cid: &'a str) -> impl Iterator<Item = (usize, &'a str)> {
    html.match_indices(cid).filter(move |(start, matched)| {
        let closing = match html[..*start].chars().next_back() {
            Some('\"') => Some('\"'), Some('\'') => Some('\''), Some('(') => Some(')'), _ => None,
        };
        html[start + matched.len()..].chars().next().is_none_or(|next| {
            next.is_ascii_whitespace() || closing.map_or_else(|| matches!(next, '\"' | '\'' | ')' | '>'), |closing| next == closing)
        })
    })
}

fn header_text(message: &Message, name: &str) -> Option<String> {
    let value = message.headers().get(name)?.raw_value();
    Some(String::from_utf8_lossy(value).trim().to_ascii_lowercase())
}

/// `None` when the bytes aren't an email at all.
pub fn parse(raw: &[u8]) -> Option<Parsed> {
    let message = MessageParser::default().parse(raw)?;
    if message.headers().is_empty() {
        return None;
    }
    let message_id = match message.message_id() {
        Some(id) => id.to_string(),
        None => format!("sha256:{:x}", Sha256::digest(raw)),
    };
    let mut refs: Vec<String> = Vec::new();
    let linked = message.references().into_iter().chain(message.in_reply_to());
    for id in linked.flat_map(|list| list.iter().map(String::from).collect::<Vec<_>>()) {
        if id != message_id && !refs.contains(&id) {
            refs.push(id);
        }
    }
    let from = people(message.from()).into_iter().chain(people(message.sender())).next();
    let body_text = message.text_bodies().collect::<Vec<_>>().join("\n").replace("\r\n", "\n");
    let has_html = message.html_body().any(|p| p.is_content_type("text", "html"));
    let body_html = has_html.then(|| message.html_bodies().collect::<Vec<_>>().join("\n"));
    let mut budget = INLINE_KEEP_BUDGET;
    let attachments = files(&message)
        .map(|part| {
            let content_type = part
                .content_type()
                .map(|ct| match ct.subtype() {
                    Some(sub) => format!("{}/{}", ct.ctype(), sub),
                    None => ct.ctype().to_string(),
                })
                .unwrap_or_else(|| "application/octet-stream".into())
                .to_ascii_lowercase();
            // Sync leaves big files on the server and says how big they were.
            let omitted = part.headers().get(OMITTED).map(|h| String::from_utf8_lossy(h.raw_value()).trim().to_string());
            let size = omitted.map_or_else(|| part.decoded_len(), |n| n.parse().unwrap_or(0));
            let shown = part.content_id().is_some_and(|id| shown_in(body_html.as_deref().unwrap_or_default(), id));
            let inline = part.is_inline() || shown;
            let keep = inline
                && part.content_id().is_some()
                && content_type.starts_with("image/")
                && size <= INLINE_KEEP_MAX.min(budget);
            if keep {
                budget -= size;
            }
            ParsedAttachment {
                name: part.attachment_name().unwrap_or("attachment").to_string(),
                content_type,
                bytes: size as u64,
                content_id: part.content_id().map(String::from),
                inline,
                part: section(&part),
                kept: keep.then(|| part.decoded().into_owned()),
            }
        })
        .collect::<Vec<_>>();
    let has_calendar = message.parts().any(|p| p.is_content_type("text", "calendar"))
        || attachments.iter().any(|a| a.name.to_ascii_lowercase().ends_with(".ics"));
    let is_list =
        message.headers().contains("List-Id") || message.headers().contains("List-Unsubscribe");
    let auto_submitted = header_text(&message, "Auto-Submitted").is_some_and(|v| v != "no");
    let bulk = header_text(&message, "Precedence")
        .is_some_and(|v| matches!(v.as_str(), "bulk" | "list" | "junk"));
    Some(Parsed {
        message_id,
        refs,
        subject: message.subject().unwrap_or_default().trim().to_string(),
        from: from.unwrap_or(Person { name: String::new(), email: String::new() }),
        to: people(message.to()),
        cc: people(message.cc()),
        bcc: people(message.bcc()),
        sent_at: message.date().filter(|d| d.is_valid()).map(|d| d.to_timestamp() * 1000),
        body_text,
        body_html,
        attachments,
        has_calendar,
        is_list,
        is_automated: auto_submitted || bulk,
        authentication_results: message
            .headers()
            .all("Authentication-Results")
            .map(|h| String::from_utf8_lossy(h.raw_value()).into_owned())
            .collect(),
        single_from: message.headers().all("From").count() == 1
            && message.from().is_some_and(|list| list.mailboxes().count() == 1),
    })
}

impl Parsed {
    /// Everyone on the message, lower-case and sorted: the fallback thread key.
    pub fn participants_key(&self) -> String {
        let mut emails: Vec<String> = std::iter::once(&self.from)
            .chain(&self.to)
            .chain(&self.cc)
            .map(|p| p.email.to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        emails.sort();
        emails.dedup();
        emails.join(",")
    }

    /// Names and addresses, for search.
    pub fn participants_text(&self) -> String {
        std::iter::once(&self.from)
            .chain(&self.to)
            .chain(&self.cc)
            .chain(&self.bcc)
            .map(|p| {
                if p.name == p.email {
                    p.email.clone()
                } else {
                    format!("{} {}", p.name, p.email)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The reply part only, which is what search indexes.
    pub fn body_stripped(&self) -> String {
        text::strip_quoted(&self.body_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn inline_id_boundaries_respect_the_enclosing_quote() {
        assert!(!shown_in("<img src=\"cid:img1'0\">", "img1"));
        assert!(shown_in("<img src=\"cid:img1'0\">", "img1'0"));
        assert!(!shown_in("<img src='cid:img1\"0'>", "img1"));
        assert!(shown_in("<img src='cid:img1\"0'>", "img1\"0"));
        assert!(!shown_in("<img src=\"cid:img1)0\">", "img1"));
    }

    #[test]
    fn inline_id_detection_requires_a_complete_uri_not_a_prefix() {
        assert!(!shown_in("<img src=\"cid:img10\">", "img1"));
        for (start,end) in [("",""), ("\"","\""), ("'","'"), ("(",")"), ("=",">"), (""," "), ("","\t"), ("","\r"), ("","\n")] {
            assert!(shown_in(&format!("{start}cid:img1{end}"),"img1"));
        }
        assert!(!shown_in("cid:img1other", "img1"));
        assert!(!shown_in("no inline image", "img1"));
    }

    #[test]
    fn keeps_every_authentication_result_top_first_and_whether_from_names_one_address() {
        let raw = mail()
            .header("Authentication-Results", "sender.example; dmarc=pass")
            .received("Received", "from mail.example.com by mx.google.com")
            .received("Authentication-Results", "mx.google.com;\r\n\tdkim=pass header.d=northwind.co")
            .build();
        let parsed = parse(&raw).unwrap();
        assert_eq!(
            parsed.authentication_results,
            [" mx.google.com;\r\n\tdkim=pass header.d=northwind.co\r\n", " sender.example; dmarc=pass\r\n"]
        );
        assert!(parsed.single_from);
        assert!(parse(&mail().build()).unwrap().authentication_results.is_empty());
        let two = mail().from("Daniel <daniel@northwind.co>, Eve <eve@evil.example>").build();
        assert!(!parse(&two).unwrap().single_from);
        let twice = mail().received("From", "Eve <eve@evil.example>").build();
        assert!(!parse(&twice).unwrap().single_from);
        assert!(!parse(&mail().without("From").header("Sender", "daniel@northwind.co").build()).unwrap().single_from);
    }

    #[test]
    fn reads_the_headers_and_body() {
        let parsed = parse(
            &mail().header("Cc", "Lena Vogt <lena@northwind.co>, jonas@northwind.co").build(),
        )
        .unwrap();
        assert_eq!(parsed.message_id, "q4@northwind.co");
        assert_eq!(parsed.subject, "Q4 budget");
        assert_eq!(
            parsed.from,
            Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() }
        );
        assert_eq!(parsed.to[0].email, "maya@northwind.co");
        assert_eq!(
            parsed.cc[1],
            Person { name: "jonas@northwind.co".into(), email: "jonas@northwind.co".into() }
        );
        assert!(parsed.bcc.is_empty());
        assert_eq!(parsed.sent_at, Some(NOV_1));
        assert_eq!(parsed.body_text, "Hi Maya,\n\nCan you sign off the Q4 budget?\n\nDaniel\n");
        assert_eq!(parsed.body_html, None);
        assert!(parsed.refs.is_empty());
        assert!(!parsed.is_list && !parsed.is_automated && !parsed.has_calendar);
    }

    #[test]
    fn rejects_bytes_that_are_not_an_email() {
        assert!(parse(b"").is_none());
        assert!(parse(b"\r\n\r\njust a body").is_none());
    }

    #[test]
    fn makes_up_a_stable_id_when_there_is_no_message_id() {
        let raw = mail().without("Message-ID").build();
        let a = parse(&raw).unwrap().message_id;
        assert!(a.starts_with("sha256:"));
        assert_eq!(parse(&raw).unwrap().message_id, a);
        assert_ne!(
            parse(&mail().without("Message-ID").body("other").build()).unwrap().message_id,
            a
        );
    }

    #[test]
    fn collects_references_once_without_its_own_id() {
        let raw = mail()
            .id("c@x")
            .header("References", "<a@x> <b@x> <c@x>")
            .header("In-Reply-To", "<b@x>")
            .build();
        assert_eq!(parse(&raw).unwrap().refs, ["a@x", "b@x"]);
    }

    #[test]
    fn falls_back_to_sender_then_to_nobody() {
        let sender = mail().without("From").header("Sender", "Bot <bot@x.co>").build();
        assert_eq!(parse(&sender).unwrap().from.email, "bot@x.co");
        let nobody = parse(&mail().without("From").build()).unwrap();
        assert_eq!(nobody.from, Person { name: String::new(), email: String::new() });
        assert_eq!(nobody.participants_key(), "maya@northwind.co");
    }

    #[test]
    fn ignores_a_date_it_cannot_read() {
        assert_eq!(parse(&mail().header("Date", "someday").build()).unwrap().sent_at, None);
    }

    #[test]
    fn keeps_html_and_lists_attachments() {
        let raw = mail()
            .html("<p>Can you <b>sign off</b>?</p>")
            .attach("Q4.pdf", "application/pdf", "PDFDATA")
            .inline("logo.png", "image/png", "PNG")
            .build();
        let parsed = parse(&raw).unwrap();
        assert_eq!(parsed.body_html.as_deref(), Some("<p>Can you <b>sign off</b>?</p>"));
        assert_eq!(parsed.attachments.len(), 2);
        let pdf = &parsed.attachments[0];
        assert_eq!(
            (pdf.name.as_str(), pdf.content_type.as_str(), pdf.bytes, pdf.inline),
            ("Q4.pdf", "application/pdf", 7, false)
        );
        assert_eq!(pdf.content_id.as_deref(), Some("Q4.pdf"));
        assert!(parsed.attachments[1].inline);
    }

    #[test]
    fn attachments_know_their_imap_section() {
        let mixed = mail()
            .html("<p>Hi</p>")
            .attach("Q4.pdf", "application/pdf", "PDFDATA")
            .inline("logo.png", "image/png", "PNG")
            .build();
        let parts: Vec<String> =
            parse(&mixed).unwrap().attachments.into_iter().map(|a| a.part).collect();
        assert_eq!(parts, ["2", "3"]);

        let related = b"From: a@x.co\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n\
            Content-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n\
            <img src=\"cid:logo\">\r\n--r\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\n\
            Content-Disposition: inline\r\n\r\nPNG\r\n--r--\r\n--m\r\nContent-Type: application/pdf\r\n\
            Content-Disposition: attachment; filename=a.pdf\r\n\r\nPDF\r\n--m--\r\n";
        let parts: Vec<String> =
            parse(related).unwrap().attachments.into_iter().map(|a| a.part).collect();
        assert_eq!(parts, ["1.2", "2"]);

        let whole = b"From: a@x.co\r\nContent-Type: application/pdf\r\n\
            Content-Disposition: attachment; filename=a.pdf\r\n\r\nPDF\r\n";
        assert_eq!(parse(whole).unwrap().attachments[0].part, "1");
    }

    #[test]
    fn a_file_left_on_the_server_keeps_its_size() {
        let raw = b"From: a@x.co\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n\
            Content-Type: text/plain\r\n\r\nHi\r\n--m\r\nContent-Type: application/pdf; name=\"Q4.pdf\"\r\n\
            Content-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"Q4.pdf\"\r\n\
            X-Moss-Omitted: 570000\r\n\r\n\r\n--m--\r\n";
        let parsed = parse(raw).unwrap();
        assert_eq!(parsed.attachments[0].name, "Q4.pdf");
        assert_eq!(parsed.attachments[0].bytes, 570_000);
        assert_eq!(parsed.attachments[0].part, "2");
        // Not a number: the size of what's there.
        let odd = String::from_utf8_lossy(raw).replace("570000", "lots");
        assert_eq!(parse(odd.as_bytes()).unwrap().attachments[0].bytes, 0);
    }

    #[test]
    fn keeps_the_bytes_of_small_inline_images_only() {
        let small = "P".repeat(INLINE_KEEP_MAX);
        let raw = mail()
            .html("<img src=\"cid:logo.png\">")
            .inline("logo.png", "image/png", &small)
            .inline("photo.png", "image/png", &"P".repeat(INLINE_KEEP_MAX + 1))
            .inline("data.bin", "application/octet-stream", "DATA")
            .attach("tiny.png", "image/png", "PNG")
            .build();
        let kept: Vec<Option<usize>> = parse(&raw)
            .unwrap()
            .attachments
            .iter()
            .map(|a| a.kept.as_ref().map(Vec::len))
            .collect();
        assert_eq!(kept, [Some(INLINE_KEEP_MAX), None, None, None]);

        // Many small images: only as many as fit the message's budget.
        let mut many = mail().html("<p>logos</p>");
        for n in 0..=INLINE_KEEP_BUDGET / INLINE_KEEP_MAX {
            many = many.inline(&format!("{n}.png"), "image/png", &small);
        }
        let kept =
            parse(&many.build()).unwrap().attachments.iter().filter(|a| a.kept.is_some()).count();
        assert_eq!(kept, INLINE_KEEP_BUDGET / INLINE_KEEP_MAX);
    }

    #[test]
    fn an_image_the_html_shows_is_part_of_the_body_however_it_was_sent() {
        // Some mailers send the logos their HTML shows as attachments, or with no disposition.
        let raw = mail()
            .html("<img src=\"cid:logo.png\">")
            .attach("logo.png", "image/png", "PNG")
            .attach("photo.png", "image/png", "PNG")
            .build();
        let shown: Vec<(bool, bool)> =
            parse(&raw).unwrap().attachments.iter().map(|a| (a.inline, a.kept.is_some())).collect();
        assert_eq!(shown, [(true, true), (false, false)]);

        let related = b"From: a@x.co\r\nContent-Type: multipart/related; boundary=r\r\n\r\n--r\r\n\
            Content-Type: text/html\r\n\r\n<img src=\"cid:files/i1.PNG\">\r\n--r\r\n\
            Content-Type: image/png; name=i1.PNG\r\nContent-ID: <files/i1.PNG>\r\n\r\nPNG\r\n--r--\r\n";
        let image = &parse(related).unwrap().attachments[0];
        assert!(image.inline && image.kept.is_some());
    }

    #[test]
    fn turns_an_html_only_email_into_text() {
        let raw = b"From: a@x.co\r\nSubject: Hi\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>there</b></p>\r\n";
        let parsed = parse(raw).unwrap();
        assert!(parsed.body_text.contains("Hello there"));
        assert!(parsed.body_html.unwrap().contains("<b>there</b>"));
    }

    #[test]
    fn names_attachments_without_names_or_types() {
        let raw = b"From: a@x.co\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b\r\nContent-Disposition: attachment\r\nContent-Type: application\r\n\r\nzzz\r\n--b--\r\n";
        let parsed = parse(raw).unwrap();
        assert_eq!(parsed.attachments[0].name, "attachment");
        assert_eq!(parsed.attachments[0].content_type, "application");
    }

    #[test]
    fn spots_calendar_invites_lists_and_robots() {
        let invite =
            mail().attach("invite.ics", "application/octet-stream", "BEGIN:VCALENDAR").build();
        assert!(parse(&invite).unwrap().has_calendar);
        let calendar_part = mail().attach("x", "text/calendar", "BEGIN:VCALENDAR").build();
        assert!(parse(&calendar_part).unwrap().has_calendar);
        assert!(parse(&mail().header("List-Id", "<news.x.co>").build()).unwrap().is_list);
        assert!(
            parse(&mail().header("List-Unsubscribe", "<mailto:u@x.co>").build()).unwrap().is_list
        );
        assert!(
            parse(&mail().header("Auto-Submitted", "auto-generated").build()).unwrap().is_automated
        );
        assert!(!parse(&mail().header("Auto-Submitted", "No").build()).unwrap().is_automated);
        assert!(parse(&mail().header("Precedence", "Bulk").build()).unwrap().is_automated);
        assert!(!parse(&mail().header("Precedence", "first-class").build()).unwrap().is_automated);
    }

    #[test]
    fn participants_for_threading_and_search() {
        let parsed = parse(
            &mail()
                .to("Maya Ross <Maya@Northwind.co>, daniel@northwind.co")
                .header("Bcc", "x@y.co")
                .build(),
        )
        .unwrap();
        assert_eq!(parsed.participants_key(), "daniel@northwind.co,maya@northwind.co");
        assert_eq!(
            parsed.participants_text(),
            "Daniel Okafor daniel@northwind.co Maya Ross Maya@Northwind.co daniel@northwind.co x@y.co"
        );
    }

    #[test]
    fn strips_the_quoted_reply_for_search() {
        let parsed =
            parse(&mail().body("Signed.\n\nOn Sun, Daniel wrote:\n> Can you?").build()).unwrap();
        assert_eq!(parsed.body_stripped(), "Signed.");
    }
}
