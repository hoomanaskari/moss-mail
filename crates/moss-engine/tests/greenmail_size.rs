//! How much space a typical mailbox takes after a real sync, end to end against a local
//! GreenMail server: 1,000 sent emails and 20 in the Inbox. Opt-in, like `greenmail.rs`:
//!
//! ```bash
//! java -Dgreenmail.setup.test.all -Dgreenmail.hostname=127.0.0.1 \
//!   -Dgreenmail.users=maya:secret@moss.test,daniel:secret@moss.test,sam:secret@moss.test \
//!   -Dgreenmail.users.login=email \
//!   -Dgreenmail.tls.keystore.file=gm.p12 -Dgreenmail.tls.keystore.password=changeit \
//!   -jar greenmail-standalone-2.1.15.jar
//! python3 -I crates/moss-engine/tests/greenmail_size_seed.py ca.der
//! MOSS_E2E_CA=ca.der cargo test -p moss-engine --release --test greenmail_size -- --ignored --nocapture
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use moss_accounts::{Accounts, MailService, MemorySecretStore, PasswordSignIn, Security, Server, ServerSettings};
use moss_engine::{Engine, Event, Events, NetConnector, Parts};
use moss_providers::TlsRoots;

#[derive(Default)]
struct Recorded(Mutex<Vec<Event>>);

impl Events for Recorded {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn kb(bytes: i64, n: i64) -> String {
    format!("{:.1} KB an email", bytes as f64 / n as f64 / 1024.0)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local GreenMail server, seeded"]
async fn a_typical_mailbox_after_a_real_sync() {
    let ca = std::fs::read(std::env::var("MOSS_E2E_CA").expect("MOSS_E2E_CA")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moss.db");
    let secrets = Arc::new(MemorySecretStore::new());
    let events = Arc::new(Recorded::default());
    let engine = Engine::new(Parts {
        store: moss_store::Store::open(&path).unwrap(),
        secrets: secrets.clone(),
        accounts: Accounts::new(secrets),
        connector: Arc::new(NetConnector { roots: TlsRoots::Custom(vec![ca]) }),
        events: events.clone(),
        clock: now,
    });
    let servers = ServerSettings {
        imap: Server::new("127.0.0.1", 3993, Security::Tls, "sam@moss.test"),
        smtp: Server::new("127.0.0.1", 3465, Security::Tls, "sam@moss.test"),
    };
    let sign_in = PasswordSignIn {
        service: MailService::Imap,
        email: "sam@moss.test".into(),
        password: "secret".into(),
        servers: Some(servers),
        login: None,
    };
    let started = std::time::Instant::now();
    let id = engine.add_password_account(sign_in).await.unwrap().account.id;

    let db = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    for _ in 0..1200 {
        if count("SELECT count(*) FROM messages") >= 1020 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let n = count("SELECT count(*) FROM messages");
    assert_eq!(n, 1020, "every email synced");
    let synced_in = started.elapsed();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let errors: Vec<Event> = events.0.lock().unwrap().iter().filter(|e| matches!(e, Event::SyncError { .. })).cloned().collect();
    assert!(errors.is_empty(), "{errors:?}");

    let files = dir.path().join("files");
    let measure = |label: &str| {
        db.execute_batch("PRAGMA wal_checkpoint(PASSIVE)").ok();
        let info = engine.storage(&files).unwrap();
        let u = info.usage;
        let wal = std::fs::metadata(path.with_extension("db-wal")).map_or(0, |m| m.len());
        let file = std::fs::metadata(&path).unwrap().len() + wal;
        println!("{label}:");
        println!("  index & summaries {:>8.1} MB  {}", u.index as f64 / 1e6, kb(u.index as i64, n));
        println!("  offline email     {:>8.1} MB  {}", u.offline as f64 / 1e6, kb(u.offline as i64, n));
        println!("  cache             {:>8.1} KB", u.cache as f64 / 1e3);
        println!("  database files    {:>8.1} MB  {}", file as f64 / 1e6, kb(file as i64, n));
        u
    };

    let downloaded: String = db
        .query_row("SELECT value FROM settings WHERE key = ?1", [format!("sync.downloaded:{id}")], |r| r.get(0))
        .unwrap();
    let downloaded: i64 = downloaded.split_once(':').unwrap().1.parse().unwrap();
    // What tidying does at the next start: the write-ahead log folded in, free pages released.
    engine.tidy().await.unwrap();
    println!("{n} emails synced in {:.1} s, downloading {:.1} MB ({})", synced_in.as_secs_f64(), downloaded as f64 / 1e6, kb(downloaded, n));
    let after_sync = measure("after the first sync");
    // Per email, the index is a few KB and the compressed text a few more.
    assert!(after_sync.index < 4 * 1024 * n as u64, "index {}", kb(after_sync.index as i64, n));
    assert!(after_sync.offline < 8 * 1024 * n as u64, "offline {}", kb(after_sync.offline as i64, n));
    assert_eq!(after_sync.cache, 0, "nothing opened yet");
    assert_eq!(count("SELECT count(*) FROM message_raw"), 0);
    assert_eq!(count("SELECT count(*) FROM attachment_data"), 0, "no file bytes");

    // Opening the inbox: the images its emails show come, and stay as cache.
    for t in engine.threads(None).unwrap() {
        for m in engine.thread(&t.id).unwrap().messages {
            if m.images_missing {
                engine.load_message(&m.id).await.unwrap();
            }
        }
    }
    let opened = measure("after opening every inbox email");
    assert!(opened.cache > 0 && opened.cache < 1_000_000, "{}", opened.cache);
    // Clearing the cache keeps the rest.
    engine.clear_cache(&files).unwrap();
    assert_eq!(measure("after Clear cache").cache, 0);
}
