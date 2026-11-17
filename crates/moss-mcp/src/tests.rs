//! The server as an agent sees it: a real MCP client over HTTP (and through the stdio
//! bridge), against an engine over a database with a little mail in it.

use std::sync::{Arc, Mutex};

use moss_accounts::{
    Accounts, MemorySecretStore, Secret, SecretRef, SecretStore, SecretStoreError,
};
use moss_core::{Account, AccountColor, ComposeMode, MessageSource, ProviderKind, SplitId};
use moss_engine::{Engine, Event, Events, NetConnector, OutgoingDraft, Parts};
use moss_store::Store;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

use super::*;

const NOV_1: i64 = 1_761_998_400_000;

#[derive(Default)]
struct Heard(Mutex<Vec<Event>>);

impl Events for Heard {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

fn now() -> i64 {
    NOV_1 + 3_600_000
}

fn account(id: &str, address: &str) -> Account {
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

fn raw(id: &str, from: &str, subject: &str, extra: &str, body: &str) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(NOV_1)
        .unwrap()
        .to_rfc2822();
    format!(
        "From: {from}\r\nTo: Maya Ross <maya@northwind.co>\r\n{extra}Subject: {subject}\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

fn with_file(id: &str) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(NOV_1 + 1000)
        .unwrap()
        .to_rfc2822();
    format!(
        "From: Lena Park <lena@studio.co>\r\nTo: Maya Ross <maya@northwind.co>\r\nSubject: Signed contract\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nSigned copy attached\r\n--m\r\n\
         Content-Type: application/pdf\r\nContent-Disposition: attachment; filename=Contract.pdf\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nUERGREFUQQ==\r\n--m--\r\n"
    )
    .into_bytes()
}

fn source(account: &str, uid: usize, headers_only: bool) -> MessageSource {
    MessageSource {
        account_id: account.into(),
        mailbox: "INBOX".into(),
        provider_id: format!("1:{uid}"),
        headers_only,
        ..Default::default()
    }
}

struct Setup {
    mcp: Mcp,
    engine: Engine,
    heard: Arc<Heard>,
    secrets: Arc<MemorySecretStore>,
    dir: tempfile::TempDir,
}

fn engine_over(store: Store, secrets: Arc<MemorySecretStore>, heard: Arc<Heard>) -> Engine {
    Engine::new(Parts {
        store,
        accounts: Accounts::new(secrets.clone()),
        secrets,
        connector: Arc::new(NetConnector {
            roots: Default::default(),
        }),
        events: heard,
        clock: now,
    })
}

/// Two accounts: work with three threads (one with a file, one a reply chain with cc), home
/// with one email whose body was never fetched.
fn setup() -> Setup {
    setup_with(&[])
}

/// `setup`, plus these raw emails in work's inbox.
fn setup_with(extra: &[Vec<u8>]) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("moss.db")).unwrap();
    store
        .upsert_account(&account("work", "maya@northwind.co"))
        .unwrap();
    store
        .upsert_account(&account("home", "maya@home.example"))
        .unwrap();
    let lunch = raw(
        "lunch@x",
        "Daniel Okafor <daniel@northwind.co>",
        "Lunch on Friday",
        "Cc: Ana <ana@northwind.co>\r\n",
        "Shall we try the ramen place?",
    );
    store.ingest(&lunch, &source("work", 1, false)).unwrap();
    let budget = raw(
        "budget@x",
        "Priya <priya@northwind.co>",
        "Re: Q4 budget",
        "",
        "The numbers for the offsite are in.",
    );
    store.ingest(&budget, &source("work", 2, false)).unwrap();
    let contract = store
        .ingest(&with_file("contract@x"), &source("work", 3, false))
        .unwrap();
    let pdf = store
        .thread(&contract.thread_id, 0)
        .unwrap()
        .unwrap()
        .messages[0]
        .attachments[0]
        .index;
    store
        .keep_attachment_text(
            &contract.message_id,
            pdf,
            "the indemnity clause covers both parties",
        )
        .unwrap();
    // Its bytes are on this Mac, as after the window opened it.
    store
        .keep_attachment(&contract.message_id, pdf, b"PDFDATA")
        .unwrap();
    let old = raw(
        "old@x",
        "Sam <sam@home.example>",
        "Old note",
        "",
        "never fetched",
    );
    store.ingest(&old, &source("home", 4, true)).unwrap();
    for (i, raw) in extra.iter().enumerate() {
        store
            .ingest(raw, &source("work", i + 5, false))
            .unwrap();
    }
    let secrets = Arc::new(MemorySecretStore::new());
    let heard = Arc::new(Heard::default());
    let engine = engine_over(store, secrets.clone(), heard.clone());
    let mcp = mcp_over(&engine, heard.clone(), secrets.clone(), dir.path());
    Setup {
        mcp,
        engine,
        heard,
        secrets,
        dir,
    }
}

fn mcp_over(
    engine: &Engine,
    heard: Arc<Heard>,
    secrets: Arc<dyn SecretStore>,
    dir: &std::path::Path,
) -> Mcp {
    Mcp::new(McpParts {
        engine: engine.clone(),
        events: heard,
        secrets,
        dir: dir.to_path_buf(),
        port: 0,
        clock: now,
    })
}

async fn client(url: &str, token: &str) -> RunningService<RoleClient, ()> {
    let config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .auth_header(token.to_string());
    ().serve(StreamableHttpClientTransport::from_config(config))
        .await
        .unwrap()
}

async fn on(s: &Setup) -> (McpStatus, RunningService<RoleClient, ()>) {
    let status = s.mcp.set_on(true).await.unwrap();
    let c = client(
        status.url.as_deref().unwrap(),
        status.token.as_deref().unwrap(),
    )
    .await;
    (status, c)
}

/// The tool's text, and whether it was an error.
async fn call(
    c: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> (bool, String) {
    let mut params = CallToolRequestParams::new(tool);
    if let Value::Object(map) = args {
        params = params.with_arguments(map);
    }
    let result = c.call_tool(params).await.unwrap();
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.clone())
        .collect::<String>();
    (result.is_error == Some(true), text)
}

async fn ok(c: &RunningService<RoleClient, ()>, tool: &'static str, args: Value) -> Value {
    let (error, text) = call(c, tool, args).await;
    assert!(!error, "{tool}: {text}");
    serde_json::from_str(&text).unwrap()
}

async fn fails(c: &RunningService<RoleClient, ()>, tool: &'static str, args: Value) -> String {
    let (error, text) = call(c, tool, args).await;
    assert!(error, "{tool} should fail: {text}");
    text
}

async fn post(url: &str, headers: &[(&str, &str)]) -> reqwest::StatusCode {
    let mut r = reqwest::Client::builder().no_proxy().build().unwrap()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    r.body(body).send().await.unwrap().status()
}

#[tokio::test]
async fn off_until_you_turn_it_on() {
    let s = setup();
    let status = s.mcp.restore().await.unwrap();
    assert_eq!(
        status,
        McpStatus {
            on: false,
            url: None,
            token: None,
            last_used: None
        }
    );
    assert!(
        s.secrets.load(&Mcp::item()).unwrap().is_none(),
        "no token until it's on"
    );
    // Stopping (as Moss quits) while off is fine.
    s.mcp.stop().await;
    assert!(!s.mcp.status().on);
}

#[tokio::test]
async fn on_it_listens_on_localhost_with_a_token_from_the_keychain() {
    let s = setup();
    let status = s.mcp.set_on(true).await.unwrap();
    let url = status.url.clone().unwrap();
    assert!(
        url.starts_with("http://127.0.0.1:") && url.ends_with("/mcp"),
        "{url}"
    );
    let token = status.token.clone().unwrap();
    assert!(
        token.starts_with("moss_") && token.len() == 5 + 64,
        "{token}"
    );
    assert_eq!(
        s.secrets.load(&Mcp::item()).unwrap(),
        Some(Secret::ApiKey { key: token.clone() })
    );
    // Turning it on again changes nothing.
    assert_eq!(s.mcp.set_on(true).await.unwrap(), status);
}

#[tokio::test]
async fn off_refuses_connections_and_on_again_keeps_the_address_and_token() {
    let s = setup();
    let first = s.mcp.set_on(true).await.unwrap();
    let url = first.url.clone().unwrap();
    let off = s.mcp.set_on(false).await.unwrap();
    assert!(!off.on && off.url.is_none() && off.token.is_none());
    let refused = reqwest::Client::builder().no_proxy().build().unwrap().post(&url).body("{}").send().await;
    assert!(refused.is_err(), "nothing listens once it's off");
    assert_eq!(s.mcp.set_on(true).await.unwrap(), first);
}

#[tokio::test]
async fn it_comes_back_on_after_a_restart_on_the_same_port() {
    let s = setup();
    let first = s.mcp.set_on(true).await.unwrap();
    s.mcp.stop().await;
    let again = mcp_over(&s.engine, s.heard.clone(), s.secrets.clone(), s.dir.path());
    let status = again.restore().await.unwrap();
    assert_eq!(status, first);
    let saved: Value =
        serde_json::from_slice(&std::fs::read(s.dir.path().join(SETTINGS_FILE)).unwrap()).unwrap();
    assert_eq!(saved["on"], true);
}

#[tokio::test]
async fn a_taken_port_gives_another_one_kept_from_then_on() {
    let s = setup();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let mcp = Mcp::new(McpParts {
        engine: s.engine.clone(),
        events: s.heard.clone(),
        secrets: s.secrets.clone(),
        dir: s.dir.path().into(),
        port,
        clock: now,
    });
    let url = mcp.set_on(true).await.unwrap().url.unwrap();
    assert_ne!(url, super::url(port));
    let saved: Value =
        serde_json::from_slice(&std::fs::read(s.dir.path().join(SETTINGS_FILE)).unwrap()).unwrap();
    assert_eq!(url, super::url(saved["port"].as_u64().unwrap() as u16));
}

#[test]
fn moss_dev_never_takes_the_installed_moss_s_port() {
    // The installed Moss asks for its saved port, or PORT.
    assert_eq!(first_port(None, PORT), PORT);
    assert_eq!(first_port(Some(51_000), PORT), 51_000);
    assert_eq!(first_port(Some(PORT), PORT), PORT);
    // Moss Dev asks for DEV_PORT, and a saved PORT (copied with production's data) is not its own.
    assert_ne!(DEV_PORT, PORT);
    assert_eq!(first_port(None, DEV_PORT), DEV_PORT);
    assert_eq!(first_port(Some(PORT), DEV_PORT), DEV_PORT);
    assert_eq!(first_port(Some(51_000), DEV_PORT), 51_000);
}

#[tokio::test]
async fn a_saved_production_port_is_left_to_production() {
    let s = setup();
    std::fs::write(s.dir.path().join(SETTINGS_FILE), format!(r#"{{"on":true,"port":{PORT}}}"#)).unwrap();
    let mcp = Mcp::new(McpParts {
        engine: s.engine.clone(),
        events: s.heard.clone(),
        secrets: s.secrets.clone(),
        dir: s.dir.path().into(),
        port: 0,
        clock: now,
    });
    let url = mcp.restore().await.unwrap().url.unwrap();
    assert_ne!(url, super::url(PORT));
    mcp.stop().await;
}

#[tokio::test]
async fn only_the_token_gets_in_and_never_a_web_page() {
    let s = setup();
    let status = s.mcp.set_on(true).await.unwrap();
    let (url, token) = (status.url.unwrap(), status.token.unwrap());
    let bearer = format!("Bearer {token}");
    assert_eq!(post(&url, &[]).await, reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(
        post(&url, &[("Authorization", "Bearer moss_wrong")]).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&url, &[("Authorization", &token)]).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert!(post(
        &url,
        &[
            ("Authorization", &bearer),
            ("Origin", "https://evil.example")
        ]
    )
    .await
    .is_client_error());
    assert!(post(
        &url,
        &[("Authorization", &bearer), ("Host", "evil.example")]
    )
    .await
    .is_client_error());
    assert_eq!(
        post(&url, &[("Authorization", &bearer)]).await,
        reqwest::StatusCode::OK
    );
}

#[tokio::test]
async fn a_new_token_stops_the_old_one_at_once() {
    let s = setup();
    let (first, _c) = on(&s).await;
    let url = first.url.clone().unwrap();
    let next = s.mcp.new_token().await.unwrap();
    assert_ne!(next.token, first.token);
    assert_eq!(next.url, first.url);
    let old = format!("Bearer {}", first.token.unwrap());
    let new = format!("Bearer {}", next.token.clone().unwrap());
    assert_eq!(
        post(&url, &[("Authorization", &old)]).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&url, &[("Authorization", &new)]).await,
        reqwest::StatusCode::OK
    );
    // While off, it's kept for next time.
    s.mcp.set_on(false).await.unwrap();
    let off = s.mcp.new_token().await.unwrap();
    assert_eq!(off.token, None);
    let later = s.mcp.set_on(true).await.unwrap();
    assert_ne!(later.token, next.token);
}

#[tokio::test]
async fn it_says_when_an_agent_last_used_it() {
    let s = setup();
    let (_, c) = on(&s).await;
    ok(&c, "list_accounts", json!({})).await;
    assert_eq!(s.mcp.status().last_used, Some(now()));
}

#[tokio::test]
async fn agents_see_tools_to_read_manage_draft_and_send() {
    let s = setup();
    let (_, c) = on(&s).await;
    let info = c.peer_info().unwrap();
    assert_eq!(info.server_info.as_ref().unwrap().name, "moss");
    assert!(info
        .instructions
        .as_deref()
        .unwrap()
        .contains("Only call send when the user clearly asked"));
    let mut names: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "accept_sender",
            "block_sender",
            "cancel_scheduled",
            "create_draft",
            "delete_draft",
            "delete_thread",
            "find_contacts",
            "get_attachment",
            "get_thread",
            "list_accounts",
            "list_blocked",
            "list_done",
            "list_drafts",
            "list_inbox",
            "list_scheduled",
            "mark_done",
            "mark_read",
            "move_thread",
            "remind_if_no_reply",
            "remind_me",
            "restore_thread",
            "search_mail",
            "send",
            "sync_now",
            "unblock_sender",
            "update_draft"
        ]
    );
}

#[tokio::test]
async fn accounts_with_the_addresses_they_send_from() {
    let s = setup();
    s.engine
        .add_address("work", "maya@northwind.example", Some("Maya"))
        .unwrap();
    let (_, c) = on(&s).await;
    let v = ok(&c, "list_accounts", json!({})).await;
    let work = v["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "work")
        .unwrap()
        .clone();
    assert_eq!(work["email"], "maya@northwind.co");
    assert_eq!(work["senderName"], "Maya Ross");
    assert_eq!(work["alsoSendsFrom"], json!(["maya@northwind.example"]));
}

#[tokio::test]
async fn the_inbox_by_split_and_account_newest_first() {
    let s = setup();
    let (_, c) = on(&s).await;
    let all = ok(&c, "list_inbox", json!({})).await;
    assert_eq!(all["total"], 4);
    let first = &all["threads"][0];
    assert_eq!(first["subject"], "Signed contract");
    assert_eq!(first["from"], "Lena Park <lena@studio.co>");
    assert_eq!(first["hasFiles"], true);
    assert_eq!(first["date"], "2025-11-01T12:00:01Z");
    let work = ok(&c, "list_inbox", json!({"account": "work", "limit": 2})).await;
    assert_eq!(
        (work["total"].clone(), work["shown"].clone()),
        (json!(3), json!(2))
    );
    let split = first["split"].as_str().unwrap().to_string();
    let same = ok(&c, "list_inbox", json!({"split": split})).await;
    assert!(same["threads"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["split"] == first["split"]));
    let none = ok(
        &c,
        "list_inbox",
        json!({"split": if split == "receipts" { "calendar" } else { "receipts" }}),
    )
    .await;
    assert_eq!(none["total"], 0);
    assert!(fails(&c, "list_inbox", json!({"split": "spam"}))
        .await
        .contains("no split called spam"));
}

#[tokio::test]
async fn a_thread_with_its_emails_and_file_names_but_no_bytes() {
    let s = setup();
    let (_, c) = on(&s).await;
    let threads = ok(&c, "list_inbox", json!({"account": "work"})).await;
    let id = |subject: &str| {
        threads["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["subject"] == subject)
            .unwrap()["id"]
            .clone()
    };
    let contract = ok(
        &c,
        "get_thread",
        json!({"thread_id": id("Signed contract")}),
    )
    .await;
    let m = &contract["messages"][0];
    assert_eq!(m["text"].as_str().unwrap().trim(), "Signed copy attached");
    assert_eq!(m["files"], json!(["Contract.pdf"]));
    assert!(m.get("cc").is_none());
    let lunch = ok(
        &c,
        "get_thread",
        json!({"thread_id": id("Lunch on Friday")}),
    )
    .await;
    assert_eq!(
        lunch["messages"][0]["cc"],
        json!(["Ana <ana@northwind.co>"])
    );
    assert_eq!(
        lunch["messages"][0]["to"],
        json!(["Maya Ross <maya@northwind.co>"])
    );
    assert!(lunch["messages"][0].get("files").is_none());
    assert!(fails(&c, "get_thread", json!({"thread_id": "nope"}))
        .await
        .contains("no thread with id nope"));
}

#[tokio::test]
async fn reading_a_thread_leaves_it_unread() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    let unread = |s: &Setup| s.engine.thread(&id).unwrap().summary.unread;
    assert!(unread(&s));
    ok(&c, "get_thread", json!({"thread_id": id})).await;
    assert!(unread(&s), "only mark_read marks it read");
}

#[tokio::test]
async fn an_email_never_downloaded_says_so() {
    let s = setup();
    let (_, c) = on(&s).await;
    let home = ok(&c, "list_inbox", json!({"account": "home"})).await;
    let thread = ok(
        &c,
        "get_thread",
        json!({"thread_id": home["threads"][0]["id"]}),
    )
    .await;
    assert!(thread["messages"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Not downloaded yet"));
}

#[tokio::test]
async fn search_finds_mail_by_its_words() {
    let s = setup();
    let (_, c) = on(&s).await;
    let v = ok(&c, "search_mail", json!({"query": "ramen"})).await;
    assert_eq!(v["threads"][0]["subject"], "Lunch on Friday");
    assert!(v["threads"][0]["match"].as_str().unwrap().contains("ramen"));
    assert!(v["threads"][0].get("matchedFile").is_none());
    let none = ok(
        &c,
        "search_mail",
        json!({"query": "ramen", "account": "home"}),
    )
    .await;
    assert_eq!(none["threads"], json!([]));
    assert!(fails(&c, "search_mail", json!({"query": "  "}))
        .await
        .contains("Say what to search for"));
}

#[tokio::test]
async fn search_says_which_file_matched() {
    let s = setup();
    let (_, c) = on(&s).await;
    let v = ok(&c, "search_mail", json!({"query": "indemnity"})).await;
    assert_eq!(v["threads"][0]["matchedFile"], "Contract.pdf");
}

#[tokio::test]
async fn a_new_draft_goes_from_the_first_account_and_is_never_sent() {
    let s = setup();
    let (_, c) = on(&s).await;
    let v = ok(&c, "create_draft", json!({"to": ["Daniel Okafor <daniel@northwind.co>", "ana@northwind.co"], "subject": "Offsite", "body": "Hi both,\n\nSee you there."})).await;
    assert!(v["note"].as_str().unwrap().contains("hasn't been sent"));
    let id = v["draftId"].as_str().unwrap();
    let drafts = s.engine.drafts().unwrap();
    let d = drafts.iter().find(|d| d.id == id).unwrap();
    assert_eq!(
        d.from_account_id,
        s.engine.accounts().unwrap()[0].account.id
    );
    assert_eq!(d.mode, ComposeMode::New);
    assert_eq!(d.to.len(), 2);
    assert_eq!(d.to[0].name, "Daniel Okafor");
    assert_eq!(d.body_html, "Hi both,<br><br>See you there.");
    assert_eq!(d.updated_at, now());
    let told = s.heard.0.lock().unwrap().iter().any(
        |e| matches!(e, Event::DraftsChanged { account_id } if *account_id == d.from_account_id),
    );
    assert!(told, "the window hears to reload its drafts");
    let listed = ok(&c, "list_drafts", json!({})).await;
    assert_eq!(listed["drafts"][0]["draftId"], id);
    assert_eq!(listed["drafts"][0]["subject"], "Offsite");
}

#[tokio::test]
async fn a_reply_goes_to_who_wrote_with_re_and_the_threads_account() {
    let s = setup();
    let (_, c) = on(&s).await;
    let found = ok(&c, "search_mail", json!({"query": "offsite numbers"})).await;
    let thread = found["threads"][0]["id"].clone();
    let v = ok(
        &c,
        "create_draft",
        json!({"reply_to_thread": thread, "body": "Thanks Priya."}),
    )
    .await;
    assert_eq!(v["mode"], "reply");
    assert_eq!(v["threadId"], thread);
    assert_eq!(v["account"], "work");
    assert_eq!(v["to"], json!(["Priya <priya@northwind.co>"]));
    assert_eq!(v["subject"], "Re: Q4 budget");
    assert!(fails(
        &c,
        "create_draft",
        json!({"reply_to_thread": "nope", "body": "x"})
    )
    .await
    .contains("no thread"));
}

#[tokio::test]
async fn a_draft_changes_by_its_id_keeping_what_isnt_said() {
    let s = setup();
    let (_, c) = on(&s).await;
    let v = ok(
        &c,
        "create_draft",
        json!({"to": ["daniel@northwind.co"], "subject": "Lunch", "body": "Friday?"}),
    )
    .await;
    let id = v["draftId"].clone();
    let changed = ok(&c, "update_draft", json!({"draft_id": id, "revision": v["revision"], "cc": ["ana@northwind.co"], "body": "Friday at one?", "account": "home"})).await;
    assert_eq!(changed["draftId"], id);
    assert_eq!(changed["subject"], "Lunch");
    assert_eq!(changed["to"], json!(["daniel@northwind.co"]));
    assert_eq!(changed["cc"], json!(["ana@northwind.co"]));
    assert_eq!(changed["body"], "Friday at one?");
    assert_eq!(changed["account"], "home");
    // The same account again keeps the address it goes from.
    let mut d = s
        .engine
        .drafts()
        .unwrap()
        .into_iter()
        .find(|d| d.id == id.as_str().unwrap())
        .unwrap();
    d.from_address = Some("maya@home.example".into());
    let d = s.engine.save_draft(&d).unwrap();
    ok(
        &c,
        "update_draft",
        json!({"draft_id": id, "revision": d.revision, "account": "home"}),
    )
    .await;
    let d = s
        .engine
        .drafts()
        .unwrap()
        .into_iter()
        .find(|d| d.id == id.as_str().unwrap())
        .unwrap();
    assert_eq!(d.from_address.as_deref(), Some("maya@home.example"));
    assert!(fails(&c, "update_draft", json!({"draft_id": "nope", "revision": "missing"}))
        .await
        .contains("no draft with id nope"));
    assert!(fails(&c, "create_draft", json!({"account": "nope"}))
        .await
        .contains("no account with id nope"));
    assert!(fails(&c, "create_draft", json!({"to": ["daniel@northwind"]}))
        .await
        .contains("isn't an email address"));
    assert!(fails(&c, "create_draft", json!({"cc": ["ana"]}))
        .await
        .contains("There's no one called ana in the user's mail"));
}

#[tokio::test]
async fn with_no_accounts_there_is_nothing_to_draft_from() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("moss.db")).unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let heard = Arc::new(Heard::default());
    let engine = engine_over(store, secrets.clone(), heard.clone());
    let mcp = mcp_over(&engine, heard, secrets, dir.path());
    let status = mcp.set_on(true).await.unwrap();
    let c = client(
        status.url.as_deref().unwrap(),
        status.token.as_deref().unwrap(),
    )
    .await;
    assert!(fails(&c, "create_draft", json!({"body": "x"}))
        .await
        .contains("no accounts"));
}

/// Fails every Keychain call.
struct Locked;

impl SecretStore for Locked {
    fn save(&self, _: &SecretRef, _: &Secret) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Unavailable("locked".into()))
    }
    fn load(&self, _: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
        Ok(None)
    }
    fn delete(&self, _: &SecretRef) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

/// Reads fine, but the item is something else.
struct Odd;

impl SecretStore for Odd {
    fn save(&self, _: &SecretRef, _: &Secret) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Unavailable("locked".into()))
    }
    fn load(&self, _: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
        Err(SecretStoreError::Corrupt("unreadable".into()))
    }
    fn delete(&self, _: &SecretRef) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

#[tokio::test]
async fn keychain_and_disk_problems_say_so_and_leave_it_off() {
    let s = setup();
    let locked = mcp_over(&s.engine, s.heard.clone(), Arc::new(Locked), s.dir.path());
    let e = locked.set_on(true).await.unwrap_err();
    assert!(matches!(e, McpError::Keychain(_)), "{e}");
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!("the keychain failed: the keychain is not available: locked")
    );
    assert!(!locked.status().on);
    let odd = mcp_over(&s.engine, s.heard.clone(), Arc::new(Odd), s.dir.path());
    assert!(matches!(
        odd.set_on(true).await.unwrap_err(),
        McpError::Keychain(_)
    ));
    // Moss's data folder can't be written.
    let file = s.dir.path().join("a-file");
    std::fs::write(&file, "").unwrap();
    let stuck = mcp_over(&s.engine, s.heard.clone(), s.secrets.clone(), &file);
    let e = stuck.set_on(true).await.unwrap_err();
    assert!(matches!(e, McpError::Settings(_)), "{e}");
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    secrets
        .save(
            &Mcp::item(),
            &Secret::Password {
                password: "x".into(),
            },
        )
        .unwrap();
    let other = mcp_over(&s.engine, s.heard.clone(), secrets, s.dir.path());
    assert!(
        other
            .set_on(true)
            .await
            .unwrap()
            .token
            .unwrap()
            .starts_with("moss_"),
        "a non-token item is replaced"
    );
}

#[tokio::test]
async fn a_broken_settings_file_counts_as_off() {
    let s = setup();
    std::fs::write(s.dir.path().join(SETTINGS_FILE), "{not json").unwrap();
    let mcp = mcp_over(&s.engine, s.heard.clone(), s.secrets.clone(), s.dir.path());
    assert!(!mcp.restore().await.unwrap().on);
}

#[test]
fn tokens_compare_in_full() {
    assert!(same(b"abc", b"abc"));
    assert!(!same(b"abc", b"abd"));
    assert!(!same(b"abc", b"ab"));
    assert_eq!(hex(&[0, 15, 255]), "000fff");
    assert_eq!(url(1), "http://127.0.0.1:1/mcp");
}

// The stdio bridge, as Claude Desktop runs it.

async fn bridged(url: String, token: String) -> RunningService<RoleClient, ()> {
    let (agent, bridge) = tokio::io::duplex(1 << 16);
    let (from_agent, to_agent) = tokio::io::split(bridge);
    tokio::spawn(async move {
        bridge::run(
            &url,
            &token,
            tokio::io::BufReader::new(from_agent),
            to_agent,
        )
        .await
        .unwrap();
    });
    let (read, write) = tokio::io::split(agent);
    ().serve((read, write)).await.unwrap()
}

#[tokio::test]
async fn the_bridge_carries_an_agent_over_stdio() {
    let s = setup();
    let status = s.mcp.set_on(true).await.unwrap();
    let c = bridged(status.url.unwrap(), status.token.unwrap()).await;
    assert_eq!(c.list_all_tools().await.unwrap().len(), 26);
    let v = ok(&c, "search_mail", json!({"query": "ramen"})).await;
    assert_eq!(v["threads"][0]["subject"], "Lunch on Friday");
}

async fn through_bridge(url: &str, token: &str, line: &str) -> String {
    let mut out = Vec::new();
    bridge::run(url, token, format!("{line}\n").as_bytes(), &mut out)
        .await
        .unwrap();
    String::from_utf8(out).unwrap()
}

const PING: &str = r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#;

#[tokio::test]
async fn the_bridge_says_when_the_token_is_wrong_or_the_address_is() {
    let s = setup();
    let url = s.mcp.set_on(true).await.unwrap().url.unwrap();
    assert!(through_bridge(&url, "moss_wrong", PING)
        .await
        .contains("refused the token"));
    let token = s.mcp.status().token.unwrap();
    assert!(through_bridge(&url.replace("/mcp", "/nope"), &token, PING)
        .await
        .contains("Moss answered 404"));
    // A request the server can't take comes back as its own JSON-RPC error.
    let out = through_bridge(
        &url,
        &token,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"x"}}"#,
    )
    .await;
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["id"], 4, "{out}");
    // A notification has no answer.
    assert_eq!(
        through_bridge(
            &url,
            &token,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .await,
        ""
    );
}

#[tokio::test]
async fn the_bridge_passes_on_streamed_answers() {
    use axum::routing::post;
    let app = axum::Router::new()
        .route(
            "/sse",
            post(|| async {
                (
                    [("content-type", "text/event-stream")],
                    "data: {\"id\":1}\n\n",
                )
            }),
        )
        .route(
            "/err",
            post(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{}}",
                )
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    assert_eq!(
        through_bridge(&format!("http://127.0.0.1:{port}/sse"), "t", PING).await,
        "{\"id\":1}\n"
    );
    assert!(
        through_bridge(&format!("http://127.0.0.1:{port}/err"), "t", PING)
            .await
            .contains("\"error\":{}")
    );
}

// Managing mail.

/// The id of the inbox thread with this subject.
async fn thread_id(c: &RunningService<RoleClient, ()>, subject: &str) -> String {
    let all = ok(c, "list_inbox", json!({"limit": 100})).await;
    let found = all["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["subject"] == subject)
        .cloned();
    found
        .map(|t| t["id"].as_str().unwrap().to_string())
        .unwrap_or_default()
}

fn told_mail_changed(s: &Setup, account: &str) -> bool {
    s.heard
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, Event::MailChanged { account_id } if account_id == account))
}

#[tokio::test]
async fn done_takes_a_thread_out_of_the_inbox_and_back() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    let v = ok(&c, "mark_done", json!({"thread_id": id})).await;
    assert_eq!(v, json!({"threadId": id, "done": "marked done"}));
    assert_eq!(thread_id(&c, "Lunch on Friday").await, "");
    assert!(told_mail_changed(&s, "work"), "the window reloads");
    ok(&c, "mark_done", json!({"thread_id": id, "done": false})).await;
    assert_eq!(thread_id(&c, "Lunch on Friday").await, id);
    assert!(fails(&c, "mark_done", json!({"thread_id": "nope"}))
        .await
        .contains("no thread with id nope"));
}

#[tokio::test]
async fn read_and_unread() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    ok(&c, "mark_read", json!({"thread_id": id})).await;
    assert!(!s.engine.thread(&id).unwrap().summary.unread);
    let v = ok(&c, "mark_read", json!({"thread_id": id, "read": false})).await;
    assert_eq!(v["done"], "marked unread");
    assert!(s.engine.thread(&id).unwrap().summary.unread);
}

#[tokio::test]
async fn remind_me_hides_a_thread_until_its_time() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    let v = ok(
        &c,
        "remind_me",
        json!({"thread_id": id, "until": "2025-11-03T08:00:00+01:00"}),
    )
    .await;
    assert_eq!(v["done"], "back at 2025-11-03T07:00:00Z");
    assert_eq!(thread_id(&c, "Lunch on Friday").await, "");
    let v = ok(&c, "remind_me", json!({"thread_id": id})).await;
    assert_eq!(v["done"], "no reminder");
    assert_eq!(thread_id(&c, "Lunch on Friday").await, id);
    assert!(fails(
        &c,
        "remind_me",
        json!({"thread_id": id, "until": "next tuesday"})
    )
    .await
    .contains("isn't a time Moss can read"));
}

#[tokio::test]
async fn remind_if_no_reply_only_waits_on_the_users_own_email() {
    let mine = raw(
        "mine@x",
        "Maya Ross <maya@northwind.co>",
        "Contract draft",
        "",
        "Here's the draft, let me know.",
    );
    let s = setup_with(&[mine]);
    let (_, c) = on(&s).await;
    let lunch = thread_id(&c, "Lunch on Friday").await;
    assert!(fails(&c, "remind_if_no_reply", json!({"thread_id": lunch}))
        .await
        .contains("isn't the user's"));
    let found = ok(&c, "search_mail", json!({"query": "let me know"})).await;
    let id = found["threads"][0]["id"].clone();
    assert_eq!(
        ok(&c, "remind_if_no_reply", json!({"thread_id": id})).await["done"],
        "waiting for a reply"
    );
    assert_eq!(
        ok(
            &c,
            "remind_if_no_reply",
            json!({"thread_id": id, "on": false})
        )
        .await["done"],
        "not waiting"
    );
    assert!(
        fails(&c, "remind_if_no_reply", json!({"thread_id": "nope"}))
            .await
            .contains("no thread")
    );
}

#[tokio::test]
async fn threads_move_between_splits_or_back_to_moss() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    assert_eq!(
        ok(
            &c,
            "move_thread",
            json!({"thread_id": id, "to": "Receipts"})
        )
        .await["done"],
        "moved to Receipts"
    );
    assert_eq!(
        s.engine.thread(&id).unwrap().summary.split,
        SplitId::Receipts
    );
    ok(
        &c,
        "move_thread",
        json!({"thread_id": id, "to": "important"}),
    )
    .await;
    assert_eq!(
        s.engine.thread(&id).unwrap().summary.split,
        SplitId::Important
    );
    ok(&c, "move_thread", json!({"thread_id": id, "to": "auto"})).await;
    assert!(
        fails(&c, "move_thread", json!({"thread_id": id, "to": "spam"}))
            .await
            .contains("no split called spam")
    );
}

#[tokio::test]
async fn delete_goes_to_trash_and_restore_brings_it_back() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    assert_eq!(
        ok(&c, "delete_thread", json!({"thread_id": id})).await["done"],
        "moved to Trash"
    );
    assert_eq!(thread_id(&c, "Lunch on Friday").await, "");
    assert_eq!(
        ok(&c, "restore_thread", json!({"thread_id": id})).await["done"],
        "restored"
    );
    assert_eq!(thread_id(&c, "Lunch on Friday").await, id);
}

#[tokio::test]
async fn senders_are_blocked_accepted_and_unblocked() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    assert_eq!(
        ok(&c, "block_sender", json!({"thread_id": id})).await["done"],
        "sender blocked"
    );
    assert_eq!(thread_id(&c, "Lunch on Friday").await, "");
    assert!(s
        .engine
        .sender_rules()
        .unwrap()
        .iter()
        .any(|r| r.sender == "daniel@northwind.co"));
    assert_eq!(
        ok(&c, "unblock_sender", json!({"thread_id": id})).await["done"],
        "sender unblocked"
    );
    assert_eq!(thread_id(&c, "Lunch on Friday").await, id);
    assert_eq!(
        ok(
            &c,
            "accept_sender",
            json!({"thread_id": id, "whole_domain": true})
        )
        .await["done"],
        "sender accepted"
    );
}

// Sending.

#[tokio::test]
async fn a_send_that_fails_keeps_the_email_as_a_draft() {
    let s = setup();
    let (_, c) = on(&s).await;
    let id = thread_id(&c, "Lunch on Friday").await;
    // These accounts have no server, so nothing can go.
    let why = fails(
        &c,
        "send",
        json!({"reply_to_thread": id, "body": "Yes, Friday works."}),
    )
    .await;
    assert!(why.starts_with("Couldn't send: "), "{why}");
    assert!(why.contains("kept as a draft"), "{why}");
    let drafts = s.engine.drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].body, "Yes, Friday works.");
    assert!(why.contains(&drafts[0].id));
    // A draft sent by id fails the same way, and stays.
    let again = fails(&c, "send", json!({"draft_id": drafts[0].id, "revision": drafts[0].revision})).await;
    assert!(again.contains("kept as a draft"));
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
    let drafts = s.engine.drafts().unwrap();
    // Send later waits in Moss and goes at its time.
    let later = ok(
        &c,
        "send",
        json!({"draft_id": drafts[0].id, "revision": drafts[0].revision, "send_at": "2030-01-01T09:00:00Z"}),
    )
    .await;
    assert_eq!(later["scheduled"], "2030-01-01T09:00:00Z");
    assert_eq!(s.engine.scheduled().unwrap().len(), 1);
    assert!(s.engine.drafts().unwrap().is_empty(), "it leaves Drafts");
    assert!(fails(
        &c,
        "send",
        json!({"draft_id": drafts[0].id, "revision": drafts[0].revision, "send_at": "soon"})
    )
    .await
    .contains("isn't a time"));
    assert!(fails(&c, "send", json!({"to": ["nobody"]}))
        .await
        .contains("There's no one called nobody"));
}

/// Sends nothing: notes what would go.
#[derive(Default)]
struct Kept {
    sent: Mutex<Vec<OutgoingDraft>>,
    later: Mutex<Vec<(String, i64)>>,
    operations: Mutex<Vec<String>>,
    refuse_later: bool,
    /// What scheduling answers instead of Scheduled.
    outcome: Option<moss_store::ScheduleOutcome>,
    /// The engine's answer for a draft id already taken.
    conflict: bool,
    uncertain: bool,
    rejected: bool,
    busy: bool,
    rewrite_after_send: Option<moss_engine::Engine>,
    restore_before_rejection: Option<std::path::PathBuf>,
}

#[async_trait::async_trait]
impl tools::Outbox for Kept {
    async fn send(&self, draft: &moss_core::Draft, email: OutgoingDraft) -> moss_engine::Result<String> {
        if self.busy { return Err(moss_engine::Error::DeliveryBusy); }
        if self.conflict { return Err(moss_engine::Error::DraftConflict); }
        self.sent.lock().unwrap().push(email);
        if self.rejected {
            if let Some(path) = &self.restore_before_rejection {
                let mut store=moss_store::Store::open(path).unwrap();
                store.prepare_delivery("rejected-operation",draft,"{}","rejected@fixture",now()).unwrap();
                store.cancel_delivery("rejected-operation").unwrap();
            }
            return Err(moss_engine::Error::Offline("fixture provider unreachable".into()));
        }
        if self.uncertain { return Err(moss_engine::Error::DeliveryUnknown("lost reply".into())); }
        if let Some(engine) = &self.rewrite_after_send {
            let mut changed = draft.clone();
            changed.body = "Newer independently kept text".into();
            engine.save_draft(&changed)?;
        }
        Ok("sent-thread".into())
    }
    fn schedule(&self, operation: &str, draft: &moss_core::Draft, _: &OutgoingDraft, at: i64) -> moss_engine::Result<moss_store::ScheduleOutcome> {
        if self.refuse_later {
            return Err(moss_engine::Error::NoRecipients);
        }
        if self.conflict { return Err(moss_engine::Error::DraftConflict); }
        self.later.lock().unwrap().push((draft.id.clone(), at));
        self.operations.lock().unwrap().push(operation.into());
        Ok(self.outcome.clone().unwrap_or(moss_store::ScheduleOutcome::Scheduled))
    }
}

fn tools_with(s: &Setup, outbox: Arc<Kept>) -> MossTools {
    MossTools::with_outbox(
        s.engine.clone(),
        s.heard.clone(),
        outbox,
        now,
        s.dir.path().join("attachments"),
    )
}

fn sent_json(r: Result<String, String>) -> Value {
    serde_json::from_str(&r.unwrap()).unwrap()
}

#[tokio::test]
async fn an_outbox_claim_race_does_not_restore_or_recreate_a_draft() {
    use rmcp::handler::server::wrapper::Parameters;
    let s=setup();let kept=Arc::new(Kept {busy:true,..Default::default()});
    let tools=tools_with(&s,kept.clone());
    let error=tools.send(Parameters(tools::SendArgs {to:Some(vec!["ana@example.test".into()]),body:Some("Racing owner".into()),..Default::default()})).await.unwrap_err();
    assert!(error.contains("in progress"));
    assert!(!error.contains("kept as a draft"));
    assert!(s.engine.drafts().unwrap().is_empty());
    assert!(kept.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn send_sends_a_reply_as_the_window_would_and_says_so() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let lunch = s
        .engine
        .threads(Some("work"))
        .unwrap()
        .into_iter()
        .find(|t| t.subject == "Lunch on Friday")
        .unwrap();
    let args = tools::SendArgs {
        reply_to_thread: Some(lunch.id.clone()),
        body: Some("Yes, Friday works.\n\nMaya".into()),
        ..Default::default()
    };
    let v = sent_json(tools.send(Parameters(args)).await);
    assert_eq!(v["sent"], true);
    assert_eq!(v["threadId"], "sent-thread");
    assert!(v.get("draftCleanupFailed").is_none());
    assert_eq!(v["to"], json!(["Daniel Okafor <daniel@northwind.co>"]));
    let sent = kept.sent.lock().unwrap();
    assert_eq!(sent[0].mode, ComposeMode::Reply);
    assert_eq!(sent[0].thread_id.as_deref(), Some(lunch.id.as_str()));
    assert_eq!(sent[0].subject, "Re: Lunch on Friday");
    assert_eq!(sent[0].body_html, "Yes, Friday works.<br><br>Maya");
    assert!(sent[0].signature.is_empty());
    drop(sent);
    assert!(
        s.engine.drafts().unwrap().is_empty(),
        "a new email leaves no draft behind"
    );
    assert!(told_mail_changed(&s, "work"));
}

#[tokio::test]
async fn an_uncertain_mcp_send_is_not_recreated_as_a_sendable_draft() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept { uncertain: true, ..Default::default() });
    let tools = tools_with(&s, kept.clone());
    let args = tools::SendArgs { to: Some(vec!["dana@example.test".into()]), subject: Some("Once".into()), body: Some("Do not duplicate".into()), ..Default::default() };
    let error = tools.send(Parameters(args)).await.unwrap_err();
    assert!(!error.contains("kept as a draft"));
    assert!(error.contains("Do not resend"));
    assert!(s.engine.drafts().unwrap().is_empty());
    assert_eq!(kept.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_mcp_draft_restore_preserves_the_provider_error() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept { rejected: true, ..Default::default() });
    let tools = tools_with(&s, kept);
    let db = rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    let error = tools.send(Parameters(tools::SendArgs {
        to: Some(vec!["dana@example.test".into()]), subject: Some("Keep text".into()), body: Some("Unsent".into()), ..Default::default()
    })).await.unwrap_err();
    assert!(error.contains("fixture provider unreachable"), "{error}");
    assert!(error.contains("could not be restored"));
    assert!(!error.contains("kept as a draft"));
    drop(db);
}

#[tokio::test]
async fn a_direct_send_with_supplied_files_leaves_no_sendable_draft() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let args: tools::SendArgs = serde_json::from_value(json!({
        "to": ["ana@northwind.co"], "subject": "Notes", "body": "Attached",
        "attachments": [{"name": "notes.txt", "content_base64": "aGk="}]
    })).unwrap();
    assert_eq!(sent_json(tools.send(Parameters(args)).await)["sent"], true);
    assert_eq!(kept.sent.lock().unwrap().len(), 1);
    assert_eq!(kept.sent.lock().unwrap()[0].attachments[0].name, "notes.txt");
    assert!(s.engine.drafts().unwrap().is_empty(), "accepted mail must not remain sendable in Drafts");
}

#[tokio::test]
async fn repeating_a_completed_mcp_draft_returns_its_receipt_without_sending() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let tools = MossTools::new(s.engine.clone(), s.heard.clone(), now, s.dir.path().join("attachments"));
    tools.create_draft(Parameters(tools::CreateArgs {
        to: Some(vec!["ana@northwind.co".into()]), subject: Some("Once".into()),
        body: Some("Already delivered".into()), ..Default::default()
    })).await.unwrap();
    let draft = s.engine.drafts().unwrap().remove(0);
    let email = OutgoingDraft::from(&draft);
    let message = moss_engine::compose::outgoing(&email,
        &moss_core::Person { name: "Maya".into(), email: "maya@northwind.co".into() },
        None, None, chrono::FixedOffset::east_opt(0).unwrap());
    let payload = json!({"email": email, "message": message, "forwarded_message": null}).to_string();
    s.engine.recover_deliveries().unwrap();
    let mut store = Store::open(s.dir.path().join("moss.db")).unwrap();
    store.prepare_delivery("completed-operation", &draft, &payload, "once@example.test", now()).unwrap();
    let busy = tools.send(Parameters(tools::SendArgs { draft_id: Some(draft.id.clone()), revision: draft.revision.clone(), ..Default::default() })).await.unwrap_err();
    assert!(busy.contains("send in progress"), "{busy}");
    assert!(!busy.contains("could not be restored"), "a busy send must not attempt restoration: {busy}");
    assert_eq!(store.delivery_state("completed-operation").unwrap(), Some(moss_store::DeliveryState::Held));
    assert!(s.engine.drafts().unwrap().is_empty());
    let stopped_db=rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
    stopped_db.execute_batch("INSERT INTO delivery_cancellations(id) VALUES('completed-operation'); UPDATE deliveries SET cancel_intent=1 WHERE id='completed-operation';").unwrap();
    let stopped=tools.send(Parameters(tools::SendArgs {draft_id:Some(draft.id.clone()),revision:draft.revision.clone(),..Default::default()})).await.unwrap_err();
    assert!(stopped.contains("cancelled"),"{stopped}");
    assert!(stopped.to_lowercase().contains("recover"),"{stopped}");
    assert!(!stopped.contains("in progress"));
    stopped_db.execute_batch("DELETE FROM delivery_cancellations WHERE id='completed-operation'; UPDATE deliveries SET cancel_intent=0 WHERE id='completed-operation';").unwrap();
    store.claim_delivery("completed-operation").unwrap();
    let busy = tools.send(Parameters(tools::SendArgs { draft_id: Some(draft.id.clone()), revision: draft.revision.clone(), ..Default::default() })).await.unwrap_err();
    assert!(busy.contains("send in progress"));
    assert_eq!(store.delivery_state("completed-operation").unwrap(), Some(moss_store::DeliveryState::Claimed));
    store.accept_delivery("completed-operation", b"accepted", "{}").unwrap();
    let sent=store.ingest(&raw("once@example.test","Maya <maya@northwind.co>","Once","","Already delivered"),&source("work",999,false)).unwrap();
    store.complete_delivery_with_receipt("completed-operation", Some(&sent.thread_id)).unwrap();
    // A terminal retry must not reconstruct content, signatures, or attachment payloads.
    let receipt_db = rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
    receipt_db.execute("UPDATE deliveries SET draft='{',outgoing='{' WHERE id='completed-operation'", []).unwrap();
    let error=tools.update_draft(Parameters(tools::UpdateArgs {
        draft_id:draft.id.clone(), revision:draft.revision.clone().unwrap(), body:Some("changed".into()), ..Default::default()
    })).await.unwrap_err();
    assert!(error.contains("already has a delivery record"),"{error}");
    for revision in [None, Some("stale".into())] {
        let error = tools.send(Parameters(tools::SendArgs { draft_id: Some(draft.id.clone()), revision, ..Default::default() })).await.unwrap_err();
        assert!(error.contains("Do not resend"),"{error}");
    }
    // Content-changing and scheduled retries must be rejected before mutable lookups.
    for extra in [json!({"body":"different"}), json!({"subject":"different"}),
        json!({"to":[]}), json!({"cc":[]}), json!({"bcc":[]}), json!({"from":"invalid"}),
        json!({"signature":"missing"}), json!({"account":"missing"}), json!({"files":[]}),
        json!({"attachments":[]}), json!({"reply_to_thread":"missing"}),
        json!({"reply_all":true}), json!({"forward_thread":"missing"}),
        json!({"send_at":"2027-01-01T00:00:00Z"})] {
        let mut args=json!({"draft_id":draft.id,"revision":draft.revision});
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let error=tools.send(Parameters(serde_json::from_value(args).unwrap())).await.unwrap_err();
        assert!(error.contains("already has a delivery record"),"{error}");
        assert!(!error.contains("could not be restored"));
    }
    for _ in 0..2 {
        let receipt = sent_json(tools.send(Parameters(tools::SendArgs {
            draft_id: Some(draft.id.clone()), revision: draft.revision.clone(), ..Default::default()
        })).await);
        assert_eq!(receipt["sent"], true);
        assert_eq!(receipt["threadId"], sent.thread_id);
        assert_eq!(receipt["alreadySent"], true);
        assert!(s.engine.drafts().unwrap().is_empty());
    }
    let db = rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
    let receipt = sent_json(tools.send(Parameters(tools::SendArgs { draft_id: Some(draft.id.clone()), revision: draft.revision.clone(), ..Default::default() })).await);
    assert_eq!(receipt["sent"], true);
    assert_eq!(receipt["threadId"], sent.thread_id);
    assert!(receipt["draftCleanupFailed"].is_null(), "a consumed row needs no cleanup");
    assert_eq!(store.delivery_state("completed-operation").unwrap(), Some(moss_store::DeliveryState::Complete));
    assert!(tools.send(Parameters(tools::SendArgs {
        draft_id: Some(draft.id), revision: draft.revision.clone(), body: Some("Different content".into()), ..Default::default()
    })).await.is_err());
    assert!(s.engine.drafts().unwrap().is_empty());
    db.execute("INSERT INTO drafts(id,account_id,mode,to_json,cc_json,bcc_json,subject,body,body_html,remind_if_no_reply,updated_at)
        SELECT draft_id,account_id,'new','[]','[]','[]','Leftover','Newer unrelated edit','',0,0 FROM deliveries WHERE id='completed-operation'",[]).unwrap();
    let leftover=s.engine.drafts().unwrap().remove(0);
    let receipt=sent_json(tools.send(Parameters(tools::SendArgs {draft_id:Some(leftover.id.clone()),revision:draft.revision,..Default::default()})).await);
    assert_eq!(receipt["alreadySent"],true);
    assert_eq!(receipt["threadId"],sent.thread_id);
    assert_eq!(s.engine.drafts().unwrap()[0].body,"Newer unrelated edit");
}

#[tokio::test]
async fn cleanup_failure_after_success_keeps_the_mcp_sent_receipt() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    tools.create_draft(Parameters(tools::CreateArgs {
        to: Some(vec!["ana@example.test".into()]), subject: Some("Send once".into()),
        body: Some("Do not repeat after cleanup failure".into()), ..Default::default()
    })).await.unwrap();
    let draft = s.engine.drafts().unwrap().remove(0);
    let db = rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_post_send_cleanup BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let result = sent_json(tools.send(Parameters(tools::SendArgs { draft_id: Some(draft.id.clone()), revision: draft.revision.clone(), ..Default::default() })).await);
    assert_eq!(result["sent"], true);
    assert_eq!(result["threadId"], "sent-thread");
    assert!(result["note"].as_str().unwrap().contains("Do not resend"));
    assert_eq!(result["draftCleanupFailed"], true);
    assert_eq!(kept.sent.lock().unwrap().len(), 1);
    assert_eq!(s.engine.drafts().unwrap()[0].id, draft.id);
}

#[tokio::test]
async fn interrupted_mcp_receipts_attempt_recovery_and_explain_a_blocked_sweep() {
    use rmcp::handler::server::wrapper::Parameters;
    for blocked in [false,true] {
        let s=setup();
        let tools=MossTools::new(s.engine.clone(),s.heard.clone(),now,s.dir.path().join("attachments"));
        tools.create_draft(Parameters(tools::CreateArgs {to:Some(vec!["ana@example.test".into()]),body:Some("Interrupted".into()),..Default::default()})).await.unwrap();
        let draft=s.engine.drafts().unwrap().remove(0);
        let mut store=Store::open(s.dir.path().join("moss.db")).unwrap();
        store.prepare_delivery("interrupted-mcp",&draft,"{}","fixture@example.test",0).unwrap();
        let db=rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
        if blocked { db.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap(); }
        let error=tools.send(Parameters(tools::SendArgs {draft_id:Some(draft.id.clone()),revision:draft.revision.clone(),..Default::default()})).await.unwrap_err();
        assert!(error.to_lowercase().contains("recover"),"{error}");
        assert!(!error.contains("in progress"),"{error}");
        if blocked { assert!(error.contains("Do not resend"),"{error}"); }
        else { assert_eq!(store.delivery_state("interrupted-mcp").unwrap(),Some(moss_store::DeliveryState::Cancelled)); }
    }
}

#[tokio::test]
async fn sending_a_draft_by_id_takes_it_out_of_drafts() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let made = tools
        .create_draft(Parameters(tools::CreateArgs {
            to: Some(vec!["ana@northwind.co".into()]),
            subject: Some("Notes".into()),
            body: Some("Attached.".into()),
            ..Default::default()
        }))
        .await
        .unwrap();
    let id = serde_json::from_str::<Value>(&made).unwrap()["draftId"]
        .as_str()
        .unwrap()
        .to_string();
    let args = tools::SendArgs {
        draft_id: Some(id.clone()),
        revision: serde_json::from_str::<Value>(&made).unwrap()["revision"].as_str().map(str::to_owned),
        subject: Some("Meeting notes".into()),
        ..Default::default()
    };
    let v = sent_json(tools.send(Parameters(args)).await);
    assert_eq!(v["subject"], "Meeting notes");
    assert_eq!(kept.sent.lock().unwrap()[0].body, "Attached.");
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn send_later_sets_a_time_instead_of_sending() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let args = tools::SendArgs {
        to: Some(vec!["ana@northwind.co".into()]),
        body: Some("Morning!".into()),
        send_at: Some("2030-01-01T09:00:00Z".into()),
        ..Default::default()
    };
    let v = sent_json(tools.send(Parameters(args)).await);
    assert_eq!(v["scheduled"], "2030-01-01T09:00:00Z");
    assert!(kept.sent.lock().unwrap().is_empty(), "nothing goes now");
    assert_eq!(kept.later.lock().unwrap()[0].1, 1_893_488_400_000);
    // A time already past sends as soon as it can, and says when.
    let past = tools::SendArgs {
        to: Some(vec!["ana@northwind.co".into()]),
        send_at: Some("2001-01-01T00:00:00Z".into()),
        ..Default::default()
    };
    assert_eq!(
        sent_json(tools.send(Parameters(past)).await)["scheduled"],
        when_now()
    );
    let refused = Arc::new(Kept {
        refuse_later: true,
        ..Default::default()
    });
    let tools = tools_with(&s, refused);
    let args = tools::SendArgs {
        send_at: Some("2030-01-01T09:00:00Z".into()),
        ..Default::default()
    };
    assert_eq!(
        tools.send(Parameters(args)).await.unwrap_err(),
        "add someone to send it to"
    );
}

#[tokio::test]
async fn every_send_result_names_its_draft_and_its_request_id_gives_the_same_one() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let email = |request: Option<&str>, at: Option<&str>| tools::SendArgs {
        to: Some(vec!["ana@northwind.co".into()]), subject: Some("Plans".into()), body: Some("Friday?".into()),
        request_id: request.map(String::from), send_at: at.map(String::from), ..Default::default()
    };
    let plain = sent_json(tools.send(Parameters(email(None, None))).await);
    assert_eq!(plain["draftId"].as_str().unwrap().len(), 36);
    let (draft, operation) = tools::request_ids("agent-1");
    let named = sent_json(tools.send(Parameters(email(Some(" agent-1 "), None))).await);
    assert_eq!(named["draftId"], draft);
    assert!(named.get("deliveryId").is_none(), "this outbox keeps no record");
    // A blank request_id is none.
    assert_ne!(sent_json(tools.send(Parameters(email(Some(" "), None))).await)["draftId"], plain["draftId"]);
    let (later, later_operation) = tools::request_ids("agent-2");
    let scheduled = sent_json(tools.send(Parameters(email(Some("agent-2"), Some("2030-01-01T09:00:00Z")))).await);
    assert_eq!((&scheduled["draftId"], &scheduled["scheduledId"], &scheduled["operationId"]), (&json!(later), &json!(later), &json!(later_operation)));
    assert_eq!(kept.operations.lock().unwrap().last(), Some(&later_operation));
    assert_ne!(operation, later_operation);
    let random = sent_json(tools.send(Parameters(email(None, Some("2030-01-01T09:00:00Z")))).await);
    assert_eq!(random["operationId"].as_str().unwrap().len(), 36);
    assert_ne!(random["operationId"], later_operation);
}

#[tokio::test]
async fn a_send_kept_as_a_draft_is_not_sent_again_by_its_retry() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept { rejected: true, ..Default::default() });
    let tools = tools_with(&s, kept.clone());
    let email = || tools::SendArgs { to: Some(vec!["ana@northwind.co".into()]), subject: Some("Once".into()), request_id: Some("kept-1".into()), ..Default::default() };
    let (id, _) = tools::request_ids("kept-1");
    let first = tools.send(Parameters(email())).await.unwrap_err();
    assert!(first.contains("kept as a draft") && first.contains(&id), "{first}");
    let draft = s.engine.drafts().unwrap().remove(0);
    let retry = tools.send(Parameters(email())).await.unwrap_err();
    assert_eq!(retry, format!("This email wasn't sent: it's in Drafts, id {id}, revision {}. Send it with that draft_id and revision, or give a new request_id for a new email.", draft.revision.unwrap()));
    assert_eq!(kept.sent.lock().unwrap().len(), 1, "never handed over twice");
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
}

#[tokio::test]
async fn a_request_id_used_for_an_email_now_gone_sends_nothing_and_says_so() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept { conflict: true, ..Default::default() });
    let tools = tools_with(&s, kept.clone());
    let email = |request: Option<&str>, at: Option<&str>| tools::SendArgs {
        to: Some(vec!["ana@northwind.co".into()]), request_id: request.map(String::from), send_at: at.map(String::from), ..Default::default()
    };
    let gone = "This request_id was used before, for an email that isn't sent, waiting or in Drafts now. Nothing was sent. Give a new request_id for a new email.";
    assert_eq!(tools.send(Parameters(email(Some("gone-1"), None))).await.unwrap_err(), gone);
    assert_eq!(tools.send(Parameters(email(Some("gone-1"), Some("2030-01-01T09:00:00Z")))).await.unwrap_err(), gone);
    assert!(s.engine.drafts().unwrap().is_empty(), "nothing kept in its name");
    // Without one, it's the engine's own words.
    let plain = tools.send(Parameters(email(None, Some("2030-01-01T09:00:00Z")))).await.unwrap_err();
    assert_eq!(plain, moss_engine::Error::DraftConflict.to_string());
    assert!(kept.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_send_later_the_engine_had_already_is_not_set_twice() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept { outcome: Some(moss_store::ScheduleOutcome::AlreadyScheduled), ..Default::default() });
    let tools = tools_with(&s, kept);
    let args = tools::SendArgs { to: Some(vec!["ana@northwind.co".into()]), request_id: Some("race-1".into()), send_at: Some("2030-01-01T09:00:00Z".into()), ..Default::default() };
    let v = sent_json(tools.send(Parameters(args)).await);
    let (draft, operation) = tools::request_ids("race-1");
    assert_eq!(v, json!({"draftId": draft, "operationId": operation, "outcome": {"status": "alreadyScheduled"},
        "note": "Moss had this request already. Nothing new was sent or set to send later."}));
}

#[tokio::test]
async fn a_send_retried_with_its_request_id_answers_what_became_of_it_and_never_sends_twice() {
    let s = setup();
    let (_, c) = on(&s).await;
    let email = |request: &str| json!({"to": ["ana@northwind.co"], "subject": "Once only", "body": "Hello", "request_id": request});
    let mut store = Store::open(s.dir.path().join("moss.db")).unwrap();
    // The first try didn't go (these accounts have no server): it's kept as a draft.
    let (id, _) = tools::request_ids("once-1");
    assert!(fails(&c, "send", email("once-1")).await.contains(&format!("kept as a draft, id {id}")));
    assert!(fails(&c, "send", email("once-1")).await.starts_with("This email wasn't sent: it's in Drafts"));
    let draft = s.engine.editable_draft(&id).unwrap().unwrap();
    // As a send that is under way records it.
    store.prepare_delivery("once-operation", &draft, "{}", "once@example.test", now()).unwrap();
    assert!(fails(&c, "send", email("once-1")).await.contains("send in progress"), "held");
    assert!(store.claim_delivery("once-operation").unwrap());
    assert!(fails(&c, "send", email("once-1")).await.contains("send in progress"), "claimed");
    assert!(store.accept_delivery("once-operation", b"accepted", "{}").unwrap());
    let copy = store.ingest(&raw("once@example.test", "Maya <maya@northwind.co>", "Once only", "", "Hello"), &source("work", 99, false)).unwrap();
    assert!(store.complete_delivery_with_receipt("once-operation", Some(&copy.thread_id)).unwrap());
    for _ in 0..2 {
        let sent = ok(&c, "send", email("once-1")).await;
        assert_eq!(sent, json!({"sent": true, "alreadySent": true, "threadId": copy.thread_id, "draftId": id, "deliveryId": "once-operation", "note": "Already sent. Do not resend this email."}));
    }
    // One whose outcome Moss couldn't tell may have gone.
    let (other, _) = tools::request_ids("once-2");
    fails(&c, "send", email("once-2")).await;
    let draft = s.engine.editable_draft(&other).unwrap().unwrap();
    store.prepare_delivery("unsure-operation", &draft, "{}", "unsure@example.test", now()).unwrap();
    assert!(store.claim_delivery("unsure-operation").unwrap());
    store.uncertain_delivery("unsure-operation", "Reply lost").unwrap();
    let unsure = fails(&c, "send", email("once-2")).await;
    assert!(unsure.contains("may have succeeded") && unsure.contains("check Sent") && unsure.contains("Do not resend"), "{unsure}");
    assert!(s.engine.drafts().unwrap().is_empty());
    let delivered: i64 = rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap()
        .query_row("SELECT count(*) FROM deliveries", [], |r| r.get(0)).unwrap();
    assert_eq!(delivered, 2, "no retry made a delivery of its own");
}

#[tokio::test]
async fn a_send_later_retried_with_its_request_id_is_set_once() {
    let s = setup();
    let (_, c) = on(&s).await;
    let later = json!({"to": ["ana@northwind.co"], "subject": "Later", "send_at": "2030-01-01T09:00:00Z", "request_id": "later-1"});
    let (id, operation) = tools::request_ids("later-1");
    let first = ok(&c, "send", later.clone()).await;
    assert_eq!((&first["scheduledId"], &first["operationId"]), (&json!(id), &json!(operation)));
    let again = json!({"scheduled": "2030-01-01T09:00:00Z", "alreadyScheduled": true, "scheduledId": id, "draftId": id,
        "operationId": operation, "subject": "Later", "to": ["ana@northwind.co"],
        "note": "Already set to send later. Nothing new was sent or set to send later. cancel_scheduled changes it."});
    assert_eq!(ok(&c, "send", later.clone()).await, again);
    // Retried as a send now, or by its draft id, it's still only set for later.
    let now = json!({"to": ["ana@northwind.co"], "subject": "Later", "request_id": "later-1"});
    assert_eq!(ok(&c, "send", now).await, again);
    assert_eq!(ok(&c, "send", json!({"draft_id": id})).await, again);
    let scheduled = s.engine.scheduled().unwrap();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].operation_id, operation);
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn the_send_tool_asks_for_a_request_id_to_retry_with() {
    let s = setup();
    let (_, c) = on(&s).await;
    let tool = c.list_all_tools().await.unwrap().into_iter().find(|t| t.name == "send").unwrap();
    let schema = serde_json::to_value(&tool).unwrap();
    let said = schema["inputSchema"]["properties"]["request_id"]["description"].as_str().unwrap();
    assert!(said.contains("the same request_id") && said.contains("never sends it twice"), "{said}");
    assert!(tool.description.unwrap().contains("use the same value when retrying the same email"));
    // Moss's own chat never sends, so has none.
    let (t, _) = chat(&s);
    let write = t.chat_specs().into_iter().find(|spec| spec["function"]["name"] == "write_email").unwrap();
    assert!(write["function"]["parameters"]["properties"].get("request_id").is_none());
}

fn when_now() -> String {
    chrono::DateTime::from_timestamp_millis(now())
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// Writing as the window does: signatures, reply all, forward, Bcc, From and files.

/// The id of the thread in `account` with this subject, from the engine.
fn subject_id(s: &Setup, account: &str, subject: &str) -> String {
    s.engine
        .threads(Some(account))
        .unwrap()
        .into_iter()
        .find(|t| t.subject == subject)
        .unwrap()
        .id
}

fn signature(id: &str, lines: &[&str]) -> moss_core::Signature {
    moss_core::Signature {
        id: id.into(),
        name: id.into(),
        lines: lines.iter().map(|l| l.to_string()).collect(),
    }
}

#[tokio::test]
async fn emails_carry_the_signature_moss_picks_unless_told_otherwise() {
    use moss_core::SignatureUse;
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    s.engine
        .save_signature(&signature("Full", &["Maya Ross", "Northwind"]))
        .unwrap();
    s.engine
        .save_signature(&signature("Short", &["Maya"]))
        .unwrap();
    s.engine
        .set_signature_choice("work", SignatureUse::NewEmails, Some("Full"))
        .unwrap();
    s.engine
        .set_signature_choice("work", SignatureUse::Replies, Some("Short"))
        .unwrap();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let new = |sig: Option<&str>| tools::SendArgs {
        account: Some("work".into()),
        to: Some(vec!["ana@northwind.co".into()]),
        signature: sig.map(Into::into),
        ..Default::default()
    };
    tools.send(Parameters(new(None))).await.unwrap();
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    tools
        .send(Parameters(tools::SendArgs {
            reply_to_thread: Some(lunch),
            ..Default::default()
        }))
        .await
        .unwrap();
    tools.send(Parameters(new(Some("none")))).await.unwrap();
    // By name, in any case.
    tools.send(Parameters(new(Some("short")))).await.unwrap();
    let lines: Vec<Vec<String>> = kept
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|e| e.signature.clone())
        .collect();
    assert_eq!(
        lines,
        [
            vec!["Maya Ross".to_string(), "Northwind".into()],
            vec!["Maya".into()],
            vec![],
            vec!["Maya".into()]
        ]
    );
    assert_eq!(
        s.engine.signatures().unwrap().recent[0],
        "Short",
        "used last"
    );
    assert!(tools
        .send(Parameters(new(Some("Fancy"))))
        .await
        .unwrap_err()
        .contains("no signature called Fancy"));
    // A draft keeps the pick, so the window shows the same signature.
    let made = tools
        .create_draft(Parameters(tools::CreateArgs {
            account: Some("work".into()),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&made).unwrap()["signature"],
        "Full"
    );
    assert_eq!(
        s.engine.drafts().unwrap()[0].signature_id.as_deref(),
        Some("Full")
    );
    // One chosen stays chosen when the draft changes later.
    let chosen = tools
        .create_draft(Parameters(tools::CreateArgs {
            account: Some("work".into()),
            signature: Some("Short".into()),
            ..Default::default()
        }))
        .await
        .unwrap();
    let id = serde_json::from_str::<Value>(&chosen).unwrap()["draftId"]
        .as_str()
        .unwrap()
        .to_string();
    let changed = tools
        .update_draft(Parameters(tools::UpdateArgs {
            draft_id: id,
            revision: serde_json::from_str::<Value>(&chosen).unwrap()["revision"].as_str().unwrap().to_owned(),
            subject: Some("Still short".into()),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&changed).unwrap()["signature"],
        "Short"
    );
}

#[tokio::test]
async fn reply_all_answers_everyone_but_the_user_and_forward_keeps_the_files() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    tools
        .send(Parameters(tools::SendArgs {
            reply_to_thread: Some(lunch.clone()),
            reply_all: Some(true),
            ..Default::default()
        }))
        .await
        .unwrap();
    let contract = subject_id(&s, "work", "Signed contract");
    let v = sent_json(
        tools
            .send(Parameters(tools::SendArgs {
                forward_thread: Some(contract.clone()),
                to: Some(vec!["sam@home.example".into()]),
                body: Some("FYI".into()),
                ..Default::default()
            }))
            .await,
    );
    assert_eq!(v["subject"], "Fwd: Signed contract");
    let sent = kept.sent.lock().unwrap();
    assert_eq!(sent[0].mode, ComposeMode::ReplyAll);
    assert_eq!(
        tools::whom(&sent[0].to),
        ["Daniel Okafor <daniel@northwind.co>"]
    );
    assert_eq!(tools::whom(&sent[0].cc), ["Ana <ana@northwind.co>"]);
    assert_eq!(sent[1].mode, ComposeMode::Forward);
    assert_eq!(sent[1].thread_id.as_deref(), Some(contract.as_str()));
    assert_eq!(sent[1].attachments.len(), 1);
    assert_eq!(sent[1].attachments[0].name, "Contract.pdf");
    assert!(sent[1].attachments[0].forwarded.is_some());
    drop(sent);
    let both = tools::SendArgs {
        reply_to_thread: Some(lunch.clone()),
        forward_thread: Some(contract),
        ..Default::default()
    };
    assert!(tools
        .send(Parameters(both))
        .await
        .unwrap_err()
        .contains("either"));
    let lonely = tools::SendArgs {
        reply_all: Some(true),
        to: Some(vec!["ana@northwind.co".into()]),
        ..Default::default()
    };
    assert!(tools
        .send(Parameters(lonely))
        .await
        .unwrap_err()
        .contains("reply_all needs reply_to_thread"));
}

fn to_alias(id: &str) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(NOV_1)
        .unwrap()
        .to_rfc2822();
    format!(
        "From: Lee <lee@client.co>\r\nTo: Maya Ross <hello@northwind.co>\r\nSubject: Hello alias\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nHi there\r\n"
    )
    .into_bytes()
}

#[tokio::test]
async fn bcc_and_the_address_it_goes_from() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup_with(&[to_alias("alias@x")]);
    s.engine
        .add_address("work", "hello@northwind.co", None)
        .unwrap();
    let kept = Arc::new(Kept::default());
    let tools = tools_with(&s, kept.clone());
    let from = |from: &str| tools::SendArgs {
        account: Some("work".into()),
        to: Some(vec!["ana@northwind.co".into()]),
        bcc: Some(vec!["boss@northwind.co".into()]),
        from: Some(from.into()),
        ..Default::default()
    };
    tools
        .send(Parameters(from("Hello@Northwind.co")))
        .await
        .unwrap();
    tools
        .send(Parameters(from("maya@northwind.co")))
        .await
        .unwrap();
    assert!(tools
        .send(Parameters(from("stranger@else.co")))
        .await
        .unwrap_err()
        .contains("stranger@else.co isn't one of the user's addresses"));
    // A reply goes from the address the email came to, as in the window.
    let alias = subject_id(&s, "work", "Hello alias");
    tools
        .send(Parameters(tools::SendArgs {
            reply_to_thread: Some(alias),
            ..Default::default()
        }))
        .await
        .unwrap();
    let sent = kept.sent.lock().unwrap();
    assert_eq!(sent[0].from_address.as_deref(), Some("hello@northwind.co"));
    assert_eq!(tools::whom(&sent[0].bcc), ["boss@northwind.co"]);
    assert_eq!(sent[1].from_address, None, "its own address");
    assert_eq!(sent[2].from_address.as_deref(), Some("hello@northwind.co"));
}

#[tokio::test]
async fn a_new_email_goes_from_where_you_write_them_from() {
    use rmcp::handler::server::wrapper::Parameters;
    let lee = "From: Maya <maya@northwind.co>\r\nTo: Lee <lee@client.co>\r\nSubject: Hi Lee\r\nMessage-ID: <lee@x>\r\nDate: Sun, 01 Nov 2026 09:00:00 +0000\r\n\r\nHello\r\n";
    let s = setup_with(&[lee.as_bytes().to_vec()]);
    let tools = tools_with(&s, Arc::new(Kept::default()));
    let to = |to: &str, account: Option<&str>| tools::CreateArgs {
        to: Some(vec![to.into()]),
        account: account.map(String::from),
        body: Some("Hi".into()),
        ..Default::default()
    };
    let draft = |text: String| s.engine.drafts().unwrap().into_iter().find(|d| text.contains(&d.id)).unwrap();
    // Sam has only written to home: it goes from home.
    let sam = draft(tools.create_draft(Parameters(to("sam@home.example", None))).await.unwrap());
    assert_eq!((sam.from_account_id.as_str(), sam.from_address), ("home", None));
    // You've written Lee from work's own address.
    let lee = draft(tools.create_draft(Parameters(to("Lee <lee@client.co>", None))).await.unwrap());
    assert_eq!((lee.from_account_id.as_str(), lee.from_address), ("work", None));
    // Someone new: the first account. An account asked for wins.
    let new = draft(tools.create_draft(Parameters(to("new@else.co", None))).await.unwrap());
    assert_eq!(new.from_account_id, "work");
    let asked = draft(tools.create_draft(Parameters(to("sam@home.example", Some("work")))).await.unwrap());
    assert_eq!(asked.from_account_id, "work");
}

#[tokio::test]
async fn an_email_can_go_to_a_name_moss_looks_up() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let tools = tools_with(&s, Arc::new(Kept::default()));
    let to = |to: &str| tools::CreateArgs { to: Some(vec![to.into()]), body: Some("Hi".into()), ..Default::default() };
    // Sam, from the user's mail: his address, and the account he writes to.
    let text = tools.create_draft(Parameters(to(" sam "))).await.unwrap();
    let sam = s.engine.drafts().unwrap().into_iter().find(|d| text.contains(&d.id)).unwrap();
    assert_eq!(tools::whom(&sam.to), ["Sam <sam@home.example>"]);
    assert_eq!(sam.from_account_id, "home");
    let err = tools.create_draft(Parameters(to("Simon Lark"))).await.unwrap_err();
    assert_eq!(err, "There's no one called Simon Lark in the user's mail. Use the address the user gave, or ask them for it.");
}

#[tokio::test]
async fn an_address_of_yours_picks_its_account_and_one_not_yours_is_said_plainly() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let tools = tools_with(&s, Arc::new(Kept::default()));
    let from = |from: &str| tools::CreateArgs {
        to: Some(vec!["ana@northwind.co".into()]),
        from: Some(from.into()),
        body: Some("Hi".into()),
        ..Default::default()
    };
    let text = tools.create_draft(Parameters(from("Maya@Home.example"))).await.unwrap();
    let home = s.engine.drafts().unwrap().into_iter().find(|d| text.contains(&d.id)).unwrap();
    assert_eq!((home.from_account_id.as_str(), home.from_address), ("home", None));
    let err = tools.create_draft(Parameters(from("ana@northwind.co"))).await.unwrap_err();
    assert!(err.contains("ana@northwind.co isn't one of the user's addresses") && err.contains("leave from out"), "{err}");
}

#[tokio::test]
async fn a_new_email_from_the_chat_needs_a_subject() {
    let s = setup();
    let (t, _) = chat(&s);
    for subject in [json!(null), json!(" ")] {
        let said = t.chat_run("write_email", json!({"to": ["sam@home.example"], "subject": subject, "body": "Long time!"})).await.unwrap().unwrap_err();
        assert_eq!(said, "A new email needs a subject. Call write_email again with one.");
    }
    assert!(s.engine.drafts().unwrap().is_empty(), "nothing saved");
    // A reply has its thread's.
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    chat_ok(&t, "write_email", json!({"reply_to_thread": lunch, "body": "Yes"})).await;
}

#[tokio::test]
async fn the_chat_leaves_out_a_from_that_isnt_yours() {
    let s = setup();
    let (t, _) = chat(&s);
    // A small model puts the person it writes to in from as well.
    let ran = chat_ok(&t, "write_email", json!({"to": ["Sam <sam@home.example>"], "from": "sam@home.example", "subject": "Hi", "body": "Long time!"})).await;
    let draft = ran.draft.unwrap();
    assert_eq!((draft.from_account_id.as_str(), draft.from_address), ("home", None));
}

#[tokio::test]
async fn supplied_file_bytes_go_with_a_draft() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let notes = s.dir.path().join("notes.txt");
    std::fs::write(&notes, "hi").unwrap();
    let tools = tools_with(&s, Arc::new(Kept::default()));
    let made = tools
        .create_draft(Parameters(tools::CreateArgs {
            to: Some(vec!["ana@northwind.co".into()]),
            attachments: Some(vec![tools::SuppliedAttachment { name: "notes.txt".into(), content_base64: "aGk=".into() }]),
            ..Default::default()
        }))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&made).unwrap();
    assert_eq!(v["files"], json!(["notes.txt"]));
    assert_eq!(
        s.engine.drafts().unwrap()[0].attachments[0].name,
        "notes.txt"
    );
    let id = v["draftId"].as_str().unwrap().to_string();
    let more = tools
        .update_draft(Parameters(tools::UpdateArgs {
            draft_id: id,
            revision: v["revision"].as_str().unwrap().to_owned(),
            attachments: Some(vec![tools::SuppliedAttachment { name: "notes.txt".into(), content_base64: "aGk=".into() }]),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&more).unwrap()["files"],
        json!(["notes.txt", "notes.txt"])
    );
    assert!(tools
        .create_draft(Parameters(tools::CreateArgs {
            files: Some(vec!["/nowhere/x.txt".into()]),
            ..Default::default()
        }))
        .await
        .unwrap_err()
        .contains("Local paths"));
}

#[tokio::test]
async fn a_draft_can_be_deleted() {
    let s = setup();
    let (_, c) = on(&s).await;
    let made = ok(&c, "create_draft", json!({"subject": "Scratch"})).await;
    let id = made["draftId"].as_str().unwrap();
    assert_eq!(
        ok(&c, "delete_draft", json!({"draft_id": id, "revision": made["revision"]})).await["done"],
        "draft deleted"
    );
    assert!(s.engine.drafts().unwrap().is_empty());
    assert!(fails(&c, "delete_draft", json!({"draft_id": id, "revision": made["revision"]}))
        .await
        .contains("no draft with id"));
}

#[tokio::test]
async fn done_blocked_and_scheduled_have_their_own_lists() {
    let s = setup();
    let (_, c) = on(&s).await;
    let lunch = thread_id(&c, "Lunch on Friday").await;
    ok(&c, "mark_done", json!({"thread_id": lunch})).await;
    let done = ok(&c, "list_done", json!({})).await;
    assert_eq!(done["threads"][0]["subject"], "Lunch on Friday");
    let home = ok(&c, "list_done", json!({"account": "home", "limit": 5})).await;
    assert_eq!(home["threads"], json!([]));
    let budget = subject_id(&s, "work", "Q4 budget");
    ok(&c, "block_sender", json!({"thread_id": budget})).await;
    let blocked = ok(&c, "list_blocked", json!({})).await;
    assert_eq!(blocked["threads"][0]["id"], budget);
    ok(
        &c,
        "send",
        json!({"to": ["ana@northwind.co"], "subject": "Later", "send_at": "2030-01-01T09:00:00Z"}),
    )
    .await;
    let later = ok(&c, "list_scheduled", json!({})).await;
    let first = &later["scheduled"][0];
    assert_eq!(first["subject"], "Later");
    assert_eq!(first["sendAt"], "2030-01-01T09:00:00Z");
    assert_eq!(first["to"], json!(["ana@northwind.co"]));
    let id = first["scheduledId"].as_str().unwrap();
    assert_eq!(
        ok(&c, "cancel_scheduled", json!({"scheduled_id": id, "operation_id": first["operationId"]})).await["done"],
        "back in Drafts, not sent"
    );
    assert_eq!(
        ok(&c, "list_scheduled", json!({})).await["scheduled"],
        json!([])
    );
    assert_eq!(s.engine.drafts().unwrap()[0].subject, "Later");
    assert!(!fails(&c, "cancel_scheduled", json!({"scheduled_id": id}))
        .await
        .is_empty());
}

#[tokio::test]
async fn a_file_is_saved_on_this_mac_for_the_agent_to_open() {
    let s = setup();
    let (_, c) = on(&s).await;
    let contract = thread_id(&c, "Signed contract").await;
    let file = ok(
        &c,
        "get_attachment",
        json!({"thread_id": contract, "name": "Contract.pdf"}),
    )
    .await;
    assert_eq!(file["name"], "Contract.pdf");
    assert_eq!(file["contentType"], "application/pdf");
    let path = file["path"].as_str().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"PDFDATA");
    assert!(fails(
        &c,
        "get_attachment",
        json!({"thread_id": contract, "name": "Other.pdf"})
    )
    .await
    .contains("no file called Other.pdf"));
    assert!(fails(
        &c,
        "get_attachment",
        json!({"thread_id": "nope", "name": "x"})
    )
    .await
    .contains("no thread with id nope"));
}

#[tokio::test]
async fn contacts_by_name_or_address() {
    let s = setup();
    let (_, c) = on(&s).await;
    let found = ok(&c, "find_contacts", json!({"query": "DAN"})).await;
    assert_eq!(
        found["contacts"],
        json!(["Daniel Okafor <daniel@northwind.co>"])
    );
    let by_address = ok(&c, "find_contacts", json!({"query": "priya@", "limit": 3})).await;
    assert_eq!(by_address["contacts"].as_array().unwrap().len(), 1);
    assert_eq!(
        ok(&c, "find_contacts", json!({"query": "zzz"})).await["contacts"],
        json!([])
    );
    assert!(fails(&c, "find_contacts", json!({"query": " "}))
        .await
        .contains("Say who"));
}

#[tokio::test]
async fn sync_now_checks_for_new_mail() {
    let s = setup();
    let (_, c) = on(&s).await;
    assert_eq!(
        ok(&c, "sync_now", json!({})).await["done"],
        "checking for new mail"
    );
    assert_eq!(
        ok(&c, "sync_now", json!({"account": "work"})).await["done"],
        "checking for new mail"
    );
    assert!(fails(&c, "sync_now", json!({"account": "nope"}))
        .await
        .contains("no account with id nope"));
}

// Moss's own chat: the same tools, without the server.

fn chat(s: &Setup) -> (MossTools, Arc<Kept>) {
    let kept = Arc::new(Kept::default());
    (tools_with(s, kept.clone()), kept)
}

async fn chat_ok(t: &MossTools, name: &str, args: Value) -> tools::Ran {
    match t.chat_run(name, args).await {
        Some(Ok(ran)) => ran,
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn results_leave_out_what_says_nothing() {
    let v = json!({"id": "1", "from": null, "unread": false, "hasFiles": true, "body": "", "count": 0, "threads": [], "nested": {"x": null, "cc": [], "y": [{"z": false, "w": "a"}]}});
    assert_eq!(
        serde_json::from_str::<Value>(&tools::compact(&v)).unwrap(),
        json!({"id": "1", "hasFiles": true, "count": 0, "threads": [], "nested": {"y": [{"w": "a"}]}}),
        "empty, false and null fields are noise for a small model; an empty list at the top says none were found"
    );
    assert_eq!(tools::compact(&json!(null)), "null");
    assert_eq!(tools::compact(&json!("hi")), "\"hi\"");
}

#[tokio::test]
async fn moss_own_chat_gets_every_tool_but_the_ones_it_has_its_own_of() {
    let s = setup();
    let (t, _) = chat(&s);
    let specs = t.chat_specs();
    // About 4,000 tokens: room beside a long thread in the writing model's context.
    assert!(Value::Array(specs.clone()).to_string().len() < 15_000);
    let names: Vec<&str> = specs
        .iter()
        .map(|f| f["function"]["name"].as_str().unwrap())
        .collect();
    // One tool writes every email: send is write_email here, and create_draft is left out.
    assert_eq!(names.len(), 26 - tools::CHAT_LEAVES_OUT.len());
    assert!(!names.contains(&"send"));
    let write = specs
        .iter()
        .find(|f| f["function"]["name"] == "write_email")
        .unwrap();
    assert!(write["function"]["description"]
        .as_str()
        .unwrap()
        .contains("never sent by itself"));
    assert!(write["function"]["parameters"]["properties"]["send_at"].is_object());
    // The two reminders, told apart for a small model.
    let described = |name: &str| {
        specs
            .iter()
            .find(|f| f["function"]["name"] == name)
            .unwrap()["function"]["description"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(described("remind_me")
        .contains("To stop a reminder and bring the email back now, call it without until."));
    assert!(
        described("remind_if_no_reply").contains("Not for other reminders: those are remind_me.")
    );
    // A small model leaves out what isn't required: the text always is.
    assert_eq!(write["function"]["parameters"]["required"], json!(["body"]));
    for left_out in tools::CHAT_LEAVES_OUT {
        assert!(!names.contains(left_out), "{left_out}");
    }
    for name in [
        "write_email",
        "mark_done",
        "list_inbox",
        "block_sender",
        "update_draft",
        "sync_now",
    ] {
        assert!(names.contains(&name), "{name}");
        assert!(
            MossTools::chat_label(name, &json!({})).is_some(),
            "{name} has a label"
        );
    }
    let done = specs
        .iter()
        .find(|f| f["function"]["name"] == "mark_done")
        .unwrap();
    assert_eq!(done["type"], "function");
    assert!(done["function"]["description"]
        .as_str()
        .unwrap()
        .starts_with("Done:"));
    let parameters = &done["function"]["parameters"];
    assert_eq!(parameters["type"], "object");
    assert!(parameters["properties"]["thread_id"].is_object());
    assert!(
        parameters.get("$schema").is_none() && parameters.get("title").is_none(),
        "no words the model doesn't need"
    );
}

#[test]
fn the_chat_says_what_each_tool_is_doing() {
    let label = |name: &str, args: Value| MossTools::chat_label(name, &args).unwrap();
    assert_eq!(
        label("mark_done", json!({"thread_id": "1"})),
        "Marking an email Done"
    );
    assert_eq!(
        label("mark_done", json!({"done": false})),
        "Moving an email back to the inbox"
    );
    assert_eq!(
        label("mark_read", json!({"read": false})),
        "Marking an email unread"
    );
    assert_eq!(label("mark_read", json!({})), "Marking an email read");
    assert_eq!(
        label("remind_me", json!({"until": "2026-11-02T09:00:00Z"})),
        "Setting a reminder"
    );
    assert_eq!(
        label("remind_me", json!({"until": null})),
        "Stopping a reminder"
    );
    assert_eq!(
        label("remind_if_no_reply", json!({})),
        "Reminding you if no one replies"
    );
    assert_eq!(
        label("remind_if_no_reply", json!({"on": false})),
        "Stopping waiting for a reply"
    );
    assert_eq!(label("write_email", json!({})), "Writing an email");
    assert_eq!(
        MossTools::chat_label("send", &json!({})),
        None,
        "write_email does it"
    );
    assert_eq!(MossTools::chat_label("create_draft", &json!({})), None);
    assert_eq!(
        MossTools::chat_label("get_thread", &json!({})),
        None,
        "the chat reads threads its own way"
    );
    assert_eq!(MossTools::chat_label("nope", &json!({})), None);
}

#[tokio::test]
async fn the_chat_acts_on_mail_and_says_what_it_did() {
    let s = setup();
    let (t, _) = chat(&s);
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    let ran = chat_ok(&t, "mark_done", json!({"thread_id": lunch})).await;
    assert_eq!(ran.did.as_deref(), Some("Marked done: Lunch on Friday"));
    assert!(ran.text.contains(&lunch));
    assert!(s
        .engine
        .done_threads(None, 10)
        .unwrap()
        .iter()
        .any(|d| d.id == lunch));
    assert!(told_mail_changed(&s, "work"), "the window shows it");
    let back = chat_ok(&t, "mark_done", json!({"thread_id": lunch, "done": false})).await;
    assert_eq!(
        back.did.as_deref(),
        Some("Back in the inbox: Lunch on Friday")
    );
    let synced = chat_ok(&t, "sync_now", Value::Null).await;
    assert_eq!(
        synced.did.as_deref(),
        Some("Checking for new mail"),
        "no thread, so just what it did"
    );
    let listed = chat_ok(&t, "list_accounts", json!({})).await;
    assert_eq!(listed.did, None, "looking changes nothing");
}

#[tokio::test]
async fn every_chat_tool_runs() {
    let s = setup();
    let (t, _) = chat(&s);
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    let budget = subject_id(&s, "work", "Q4 budget");
    let on = |extra: Value| {
        let mut v = json!({"thread_id": lunch});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    for (name, args) in [
        ("list_drafts", json!({})),
        ("list_done", json!({})),
        ("list_blocked", json!({})),
        ("list_scheduled", json!({})),
        ("mark_read", on(json!({"read": false}))),
        ("remind_me", on(json!({"until": "2026-11-09T09:00:00Z"}))),
        ("remind_me", on(json!({}))),
        ("move_thread", on(json!({"to": "other"}))),
        ("delete_thread", on(json!({}))),
        ("restore_thread", on(json!({}))),
        ("accept_sender", on(json!({}))),
        ("block_sender", on(json!({}))),
        ("unblock_sender", on(json!({}))),
    ] {
        let ran = chat_ok(&t, name, args).await;
        assert!(!ran.text.is_empty(), "{name}");
    }
    // What the chat lists says it plainly, not as an agent reads it.
    let said = |ran: tools::Ran| ran.did.unwrap();
    assert_eq!(
        said(
            chat_ok(
                &t,
                "remind_me",
                on(json!({"until": "2026-11-09T09:00:00Z"}))
            )
            .await
        ),
        "Reminder set: Lunch on Friday"
    );
    assert_eq!(
        said(chat_ok(&t, "remind_me", on(json!({}))).await),
        "Reminder stopped: Lunch on Friday"
    );
    assert_eq!(
        said(chat_ok(&t, "move_thread", on(json!({"to": "newsletters"}))).await),
        "Moved to Newsletters: Lunch on Friday"
    );
    assert_eq!(
        said(chat_ok(&t, "move_thread", on(json!({"to": "auto"}))).await),
        "Back to Moss’s sorting: Lunch on Friday"
    );
    // The user's own email is the latest in no thread here, so there's no reply to wait for.
    let refused = t
        .chat_run("remind_if_no_reply", json!({"thread_id": budget}))
        .await
        .unwrap();
    assert!(refused.unwrap_err().contains("isn't the user's"));
    let stopped = chat_ok(
        &t,
        "remind_if_no_reply",
        json!({"thread_id": budget, "on": false}),
    )
    .await;
    assert_eq!(stopped.did.as_deref(), Some("Not waiting: Q4 budget"));
}

#[tokio::test]
async fn the_chat_lists_threads_it_can_cite() {
    let s = setup();
    let (t, _) = chat(&s);
    let ran = chat_ok(&t, "list_inbox", json!({"account": "work"})).await;
    assert!(ran
        .threads
        .contains(&subject_id(&s, "work", "Lunch on Friday")));
    assert_eq!(ran.threads.len(), 3);
}

#[tokio::test]
async fn the_chat_shows_the_drafts_it_writes() {
    let s = setup();
    let (t, _) = chat(&s);
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    let made = chat_ok(
        &t,
        "write_email",
        json!({"reply_to_thread": lunch, "body": "Friday works."}),
    )
    .await;
    let draft = made.draft.unwrap();
    assert_eq!(draft.thread_id.as_deref(), Some(lunch.as_str()));
    assert_eq!(made.did, None, "the draft shows as itself");
    let changed = chat_ok(
        &t,
        "update_draft",
        json!({"draft_id": draft.id, "revision": draft.revision, "body": "Friday at 1 works."}),
    )
    .await;
    let changed = changed.draft.unwrap();
    assert_eq!(changed.body, "Friday at 1 works.");
    let gone = chat_ok(&t, "delete_draft", json!({"draft_id": draft.id, "revision": changed.revision})).await;
    assert_eq!(gone.draft, None);
    assert_eq!(
        gone.did.as_deref(),
        Some("Draft deleted: Re: Lunch on Friday")
    );
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn the_chat_hears_what_kind_of_email_it_wrote() {
    let s = setup();
    let (t, _) = chat(&s);
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    // Said plainly: a model that meant a reply sees when it wasn't one.
    let reply = chat_ok(&t, "write_email", json!({"reply_to_thread": lunch, "body": "See you Friday."})).await;
    assert!(reply.text.contains("A reply in its thread. Not sent yet"), "{}", reply.text);
    // The card shows it: the answer that repeats it says it twice.
    assert!(reply.text.contains("Don't write it out in your answer: say in one sentence what you wrote"), "{}", reply.text);
    // Nor are its words given back to copy: list_drafts has them when it's to be changed.
    assert!(!reply.text.contains("See you Friday") && reply.text.contains("Re: Lunch on Friday"), "{}", reply.text);
    let new = chat_ok(&t, "write_email", json!({"to": ["ana@northwind.co"], "subject": "Hi", "body": "Hello."})).await;
    assert!(new.text.contains("A new email, not a reply in any thread."), "{}", new.text);
    let forward = chat_ok(&t, "write_email", json!({"forward_thread": lunch, "to": ["ana@northwind.co"]})).await;
    assert!(forward.text.contains("A forward of “Lunch on Friday”."), "{}", forward.text);
}

#[tokio::test]
async fn the_chat_never_sends_the_email_waits_for_the_user_to_press_send() {
    let s = setup();
    let (t, kept) = chat(&s);
    let lunch = subject_id(&s, "work", "Lunch on Friday");
    let ran = chat_ok(
        &t,
        "write_email",
        json!({"reply_to_thread": lunch, "body": "See you Friday."}),
    )
    .await;
    assert!(
        kept.sent.lock().unwrap().is_empty() && kept.later.lock().unwrap().is_empty(),
        "nothing went"
    );
    let draft = ran.draft.unwrap();
    assert_eq!(
        s.engine.drafts().unwrap(),
        [draft.clone()],
        "it waits in Drafts"
    );
    assert_eq!(ran.send_at, None);
    assert!(ran.text.contains("press Send"));

    let later = chat_ok(
        &t,
        "write_email",
        json!({"draft_id": draft.id, "revision": draft.revision, "send_at": "2026-11-02T09:00:00Z"}),
    )
    .await;
    let at = chrono::DateTime::parse_from_rfc3339("2026-11-02T09:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(later.send_at, Some(at), "Send later, when they asked");
    assert_eq!(later.draft.unwrap().id, draft.id);
    assert!(kept.later.lock().unwrap().is_empty());

    let wrong = t
        .chat_run(
            "write_email",
            json!({"to": ["ana@northwind.co"], "send_at": "soon"}),
        )
        .await
        .unwrap();
    assert!(wrong.is_err());

    // Nothing to send: no text and no draft. Nothing is saved, and the model hears how to fix it.
    let empty = t
        .chat_run("write_email", json!({"reply_to_thread": lunch}))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        empty,
        "Nothing was written. Write it yourself: a short, sensible email from what the user asked (read the thread first if it's a reply), and call write_email again with it in body. Don't ask the user for the words."
    );
    assert_eq!(s.engine.drafts().unwrap().len(), 1);
    // A forward can go with no words of its own.
    let forward = chat_ok(
        &t,
        "write_email",
        json!({"forward_thread": lunch, "to": ["sam@acme.co"]}),
    )
    .await;
    assert_eq!(forward.draft.unwrap().mode, ComposeMode::Forward);
}

#[tokio::test]
async fn a_send_later_cancelled_from_the_chat_shows_its_draft() {
    let s = setup();
    // Set to send later by an agent, through the engine's own outbox.
    let t = MossTools::new(
        s.engine.clone(),
        s.heard.clone(),
        now,
        s.dir.path().join("attachments"),
    );
    let args = tools::SendArgs {
        to: Some(vec!["ana@northwind.co".into()]),
        subject: Some("Notes".into()),
        send_at: Some("2030-01-01T09:00:00Z".into()),
        ..Default::default()
    };
    t.send(rmcp::handler::server::wrapper::Parameters(args))
        .await
        .unwrap();
    let scheduled = s.engine.scheduled().unwrap().remove(0);
    let id = scheduled.draft.id;
    let ran = chat_ok(&t, "cancel_scheduled", json!({"scheduled_id": id, "operation_id": scheduled.operation_id})).await;
    assert_eq!(ran.did.as_deref(), Some("Back in Drafts, not sent: Notes"));
    assert_eq!(ran.draft.unwrap().id, id);
}

#[tokio::test]
async fn the_chat_says_when_a_tool_is_not_its_own_or_its_arguments_are_wrong() {
    let s = setup();
    let (t, _) = chat(&s);
    assert!(t
        .chat_run("get_thread", json!({"thread_id": "1"}))
        .await
        .is_none());
    assert!(t.chat_run("nope", json!({})).await.is_none());
    let bad = t
        .chat_run("mark_done", json!({}))
        .await
        .unwrap()
        .unwrap_err();
    // No email named: the model hears how to find one, not to ask the user.
    assert_eq!(bad, "Say which email in thread_id: find it with search_mail first, then use the id it shows as [t:ID]. Don't ask the user for it.");
    let unclear = t
        .chat_run("move_thread", json!({"thread_id": "1"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        unclear.starts_with("Couldn't read the arguments"),
        "{unclear}"
    );
    let unknown = t
        .chat_run("mark_done", json!({"thread_id": "999"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(unknown.contains("999"), "{unknown}");
    let not_a_draft = t
        .chat_run("write_email", json!({"draft_id": "nope", "revision": "missing"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(!not_a_draft.is_empty());
}

#[tokio::test]
async fn untrusted_tools_cannot_attach_a_local_path_even_to_an_unsent_draft() {
    use rmcp::handler::server::wrapper::Parameters;
    for action in ["create_draft", "update_draft", "send", "write_email"] {
        let s = setup();
        let path = s.dir.path().join("private.txt");
        std::fs::write(&path, "private fixture bytes").unwrap();
        let (tools, sent) = chat(&s);
        let made = tools.create_draft(Parameters(tools::CreateArgs {
            to: Some(vec!["ana@northwind.co".into()]), subject: Some("Safe draft".into()),
            body: Some("Safe words".into()), ..Default::default()
        })).await.unwrap();
        let id = serde_json::from_str::<Value>(&made).unwrap()["draftId"].as_str().unwrap().to_string();
        let before = s.engine.drafts().unwrap();
        let args = json!({"draft_id": id, "revision": before[0].revision, "to": ["ana@northwind.co"], "subject": "Test", "body": "Words", "files": [path]});
        let result = match action {
            "create_draft" => tools.create_draft(Parameters(serde_json::from_value(args).unwrap())).await,
            "update_draft" => tools.update_draft(Parameters(serde_json::from_value(args).unwrap())).await,
            "send" => tools.send(Parameters(serde_json::from_value(args).unwrap())).await,
            _ => tools.chat_run(action, args).await.unwrap().map(|r| r.text),
        };
        assert!(result.is_err(), "{action} must not read a model-supplied path");
        assert_eq!(s.engine.drafts().unwrap(), before, "no draft changed or queued for upload");
        assert!(sent.sent.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn agent_supplied_contents_attach_without_moss_reading_the_filesystem() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let (tools, _) = chat(&s);
    let args = json!({"to": ["ana@northwind.co"], "subject": "Notes", "body": "Attached", "attachments": [{"name": "notes.txt", "content_base64": "aGk="}]});
    let made = tools.create_draft(Parameters(serde_json::from_value(args).unwrap())).await.unwrap();
    let v: Value = serde_json::from_str(&made).unwrap();
    assert_eq!(v["files"], json!(["notes.txt"]));
    let draft = &s.engine.drafts().unwrap()[0];
    assert_eq!(draft.attachments[0].bytes, 2);
}

#[tokio::test]
async fn local_chat_rejects_supplied_file_contents_instead_of_uploading_them() {
    let s = setup();
    let (tools, _) = chat(&s);
    for name in ["write_email", "update_draft"] {
        let result = tools.chat_run(name, json!({"to": ["ana@northwind.co"], "subject": "Notes", "body": "Attached", "attachments": [{"name": "notes.txt", "content_base64": "aGk="}]})).await.unwrap();
        assert!(result.is_err(), "chat must use user-staged or mailbox files, not generate file contents");
        assert!(s.engine.drafts().unwrap().is_empty());
    }
}

#[tokio::test]
async fn supplied_attachments_validate_all_payloads_before_storing_any() {
    use rmcp::handler::server::wrapper::Parameters;
    let s = setup();
    let (t, _) = chat(&s);
    let cases = [
        json!([{"name":"notes.txt","content_base64":"not base64!"}]),
        json!([{"name":"../private.txt","content_base64":"aGk="}]),
        json!([{"name":"notes.txt","content_base64":"A".repeat((tools::MAX_SUPPLIED_BYTES.div_ceil(3) * 4) + 4)}]),
        json!((0..101).map(|_| json!({"name":"empty.txt","content_base64":""})).collect::<Vec<_>>()),
    ];
    for attachments in cases {
        let result = t.create_draft(Parameters(serde_json::from_value(json!({"subject":"Test","attachments":attachments})).unwrap())).await;
        assert!(result.is_err());
        assert!(s.engine.drafts().unwrap().is_empty());
    }
    // Padding can let a decoded payload exceed the remaining byte allowance by up to two bytes.
    use base64::Engine as _;
    let too_many = base64::engine::general_purpose::STANDARD.encode(vec![0; tools::MAX_SUPPLIED_BYTES + 1]);
    assert!(t.create_draft(Parameters(serde_json::from_value(json!({"attachments":[{"name":"large.bin","content_base64":too_many}]})).unwrap())).await.is_err());
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[test]
fn chat_attachment_authority_is_not_advertised_in_the_schema() {
    let s = setup();
    let (t, _) = chat(&s);
    for spec in t.chat_specs() {
        assert!(spec["function"]["parameters"]["properties"].get("files").is_none());
        assert!(spec["function"]["parameters"]["properties"].get("attachments").is_none());
    }
}

#[tokio::test]
async fn scheduled_agent_actions_require_and_preserve_operation_identity() {
    let s = setup();
    let (_, c) = on(&s).await;
    ok(&c, "send", json!({"to": ["ana@northwind.co"], "subject": "Identity", "send_at": "2030-01-01T09:00:00Z"})).await;
    let original = s.engine.scheduled().unwrap().remove(0);
    let listed = ok(&c, "list_scheduled", json!({})).await;
    assert_eq!(listed["scheduled"][0]["operationId"], original.operation_id);
    assert_eq!(listed["scheduled"][0]["revision"], 1);
    assert!(!fails(&c, "cancel_scheduled", json!({"scheduled_id": original.draft.id})).await.is_empty());
    let args = json!({"scheduled_id": original.draft.id, "operation_id": original.operation_id});
    let cancelled = ok(&c, "cancel_scheduled", args.clone()).await;
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["draftId"], original.draft.id);
    assert_eq!(cancelled["revision"], s.engine.editable_draft(&original.draft.id).unwrap().unwrap().revision.unwrap());
    let outgoing = OutgoingDraft::from(&original.draft);
    s.engine.schedule_with_operation("replacement", &s.engine.editable_draft(&original.draft.id).unwrap().unwrap(), &outgoing, original.send_at).unwrap();
    let replay = ok(&c, "cancel_scheduled", args).await;
    assert_eq!(replay["status"], "cancelled");
    assert_eq!(replay["alreadyCancelled"], true);
    assert_eq!(replay["note"], "This operation was already cancelled. Any other schedule for this email is unchanged.");
    assert!(replay.get("draftId").is_none());
    let remaining = s.engine.scheduled().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].operation_id, "replacement");
}

#[tokio::test]
async fn agent_cancellation_reports_delivery_state_without_restoring_a_draft() {
    let s = setup();
    let (_, c) = on(&s).await;
    ok(&c, "send", json!({"to": ["ana@northwind.co"], "subject": "Delivery", "send_at": "2030-01-01T09:00:00Z"})).await;
    let scheduled = s.engine.scheduled().unwrap().remove(0);
    let args = json!({"scheduled_id": scheduled.draft.id, "operation_id": scheduled.operation_id});
    let mut store = Store::open(s.dir.path().join("moss.db")).unwrap();
    store.prepare_delivery(&scheduled.operation_id, &scheduled.draft, "{}", "scheduled@example.test", now()).unwrap();
    store.claim_delivery(&scheduled.operation_id).unwrap();
    let claimed = ok(&c, "cancel_scheduled", args.clone()).await;
    assert_eq!(claimed, json!({"status": "inDelivery", "state": "claimed"}));
    assert!(s.engine.drafts().unwrap().is_empty());
    store.uncertain_delivery(&scheduled.operation_id, "Reply lost").unwrap();
    assert_eq!(ok(&c, "cancel_scheduled", args.clone()).await, json!({"status": "inDelivery", "state": "uncertain"}));
    assert!(s.engine.drafts().unwrap().is_empty());
    // Use a different operation to exercise accepted and completed delivery honestly.
    ok(&c, "send", json!({"to": ["ana@northwind.co"], "subject": "Completed", "send_at": "2030-01-01T09:00:00Z"})).await;
    let completed = s.engine.scheduled().unwrap().into_iter().find(|e| e.draft.subject == "Completed").unwrap();
    let args = json!({"scheduled_id": completed.draft.id, "operation_id": completed.operation_id});
    store.prepare_delivery(&completed.operation_id, &completed.draft, "{}", "completed@example.test", now()).unwrap();
    assert!(store.claim_delivery(&completed.operation_id).unwrap());
    assert!(store.accept_delivery(&completed.operation_id, b"accepted", "{}").unwrap());
    assert_eq!(ok(&c, "cancel_scheduled", args.clone()).await, json!({"status": "inDelivery", "state": "accepted"}));
    assert!(store.complete_delivery(&completed.operation_id).unwrap());
    assert_eq!(ok(&c, "cancel_scheduled", args.clone()).await, json!({"status": "completed"}));
    assert!(store.email_sent(&completed.draft.id, now()).unwrap());
    assert_eq!(ok(&c, "cancel_scheduled", args).await, json!({"status": "completed"}));
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn agent_cancellation_refuses_unknown_operations_without_recording_them() {
    let s = setup();
    let (_, c) = on(&s).await;
    ok(&c, "send", json!({"to": ["ana@northwind.co"], "subject": "Keep scheduled", "send_at": "2030-01-01T09:00:00Z"})).await;
    let scheduled = s.engine.scheduled().unwrap().remove(0);
    let error = fails(&c, "cancel_scheduled", json!({"scheduled_id": scheduled.draft.id, "operation_id": "mistyped-operation"})).await;
    assert!(error.contains("list_scheduled"), "{error}");
    assert_eq!(s.engine.scheduled().unwrap(), vec![scheduled]);
    let db = rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
    let count: i64 = db.query_row("SELECT (SELECT count(*) FROM schedule_outcomes WHERE id = 'mistyped-operation') + (SELECT count(*) FROM delivery_cancellations WHERE id = 'mistyped-operation')", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn agent_schedule_cancellation_refuses_ordinary_send_identity() {
    let s = setup();
    let (_, c) = on(&s).await;
    ok(&c, "create_draft", json!({"to": ["ana@northwind.co"], "subject": "Ordinary send"})).await;
    let draft = s.engine.drafts().unwrap().remove(0);
    let mut store = Store::open(s.dir.path().join("moss.db")).unwrap();
    store.prepare_delivery("ordinary", &draft, "{}", "ordinary@example.test", now()).unwrap();
    assert!(store.claim_delivery("ordinary").unwrap());
    let error = fails(&c, "cancel_scheduled", json!({"scheduled_id": draft.id, "operation_id": "ordinary"})).await;
    assert!(error.contains("list_scheduled"), "{error}");
    assert_eq!(store.delivery_state("ordinary").unwrap(), Some(moss_store::DeliveryState::Claimed));
    let db = rusqlite::Connection::open(s.dir.path().join("moss.db")).unwrap();
    let records: i64 = db.query_row("SELECT (SELECT count(*) FROM schedule_outcomes WHERE id = 'ordinary') + (SELECT count(*) FROM delivery_cancellations WHERE id = 'ordinary')", [], |row| row.get(0)).unwrap();
    assert_eq!(records, 0);
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn agent_schedule_identity_cannot_be_used_for_another_draft_or_invented_in_chat() {
    let s = setup();
    let (_, c) = on(&s).await;
    for subject in ["First", "Second"] {
        ok(&c, "send", json!({"to": ["ana@northwind.co"], "subject": subject, "send_at": "2030-01-01T09:00:00Z"})).await;
    }
    let before = s.engine.scheduled().unwrap();
    let wrong = json!({"scheduled_id": before[1].draft.id, "operation_id": before[0].operation_id});
    assert!(fails(&c, "cancel_scheduled", wrong.clone()).await.contains("another draft"));
    let (t, _) = chat(&s);
    assert!(t.chat_run("cancel_scheduled", wrong).await.unwrap().unwrap_err().contains("another draft"));
    assert!(t.chat_run("cancel_scheduled", json!({"scheduled_id": before[0].draft.id, "operation_id": "invented"})).await.unwrap().unwrap_err().contains("list_scheduled"));
    assert_eq!(s.engine.scheduled().unwrap(), before);
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_tool_schema_requires_the_operation_identity() {
    let s = setup();
    let (_, c) = on(&s).await;
    let tool = c.list_all_tools().await.unwrap().into_iter().find(|t| t.name == "cancel_scheduled").unwrap();
    let schema = serde_json::to_value(tool).unwrap();
    let required = schema["inputSchema"]["required"].as_array().unwrap();
    assert!(required.contains(&json!("scheduled_id")));
    assert!(required.contains(&json!("operation_id")));
}

#[tokio::test]
async fn external_draft_edits_require_the_revision_returned_to_the_agent() {
    let s=setup();let (_,c)=on(&s).await;
    let first=ok(&c,"create_draft",json!({"to":["recipient@example.test"],"subject":"Versioned","body":"First"})).await;
    let id=first["draftId"].clone();let revision=first["revision"].clone();
    assert_eq!(revision.as_str().map(str::len),Some(32));
    let next=ok(&c,"update_draft",json!({"draft_id":id,"revision":revision,"body":"Winner"})).await;
    assert_ne!(next["revision"],revision);
    assert!(!fails(&c,"update_draft",json!({"draft_id":id,"revision":revision,"body":"Stale overwrite"})).await.is_empty());
    let missing=fails(&c,"send",json!({"draft_id":id})).await;
    assert!(missing.contains("revision returned by list_drafts"),"{missing}");
    assert_eq!(s.engine.drafts().unwrap()[0].body,"Winner");
    assert!(!fails(&c,"delete_draft",json!({"draft_id":id})).await.is_empty());
    assert!(!fails(&c,"send",json!({"draft_id":id,"revision":revision})).await.is_empty());
    assert_eq!(s.engine.drafts().unwrap()[0].body,"Winner");
    ok(&c,"delete_draft",json!({"draft_id":id,"revision":next["revision"]})).await;
    assert!(s.engine.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn a_definitive_mcp_send_failure_recognizes_the_engine_restored_revision() {
    use rmcp::handler::server::wrapper::Parameters;
    let s=setup();
    let kept=Arc::new(Kept { rejected:true,restore_before_rejection:Some(s.dir.path().join("moss.db")),..Default::default() });
    let tools=tools_with(&s,kept);
    tools.create_draft(Parameters(tools::CreateArgs { to:Some(vec!["recipient@example.test".into()]),subject:Some("Keep safely".into()),body:Some("Preserved text".into()),..Default::default() })).await.unwrap();
    let before=s.engine.drafts().unwrap().remove(0);
    let error=tools.send(Parameters(tools::SendArgs { draft_id:Some(before.id.clone()),revision:before.revision.clone(),..Default::default() })).await.unwrap_err();
    assert!(error.contains("kept as a draft"),"{error}");
    let restored=s.engine.drafts().unwrap().remove(0);
    assert_eq!(restored.body,"Preserved text");
    assert_ne!(restored.revision,before.revision);
}

#[tokio::test]
async fn successful_mcp_send_cleanup_cannot_delete_a_newer_edit() {
    use rmcp::handler::server::wrapper::Parameters;
    let s=setup();
    let kept=Arc::new(Kept { rewrite_after_send: Some(s.engine.clone()), ..Default::default() });
    let tools=tools_with(&s,kept);
    tools.create_draft(Parameters(tools::CreateArgs { to:Some(vec!["recipient@example.test".into()]),subject:Some("Original".into()),body:Some("Original text".into()),..Default::default() })).await.unwrap();
    let before=s.engine.drafts().unwrap().remove(0);
    let result=sent_json(tools.send(Parameters(tools::SendArgs { draft_id:Some(before.id.clone()),revision:before.revision.clone(),..Default::default() })).await);
    assert_eq!(result["sent"],true);
    let remaining=s.engine.drafts().unwrap();
    assert_eq!(remaining.len(),1);
    assert_eq!(remaining[0].body,"Newer independently kept text");
    assert_eq!(result["draftCleanupFailed"],true);
}
