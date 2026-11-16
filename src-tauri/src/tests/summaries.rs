//! The summary commands, as the window calls them.

use super::*;
use moss_core::{SummaryContent, SummaryKind, SummaryRequest};
use moss_engine::SummaryNeed;

/// An app with one conversation from Dana; returns its id.
fn with_a_thread() -> (App, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let mut store = moss_store::Store::open(dir.path().join("data/moss.db")).unwrap();
    store
        .upsert_account(&moss_core::Account {
            id: "work".into(),
            name: "Work".into(),
            letter: "W".into(),
            address: "maya@northwind.co".into(),
            sender_name: "Maya".into(),
            color: moss_core::AccountColor::Work,
            provider: moss_core::ProviderKind::Imap,
            org: None,
            org_domain: None,
        })
        .unwrap();
    let raw = b"From: Dana <dana@acme.co>\r\nTo: maya@northwind.co\r\nSubject: Deck\r\n\
        Message-ID: <d@acme.co>\r\n\r\nCan you send it by Friday?\r\n";
    let source = moss_core::MessageSource {
        account_id: "work".into(),
        mailbox: "INBOX".into(),
        provider_id: "1".into(),
        ..Default::default()
    };
    let thread = store.ingest(raw, &source).unwrap().thread_id;
    drop(store);
    (app_in(dir), thread)
}

#[test]
fn a_kept_summary_comes_back_and_a_new_one_needs_the_writing_model() {
    let (a, id) = with_a_thread();
    let engine = a.app.state::<Engine>();
    let actions = SummaryRequest::of(SummaryKind::Actions);
    let SummaryNeed::Write(c) = engine.summary_for(&id, &actions, "e4b", false).unwrap() else { panic!("write") };
    let text = crate::summaries::text(&c);
    assert!(text.starts_with("The user is maya@northwind.co. Today is "), "{text}");
    assert!(text.contains("\n\nSubject: Deck\n\nFrom: Dana <dana@acme.co>\nDate: "), "{text}");
    // One written with older prompts still shows as it opens (a rebuild keeps it), but asking
    // for it writes it again.
    engine.keep_summary(&c, &actions, "e4b", &SummaryContent::Actions { items: vec![] }).unwrap();
    assert_eq!(a.call("ai_summaries", json!({"id": id})).unwrap()["summary"]["model"], "e4b");
    assert_eq!(a.kind("ai_summarize", json!({"stream": "s1", "id": id, "kind": "actions"})), "notReady");
    let key = moss_ai::summaries::model_key("e4b");
    engine.keep_summary(&c, &actions, &key, &SummaryContent::Actions { items: vec![] }).unwrap();

    let kept = a.call("ai_summarize", json!({"stream": "s1", "id": id, "kind": "actions"})).unwrap();
    assert_eq!(kept["content"], json!({"format": "actions", "items": []}));
    assert_eq!(kept["why"], "asked");
    assert_eq!(a.kind("ai_summarize", json!({"stream": "s1", "id": id, "kind": "actions", "again": true})), "notReady");
    assert_eq!(a.kind("ai_summarize", json!({"stream": "s1", "id": id, "kind": "facts"})), "notReady");
    assert_eq!(a.kind("ai_summarize", json!({"stream": "s1", "id": id, "kind": "custom", "instruction": " "})), "invalid");
    assert_eq!(a.kind("ai_summarize", json!({"stream": "s1", "id": "99", "kind": "short"})), "unknownThread");
    assert_eq!(a.call("ai_summaries", json!({"id": id})).unwrap()["summary"], kept);
    assert_eq!(a.kind("ai_summaries", json!({"id": "99"})), "unknownThread");
    // Discarded, it's gone.
    a.call("discard_summary", json!({"id": id})).unwrap();
    assert_eq!(a.call("ai_summaries", json!({"id": id})).unwrap(), json!({}));
    assert_eq!(a.kind("discard_summary", json!({"id": "99"})), "unknownThread");
}

#[test]
fn rules_and_the_automatic_setting_are_kept() {
    let (a, id) = with_a_thread();
    let rule = a.call("save_summary_rule", json!({"sender": "Acme.co", "kind": "custom", "instruction": "Only dates"})).unwrap();
    assert_eq!((rule["sender"].as_str(), rule["instruction"].as_str()), (Some("acme.co"), Some("Only dates")));
    let changed = a.call("save_summary_rule", json!({"id": rule["id"], "sender": "acme.co", "kind": "facts"})).unwrap();
    assert_eq!(changed["kind"], "facts");
    assert_eq!(a.call("summary_rules", json!({})).unwrap(), json!([changed]));
    assert_eq!(a.call("ai_summaries", json!({"id": id})).unwrap()["rule"], changed);
    assert_eq!(a.kind("save_summary_rule", json!({"sender": "gmail.com", "kind": "short"})), "invalid");
    assert_eq!(a.kind("save_summary_rule", json!({"sender": "x.co", "kind": "custom"})), "invalid");
    assert_eq!(a.call("remove_summary_rule", json!({"id": rule["id"]})).unwrap(), json!(true));
    assert_eq!(a.call("automatic_summaries", json!({})).unwrap(), json!(true));
    a.call("set_automatic_summaries", json!({"on": false})).unwrap();
    assert_eq!(a.call("automatic_summaries", json!({})).unwrap(), json!(false));
}
