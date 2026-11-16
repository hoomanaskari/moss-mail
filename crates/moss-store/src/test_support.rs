//! Builders for tests: accounts, raw emails and an ingest shortcut.

use moss_core::{Account, AccountColor, ComposeMode, Draft, MessageSource, Person, ProviderKind};

use crate::{Ingested, Store};

pub fn account(id: &str, address: &str) -> Account {
    Account {
        id: id.into(),
        name: id.into(),
        letter: id[..1].to_uppercase(),
        address: address.into(),
        sender_name: "Maya Ross".into(),
        color: AccountColor::Work,
        provider: ProviderKind::Imap,
        org: None,
        org_domain: None,
    }
}

pub fn draft(id: &str) -> Draft {
    Draft {
        revision: None,
        id: id.into(),
        mode: ComposeMode::New,
        thread_id: None,
        from_account_id: "work".into(),
        to: vec![Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() }],
        cc: vec![],
        bcc: vec![],
        subject: "Lunch?".into(),
        body: "Friday?".into(),
        body_html: "<p>Friday?</p>".into(),
        remind_if_no_reply: false,
        updated_at: NOV_1,
        signature_id: None,
        signature_chosen: false,
        from_address: None,
        attachments: vec![],
    }
}

pub fn store_with_work() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account(&account("work", "maya@northwind.co")).unwrap();
    store
}

pub fn source(provider_id: &str) -> MessageSource {
    MessageSource {
        account_id: "work".into(),
        mailbox: "INBOX".into(),
        provider_id: provider_id.into(),
        ..Default::default()
    }
}

pub fn ingest(store: &mut Store, provider_id: &str, raw: &[u8]) -> Ingested {
    store.ingest(raw, &source(provider_id)).unwrap()
}

/// Makes every later write fail, as a full disk or a read-only file would.
pub fn read_only(store: &Store) {
    store.conn.pragma_update(None, "query_only", true).unwrap();
}

/// Runs SQL that breaks the schema, so a statement that relies on it fails.
pub fn break_schema(store: &Store, sql: &str) {
    store.conn.execute_batch(sql).unwrap();
}

/// 1 November 2026, 09:12 UTC, in milliseconds.
pub const NOV_1: i64 = 1_793_524_320_000;
pub const DAY: i64 = 86_400_000;

/// A raw RFC 822 email, by default Daniel asking Maya about the Q4 budget.
#[derive(Clone)]
pub struct Mail {
    headers: Vec<(String, String)>,
    body: String,
    html: Option<String>,
    attachments: Vec<(String, String, String, bool)>,
}

pub fn mail() -> Mail {
    Mail {
        headers: vec![
            ("From".into(), "Daniel Okafor <daniel@northwind.co>".into()),
            ("To".into(), "Maya Ross <maya@northwind.co>".into()),
            ("Subject".into(), "Q4 budget".into()),
            ("Message-ID".into(), "<q4@northwind.co>".into()),
            ("Date".into(), "Sun, 01 Nov 2026 09:12:00 +0000".into()),
        ],
        body: "Hi Maya,\n\nCan you sign off the Q4 budget?\n\nDaniel".into(),
        html: None,
        attachments: vec![],
    }
}

impl Mail {
    /// Sets a header, replacing one of the same name.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self = self.without(name);
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Adds a header on top, as a receiving server does, keeping any of the same name.
    pub fn received(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(0, (name.into(), value.into()));
        self
    }

    pub fn without(mut self, name: &str) -> Self {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self
    }

    pub fn from(self, value: &str) -> Self {
        self.header("From", value)
    }

    pub fn to(self, value: &str) -> Self {
        self.header("To", value)
    }

    pub fn subject(self, value: &str) -> Self {
        self.header("Subject", value)
    }

    pub fn id(self, value: &str) -> Self {
        self.header("Message-ID", &format!("<{value}>"))
    }

    /// Makes this a reply: In-Reply-To the last id, References all of them.
    pub fn reply_to(self, ids: &[&str]) -> Self {
        let refs: Vec<String> = ids.iter().map(|id| format!("<{id}>")).collect();
        self.header("In-Reply-To", refs.last().unwrap()).header("References", &refs.join(" "))
    }

    /// Date at `ms` milliseconds since the epoch (UTC).
    pub fn at(self, ms: i64) -> Self {
        let date = mail_parser::DateTime::from_timestamp(ms / 1000).to_rfc822();
        self.header("Date", &date)
    }

    pub fn body(mut self, text: &str) -> Self {
        self.body = text.into();
        self
    }

    pub fn html(mut self, html: &str) -> Self {
        self.html = Some(html.into());
        self
    }

    pub fn attach(mut self, name: &str, content_type: &str, content: &str) -> Self {
        self.attachments.push((name.into(), content_type.into(), content.into(), false));
        self
    }

    pub fn inline(mut self, name: &str, content_type: &str, content: &str) -> Self {
        self.attachments.push((name.into(), content_type.into(), content.into(), true));
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut out = String::new();
        for (name, value) in &self.headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str("MIME-Version: 1.0\r\n");
        let text = format!(
            "Content-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n",
            self.body.replace('\n', "\r\n")
        );
        let alternative = match &self.html {
            None => text,
            Some(html) => format!(
                "Content-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n--alt\r\n{text}--alt\r\n\
                 Content-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--alt--\r\n"
            ),
        };
        if self.attachments.is_empty() {
            out.push_str(&alternative);
        } else {
            out.push_str("Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\n");
            out.push_str(&alternative);
            for (name, content_type, content, inline) in &self.attachments {
                let disposition = if *inline { "inline" } else { "attachment" };
                out.push_str(&format!(
                    "--mix\r\nContent-Type: {content_type}; name=\"{name}\"\r\n\
                     Content-Disposition: {disposition}; filename=\"{name}\"\r\nContent-ID: <{name}>\r\n\r\n{content}\r\n"
                ));
            }
            out.push_str("--mix--\r\n");
        }
        out.into_bytes()
    }
}

/// Build a real historical schema, without downgrading a newer database in place.
pub fn historical_store(path: &std::path::Path, version: usize) -> Store {
    let mut conn = rusqlite::Connection::open(path).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    crate::migrate(&mut conn, &crate::schema::MIGRATIONS[..version]).unwrap();
    Store { conn, delivery_session: None }
}

/// The V20 through V29 draft representation, before content revisions existed.
pub fn insert_historical_draft(store: &Store, d: &Draft) {
    store.conn.execute(
        "INSERT INTO drafts (id,mode,thread_id,account_id,to_json,cc_json,bcc_json,subject,body,body_html,remind_if_no_reply,updated_at,signature_id,signature_chosen,from_address,files_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![d.id,crate::codec::name(&d.mode),d.thread_id.as_deref().map(crate::codec::parse_id),d.from_account_id,
            serde_json::to_string(&d.to).unwrap(),serde_json::to_string(&d.cc).unwrap(),serde_json::to_string(&d.bcc).unwrap(),
            d.subject,d.body,d.body_html,d.remind_if_no_reply,d.updated_at,d.signature_id,d.signature_chosen,d.from_address,
            serde_json::to_string(&d.attachments).unwrap()],
    ).unwrap();
}

/// How SQLite will run `sql`, one step a line ("SEARCH m USING INDEX …", "SCAN t"), with
/// every parameter NULL, so a test can check a query finds its rows by an index.
pub fn plan(store: &Store, sql: &str) -> String {
    let mut stmt = store.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let mut rows = stmt.raw_query();
    let mut steps = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        steps.push(row.get::<_, String>(3).unwrap());
    }
    steps.join("\n")
}

/// Whether a plan reads every row of `table` (as `alias`) instead of searching an index.
pub fn scans(plan: &str, alias: &str) -> bool {
    plan.lines().any(|step| step.trim() == format!("SCAN {alias}") || step.starts_with(&format!("SCAN {alias} ")))
}
