//! How much disk a typical mailbox takes. The fixture is a made-up but realistic mix of
//! newsletters, notifications, conversations and attachments; attachment bytes are random,
//! so they don't compress, like real PDFs and photos.
//!
//! `cargo test -p moss-store --release --test storage_size -- --ignored --nocapture` prints
//! where the space goes for 1,000 messages.

use base64::Engine as _;
use moss_core::{Account, AccountColor, MessageSource, ProviderKind};
use moss_store::Store;
use rusqlite::Connection;

/// A small deterministic random source (xorshift), so every run builds the same mailbox.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }

    /// A word from a 5,000-word vocabulary, common words far more often than rare ones.
    fn word(&mut self) -> String {
        let r = self.below(1_000) as f64 / 1_000.0;
        let index = (r * r * r * 5_000.0) as u64;
        let syllables = ["ka", "lo", "mer", "ti", "sun", "va", "dre", "po", "nix", "ul"];
        let mut word = String::new();
        let mut i = index + 1;
        while i > 0 {
            word.push_str(syllables[(i % 10) as usize]);
            i /= 10;
        }
        word
    }

    fn prose(&mut self, len: usize) -> String {
        let mut out = String::new();
        while out.len() < len {
            out.push_str(&self.word());
            out.push(if self.below(12) == 0 { '\n' } else { ' ' });
        }
        out
    }
}

/// Wraps base64 at 76 columns, as mail does.
fn base64_lines(bytes: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    encoded.as_bytes().chunks(76).map(|l| std::str::from_utf8(l).unwrap()).collect::<Vec<_>>().join("\r\n")
}

struct File {
    name: String,
    content_type: &'static str,
    inline: bool,
    bytes: Vec<u8>,
}

/// Newsletter-style HTML: lots of markup around the words.
fn html_from(text: &str, len: usize) -> String {
    let mut out = String::from("<!doctype html><html><body style=\"margin:0;padding:0\"><table width=\"100%\">");
    for line in text.lines().cycle() {
        if out.len() >= len {
            break;
        }
        out.push_str(&format!(
            "<tr><td style=\"font-family:Helvetica,Arial,sans-serif;font-size:15px;line-height:22px;color:#333333;padding:8px 24px\">{line}</td></tr>\r\n"
        ));
    }
    out.push_str("</table></body></html>");
    out
}

fn message(i: usize, rng: &mut Rng) -> Vec<u8> {
    let kind = i % 100;
    let mut headers = vec![
        format!("Message-ID: <m{i}@fixture.test>"),
        format!("Date: {}", mail_date(i)),
        "To: Maya Ross <maya@northwind.co>".to_string(),
        "MIME-Version: 1.0".to_string(),
    ];
    let (text, html_len, files): (String, usize, Vec<File>) = match kind {
        // Newsletters and marketing.
        0..40 => {
            headers.push(format!("From: Store {} <news@shop{}.test>", i % 30, i % 30));
            headers.push("List-Unsubscribe: <mailto:unsub@shop.test>".into());
            headers.push(format!("Subject: This week {}", rng.prose(40).trim()));
            let logos = if i % 4 == 0 {
                (0..3)
                    .map(|n| File {
                        name: format!("logo{n}.png"),
                        content_type: "image/png",
                        inline: true,
                        bytes: rng.bytes(8_000),
                    })
                    .collect()
            } else {
                vec![]
            };
            (rng.prose(5_000), 45_000, logos)
        }
        // Notifications and receipts.
        40..65 => {
            headers.push(format!("From: Billing <billing@service{}.test>", i % 12));
            headers.push("Auto-Submitted: auto-generated".into());
            headers.push(format!("Subject: Your receipt {i}"));
            let pdf = (i % 5 == 0).then(|| File {
                name: format!("receipt-{i}.pdf"),
                content_type: "application/pdf",
                inline: false,
                bytes: rng.bytes(80_000),
            });
            (rng.prose(2_000), 20_000, pdf.into_iter().collect())
        }
        // People writing, some with documents or photos.
        _ => {
            headers.push(format!("From: Person {} <p{}@friends.test>", i % 40, i % 40));
            headers.push(format!("Subject: Re: {}", rng.prose(30).trim()));
            headers.push(format!("In-Reply-To: <m{}@fixture.test>", i.saturating_sub(7)));
            headers.push(format!("References: <m{}@fixture.test>", i.saturating_sub(7)));
            let reply = rng.prose(1_500);
            let quoted: String =
                rng.prose(3_000).lines().map(|l| format!("> {l}\n")).collect();
            let text = format!("{reply}\n\nOn Sun, Person wrote:\n{quoted}");
            let mut files = Vec::new();
            if i % 10 < 3 {
                files.push(File {
                    name: "signature.png".into(),
                    content_type: "image/png",
                    inline: true,
                    bytes: rng.bytes(15_000),
                });
            }
            if (90..97).contains(&kind) {
                files.push(File {
                    name: format!("document-{i}.pdf"),
                    content_type: "application/pdf",
                    inline: false,
                    bytes: rng.bytes(600_000),
                });
            }
            if kind >= 97 {
                for n in 0..2 {
                    files.push(File {
                        name: format!("IMG_{i}{n}.jpg"),
                        content_type: "image/jpeg",
                        inline: false,
                        bytes: rng.bytes(2_500_000),
                    });
                }
            }
            (text, 6_000, files)
        }
    };
    let html = html_from(&text, html_len);
    let text = text.replace('\n', "\r\n");
    let alternative = format!(
        "Content-Type: multipart/alternative; boundary=\"alt{i}\"\r\n\r\n\
         --alt{i}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n\
         --alt{i}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--alt{i}--\r\n"
    );
    let mut out = headers.join("\r\n") + "\r\n";
    if files.is_empty() {
        out.push_str(&alternative);
    } else {
        out.push_str(&format!("Content-Type: multipart/mixed; boundary=\"mix{i}\"\r\n\r\n--mix{i}\r\n"));
        out.push_str(&alternative);
        for f in &files {
            let disposition = if f.inline { "inline" } else { "attachment" };
            out.push_str(&format!(
                "--mix{i}\r\nContent-Type: {}; name=\"{}\"\r\nContent-Transfer-Encoding: base64\r\n\
                 Content-Disposition: {disposition}; filename=\"{}\"\r\nContent-ID: <{}>\r\n\r\n{}\r\n",
                f.content_type,
                f.name,
                f.name,
                f.name,
                base64_lines(&f.bytes)
            ));
        }
        out.push_str(&format!("--mix{i}--\r\n"));
    }
    out.into_bytes()
}

fn mail_date(i: usize) -> String {
    let seconds = 1_793_524_320 - (i as i64) * 3_600;
    chrono_like(seconds)
}

/// RFC 822 date without pulling in a date crate.
fn chrono_like(seconds: i64) -> String {
    mail_parser::DateTime::from_timestamp(seconds).to_rfc822()
}

fn account() -> Account {
    Account {
        id: "work".into(),
        name: "Work".into(),
        letter: "W".into(),
        address: "maya@northwind.co".into(),
        sender_name: "Maya Ross".into(),
        color: AccountColor::Work,
        provider: ProviderKind::Gmail,
        org: None,
        org_domain: None,
    }
}

/// Stores `n` typical messages in a fresh database file and returns its path.
fn build(dir: &std::path::Path, n: usize) -> (std::path::PathBuf, u64) {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("moss.db");
    let mut store = Store::open(&path).unwrap();
    store.upsert_account(&account()).unwrap();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut raw_total = 0u64;
    for i in 0..n {
        let raw = message(i, &mut rng);
        raw_total += raw.len() as u64;
        let source = MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: format!("1:{}", i + 1),
            ..Default::default()
        };
        store.ingest(&raw, &source).unwrap();
    }
    drop(store);
    (path, raw_total)
}

/// Roughly what IMAP sync downloads: messages up to 128 KB whole; bigger ones, the header
/// and text parts (see moss-providers' `imap/slim.rs`). Sizes and structures are left out.
fn synced_bytes(n: usize) -> u64 {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    (0..n)
        .map(|i| {
            let raw = message(i, &mut rng);
            if raw.len() <= 128 * 1024 {
                return raw.len() as u64;
            }
            let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
            let header = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() as u64;
            let text: u64 = parsed.text_bodies().chain(parsed.html_bodies()).map(|p| p.len() as u64).sum();
            header + text
        })
        .sum()
}

struct Sizes {
    file: i64,
    by_table: Vec<(String, i64)>,
    /// Bytes in the biggest columns: compressed text and HTML, previews, file bytes.
    columns: Vec<(&'static str, i64)>,
}

fn sizes(path: &std::path::Path) -> Sizes {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let file: i64 = conn
        .query_row("SELECT page_count * page_size FROM pragma_page_count, pragma_page_size", [], |r| r.get(0))
        .unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT CASE WHEN name LIKE 'message_search%' THEN 'search index (FTS5)'
                         WHEN name LIKE 'sqlite_autoindex%' OR name LIKE '%_by_%' OR name = 'messages_open' THEN 'indexes'
                         ELSE name END AS part,
                    sum(pgsize) FROM dbstat GROUP BY part ORDER BY 2 DESC",
        )
        .unwrap();
    let by_table = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let columns = [("bodies", "text"), ("bodies", "html"), ("messages", "snippet"), ("attachment_data", "data")]
        .into_iter()
        .map(|(table, c)| {
            let sql = format!("SELECT coalesce(sum(length(CAST({c} AS BLOB))), 0) FROM {table}");
            (c, conn.query_row(&sql, [], |r| r.get(0)).unwrap())
        })
        .collect();
    Sizes { file, by_table, columns }
}

fn kb(bytes: i64, n: usize) -> String {
    format!("{:>8.1} KB/message", bytes as f64 / n as f64 / 1024.0)
}

#[test]
#[ignore = "a report: run with --ignored --nocapture, ideally --release"]
fn report_where_the_space_goes() {
    let n = 1_000;
    let dir = tempfile::tempdir().unwrap();
    let (path, raw_total) = build(dir.path(), n);
    let s = sizes(&path);
    println!("{n} messages, {:.1} MB whole ({})", raw_total as f64 / 1e6, kb(raw_total as i64, n));
    let synced = synced_bytes(n);
    println!("sync downloads about {:.1} MB ({})", synced as f64 / 1e6, kb(synced as i64, n));
    println!("database file: {:.1} MB ({})", s.file as f64 / 1e6, kb(s.file, n));
    for (name, bytes) in &s.by_table {
        println!("  {name:<28} {:>8.1} MB  {}", *bytes as f64 / 1e6, kb(*bytes, n));
    }
    let offline: i64 = s.by_table.iter().filter(|(name, _)| name == "bodies").map(|(_, b)| *b).sum();
    println!("index and details (all but bodies): {}", kb(s.file - offline, n));
    println!("of which, in columns:");
    for (name, bytes) in &s.columns {
        println!("  {name:<28} {:>8.1} MB  {}", *bytes as f64 / 1e6, kb(*bytes, n));
    }
}

/// Attachments stay on the server, and the text is compressed in its own table, so a
/// typical mailbox costs a few KB a message. Before lean storage this mix took about 330
/// KB a message; with text kept plain, about 16 KB.
#[test]
fn a_typical_mailbox_stays_within_its_budget() {
    let n = 100;
    let dir = tempfile::tempdir().unwrap();
    let (path, raw_total) = build(dir.path(), n);
    let s = sizes(&path);
    // An empty database is a page per table and index, whatever the mail.
    let empty = sizes(&build(&dir.path().join("empty"), 0).0).file;
    assert!(raw_total > 25_000_000, "the fixture holds real attachments: {raw_total}");
    let bodies: i64 = s.by_table.iter().filter(|(name, _)| name == "bodies").map(|(_, b)| *b).sum();
    let index = s.file - bodies - empty;
    assert!(index < n as i64 * 4 * 1024, "index: {}", kb(index, n));
    assert!(bodies < n as i64 * 6 * 1024, "offline text: {}", kb(bodies, n));
    let files: i64 = s.by_table.iter().filter(|(name, _)| name == "attachment_data").map(|(_, b)| *b).sum();
    assert!(files <= 4096, "no file bytes by default: {files}");
    let raw: i64 = s.by_table.iter().filter(|(name, _)| name == "message_raw").map(|(_, b)| *b).sum();
    assert!(raw <= 4096, "originals stay on the server: {raw}");
}
