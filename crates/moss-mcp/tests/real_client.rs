//! For trying Moss's MCP server with a real agent, on made-up mail (never yours):
//!
//! ```sh
//! MOSS_MCP_E2E=/tmp/moss-mcp-e2e cargo test -p moss-mcp --test real_client -- --ignored --nocapture
//! ```
//!
//! It seeds a database in that folder, turns the server on and writes `url` and `token`
//! there, then serves for ten minutes. Point an MCP client at it (see docs/research/mcp-server.md).

use std::path::PathBuf;
use std::sync::Arc;

use moss_accounts::{Accounts, MemorySecretStore};
use moss_core::{Account, AccountColor, MessageSource, ProviderKind};
use moss_engine::{Engine, Event, Events, NetConnector, Parts};
use moss_mcp::{Mcp, McpParts};
use moss_store::Store;

struct Quiet;
impl Events for Quiet {
    fn emit(&self, _: Event) {}
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn email(id: &str, from: &str, subject: &str, body: &str, hours_ago: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(now() - hours_ago * 3_600_000)
        .unwrap()
        .to_rfc2822();
    format!(
        "From: {from}\r\nTo: Maya Ross <maya@northwind.example>\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

#[tokio::test]
#[ignore = "serves for ten minutes, for a real MCP client"]
async fn serve_made_up_mail_for_a_real_client() {
    let dir =
        PathBuf::from(std::env::var("MOSS_MCP_E2E").expect("set MOSS_MCP_E2E to an empty folder"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::open(dir.join("moss.db")).unwrap();
    store
        .upsert_account(&Account {
            id: "work".into(),
            name: "Work".into(),
            letter: "W".into(),
            address: "maya@northwind.example".into(),
            sender_name: "Maya Ross".into(),
            color: AccountColor::Work,
            provider: ProviderKind::Imap,
            org: None,
            org_domain: None,
        })
        .unwrap();
    let mail = [
        (
            "q4@x",
            "Priya Shah <priya@northwind.example>",
            "Q4 budget for the offsite",
            "Hi Maya, the venue quote came in at 18,400 euros. Can you approve it by Friday?",
            2,
        ),
        (
            "lunch@x",
            "Daniel Okafor <daniel@northwind.example>",
            "Lunch on Thursday?",
            "Shall we try the new ramen place near the office?",
            5,
        ),
        (
            "invoice@x",
            "Harbour Bikes <orders@harbour.example>",
            "Your Harbour Bikes order",
            "Thanks for your order. Total paid: 249 euros.",
            30,
        ),
    ];
    for (i, (id, from, subject, body, ago)) in mail.iter().enumerate() {
        let source = MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: i.to_string(),
            ..Default::default()
        };
        store
            .ingest(&email(id, from, subject, body, *ago), &source)
            .unwrap();
    }
    let secrets = Arc::new(MemorySecretStore::new());
    let engine = Engine::new(Parts {
        store,
        accounts: Accounts::new(secrets.clone()),
        secrets: secrets.clone(),
        connector: Arc::new(NetConnector {
            roots: Default::default(),
        }),
        events: Arc::new(Quiet),
        clock: now,
    });
    let mcp = Mcp::new(McpParts {
        engine: engine.clone(),
        events: Arc::new(Quiet),
        secrets,
        dir: dir.clone(),
        port: 0,
        clock: now,
    });
    let status = mcp.set_on(true).await.unwrap();
    std::fs::write(dir.join("url"), status.url.clone().unwrap()).unwrap();
    std::fs::write(dir.join("token"), status.token.clone().unwrap()).unwrap();
    println!("serving {}", status.url.unwrap());
    let stop = dir.join("stop");
    for _ in 0..600 {
        if stop.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    for d in engine.drafts().unwrap() {
        println!(
            "draft: {} to {:?}: {}",
            d.subject,
            d.to.iter().map(|p| &p.email).collect::<Vec<_>>(),
            d.body
        );
    }
}
