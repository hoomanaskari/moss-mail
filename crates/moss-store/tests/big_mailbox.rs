//! How fast the hot reads are on a big mailbox: 50,000 threads and 100,000 messages in a
//! database file, built with SQL so it takes seconds, not minutes.
//!
//! `cargo test -p moss-store --release --test big_mailbox -- --ignored --nocapture` prints the
//! timings.
//! `MOSS_BENCH_OPEN` is the percent of threads still in the inbox (default 20; 100 is someone
//! who never archives).

use std::time::{Duration, Instant};

use moss_core::{Account, AccountColor, ProviderKind, SplitId};
use moss_store::{Store, NEEDS_REPLY};
use rusqlite::Connection;

const THREADS: i64 = 50_000;
const PEOPLE: i64 = 5_000;
const NOW: i64 = 1_793_524_320_000;

/// A mailbox of `THREADS` threads of two messages each: one from someone, then your reply
/// (every other thread) or theirs. `open` percent of threads are in the inbox.
fn mailbox(path: &std::path::Path, open: i64) -> Store {
    let mut store = Store::open(path).unwrap();
    store.upsert_account(&Account {
        id: "work".into(),
        name: "Work".into(),
        letter: "W".into(),
        address: "maya@northwind.co".into(),
        sender_name: "Maya Ross".into(),
        color: AccountColor::Work,
        provider: ProviderKind::Gmail,
        org: None,
        org_domain: None,
    }).unwrap();
    Connection::open(path)
        .unwrap()
        .execute_batch(&format!(
            "BEGIN;
             INSERT INTO mailboxes (id, account_id, name, role) VALUES (1, 'work', 'INBOX', 'inbox'), (2, 'work', 'Sent', 'sent');
             WITH RECURSIVE n(i) AS (VALUES (0) UNION ALL SELECT i + 1 FROM n WHERE i + 1 < {THREADS})
             INSERT INTO threads (id, account_id, subject, subject_key, participants_key, last_message_at)
                 SELECT i + 1, 'work', 'Subject ' || i, 'subject ' || i, '', {NOW} - i * 60000 FROM n;
             WITH RECURSIVE n(i) AS (VALUES (0) UNION ALL SELECT i + 1 FROM n WHERE i + 1 < {THREADS})
             INSERT INTO messages (id, account_id, thread_id, message_id, subject, from_name, from_email, to_json, cc_json,
                     bcc_json, participants, sent_at, received_at, snippet, from_me, split, section, read, done)
                 SELECT 2 * i + 1, 'work', i + 1, 'a' || i || '@example.test', 'Subject ' || i,
                     'Person ' || (i % {PEOPLE}), 'p' || (i % {PEOPLE}) || '@d' || (i % 500) || '.example',
                     '[{{\"name\":\"Maya Ross\",\"email\":\"maya@northwind.co\"}}]', '[]', '[]', 'Person ' || (i % {PEOPLE}),
                     {NOW} - i * 60000 - 30000, {NOW} - i * 60000 - 30000, 'Can we talk about item ' || i, 0,
                     CASE i % 5 WHEN 0 THEN 'important' WHEN 1 THEN 'other' WHEN 2 THEN 'newsletters' WHEN 3 THEN 'receipts' ELSE 'calendar' END,
                     CASE i % 3 WHEN 0 THEN 'reply' ELSE 'info' END, i % 4 <> 0, (i * 7) % 100 >= {open}
                 FROM n;
             WITH RECURSIVE n(i) AS (VALUES (0) UNION ALL SELECT i + 1 FROM n WHERE i + 1 < {THREADS})
             INSERT INTO messages (id, account_id, thread_id, message_id, subject, from_name, from_email, to_json, cc_json,
                     bcc_json, participants, sent_at, received_at, snippet, from_me, split, section, read, done)
                 SELECT 2 * i + 2, 'work', i + 1, 'b' || i || '@example.test', 'Re: Subject ' || i,
                     CASE WHEN i % 2 = 0 THEN 'Maya Ross' ELSE 'Person ' || (i % {PEOPLE}) END,
                     CASE WHEN i % 2 = 0 THEN 'maya@northwind.co' ELSE 'p' || (i % {PEOPLE}) || '@d' || (i % 500) || '.example' END,
                     CASE WHEN i % 2 = 0
                         THEN '[{{\"name\":\"Person ' || (i % {PEOPLE}) || '\",\"email\":\"p' || (i % {PEOPLE}) || '@d' || (i % 500) || '.example\"}}]'
                         ELSE '[{{\"name\":\"Maya Ross\",\"email\":\"maya@northwind.co\"}}]' END,
                     '[]', '[]', 'Person ' || (i % {PEOPLE}),
                     {NOW} - i * 60000, {NOW} - i * 60000, 'Sounds good, item ' || i, i % 2 = 0,
                     CASE i % 5 WHEN 0 THEN 'important' WHEN 1 THEN 'other' WHEN 2 THEN 'newsletters' WHEN 3 THEN 'receipts' ELSE 'calendar' END,
                     CASE i % 3 WHEN 0 THEN 'reply' ELSE 'info' END, 1, (i * 7) % 100 >= {open} OR i % 2 = 0
                 FROM n;
             INSERT INTO message_locations (message_id, mailbox_id, provider_id)
                 SELECT id, CASE WHEN from_me THEN 2 ELSE 1 END, '1:' || id FROM messages WHERE from_me OR NOT done;
             INSERT INTO message_search (rowid, subject, participants, body)
                 SELECT id, subject, participants, snippet || ' word' || (id % 1000) FROM messages;
             INSERT INTO people (account_id, email, name, from_them, to_them, last_at)
                 SELECT 'work', from_email, from_name, count(*), 0, max(sent_at) FROM messages WHERE from_me = 0 GROUP BY from_email;
             WITH RECURSIVE n(i) AS (VALUES (0) UNION ALL SELECT i + 1 FROM n WHERE i + 1 < 100)
             INSERT INTO sender_rules (account_id, sender, rule, created_at)
                 SELECT 'work', 'p' || (i * 37) || '@d' || ((i * 37) % 500) || '.example', CASE WHEN i % 2 = 0 THEN 'block' ELSE 'accept' END, {NOW} FROM n;
             UPDATE messages AS m SET blocked = 1 WHERE from_me = 0
                 AND EXISTS (SELECT 1 FROM sender_rules r WHERE r.sender = m.from_email AND r.rule = 'block');
             INSERT INTO ai_decisions (message_id, question, yes, source, acted, value, decided_at)
                 SELECT id, 'needsReply', 0.9, 'model', 1, section, {NOW} FROM messages WHERE id % 3 = 0;
             UPDATE messages SET conversation_id = thread_id;
             INSERT INTO conversations (account_id, id, filled) SELECT 'work', id, id % 10 <> 0 FROM threads;
             COMMIT;"
        ))
        .unwrap();
    store
}

/// The best of `runs` timings of `f`, so one slow run (a page fault, another process) doesn't count.
fn best<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut fastest = Duration::MAX;
    let mut last = None;
    for _ in 0..runs {
        let start = Instant::now();
        let out = f();
        fastest = fastest.min(start.elapsed());
        last = Some(out);
    }
    (fastest, last.unwrap())
}

fn report(name: &str, took: Duration, what: impl std::fmt::Display) {
    eprintln!("{name:<28} {:>10.1} ms   {what}", took.as_secs_f64() * 1000.0);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn bench_big_mailbox() {
    let open = std::env::var("MOSS_BENCH_OPEN").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let dir = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let path = dir.path().join("moss.db");
    let store = mailbox(&path, open);
    eprintln!("built {THREADS} threads, {} messages, {open}% open, in {:.1} s", 2 * THREADS, start.elapsed().as_secs_f64());

    let (took, n) = best(3, || {
        let mut all = Vec::new();
        for split in [SplitId::Important, SplitId::Other, SplitId::Calendar, SplitId::Newsletters, SplitId::Receipts] {
            all.extend(store.list_threads(None, split, NOW).unwrap());
        }
        all.len()
    });
    report("inbox (5 splits)", took, format!("{n} threads"));
    let (took, n) = best(3, || store.list_open(None, NOW).unwrap().len());
    report("inbox (one pass)", took, format!("{n} threads"));
    let (took, counts) = best(3, || store.split_counts(None, NOW).unwrap());
    report("split counts", took, format!("{} important", counts.important));
    let (took, thread) = best(3, || store.thread("2", NOW).unwrap().unwrap());
    report("open a thread", took, format!("{} messages", thread.messages.len()));
    let (took, hits) = best(3, || store.search("word7", None, 50, NOW).unwrap());
    report("search", took, format!("{} hits", hits.len()));
    let (took, trust) = best(3, || store.image_trust("work", "p4999@d499.example").unwrap());
    report("image trust (new sender)", took, format!("{trust:?}"));
    let (took, trust) = best(3, || store.image_trust("work", "nobody@d2.example").unwrap());
    report("image trust (same company)", took, format!("{trust:?}"));
    let (took, n) = best(3, || store.undecided(NEEDS_REPLY, NOW - 365 * 86_400_000, 20, true).unwrap().len());
    report("undecided (20)", took, format!("{n}"));
    let (took, n) = best(3, || store.unembedded("bench-model", 20).unwrap().len());
    report("unembedded (20)", took, format!("{n}"));
    let (took, n) = best(3, || store.conversations_to_fill("work", 20).unwrap().len());
    report("conversations to fill (20)", took, format!("{n}"));
    let (took, n) = best(3, || store.to_summarize("bench-model", 20).unwrap().len());
    report("to summarize (20)", took, format!("{n}"));
    let (took, n) = best(3, || store.list_blocked(None, 50, NOW).unwrap().len());
    report("blocked list (50)", took, format!("{n}"));
    let (took, n) = best(3, || store.list_done(None, 50, NOW).unwrap().len());
    report("done list (50)", took, format!("{n}"));
    let (took, n) = best(3, || store.provider_ids("work", "INBOX").unwrap().len());
    report("provider ids (inbox)", took, format!("{n} ids"));
    let on_server: Vec<String> = store.provider_ids("work", "INBOX").unwrap().into_iter().skip(1).collect();
    let (took, gone) = best(1, || {
        let stored = store.provider_ids("work", "INBOX").unwrap();
        stored.into_iter().filter(|p| !p.starts_with("local:") && !on_server.contains(p)).count()
    });
    report("reconcile diff (Vec)", took, format!("{gone} gone"));
    let (took, gone) = best(3, || {
        let on_server: std::collections::HashSet<&str> = on_server.iter().map(String::as_str).collect();
        let stored = store.provider_ids("work", "INBOX").unwrap();
        stored.into_iter().filter(|p| !p.starts_with("local:") && !on_server.contains(p.as_str())).count()
    });
    report("reconcile diff (HashSet)", took, format!("{gone} gone"));
    let mut store = store;
    let (took, _) = best(1, || store.block_sender("work", "p1@d1.example", false, NOW).unwrap());
    report("block a sender", took, "");
    let (took, _) = best(1, || store.compact().unwrap());
    report("compact", took, "");
    if let Ok(keep) = std::env::var("MOSS_BENCH_KEEP") {
        Connection::open(&path).unwrap().execute("VACUUM INTO ?1", [keep]).unwrap();
    }
}
