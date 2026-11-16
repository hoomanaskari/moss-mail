use super::*;
use crate::test_support::*;
use moss_core::ThreadSummary;

const MAYA: &str = "Maya Ross <maya@northwind.co>";
const DANIEL: &str = "Daniel Okafor <daniel@northwind.co>";

fn pk(store: &Store, provider_id: &str) -> i64 {
    store
        .conn
        .query_row("SELECT message_id FROM message_locations WHERE provider_id = ?1", [provider_id], |r| r.get(0))
        .unwrap()
}

fn row(store: &Store, thread: &str, now: i64) -> ThreadSummary {
    store.thread(thread, now).unwrap().unwrap().summary
}

fn section(store: &Store, thread: &str) -> (Section, Option<u8>) {
    let s = row(store, thread, NOV_1);
    (s.section, s.ai_sure)
}

fn model(yes: f64, acted: bool, value: &str) -> Decision {
    Decision { source: Source::Model, yes: Some(yes), acted, value: Some(value.into()), evidence: None }
}

fn rule(acted: bool, value: Option<&str>, evidence: &str) -> Decision {
    Decision { source: Source::Rule, yes: None, acted, value: value.map(String::from), evidence: Some(evidence.into()) }
}

fn subjects(list: &[Undecided]) -> Vec<&str> {
    list.iter().map(|u| u.subject.as_str()).collect()
}

/// Maya writes to Daniel, at `at`.
fn sent(store: &mut Store, provider_id: &str, subject: &str, at: i64) -> crate::Ingested {
    let id = format!("{provider_id}@sent");
    in_sent(store, provider_id, &mail().id(&id).from(MAYA).to(DANIEL).subject(subject).body("Can you send it?").at(at).build())
}

/// Mail you sent, as your Sent folder has it.
fn in_sent(store: &mut Store, provider_id: &str, raw: &[u8]) -> crate::Ingested {
    store.upsert_mailbox(&moss_core::Mailbox { account_id: "work".into(), name: "Sent".into(), role: Some(moss_core::MailboxRole::Sent) }).unwrap();
    store.ingest(raw, &moss_core::MessageSource { mailbox: "Sent".into(), ..source(provider_id) }).unwrap()
}

#[test]
fn lists_new_mail_from_people_newest_first() {
    let mut store = store_with_work();
    ingest(&mut store, "old", &mail().id("old@x").subject("Old").at(NOV_1 - 40 * DAY).build());
    ingest(&mut store, "a", &mail().id("a@x").subject("Deck").body("Can you send it?").at(NOV_1 - DAY).build());
    ingest(&mut store, "b", &mail().id("b@x").subject("Lunch").attach("menu.pdf", "application/pdf", "x").at(NOV_1).build());
    ingest(&mut store, "news", &mail().id("n@x").header("List-Unsubscribe", "<mailto:u@x.co>").at(NOV_1).build());
    ingest(&mut store, "mine", &mail().id("m@x").from(MAYA).at(NOV_1).build());

    let list = store.undecided(NEEDS_REPLY, NOV_1 - 30 * DAY, 10, true).unwrap();
    assert_eq!(subjects(&list), ["Lunch", "Deck"]);
    assert_eq!(list[0].attachments, ["menu.pdf"]);
    assert_eq!(list[0].account_id, "work");
    assert_eq!(list[1].body, "Can you send it?");
    assert_eq!(list[1].from.email, "daniel@northwind.co");
    assert_eq!(list[1].to[0].email, "maya@northwind.co");
    assert_eq!(list[1].sent_at, NOV_1 - DAY);
    assert_eq!(list[1].split, SplitId::Important);
    assert_eq!(list[1].message_id, pk(&store, "a"));
    assert_eq!(store.undecided(NEEDS_REPLY, NOV_1 - 30 * DAY, 1, true).unwrap().len(), 1);
    assert!(store.undecided("unknown", 0, 10, true).unwrap().is_empty());
}

#[test]
fn each_question_is_about_its_own_mail() {
    let mut store = store_with_work();
    ingest(&mut store, "a", &mail().id("a@x").subject("Deck").at(NOV_1).build());
    ingest(&mut store, "r", &mail().id("r@x").subject("Your receipt").at(NOV_1).build());
    ingest(&mut store, "n", &mail().id("n@x").subject("News").header("List-Id", "<x>").at(NOV_1).build());
    sent(&mut store, "s", "Lunch?", NOV_1);
    // Sent to no one but Bcc: nobody to wait on.
    ingest(&mut store, "b", &mail().id("b@x").from(MAYA).without("To").subject("Note").at(NOV_1).build());

    let about = |q| subjects(&store.undecided(q, 0, 10, true).unwrap()).into_iter().map(String::from).collect::<Vec<_>>();
    assert_eq!(about(NEEDS_REPLY), ["Deck"]);
    // Mail its list headers put in Newsletters is asked too: the decision model says what it is.
    assert_eq!(about(SPLIT), ["News", "Your receipt", "Deck"]);
    assert_eq!(about(DEADLINE), ["Your receipt", "Deck"]);
    assert_eq!(about(EXPECTS_REPLY), ["Lunch?"]);
}

#[test]
fn only_the_latest_open_message_in_a_thread_is_asked() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().id("q@x").at(NOV_1 - DAY).build()).thread_id;
    ingest(&mut store, "2", &mail().id("r@x").reply_to(&["q@x"]).at(NOV_1).build());
    let list = store.undecided(NEEDS_REPLY, 0, 10, true).unwrap();
    assert_eq!(list.iter().map(|u| u.message_id).collect::<Vec<_>>(), [pk(&store, "2")]);
    store.mark_done(&thread, true, NOV_1).unwrap();
    assert!(store.undecided(NEEDS_REPLY, 0, 10, true).unwrap().is_empty());
    store.mark_done(&thread, false, NOV_1).unwrap();
    store.delete_thread(&thread).unwrap();
    assert!(store.undecided(NEEDS_REPLY, 0, 10, true).unwrap().is_empty(), "deleted mail isn't asked about");
}

#[test]
fn the_model_looks_again_at_what_only_a_rule_answered_but_never_at_yours() {
    let mut store = store_with_work();
    let a = ingest(&mut store, "a", &mail().id("a@x").subject("Deck").at(NOV_1).build());
    ingest(&mut store, "b", &mail().id("b@x").subject("Lunch").at(NOV_1 - 1).build());
    ingest(&mut store, "c", &mail().id("c@x").subject("Tea").at(NOV_1 - 2).build());
    store.decide(pk(&store, "a"), DEADLINE, &rule(false, None, ""), NOV_1).unwrap();
    store.decide(pk(&store, "b"), DEADLINE, &model(0.5, false, "2026-11-13"), NOV_1).unwrap();
    assert_eq!(subjects(&store.undecided(DEADLINE, 0, 10, false).unwrap()), ["Tea"], "rules answer each once");
    assert_eq!(subjects(&store.undecided(DEADLINE, 0, 10, true).unwrap()), ["Deck", "Tea"], "the model asks once");
    store.clear_deadline(&a.thread_id, NOV_1).unwrap();
    assert_eq!(subjects(&store.undecided(DEADLINE, 0, 10, true).unwrap()), ["Tea"]);
}

#[test]
fn a_sure_answer_moves_the_message_and_says_how_sure() {
    let mut store = store_with_work();
    // Sent to me directly: the rule says Reply.
    let thread = ingest(&mut store, "1", &mail().build()).thread_id;
    assert_eq!(section(&store, &thread), (Section::Reply, None));
    let id = pk(&store, "1");

    assert!(store.decide(id, NEEDS_REPLY, &model(0.05, true, "info"), NOV_1).unwrap());
    assert_eq!(section(&store, &thread), (Section::Info, Some(95)));
    assert!(store.undecided(NEEDS_REPLY, 0, 10, true).unwrap().is_empty(), "asked once");
    // Already there: sure again, but nothing moves.
    assert!(!store.decide(id, NEEDS_REPLY, &model(0.02, true, "info"), NOV_1).unwrap());
    assert_eq!(section(&store, &thread), (Section::Info, Some(98)));
}

#[test]
fn unsure_means_the_rule_stays_and_no_ai_is_claimed() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().build()).thread_id;
    assert!(!store.decide(pk(&store, "1"), NEEDS_REPLY, &model(0.6, false, "reply"), NOV_1).unwrap());
    assert_eq!(section(&store, &thread), (Section::Reply, None));
    assert!(store.undecided(NEEDS_REPLY, 0, 10, true).unwrap().is_empty());
}

#[test]
fn a_sure_split_moves_the_message_out_of_important_and_says_why() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().subject("Lunch").build()).thread_id;
    assert_eq!(row(&store, &thread, NOV_1).split_why, None);
    assert!(store.decide(pk(&store, "1"), SPLIT, &model(0.88, true, "receipts"), NOV_1).unwrap());
    let moved = row(&store, &thread, NOV_1);
    assert_eq!((moved.split, moved.section), (SplitId::Receipts, Section::Info));
    assert_eq!(moved.split_why, Some(Why { by: DecidedBy::Ai, sure: Some(88), evidence: None }));
    assert_eq!(moved.ai_sure, None, "the split isn't the needs-reply answer");
    // Back to Important keeps the section it has.
    assert!(store.decide(pk(&store, "1"), SPLIT, &model(0.9, true, "important"), NOV_1).unwrap());
    assert_eq!(row(&store, &thread, NOV_1).section, Section::Info);
    assert!(!store.decide(pk(&store, "1"), SPLIT, &model(0.6, false, "important"), NOV_1).unwrap());
    assert_eq!(row(&store, &thread, NOV_1).split_why, None);
}

#[test]
fn a_deadline_shows_on_the_thread_with_its_reason() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().id("q@x").build()).thread_id;
    assert_eq!(row(&store, &thread, NOV_1).due, None);
    assert!(store.decide(pk(&store, "1"), DEADLINE, &rule(true, Some("2026-11-06"), "by Friday"), NOV_1).unwrap());
    let due = Due { date: "2026-11-06".into(), why: Why { by: DecidedBy::Rule, sure: None, evidence: Some("by Friday".into()) } };
    assert_eq!(row(&store, &thread, NOV_1).due, Some(due.clone()));
    // You answer: it's still Daniel's deadline.
    ingest(&mut store, "3", &mail().id("y@x").from(MAYA).to(DANIEL).reply_to(&["q@x"]).at(NOV_1 + 1).build());
    assert_eq!(row(&store, &thread, NOV_1).due, Some(due));
    assert!(store.decide(pk(&store, "1"), DEADLINE, &model(0.9, true, "2026-11-13"), NOV_1).unwrap());
    assert_eq!(row(&store, &thread, NOV_1).due.unwrap().why, Why { by: DecidedBy::Ai, sure: Some(90), evidence: None });
    assert!(!store.decide(pk(&store, "1"), DEADLINE, &model(0.1, false, "2026-11-13"), NOV_1).unwrap());
    assert_eq!(row(&store, &thread, NOV_1).due, None);
    // An acted answer with no day shows nothing.
    store.decide(pk(&store, "1"), DEADLINE, &rule(true, None, "soon"), NOV_1).unwrap();
    assert_eq!(row(&store, &thread, NOV_1).due, None);
}

#[test]
fn mail_you_sent_waits_out_of_sight_and_comes_back_if_no_one_answers() {
    let mut store = store_with_work();
    let got = sent(&mut store, "1", "Lunch?", NOV_1);
    assert!(store.list_threads(None, SplitId::Important, NOV_1).unwrap().is_empty(), "sent mail is done");
    let until = NOV_1 + 3 * DAY;
    let decision = Decision { evidence: Some("Can you send it?".into()), ..rule(true, Some(&until.to_string()), "") };
    assert!(store.decide(pk(&store, "1"), EXPECTS_REPLY, &decision, NOV_1).unwrap());

    let waiting = row(&store, &got.thread_id, NOV_1);
    assert_eq!(waiting.section, Section::Later);
    assert_eq!(waiting.snoozed_until, Some(until));
    assert!(waiting.you_wrote_last);
    assert_eq!(
        waiting.waiting,
        Some(Waiting { until, why: Why { by: DecidedBy::Rule, sure: None, evidence: Some("Can you send it?".into()) } })
    );
    assert_eq!(store.list_threads(None, SplitId::Important, NOV_1).unwrap().len(), 1);
    // No answer by then: back, needing you.
    assert_eq!(row(&store, &got.thread_id, until).section, Section::Reply);
    assert_eq!(store.split_counts(None, until).unwrap().important, 1);

    // Daniel answers: an ordinary email from him, nothing waiting.
    ingest(&mut store, "2", &mail().id("ans@x").reply_to(&["1@sent"]).at(NOV_1 + DAY).build());
    let answered = row(&store, &got.thread_id, until);
    assert_eq!((answered.waiting, answered.you_wrote_last), (None, false));
    assert_eq!(answered.section, Section::Reply);
}

#[test]
fn unsure_or_no_means_your_sent_mail_stays_as_it_was() {
    let mut store = store_with_work();
    let got = sent(&mut store, "1", "Thanks!", NOV_1);
    assert!(!store.decide(pk(&store, "1"), EXPECTS_REPLY, &model(0.5, false, "0"), NOV_1).unwrap());
    let row = row(&store, &got.thread_id, NOV_1);
    assert!(row.done && row.waiting.is_none() && row.you_wrote_last);
}

#[test]
fn moving_it_yourself_wins_now_and_later() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().build()).thread_id;
    let id = pk(&store, "1");
    store.decide(id, NEEDS_REPLY, &model(0.1, true, "info"), NOV_1).unwrap();
    store.set_section(&thread, Section::Reply, NOV_1).unwrap();
    assert_eq!(section(&store, &thread), (Section::Reply, None));
    // Moved before the model got to it: the model can't move it back.
    let other = ingest(&mut store, "2", &mail().id("two@x").build()).thread_id;
    store.set_section(&other, Section::Info, NOV_1).unwrap();
    assert!(store.undecided(NEEDS_REPLY, 0, 10, true).unwrap().is_empty());
    assert!(!store.decide(pk(&store, "2"), NEEDS_REPLY, &model(0.99, true, "reply"), NOV_1).unwrap());
    assert_eq!(section(&store, &other), (Section::Info, None));
}

#[test]
fn the_model_takes_list_mail_out_of_newsletters_and_says_why() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "n", &mail().id("n@x").subject("Build failed").header("List-Id", "<builds.ci.example>").build()).thread_id;
    let asked = store.undecided(SPLIT, 0, 10, true).unwrap();
    assert_eq!((subjects(&asked), asked[0].split), (vec!["Build failed"], SplitId::Newsletters));
    assert!(store.decide(pk(&store, "n"), SPLIT, &model(0.55, true, "other"), NOV_1).unwrap());
    let moved = row(&store, &thread, NOV_1);
    assert_eq!((moved.split, moved.section), (SplitId::Other, Section::Info));
    assert_eq!(moved.split_why, Some(Why { by: DecidedBy::Ai, sure: Some(55), evidence: None }));
    assert!(store.undecided(SPLIT, 0, 10, true).unwrap().is_empty(), "asked once");
}

#[test]
fn mail_in_other_is_asked_which_tab_too_and_each_email_says_whether_you_know_the_sender() {
    let mut store = store_with_work();
    ingest(&mut store, "p", &mail().id("p@x").from("Leo Grant <leo@clipcast.example>").subject("Faster product videos").at(NOV_1 - 3 * DAY).build());
    ingest(&mut store, "o", &mail().id("o@x").from("Lumen <no-reply@lumen.example>").subject("New sign-in").at(NOV_1 - 2 * DAY).build());
    ingest(&mut store, "d", &mail().id("d@x").subject("Deck").at(NOV_1 - DAY).build());
    // Maya has written to Daniel, so she knows him; Leo and the sign-in robot are strangers.
    sent(&mut store, "s", "Lunch?", NOV_1 - 10 * DAY);
    let asked = store.undecided(SPLIT, 0, 10, true).unwrap();
    let seen: Vec<(&str, SplitId, bool)> = asked.iter().map(|u| (u.subject.as_str(), u.split, u.known)).collect();
    assert_eq!(seen, [("Deck", SplitId::Important, true), ("New sign-in", SplitId::Other, false), ("Faster product videos", SplitId::Important, false)]);
}

#[test]
fn a_thread_you_moved_is_never_asked_which_tab_again_even_about_new_mail() {
    let mut store = store_with_work();
    let list = |b: Mail| b.header("List-Id", "<forum.example>");
    let thread = ingest(&mut store, "1", &list(mail().id("q@x").subject("New reply").at(NOV_1 - DAY)).build()).thread_id;
    store.move_thread(&thread, Some(SplitId::Other), NOV_1).unwrap();
    ingest(&mut store, "2", &list(mail().id("r@x").subject("New reply").reply_to(&["q@x"]).at(NOV_1)).build());
    assert!(store.undecided(SPLIT, 0, 10, true).unwrap().is_empty());
    assert_eq!(row(&store, &thread, NOV_1).split, SplitId::Other);
    // Given back to Moss's sorting: the model may decide again.
    store.move_thread(&thread, None, NOV_1).unwrap();
    assert_eq!(subjects(&store.undecided(SPLIT, 0, 10, true).unwrap()), ["New reply"]);
}

#[test]
fn moving_a_thread_to_another_split_is_for_good() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().build()).thread_id;
    store.decide(pk(&store, "1"), SPLIT, &model(0.9, true, "receipts"), NOV_1).unwrap();
    store.set_split(&thread, SplitId::Important, NOV_1).unwrap();
    let back = row(&store, &thread, NOV_1);
    assert_eq!((back.split, back.split_why), (SplitId::Important, None));
    assert!(!store.decide(pk(&store, "1"), SPLIT, &model(0.99, true, "newsletters"), NOV_1).unwrap());
    store.set_split(&thread, SplitId::Other, NOV_1).unwrap();
    assert_eq!((row(&store, &thread, NOV_1).split, row(&store, &thread, NOV_1).section), (SplitId::Other, Section::Info));
}

#[test]
fn no_deadline_takes_the_chip_away_for_good() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().build()).thread_id;
    store.decide(pk(&store, "1"), DEADLINE, &model(0.9, true, "2026-11-13"), NOV_1).unwrap();
    store.clear_deadline(&thread, NOV_1).unwrap();
    assert_eq!(row(&store, &thread, NOV_1).due, None);
    assert!(!store.decide(pk(&store, "1"), DEADLINE, &model(0.95, true, "2026-11-13"), NOV_1).unwrap());
    assert_eq!(row(&store, &thread, NOV_1).due, None);
}

#[test]
fn remind_me_if_no_reply_waits_on_your_latest_email_and_can_be_turned_off() {
    let mut store = store_with_work();
    let got = sent(&mut store, "1", "Lunch?", NOV_1);
    let wait = 3 * DAY;
    assert!(store.remind_if_no_reply(&got.thread_id, true, wait, NOV_1 + DAY).unwrap());
    let on = row(&store, &got.thread_id, NOV_1 + DAY);
    assert_eq!(on.waiting, Some(Waiting { until: NOV_1 + wait, why: Why { by: DecidedBy::You, sure: None, evidence: None } }));
    assert_eq!((on.section, on.snoozed_until), (Section::Later, Some(NOV_1 + wait)));
    // Yours wins: the model can't change it.
    assert!(!store.decide(pk(&store, "1"), EXPECTS_REPLY, &model(0.1, false, "0"), NOV_1).unwrap());
    assert!(store.undecided(EXPECTS_REPLY, 0, 10, true).unwrap().is_empty());

    assert!(store.remind_if_no_reply(&got.thread_id, false, wait, NOV_1 + DAY).unwrap());
    let off = row(&store, &got.thread_id, NOV_1 + DAY);
    assert!(off.done && off.waiting.is_none() && off.snoozed_until.is_none());

    // Sent long ago: it waits from now instead.
    assert!(store.remind_if_no_reply(&got.thread_id, true, wait, NOV_1 + 9 * DAY).unwrap());
    assert_eq!(row(&store, &got.thread_id, NOV_1 + 9 * DAY).waiting.unwrap().until, NOV_1 + 12 * DAY);
}

#[test]
fn remind_me_snoozes_the_whole_thread_like_remind_me_does() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().id("q@x").at(NOV_1).build()).thread_id;
    assert!(!store.remind_if_no_reply(&thread, true, DAY, NOV_1).unwrap(), "Daniel wrote last");
    ingest(&mut store, "2", &mail().id("y@x").from(MAYA).to(DANIEL).reply_to(&["q@x"]).at(NOV_1 + 1).build());
    assert!(store.remind_if_no_reply(&thread, true, DAY, NOV_1 + 1).unwrap());
    assert_eq!(row(&store, &thread, NOV_1 + 2).section, Section::Later);
    assert!(store.remind_if_no_reply(&thread, false, DAY, NOV_1 + 2).unwrap());
    let off = row(&store, &thread, NOV_1 + 2);
    assert_eq!((off.section, off.done), (Section::Info, false), "back as it was: Daniel’s email is still open");
}

#[test]
fn the_model_s_own_reminder_never_hides_your_other_mail() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().id("q@x").at(NOV_1).build()).thread_id;
    ingest(&mut store, "2", &mail().id("y@x").from(MAYA).to(DANIEL).reply_to(&["q@x"]).at(NOV_1 + 1).build());
    let until = NOV_1 + 3 * DAY;
    store.decide(pk(&store, "2"), EXPECTS_REPLY, &model(0.9, true, &until.to_string()), NOV_1).unwrap();
    let row_now = row(&store, &thread, NOV_1 + 2);
    assert_eq!(row_now.section, Section::Info, "Daniel's email is still there, for your information");
    assert_eq!(row_now.waiting.unwrap().why, Why { by: DecidedBy::Ai, sure: Some(90), evidence: None });
    assert_eq!(row(&store, &thread, until).section, Section::Reply);
}

#[test]
fn reasons_name_who_decided() {
    assert_eq!(why("model", Some(0.2), None), Why { by: DecidedBy::Ai, sure: Some(80), evidence: None });
    assert_eq!(why("you", Some(0.9), None), Why { by: DecidedBy::You, sure: None, evidence: None });
    assert_eq!(why("rule", None, Some("x".into())), Why { by: DecidedBy::Rule, sure: None, evidence: Some("x".into()) });
}

#[test]
fn a_failed_write_is_an_error() {
    let mut store = store_with_work();
    let thread = ingest(&mut store, "1", &mail().build()).thread_id;
    let sent_thread = sent(&mut store, "2", "Lunch?", NOV_1).thread_id;
    let id = pk(&store, "1");
    read_only(&store);
    assert!(store.decide(id, NEEDS_REPLY, &model(0.1, true, "info"), NOV_1).is_err());
    assert!(store.set_section(&thread, Section::Info, NOV_1).is_err());
    assert!(store.set_split(&thread, SplitId::Other, NOV_1).is_err());
    assert!(store.clear_deadline(&thread, NOV_1).is_err());
    assert!(store.remind_if_no_reply(&sent_thread, true, DAY, NOV_1).is_err());
    assert!(store.clear_deadline("x", NOV_1).is_err());
    assert!(store.remind_if_no_reply("x", true, DAY, NOV_1).is_err());

    let mut broken = store_with_work();
    let thread = ingest(&mut broken, "1", &mail().build()).thread_id;
    let sent_thread = sent(&mut broken, "2", "Lunch?", NOV_1).thread_id;
    break_schema(&broken, "DROP TABLE ai_decisions");
    assert!(broken.undecided(NEEDS_REPLY, 0, 10, true).is_err());
    assert!(broken.decide(id, NEEDS_REPLY, &model(0.1, true, "info"), NOV_1).is_err());
    assert!(broken.set_section(&thread, Section::Info, NOV_1).is_err());
    assert!(broken.remind_if_no_reply(&sent_thread, true, DAY, NOV_1).is_err());
}

#[test]
fn the_decision_queue_reads_the_newest_mail_first_by_index() {
    let store = store_with_work();
    for question in [NEEDS_REPLY, SPLIT, DEADLINE, EXPECTS_REPLY] {
        let plan = plan(&store, &undecided_sql(about(question).unwrap()));
        assert!(plan.contains("SEARCH m USING INDEX messages_by_received_at (received_at>?)") && !plan.contains("TEMP B-TREE"), "{plan}");
    }
}
