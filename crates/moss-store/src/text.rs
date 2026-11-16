//! Text helpers: the new part of a message body, previews, subject keys and search queries.

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// Words that open a reply attribution, e.g. "On Sun, 1 Nov … wrote:" or "Am 1. Nov … schrieb …:".
const ATTRIBUTION_OPENERS: &[&str] = &["On ", "Am ", "Le ", "El ", "Il ", "Op ", "Den "];
const ATTRIBUTION_VERBS: &[&str] =
    &["wrote", "schrieb", "a écrit", "escribió", "scrisse", "schreef", "skrev"];
/// Lines a phone or mail app appends on its own.
const DEVICE_SIGNATURES: &[&str] = &["Sent from my ", "Get Outlook for "];

fn is_attribution(line: &str) -> bool {
    let line = line.trim();
    ATTRIBUTION_OPENERS.iter().any(|w| line.starts_with(w))
        && line.ends_with(':')
        && ATTRIBUTION_VERBS.iter().any(|w| line.contains(w))
}

/// Where the quoted email starts: a reply attribution, an Outlook header block or an
/// "Original Message" divider.
fn starts_quote(lines: &[&str], i: usize) -> bool {
    let line = lines[i].trim();
    let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or_default();
    if is_attribution(line) || is_attribution(&format!("{line} {next}")) {
        return true;
    }
    if line.starts_with("-----") && line.contains("Original Message") {
        return true;
    }
    if line.len() >= 10 && line.chars().all(|c| c == '_') {
        return next.starts_with("From:");
    }
    line.starts_with("From:")
        && lines[i + 1..].iter().take(4).any(|l| l.starts_with("Sent:") || l.starts_with("Date:"))
}

/// The new text of a message: no quoted lines, reply headers ("On … wrote:") or signature.
/// When nothing is left (a bare forward, say) the whole body is kept so it can still be found.
pub fn strip_quoted(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut kept: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_end() == "--" || starts_quote(&lines, i) {
            break;
        }
        let quoted = line.trim_start().starts_with('>');
        let device = DEVICE_SIGNATURES.iter().any(|s| line.trim().starts_with(s));
        let repeated_blank =
            line.trim().is_empty() && kept.last().is_some_and(|l| l.trim().is_empty());
        if !(quoted || device || repeated_blank) {
            kept.push(line);
        }
    }
    let text = kept.join("\n").trim().to_string();
    if text.is_empty() {
        body.trim().replace("\r\n", "\n")
    } else {
        text
    }
}

/// Paragraphs separated by blank lines. Lines inside a paragraph keep their line breaks.
pub fn paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines().chain([""]) {
        if line.trim().is_empty() {
            if !current.is_empty() {
                out.push(current.join("\n").trim().to_string());
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    out
}

/// One-line preview, whitespace collapsed, cut to `max` characters with an ellipsis.
/// Leaves out what an HTML email's text keeps that no one reads: stray tags, link addresses,
/// pictures (`[logo]`, `[cid:…]`), divider lines and the invisible padding after a preheader.
pub fn snippet(text: &str, max: usize) -> String {
    let flat = readable(text).split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// `[words](address)` keeps its words; any other `[…]` is a picture and goes. Innermost first,
/// so a link around a logo goes too.
static BRACKETED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\[\]]*)\](\((?:https?://|mailto:)[^)\s]*\))?").expect("compiles"));
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:https?://|www\.)\S+").expect("compiles"));
/// A divider: three or more of the same line character, like `-----` or `====`.
static DIVIDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-{3,}|={3,}|_{3,}|\*{3,}|~{3,}|#{3,}").expect("compiles"));
/// An HTML tag, but not `<jane@x.co>` or `3 < 4`.
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?[A-Za-z][A-Za-z0-9-]*(?:\s[^<>@]*)?/?>").expect("compiles"));

fn invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{34f}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}')
}

fn readable(text: &str) -> String {
    let mut text: String = text.chars().filter(|&c| !invisible(c)).collect();
    loop {
        let next = BRACKETED.replace_all(&text, |c: &Captures| match c.get(2) {
            Some(_) => c[1].to_string(),
            None => " ".to_string(),
        });
        if next == text {
            break;
        }
        text = next.into_owned();
    }
    let text = TAG.replace_all(&text, " ");
    let text = ADDRESS.replace_all(&text, " ");
    DIVIDER.replace_all(&text, " ").into_owned()
}

/// Subject without "Re:" / "Fwd:" / "[list]" prefixes, for display.
pub fn base_subject(subject: &str) -> String {
    mail_parser::thread_name(subject).trim().to_string()
}

/// Key for grouping messages by subject when headers don't link them: base subject,
/// lower-case, whitespace collapsed.
pub fn subject_key(subject: &str) -> String {
    base_subject(subject).split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Turns what someone typed into an FTS5 query: every word must match, the last one as a
/// prefix while they are still typing it. `None` when there is nothing to search for.
pub fn fts_query(input: &str) -> Option<String> {
    let words: Vec<String> = input
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut query = words.join(" ");
    if !input.ends_with(char::is_whitespace) {
        query.push('*');
    }
    Some(query)
}

/// HTML as text with a line for each block, as a mail app shows it: Gmail web writes each line
/// of a draft as a `<div>`. Entities are decoded and spaces collapsed; comments, styles and
/// scripts are left out. (The parser's own conversion only breaks lines at `<br>` and `</p>`.)
pub fn html_lines(html: &str) -> String {
    const BLOCKS: &[&str] = &[
        "div", "p", "li", "ul", "ol", "tr", "table", "blockquote", "h1", "h2", "h3", "h4", "h5", "h6",
    ];
    const HIDDEN: &[&str] = &["head", "style", "script", "template", "title"];
    // Where a block starts or ends: a line break, unless the line is empty anyway.
    const MARK: char = '\u{E000}';
    let mut out = String::with_capacity(html.len());
    let mut hidden: Option<String> = None;
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |i| &after[i + 3..]);
            continue;
        }
        let tag = rest.strip_prefix('<').filter(|t| t.starts_with(|c: char| c == '/' || c.is_ascii_alphabetic()));
        if let Some(tag) = tag {
            let Some(end) = tag.find('>') else { break };
            let closing = tag.starts_with('/');
            let name = tag.trim_start_matches('/').chars().take_while(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
            rest = &tag[end + 1..];
            if let Some(h) = &hidden {
                if closing && *h == name {
                    hidden = None;
                }
                continue;
            }
            match name.as_str() {
                n if HIDDEN.contains(&n) => hidden = (!closing).then_some(name),
                "br" => out.push('\n'),
                // A paragraph leaves a blank line after it.
                "p" if closing => out.extend([MARK, '\n']),
                n if BLOCKS.contains(&n) => out.push(MARK),
                "td" | "th" => out.push(' '),
                _ => {}
            }
            continue;
        }
        let next = rest[1..].find('<').map_or(rest.len(), |i| i + 1);
        let (text, after) = rest.split_at(next);
        rest = after;
        if hidden.is_none() {
            push_text(&mut out, text);
        }
    }
    let mut text = String::new();
    for c in out.chars() {
        match c {
            MARK if text.is_empty() || text.ends_with('\n') => {}
            MARK => text.push('\n'),
            c => text.push(c),
        }
    }
    let lines: Vec<String> = text.split('\n').map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    lines.join("\n").trim_matches('\n').to_string()
}

/// Text between tags: its line breaks are spaces, its entities ("&amp;", "&#233;") decoded.
fn push_text(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp].replace(['\r', '\n', '\t'], " "));
        let entity = rest[amp..].find(';').filter(|&i| i <= 32).map(|i| &rest[amp..amp + i + 1]);
        let entity = entity.filter(|e| !e.contains(char::is_whitespace)).unwrap_or("&");
        mail_parser::decoders::html::add_html_token(out, entity.as_bytes(), false);
        rest = &rest[amp + entity.len()..];
    }
    out.push_str(&rest.replace(['\r', '\n', '\t'], " "));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_blocks_become_lines() {
        // Gmail web: a <div> a line, <div><br></div> a blank one.
        let gmail = "<div dir=\"ltr\">Hi Daniel,<div><br></div><div>Friday &amp; Saturday work.</div><div><br></div>\
                     <div>-- </div><div class=\"gmail_signature\"><div>Maya  Ross</div><DIV>Design lead</DIV></div></div>";
        assert_eq!(html_lines(gmail), "Hi Daniel,\n\nFriday & Saturday work.\n\n--\nMaya Ross\nDesign lead");
        assert_eq!(html_lines("<p>One</p><p>Two<br>Three</p>"), "One\n\nTwo\nThree");
        assert_eq!(html_lines("<ul><li>One</li><li>Two</li></ul><table><tr><td>A</td><td>B</td></tr><tr><td>C</td></tr></table>"), "One\nTwo\nA B\nC");
        // Inline tags stay on the line.
        assert_eq!(html_lines("<span>a</span><span>b</span> and <b>1 &lt; 2</b>"), "ab and 1 < 2");
        assert_eq!(html_lines(""), "");
        // A bold name keeps its space; source line breaks are spaces.
        assert_eq!(html_lines("<div><b>Maya</b> Ross</div><div>Design\r\nlead</div>"), "Maya Ross\nDesign lead");
        // Comments, styles and scripts aren't text.
        assert_eq!(html_lines("<head><title>T</title></head><style>p{}</style>a<!-- b --><script>c</script>d<!-- e"), "ad");
        // Entities: named, numbered, unknown, and an "&" that isn't one.
        assert_eq!(html_lines("&eacute;&#233;&#x41; &nope; R&D &amp"), "\u{e9}\u{e9}A &nope; R&D &amp");
        // A "<" that isn't a tag is text; a tag that never ends is dropped.
        assert_eq!(html_lines("1 < 2 <3</SPAN> <b class=\"x\""), "1 < 2 <3");
    }

    #[test]
    fn keeps_a_plain_body() {
        assert_eq!(
            strip_quoted("Hi Maya,\r\n\r\nCan you sign off?\r\n"),
            "Hi Maya,\n\nCan you sign off?"
        );
    }

    #[test]
    fn drops_gmail_style_quotes() {
        let body = "Signed, thanks.\n\nOn Sun, 1 Nov 2026 at 09:12, Daniel <daniel@northwind.co> wrote:\n> Can you sign off?\n> Thanks\n";
        assert_eq!(strip_quoted(body), "Signed, thanks.");
    }

    #[test]
    fn drops_an_attribution_wrapped_over_two_lines() {
        let body = "Yes.\n\nOn Sun, 1 Nov 2026 at 09:12, Daniel Okafor\n<daniel@northwind.co> wrote:\n> Can you?\n";
        assert_eq!(strip_quoted(body), "Yes.");
    }

    #[test]
    fn drops_attributions_in_other_languages() {
        assert_eq!(
            strip_quoted("Ja.\n\nAm 1. Nov. 2026 schrieb Jonas <j@x.de>:\n> Geht das?"),
            "Ja."
        );
        assert_eq!(strip_quoted("Oui.\nLe 1 nov. 2026, Ines <i@x.pt> a écrit :\n> Ok ?"), "Oui.");
    }

    #[test]
    fn keeps_a_sentence_that_happens_to_end_in_wrote() {
        assert_eq!(
            strip_quoted("Here is what she wrote:\nall good"),
            "Here is what she wrote:\nall good"
        );
    }

    #[test]
    fn keeps_inline_replies_but_not_the_quoted_lines() {
        let body = "> Can you sign off?\nYes.\n> And the slides?\nTomorrow.";
        assert_eq!(strip_quoted(body), "Yes.\nTomorrow.");
    }

    #[test]
    fn drops_outlook_style_headers() {
        let body = "Done.\n\n________________________________\nFrom: Daniel Okafor\nSent: Sunday, 1 November 2026 09:12\nTo: Maya\nSubject: Q4\n\nCan you?";
        assert_eq!(strip_quoted(body), "Done.");
        let bare = "Done.\n\nFrom: Daniel Okafor <d@x.co>\nDate: Sunday, 1 November\nSubject: Q4\n\nCan you?";
        assert_eq!(strip_quoted(bare), "Done.");
        let original = "Done.\n-----Original Message-----\nFrom: Daniel\nCan you?";
        assert_eq!(strip_quoted(original), "Done.");
    }

    #[test]
    fn keeps_a_from_line_that_is_not_a_header_block() {
        let body = "From: the team\nWelcome aboard.";
        assert_eq!(strip_quoted(body), body);
        assert_eq!(strip_quoted("Hi\n__________\nnotes"), "Hi\n__________\nnotes");
    }

    #[test]
    fn drops_signatures() {
        assert_eq!(strip_quoted("Thanks!\n-- \nMaya Ross\nNorthwind"), "Thanks!");
        assert_eq!(strip_quoted("Thanks!\n--\nMaya"), "Thanks!");
        assert_eq!(strip_quoted("Thanks!\n\nSent from my iPhone"), "Thanks!");
        assert_eq!(strip_quoted("Thanks!\n\nGet Outlook for iOS"), "Thanks!");
    }

    #[test]
    fn keeps_the_whole_body_when_everything_is_quoted() {
        assert_eq!(strip_quoted("> only a quote\n"), "> only a quote");
        assert_eq!(strip_quoted("  \n"), "");
    }

    #[test]
    fn splits_paragraphs_on_blank_lines() {
        assert_eq!(
            paragraphs("Hi Maya,\n\nLine one\nline two\n \n\n  Bye  \n"),
            vec!["Hi Maya,", "Line one\nline two", "Bye"]
        );
        assert!(paragraphs("").is_empty());
    }

    #[test]
    fn snippets_collapse_whitespace_and_cut_on_characters() {
        assert_eq!(snippet("  Can you\n\nsign   off? ", 40), "Can you sign off?");
        assert_eq!(snippet("Café au lait", 5), "Café…");
        assert_eq!(snippet("Hello world", 6), "Hello…");
        assert_eq!(snippet("Hello", 5), "Hello");
    }

    #[test]
    fn snippets_leave_out_what_the_html_and_preheader_left_behind() {
        // Ledgerly: a stray tag, a run of invisible padding, a logo inside a link.
        let filler = "\u{34f} ".repeat(80);
        let text = format!(
            "<body></body>{filler}\n[ [Ledgerly] [Ledgerly] ](https://clicks.example.com/f/a/Zq7~~/BBX) Your savings deposit arrived. Deposited: €240.50"
        );
        assert_eq!(snippet(&text, 150), "Your savings deposit arrived. Deposited: €240.50");
        // Outlook's text part names its pictures.
        assert_eq!(snippet("[cid:filesQi6.PNG] Account Number: 7731", 150), "Account Number: 7731");
        // A link keeps its words; addresses go.
        assert_eq!(snippet("Read [the agenda](https://x.co/a) first, or see https://x.co/b?q=1 now", 150), "Read the agenda first, or see now");
        assert_eq!(snippet("\u{200b}\u{feff}Hi\u{ad}\u{2060} <br/>there<p class=\"x\">", 150), "Hi there");
        // Divider lines go; a dash or two in a sentence stays.
        assert_eq!(snippet("------------\nWe got your deposit ==== *** ___ — well -- mostly", 150), "We got your deposit — well -- mostly");
        // What only looks like a tag stays.
        assert_eq!(snippet("Jane <jane@x.co> says 3 < 4 and 5 > 2", 150), "Jane <jane@x.co> says 3 < 4 and 5 > 2");
    }

    #[test]
    fn subjects_lose_reply_prefixes() {
        assert_eq!(base_subject("Re: Fwd: Q4 budget"), "Q4 budget");
        assert_eq!(base_subject("Q4 budget"), "Q4 budget");
        assert_eq!(subject_key("RE:  Q4   Budget "), "q4 budget");
        assert_eq!(subject_key("Fw: q4 budget"), "q4 budget");
        assert_eq!(subject_key(""), "");
    }

    #[test]
    fn search_queries_quote_words_and_prefix_the_last() {
        assert_eq!(fts_query("budget").as_deref(), Some("\"budget\"*"));
        assert_eq!(fts_query("q4 budg").as_deref(), Some("\"q4\" \"budg\"*"));
        assert_eq!(fts_query("q4 budget ").as_deref(), Some("\"q4\" \"budget\""));
        assert_eq!(fts_query("say \"hi\"").as_deref(), Some("\"say\" \"\"\"hi\"\"\"*"));
        assert_eq!(fts_query("NEAR( - OR").as_deref(), Some("\"NEAR(\" \"OR\"*"));
        assert_eq!(fts_query("  - "), None);
        assert_eq!(fts_query(""), None);
    }
}
