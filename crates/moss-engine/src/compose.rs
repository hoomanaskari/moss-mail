//! Turning a draft from the composer into the email that goes out: addresses, threading
//! headers, the signature, and the quoted message underneath a reply or forward.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use moss_core::{ComposeMode, Draft, DraftFile, Message, Person};
use moss_providers::{Address, OutgoingMessage};
use moss_store::ReplyHeaders;

/// What the composer sends. Mirrors the UI's `Draft`; layout fields are ignored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingDraft {
    pub mode: ComposeMode,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub from_account_id: String,
    pub to: Vec<Person>,
    #[serde(default)]
    pub cc: Vec<Person>,
    #[serde(default)]
    pub bcc: Vec<Person>,
    pub subject: String,
    /// Plain text from the editor.
    pub body: String,
    /// Email-ready HTML from the editor.
    pub body_html: String,
    /// The lines of the signature chosen in the composer; empty for none.
    #[serde(default)]
    pub signature: Vec<String>,
    /// Which of the account's addresses it goes from; `None` for the account's own.
    #[serde(default)]
    pub from_address: Option<String>,
    /// Bring it back if no one answers (see [`crate::sort::WAIT`]).
    #[serde(default)]
    pub remind_if_no_reply: bool,
    /// Files you attached, and the forwarded email's files you kept, in order.
    #[serde(default)]
    pub attachments: Vec<DraftFile>,
}

fn address(person: &Person) -> Address {
    let name = person.name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(&person.email) {
        Address::new(person.email.trim())
    } else {
        Address::named(name, person.email.trim())
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Plain text as HTML, keeping line breaks.
pub fn text_to_html(text: &str) -> String {
    escape(text).replace("\r\n", "\n").replace('\n', "<br>")
}

/// Where a signature line links to, when it is a web address ("northwind.co"). Same rule as
/// `linkFor` in the UI, so the email links what the composer showed as a link.
fn link(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let scheme = ["https://", "http://"].into_iter().find(|s| lower.starts_with(s));
    let rest = &line[scheme.map_or(0, str::len)..];
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let label =
        |l: &str| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || !labels.iter().all(|l| label(l)) || path.chars().any(char::is_whitespace) {
        return None;
    }
    Some(if scheme.is_some() { line.to_string() } else { format!("https://{line}") })
}

/// The signature as plain text and HTML. Plain text opens with the usual "-- " line, so mail
/// apps (Moss too) know where it starts; the HTML is marked the way Gmail marks its own.
pub(crate) fn signature(lines: &[String]) -> Option<(String, String)> {
    if lines.iter().all(|l| l.trim().is_empty()) {
        return None;
    }
    let html: Vec<String> = lines
        .iter()
        .map(|l| match link(l) {
            Some(href) => format!("<a href=\"{}\">{}</a>", escape(&href), escape(l)),
            None => escape(l),
        })
        .collect();
    Some((
        format!("-- \n{}", lines.join("\n")),
        format!("<div class=\"gmail_signature\">{}</div>", html.join("<br>")),
    ))
}

/// The saved signature `body` already ends with, by its id, and the text before it. Exact: each
/// of the signature's lines, in order, as the text's last lines (spaces and blank lines aside,
/// and a web address as a plain-text link "northwind.co <https://northwind.co>"), with the "-- "
/// line above it taken off too. The longest one wins.
pub(crate) fn ending_signature(signatures: &[moss_core::Signature], body: &str) -> Option<(String, String)> {
    let words = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");
    let lines: Vec<&str> = body.lines().collect();
    // The text's lines that aren't blank, last first, with where each one is.
    let filled: Vec<(usize, String)> =
        lines.iter().enumerate().map(|(i, l)| (i, words(l))).filter(|(_, l)| !l.is_empty()).rev().collect();
    let same = |want: &String, got: &String| got == want || link(want).is_some_and(|href| *got == format!("{want} <{href}>"));
    let mut best: Option<(&moss_core::Signature, usize, usize)> = None;
    for sig in signatures {
        let want: Vec<String> = sig.lines.iter().map(|l| words(l)).filter(|l| !l.is_empty()).collect();
        if want.is_empty() || want.len() > filled.len() || best.is_some_and(|(_, n, _)| n >= want.len()) {
            continue;
        }
        let tail = &filled[..want.len()];
        if want.iter().rev().zip(tail).all(|(w, (_, got))| same(w, got)) {
            best = Some((sig, want.len(), tail[want.len() - 1].0));
        }
    }
    let (sig, _, start) = best?;
    let mut rest = &lines[..start];
    let blank_end = |rest: &mut &[&str]| {
        while rest.last().is_some_and(|l| l.trim().is_empty()) {
            *rest = &rest[..rest.len() - 1];
        }
    };
    blank_end(&mut rest);
    if rest.last().is_some_and(|l| l.trim() == "--") {
        rest = &rest[..rest.len() - 1];
        blank_end(&mut rest);
    }
    Some((sig.id.clone(), rest.join("\n")))
}

fn when(ms: i64, tz: FixedOffset) -> String {
    DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .with_timezone(&tz)
        .format("%a, %-d %b %Y at %H:%M")
        .to_string()
}

fn who(person: &Person) -> String {
    if person.name.is_empty() || person.name == person.email {
        format!("<{}>", person.email)
    } else {
        format!("{} <{}>", person.name, person.email)
    }
}

/// The borrowed fields needed to render a reply or forward quote.
pub(crate) struct QuoteSource<'a> {
    pub from: &'a Person,
    pub to: &'a [Person],
    pub subject: &'a str,
    pub sent_at: i64,
    pub body_text: &'a str,
    pub body_html: Option<&'a str>,
}

pub(crate) fn quote(mode: ComposeMode, original: &Message, tz: FixedOffset) -> (String, String) {
    quote_source(mode, QuoteSource {
        from:&original.from, to:&original.to, subject:&original.subject, sent_at:original.sent_at,
        body_text:&original.body_text, body_html:original.body_html.as_deref(),
    }, tz)
}

pub(crate) fn quote_source(mode: ComposeMode, original: QuoteSource<'_>, tz: FixedOffset) -> (String, String) {
    let original_html =
        original.body_html.map(str::to_owned).unwrap_or_else(|| text_to_html(original.body_text));
    if mode == ComposeMode::Forward {
        let to: Vec<String> = original.to.iter().map(who).collect();
        let head = format!(
            "---------- Forwarded message ---------\nFrom: {}\nDate: {}\nSubject: {}\nTo: {}",
            who(&original.from),
            when(original.sent_at, tz),
            original.subject,
            to.join(", ")
        );
        let text = format!("{head}\n\n{}", original.body_text);
        let html = format!("<div>{}</div><br>{original_html}", text_to_html(&head));
        return (text, html);
    }
    let line = format!("On {}, {} wrote:", when(original.sent_at, tz), who(&original.from));
    let mut text = format!("{line}\n");
    for (index, line) in original.body_text.lines().enumerate() {
        if index > 0 { text.push('\n'); }
        text.push_str("> ");
        text.push_str(line);
    }
    let html = format!(
        "<div>{}</div><blockquote style=\"margin:0 0 0 0.8ex;border-left:1px solid #ccc;padding-left:1ex\">{original_html}</blockquote>",
        escape(&line)
    );
    (text, html)
}

/// The email to send, from `from`. `original` is the message replied to or forwarded,
/// `headers` link a reply into its conversation.
pub fn outgoing(
    draft: &OutgoingDraft,
    from: &Person,
    headers: Option<ReplyHeaders>,
    original: Option<&Message>,
    tz: FixedOffset,
) -> OutgoingMessage {
    let (mut text, mut html) = (draft.body.clone(), draft.body_html.clone());
    // Under the body and above any quote, where the composer shows it.
    if let Some((signature_text, signature_html)) = signature(&draft.signature) {
        text = format!("{text}\n\n{signature_text}");
        html = format!("{html}<br>{signature_html}");
    }
    if let Some(original) = original.filter(|_| draft.mode != ComposeMode::New) {
        let (quoted_text, quoted_html) = quote(draft.mode, original, tz);
        text = format!("{text}\n\n{quoted_text}");
        html = format!("{html}<br>{quoted_html}");
    }
    let replying = matches!(draft.mode, ComposeMode::Reply | ComposeMode::ReplyAll);
    let headers = headers
        .filter(|_| replying)
        .unwrap_or(ReplyHeaders { in_reply_to: None, references: vec![] });
    OutgoingMessage {
        from: address(from),
        to: draft.to.iter().map(address).collect(),
        cc: draft.cc.iter().map(address).collect(),
        bcc: draft.bcc.iter().map(address).collect(),
        subject: draft.subject.trim().to_string(),
        text: Some(text),
        html: Some(html),
        in_reply_to: headers.in_reply_to,
        references: headers.references,
        ..Default::default()
    }
}

impl From<&Draft> for OutgoingDraft {
    /// What you wrote, without a signature: the server copy of a draft.
    fn from(d: &Draft) -> Self {
        OutgoingDraft {
            mode: d.mode,
            thread_id: d.thread_id.clone(),
            from_account_id: d.from_account_id.clone(),
            to: d.to.clone(),
            cc: d.cc.clone(),
            bcc: d.bcc.clone(),
            subject: d.subject.clone(),
            body: d.body.clone(),
            body_html: d.body_html.clone(),
            signature: vec![],
            from_address: d.from_address.clone(),
            attachments: d.attachments.clone(),
            remind_if_no_reply: d.remind_if_no_reply,
        }
    }
}

/// A draft's copy for the server's Drafts folder: built like the email it becomes (so
/// anything a sent email carries, such as attachments, goes here too), but only what you
/// wrote — no signature or quote, which Moss adds when it sends — and marked with the
/// draft it is, so Moss knows it again.
pub fn draft_copy(draft: &Draft, from: &Person, headers: Option<ReplyHeaders>) -> OutgoingMessage {
    let tz = FixedOffset::east_opt(0).expect("UTC");
    let mut message = outgoing(&OutgoingDraft::from(draft), from, headers, None, tz);
    message.headers = crate::drafts::moss_headers(draft);
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, email: &str) -> Person {
        Person { name: name.into(), email: email.into() }
    }

    fn maya() -> Person {
        person("Maya Ross", "maya@gmail.com")
    }

    fn draft(mode: ComposeMode) -> OutgoingDraft {
        OutgoingDraft {
            mode,
            thread_id: Some("7".into()),
            from_account_id: "gmail".into(),
            to: vec![person("Daniel Okafor", "daniel@northwind.co")],
            cc: vec![person("lena@northwind.co", "lena@northwind.co")],
            bcc: vec![person("", "me@x.io")],
            subject: " Re: Q4 budget ".into(),
            body: "Approved.".into(),
            body_html: "<p>Approved.</p>".into(),
            signature: vec![],
            from_address: None,
            remind_if_no_reply: false,
            attachments: vec![],
        }
    }

    fn signed(mode: ComposeMode) -> OutgoingDraft {
        let lines = ["Maya Ross", "Design lead, Northwind & Co", "northwind.co"];
        OutgoingDraft { signature: lines.map(String::from).to_vec(), ..draft(mode) }
    }

    const SIGNATURE_TEXT: &str = "-- \nMaya Ross\nDesign lead, Northwind & Co\nnorthwind.co";
    const SIGNATURE_HTML: &str = "<div class=\"gmail_signature\">Maya Ross<br>Design lead, Northwind &amp; Co<br><a href=\"https://northwind.co\">northwind.co</a></div>";

    fn original(html: Option<&str>) -> Message {
        Message {
            id: "1".into(),
            thread_id: "7".into(),
            from: person("Daniel Okafor", "daniel@northwind.co"),
            to: vec![person("Maya Ross", "maya@gmail.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Q4 budget".into(),
            sent_at: 1_793_524_320_000,
            paragraphs: vec![],
            body_text: "Can you sign off?\nThanks".into(),
            body_html: html.map(String::from),
            attachments: vec![],
            read: true,
            image_trust: Default::default(),
            sender_unverified: false,
            headers_only: false,
            images_missing: false,
            forwards: vec![],
            forwarded: false,
        }
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn headers() -> ReplyHeaders {
        ReplyHeaders {
            in_reply_to: Some("q4@x".into()),
            references: vec!["a@x".into(), "q4@x".into()],
        }
    }

    #[test]
    fn quoted_plain_text_preserves_empty_lines_crlf_and_unicode() {
        for (body,expected) in [("",""),("\n","> "),("a\r\nb\n\n","> a\n> b\n> "),("é\n<&","> é\n> <&")] {
            let mut message=original(None);
            message.body_text=body.into();
            for mode in [ComposeMode::Reply,ComposeMode::ReplyAll] {
                let (text,_)=quote(mode,&message,utc());
                assert_eq!(text.split_once("wrote:\n").unwrap().1,expected);
            }
        }
    }

    #[test]
    fn the_composer_draft_deserialises_from_the_ui_shape() {
        let json = serde_json::json!({
            "mode": "reply-all", "threadId": "7", "fromAccountId": "gmail",
            "to": [{"name": "Daniel", "email": "d@x.io", "initials": "D"}],
            "subject": "Re: x", "body": "Hi", "bodyHtml": "<p>Hi</p>", "fieldsOpen": true
        });
        let draft: OutgoingDraft = serde_json::from_value(json).unwrap();
        assert_eq!(draft.mode, ComposeMode::ReplyAll);
        assert!(draft.cc.is_empty() && draft.bcc.is_empty() && draft.signature.is_empty());
        assert!(draft.attachments.is_empty());
    }

    #[test]
    fn a_new_email_ends_with_its_signature() {
        let out = outgoing(&signed(ComposeMode::New), &maya(), None, None, utc());
        assert_eq!(out.text.unwrap(), format!("Approved.\n\n{SIGNATURE_TEXT}"));
        assert_eq!(out.html.unwrap(), format!("<p>Approved.</p><br>{SIGNATURE_HTML}"));
    }

    #[test]
    fn a_reply_or_forward_has_its_signature_above_the_quote() {
        for mode in [ComposeMode::Reply, ComposeMode::ReplyAll, ComposeMode::Forward] {
            let out = outgoing(&signed(mode), &maya(), None, Some(&original(None)), utc());
            let (text, html) = (out.text.unwrap(), out.html.unwrap());
            assert!(text.starts_with(&format!("Approved.\n\n{SIGNATURE_TEXT}\n\n")), "{text}");
            assert!(html.starts_with(&format!("<p>Approved.</p><br>{SIGNATURE_HTML}<br>")), "{html}");
            assert!(text.contains("Can you sign off?") && html.contains("Can you sign off?"));
        }
    }

    #[test]
    fn no_signature_or_a_blank_one_adds_nothing() {
        let blank = OutgoingDraft { signature: vec![" ".into(), String::new()], ..draft(ComposeMode::New) };
        for d in [draft(ComposeMode::New), blank] {
            let out = outgoing(&d, &maya(), None, None, utc());
            assert_eq!(out.text.as_deref(), Some("Approved."));
            assert_eq!(out.html.as_deref(), Some("<p>Approved.</p>"));
        }
    }

    // Same cases as `linkFor` in src/state/signatures.ts, which links them in the composer.
    #[test]
    fn signature_lines_that_are_web_addresses_become_links() {
        assert_eq!(link("northwind.co").as_deref(), Some("https://northwind.co"));
        assert_eq!(link("www.northwind.co/team?x=1").as_deref(), Some("https://www.northwind.co/team?x=1"));
        assert_eq!(link("https://maya.design/work").as_deref(), Some("https://maya.design/work"));
        assert_eq!(link("HTTP://X.IO").as_deref(), Some("HTTP://X.IO"));
        assert_eq!(link("ross_studio-1.art").as_deref(), Some("https://ross_studio-1.art"));
        let plain = [
            "Maya Ross",
            "Design lead, Northwind",
            "maya@northwind.co",
            "northwind",
            "northwind.co is great",
            "ftp://x.io",
            "a..b",
            "x.io/a b",
            ".io",
            "",
        ];
        for line in plain {
            assert_eq!(link(line), None, "{line}");
        }
    }

    #[test]
    fn a_reply_threads_and_quotes_the_message_it_answers() {
        let out = outgoing(
            &draft(ComposeMode::Reply),
            &maya(),
            Some(headers()),
            Some(&original(None)),
            utc(),
        );
        assert_eq!(out.from, Address::named("Maya Ross", "maya@gmail.com"));
        assert_eq!(out.to, [Address::named("Daniel Okafor", "daniel@northwind.co")]);
        assert_eq!(out.cc, [Address::new("lena@northwind.co")]);
        assert_eq!(out.bcc, [Address::new("me@x.io")]);
        assert_eq!(out.subject, "Re: Q4 budget");
        assert_eq!(out.in_reply_to.as_deref(), Some("q4@x"));
        assert_eq!(out.references, ["a@x", "q4@x"]);
        let text = out.text.unwrap();
        assert!(text.starts_with("Approved.\n\nOn Sun, 1 Nov 2026 at 09:12, Daniel Okafor <daniel@northwind.co> wrote:\n> Can you sign off?\n> Thanks"), "{text}");
        let html = out.html.unwrap();
        assert!(html.starts_with("<p>Approved.</p><br><div>On Sun"), "{html}");
        assert!(html.contains("Daniel Okafor &lt;daniel@northwind.co&gt; wrote:"));
        assert!(html.contains("<blockquote") && html.contains("Can you sign off?<br>Thanks"));
    }

    #[test]
    fn a_reply_keeps_the_original_html() {
        let out = outgoing(
            &draft(ComposeMode::ReplyAll),
            &maya(),
            Some(headers()),
            Some(&original(Some("<b>Sign</b>"))),
            utc(),
        );
        assert!(out.html.unwrap().contains("<b>Sign</b></blockquote>"));
    }

    #[test]
    fn a_forward_starts_a_new_conversation_with_the_original_below() {
        let mut o = original(None);
        o.from = person("", "daniel@northwind.co");
        let out =
            outgoing(&draft(ComposeMode::Forward), &maya(), Some(headers()), Some(&o), utc());
        assert_eq!(out.in_reply_to, None);
        assert!(out.references.is_empty());
        let text = out.text.unwrap();
        assert!(text.contains("---------- Forwarded message ---------\nFrom: <daniel@northwind.co>\nDate: Sun, 1 Nov 2026 at 09:12\nSubject: Q4 budget\nTo: Maya Ross <maya@gmail.com>\n\nCan you sign off?"), "{text}");
        assert!(out
            .html
            .unwrap()
            .contains("Forwarded message ---------<br>From: &lt;daniel@northwind.co&gt;"));
    }

    #[test]
    fn a_new_email_has_no_quote_or_threading() {
        let unnamed = person("", "maya@gmail.com");
        let out = outgoing(
            &draft(ComposeMode::New),
            &unnamed,
            Some(headers()),
            Some(&original(None)),
            utc(),
        );
        assert_eq!(out.from, Address::new("maya@gmail.com"));
        assert_eq!(out.text.as_deref(), Some("Approved."));
        assert_eq!(out.html.as_deref(), Some("<p>Approved.</p>"));
        assert_eq!(out.in_reply_to, None);
        let none = outgoing(&draft(ComposeMode::Reply), &unnamed, None, None, utc());
        assert_eq!(none.in_reply_to, None);
    }

    fn saved(id: &str, lines: &[&str]) -> moss_core::Signature {
        moss_core::Signature { id: id.into(), name: id.into(), lines: lines.iter().map(|l| l.to_string()).collect() }
    }

    fn saved_signatures() -> Vec<moss_core::Signature> {
        vec![
            saved("short", &["Maya"]),
            saved("personal", &["Maya Ross"]),
            saved("full", &["Maya Ross", "Design lead, Northwind", "northwind.co"]),
            saved("blank", &[" ", ""]),
        ]
    }

    fn ending(body: &str) -> Option<(String, String)> {
        ending_signature(&saved_signatures(), body)
    }

    #[test]
    fn a_saved_signature_at_the_end_of_the_text_is_found_and_taken_off() {
        let found = |id: &str, rest: &str| Some((id.to_string(), rest.to_string()));
        // As Moss writes it: the "-- " line, then the lines.
        let (text, _) = signature(&saved_signatures()[2].lines).unwrap();
        assert_eq!(ending(&format!("Friday works.\n\n{text}")), found("full", "Friday works."));
        // As other apps leave it: no delimiter, Windows line ends, odd spaces, blank lines.
        let messy = "Hi Daniel,\r\n\r\nFriday works.\r\n\r\n--\r\nMaya  Ross \r\n\r\nDesign lead,\u{a0}Northwind\r\nnorthwind.co\r\n\r\n";
        assert_eq!(ending(messy), found("full", "Hi Daniel,\n\nFriday works."));
        // A web address as a plain-text copy of a link writes it.
        let linked = "Friday works.\nMaya Ross\nDesign lead, Northwind\nnorthwind.co <https://northwind.co>";
        assert_eq!(ending(linked), found("full", "Friday works."));
        // The longest one that fits; just your name is a one-line signature too.
        assert_eq!(ending("Friday works.\n\nMaya Ross"), found("personal", "Friday works."));
        assert_eq!(ending("Friday works.\nMaya"), found("short", "Friday works."));
        assert_eq!(ending("Maya Ross"), found("personal", ""));
    }

    #[test]
    fn only_a_whole_saved_signature_at_the_very_end_counts() {
        for body in [
            "",
            "Friday works.",
            "Friday works.\nMaya Ross\nSee you then.",
            "Friday works.\nLove, Maya Ross",
            "Friday works.\nmaya ross",
            "Friday works.\nDesign lead, Northwind\nnorthwind.co",
            "Friday works.\nMaya Ross\nDesign lead, Northwind\nnorthwind.co <https://elsewhere.co>",
            " \n\n",
        ] {
            assert_eq!(ending(body), None, "{body:?}");
        }
        assert_eq!(ending_signature(&[], "Friday works.\nMaya"), None);
    }

    #[test]
    fn text_becomes_safe_html() {
        assert_eq!(text_to_html("a < b & \"c\"\r\nd"), "a &lt; b &amp; &quot;c&quot;<br>d");
        assert_eq!(when(i64::MAX, utc()), when(0, utc()));
    }
}
