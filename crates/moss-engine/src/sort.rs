//! Deciding about new mail in the background: which split it belongs in, whether it needs
//! your reply, whether it sets you a deadline, and whether your own email expects an answer.
//!
//! The rules sort every message as it arrives (in the store). Then this loop takes each
//! question in turn, newest mail first, a batch at a time. The decision model answers when
//! it's downloaded, and Moss acts only when it's sure; until then, and whenever it's unsure,
//! plain rules answer the questions they can (see [`crate::rules`]). It runs whenever mail
//! changes. See `docs/research/ai-split.md`.

use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Local, TimeZone};
use moss_core::SplitId;
use moss_store::{Decision, Source, Undecided, DEADLINE, EXPECTS_REPLY, NEEDS_REPLY, SPLIT};

use crate::events::Event;
use crate::rules;
use crate::Shared;

/// The decision model, as the engine sees it. The app passes its AI; tests pass a fake.
/// Each answer is `None` while the model can't run (not downloaded yet).
#[async_trait]
pub trait Sorter: Send + Sync {
    /// How likely `email` needs your reply, 0 to 1.
    async fn needs_reply(&self, email: &Undecided) -> Option<f64>;

    /// Which split tab `email` belongs in, and how likely.
    async fn split(&self, _email: &Undecided) -> Option<(SplitId, f64)> {
        None
    }

    /// Whether `email` is a sales pitch: see [`Pitch`].
    async fn pitch(&self, _email: &Undecided) -> Option<Pitch> {
        None
    }

    /// How likely `email` sets you a deadline. The day itself is found by rules.
    async fn deadline(&self, _email: &Undecided) -> Option<f64> {
        None
    }

    /// How likely your `email` asks its recipients for an answer.
    async fn expects_reply(&self, _email: &Undecided) -> Option<f64> {
        None
    }
}

/// What the decision model says about a letter that may be a sales pitch: how likely it is one
/// (someone selling their services, funding or a partnership, unasked), and how likely its
/// sender needs something of the user's (an answer, help, a quote, the user's own product). A
/// would-be customer reads like a pitch, so the second answer keeps them out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pitch {
    pub pitch: f64,
    pub wants: f64,
}

impl From<(f64, f64)> for Pitch {
    /// `(pitch, wants)`, the two answers as the decision model gives them.
    fn from((pitch, wants): (f64, f64)) -> Self {
        Pitch { pitch, wants }
    }
}

impl Pitch {
    /// Sure it is a pitch, and not sure the sender needs something of the user's.
    fn sure(self) -> bool {
        self.pitch >= SURE && self.wants < SURE
    }
}

/// How sure the model must be, either way, before Moss acts.
pub const SURE: f64 = 0.8;
/// How long Moss waits for an answer to your email before bringing it back.
pub const WAIT: i64 = 3 * DAY;
const DAY: i64 = 86_400_000;
/// Only recent mail is decided about; older mail keeps the rules' answer.
const RECENT: i64 = 30 * DAY;
/// Your sent mail from the last week only, so a first sync doesn't bring back months of it.
const RECENT_SENT: i64 = 7 * DAY;
const PER_ROUND: usize = 20;

/// The questions, in the order they're asked: the split first, since only Important mail
/// is asked whether it needs your reply.
const QUESTIONS: [&str; 4] = [SPLIT, NEEDS_REPLY, DEADLINE, EXPECTS_REPLY];

impl Shared {
    /// Decides everything waiting, question by question, newest first. Stops at the first
    /// answer it can't save, to try again the next time mail changes.
    pub(crate) async fn sort_waiting(&self, sorter: &dyn Sorter) {
        let mut changed = BTreeSet::new();
        for question in QUESTIONS {
            if self.sort_question(sorter, question, &mut changed).await.is_err() {
                break;
            }
        }
        for account_id in changed {
            self.events.emit(Event::MailChanged { account_id });
        }
    }

    /// Answers `question` about every message waiting for it. Once the model can't run, the
    /// rest are answered by rules, or left for later when there's no rule.
    async fn sort_question(&self, sorter: &dyn Sorter, question: &str, changed: &mut BTreeSet<String>) -> Result<(), ()> {
        let recent = if question == EXPECTS_REPLY { RECENT_SENT } else { RECENT };
        let mut model = true;
        loop {
            let since = self.now() - recent;
            let batch = self.store().undecided(question, since, PER_ROUND, model).map_err(drop)?;
            if batch.is_empty() {
                return Ok(());
            }
            for email in &batch {
                let asked = if model { ask(sorter, question, email).await } else { None };
                model = asked.is_some();
                let Some(decision) = decide(question, asked, email) else {
                    return Ok(());
                };
                if self.store().decide(email.message_id, question, &decision, self.now()).map_err(drop)? {
                    changed.insert(email.account_id.clone());
                }
            }
        }
    }
}

/// What the model said: how likely yes, or its pick of split and how likely, with whether it
/// is a sales pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Asked {
    Yes(f64),
    /// Mail in Other has no split answer: it is only asked whether it is a pitch.
    Split(Option<(SplitId, f64)>, Option<Pitch>),
}

/// Whether to ask if `email` is a sales pitch: mail the rules took for a person's, unless you
/// know the sender, and anything they put in Other, where pitches from no-reply and helpdesk
/// addresses land. Never list mail (its own question has an answer for pitches) or receipts.
fn may_be_a_pitch(email: &Undecided) -> bool {
    match email.split {
        SplitId::Important => !email.known,
        SplitId::Other => true,
        _ => false,
    }
}

async fn ask(sorter: &dyn Sorter, question: &str, email: &Undecided) -> Option<Asked> {
    match question {
        SPLIT => {
            let split = match email.split {
                SplitId::Other => None,
                _ => Some(sorter.split(email).await?),
            };
            let pitch = if may_be_a_pitch(email) { Some(sorter.pitch(email).await?) } else { None };
            Some(Asked::Split(split, pitch))
        }
        NEEDS_REPLY => sorter.needs_reply(email).await.map(Asked::Yes),
        DEADLINE => sorter.deadline(email).await.map(Asked::Yes),
        _ => sorter.expects_reply(email).await.map(Asked::Yes),
    }
}

/// Sure either way: `Some(true)` yes, `Some(false)` no, `None` unsure.
fn sure(yes: f64) -> Option<bool> {
    if yes >= SURE {
        Some(true)
    } else if yes <= 1.0 - SURE {
        Some(false)
    } else {
        None
    }
}

/// What to save for `question` about `email`, from the model's answer (`asked`) or, without
/// one, a rule. `None` when there's no answer and no rule (split and needs reply: the rules
/// already sorted the mail as it arrived).
pub(crate) fn decide(question: &str, asked: Option<Asked>, email: &Undecided) -> Option<Decision> {
    let model = |yes: f64, acted: bool, value: Option<String>| Decision {
        source: Source::Model,
        yes: Some(yes),
        acted,
        value,
        evidence: None,
    };
    match (question, asked) {
        // A sales pitch reads like a letter from a person, but it is marketing: Newsletters,
        // when the model is sure, whatever the split answer says.
        (SPLIT, Some(Asked::Split(_, Some(pitch)))) if pitch.sure() => {
            Some(model(pitch.pitch, true, Some(split_name(SplitId::Newsletters).into())))
        }
        // Mail in Other that isn't a pitch stays where the rules put it.
        (SPLIT, Some(Asked::Split(None, Some(pitch)))) => {
            Some(model(pitch.pitch, false, Some(split_name(email.split).into())))
        }
        // Mail in Newsletters that no one has decided about is there only for its list headers,
        // which newsletters and notices about your own work both have. Whether it is a
        // newsletter is the model's call, so its pick decides, sure or not: there is no rule
        // to fall back on. Important asks for your attention, so only when the model is sure;
        // a weaker "important" still says it isn't a newsletter, and Other holds notices.
        (SPLIT, Some(Asked::Split(Some((split, p)), _))) if email.split == SplitId::Newsletters => {
            let to = if split == SplitId::Important && p < SURE { SplitId::Other } else { split };
            Some(model(p, to != email.split, Some(split_name(to).into())))
        }
        (SPLIT, Some(Asked::Split(Some((split, p)), _))) => {
            Some(model(p, p >= SURE && split != email.split, Some(split_name(split).into())))
        }
        (NEEDS_REPLY, Some(Asked::Yes(yes))) => {
            let section = sure(yes).map(|reply| if reply { "reply" } else { "info" }.to_string());
            Some(model(yes, section.is_some(), section))
        }
        (DEADLINE, asked) => {
            let sent = local(email.sent_at);
            let rule = rules::deadline(&email.subject, &email.body, sent);
            let yes = match asked {
                Some(Asked::Yes(yes)) => Some(yes),
                _ => None,
            };
            Some(match yes.and_then(sure) {
                // The model says so; the day is the one the words name, or the first one.
                Some(true) => {
                    let day = rule.or_else(|| rules::first_day(&email.subject, &email.body, sent));
                    model(yes?, day.is_some(), day.map(|(d, _)| d.to_string()))
                }
                Some(false) => model(yes?, false, None),
                None => Decision {
                    source: Source::Rule,
                    yes,
                    acted: rule.is_some(),
                    value: rule.as_ref().map(|(d, _)| d.to_string()),
                    evidence: rule.map(|(_, words)| words),
                },
            })
        }
        (EXPECTS_REPLY, asked) => {
            let until = Some((email.sent_at + WAIT).to_string());
            let yes = match asked {
                Some(Asked::Yes(yes)) => Some(yes),
                _ => None,
            };
            Some(match yes.and_then(sure) {
                Some(true) => model(yes?, true, until),
                Some(false) => model(yes?, false, None),
                None => {
                    let words = rules::expects_reply(&email.body);
                    Decision { source: Source::Rule, yes, acted: words.is_some(), value: words.as_ref().and(until), evidence: words }
                }
            })
        }
        _ => None,
    }
}

fn split_name(split: SplitId) -> &'static str {
    match split {
        SplitId::Important => "important",
        SplitId::Other => "other",
        SplitId::Calendar => "calendar",
        SplitId::Newsletters => "newsletters",
        SplitId::Receipts => "receipts",
    }
}

/// When an email was sent, on this Mac's clock: "by Friday" means your Friday.
fn local(ms: i64) -> DateTime<FixedOffset> {
    let utc = DateTime::from_timestamp_millis(ms).unwrap_or_default();
    Local.from_utc_datetime(&utc.naive_utc()).fixed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moss_core::Person;

    fn email(subject: &str, body: &str) -> Undecided {
        Undecided {
            message_id: 1,
            account_id: "gmail".into(),
            from: Person { name: "Dana".into(), email: "dana@acme.co".into() },
            to: vec![],
            subject: subject.into(),
            attachments: vec![],
            body: body.into(),
            sent_at: 1_793_524_320_000,
            split: SplitId::Important,
            known: false,
        }
    }

    fn decision(source: Source, yes: Option<f64>, acted: bool, value: Option<&str>, evidence: Option<&str>) -> Decision {
        Decision { source, yes, acted, value: value.map(String::from), evidence: evidence.map(String::from) }
    }

    #[test]
    fn needs_reply_moves_mail_only_when_sure_and_has_no_rule() {
        let e = email("Deck", "");
        let yes = |p| decide(NEEDS_REPLY, Some(Asked::Yes(p)), &e).unwrap();
        assert_eq!(yes(0.9), decision(Source::Model, Some(0.9), true, Some("reply"), None));
        assert_eq!(yes(0.1), decision(Source::Model, Some(0.1), true, Some("info"), None));
        assert_eq!(yes(0.5), decision(Source::Model, Some(0.5), false, None, None));
        assert_eq!(decide(NEEDS_REPLY, None, &e), None);
        assert_eq!(decide("unknown", Some(Asked::Yes(0.9)), &e), None);
    }

    #[test]
    fn a_split_moves_mail_only_when_sure_and_somewhere_else() {
        let e = email("Your order", "");
        let split = |s, p| decide(SPLIT, Some(Asked::Split(Some((s, p)), None)), &e).unwrap();
        assert_eq!(split(SplitId::Receipts, 0.9), decision(Source::Model, Some(0.9), true, Some("receipts"), None));
        assert!(!split(SplitId::Receipts, 0.7).acted);
        assert!(!split(SplitId::Important, 0.99).acted, "already there");
        assert_eq!(decide(SPLIT, None, &e), None);
        let names: Vec<_> = [SplitId::Important, SplitId::Other, SplitId::Calendar, SplitId::Newsletters, SplitId::Receipts]
            .map(split_name)
            .to_vec();
        assert_eq!(names, ["important", "other", "calendar", "newsletters", "receipts"]);
    }

    #[test]
    fn list_mail_goes_where_the_model_says_sure_or_not() {
        let e = Undecided { split: SplitId::Newsletters, ..email("Build failed", "") };
        let split = |s, p| decide(SPLIT, Some(Asked::Split(Some((s, p)), None)), &e).unwrap();
        // Its list headers only held it in Newsletters: the model's pick decides, even unsure.
        assert_eq!(split(SplitId::Other, 0.45), decision(Source::Model, Some(0.45), true, Some("other"), None));
        assert_eq!(split(SplitId::Receipts, 0.6), decision(Source::Model, Some(0.6), true, Some("receipts"), None));
        assert_eq!(split(SplitId::Newsletters, 0.5), decision(Source::Model, Some(0.5), false, Some("newsletters"), None));
        // Important needs your attention, so only when sure; otherwise it's a notice, in Other.
        assert_eq!(split(SplitId::Important, 0.9), decision(Source::Model, Some(0.9), true, Some("important"), None));
        assert_eq!(split(SplitId::Important, 0.6), decision(Source::Model, Some(0.6), true, Some("other"), None));
        // No model: it stays where its headers put it.
        assert_eq!(decide(SPLIT, None, &e), None);
    }

    #[test]
    fn a_strangers_sales_pitch_goes_to_newsletters_only_when_the_model_is_sure() {
        let e = email("Faster product videos", "");
        let asked = |pitch, wants| decide(SPLIT, Some(Asked::Split(Some((SplitId::Important, 0.9)), Some(Pitch::from((pitch, wants))))), &e).unwrap();
        // Sure it is a pitch: marketing, so Newsletters, and the pitch answer says how sure.
        assert_eq!(asked(0.85, 0.3), decision(Source::Model, Some(0.85), true, Some("newsletters"), None));
        // Not sure: the split answer decides, as for anyone.
        assert_eq!(asked(0.7, 0.3), decision(Source::Model, Some(0.9), false, Some("important"), None));
        // Sure, but just as sure the sender needs something of the user's: a would-be customer.
        assert_eq!(asked(0.9, 0.85), decision(Source::Model, Some(0.9), false, Some("important"), None));
        // Not a pitch, and sure it is a receipt: the split answer moves it.
        let receipt = Asked::Split(Some((SplitId::Receipts, 0.9)), Some(Pitch { pitch: 0.1, wants: 0.1 }));
        assert_eq!(decide(SPLIT, Some(receipt), &e).unwrap(), decision(Source::Model, Some(0.9), true, Some("receipts"), None));
    }

    #[test]
    fn mail_in_other_moves_only_when_it_is_a_sales_pitch() {
        let e = Undecided { split: SplitId::Other, ..email("Acme: an invitation to partner", "") };
        let asked = |pitch, wants| decide(SPLIT, Some(Asked::Split(None, Some(Pitch { pitch, wants }))), &e).unwrap();
        assert_eq!(asked(0.9, 0.2), decision(Source::Model, Some(0.9), true, Some("newsletters"), None));
        assert_eq!(asked(0.5, 0.2), decision(Source::Model, Some(0.5), false, Some("other"), None));
        assert_eq!(asked(0.9, 0.9), decision(Source::Model, Some(0.9), false, Some("other"), None));
    }

    #[test]
    fn a_deadline_comes_from_the_model_when_sure_and_from_the_words_otherwise() {
        let words = email("Contract", "Please sign it by 2026-11-13.");
        let rule = decision(Source::Rule, None, true, Some("2026-11-13"), Some("by 2026-11-13"));
        assert_eq!(decide(DEADLINE, None, &words), Some(rule.clone()));
        assert_eq!(decide(DEADLINE, Some(Asked::Yes(0.5)), &words), Some(Decision { yes: Some(0.5), ..rule }));
        assert_eq!(
            decide(DEADLINE, Some(Asked::Yes(0.9)), &words),
            Some(decision(Source::Model, Some(0.9), true, Some("2026-11-13"), None))
        );
        assert_eq!(decide(DEADLINE, Some(Asked::Yes(0.1)), &words), Some(decision(Source::Model, Some(0.1), false, None, None)));
        // Sure, without a deadline word: the first day the email names.
        let named = email("Contract", "We sign on 2026-11-20, all being well.");
        assert_eq!(decide(DEADLINE, Some(Asked::Yes(0.9)), &named).unwrap().value.as_deref(), Some("2026-11-20"));
        // Sure, but no day at all: nothing to show.
        let dayless = email("Contract", "Please sign it soon.");
        assert_eq!(decide(DEADLINE, Some(Asked::Yes(0.9)), &dayless), Some(decision(Source::Model, Some(0.9), false, None, None)));
        assert_eq!(decide(DEADLINE, None, &dayless), Some(decision(Source::Rule, None, false, None, None)));
    }

    #[test]
    fn your_email_waits_for_an_answer_when_the_model_or_the_words_say_so() {
        let asks = email("Lunch?", "Can you do Friday?");
        let until = (asks.sent_at + WAIT).to_string();
        assert_eq!(decide(EXPECTS_REPLY, None, &asks), Some(decision(Source::Rule, None, true, Some(&until), Some("Can you do Friday?"))));
        assert_eq!(decide(EXPECTS_REPLY, Some(Asked::Yes(0.9)), &asks), Some(decision(Source::Model, Some(0.9), true, Some(&until), None)));
        assert_eq!(decide(EXPECTS_REPLY, Some(Asked::Yes(0.1)), &asks), Some(decision(Source::Model, Some(0.1), false, None, None)));
        let thanks = email("Re: Lunch", "Thanks!");
        assert_eq!(decide(EXPECTS_REPLY, Some(Asked::Yes(0.5)), &thanks), Some(decision(Source::Rule, Some(0.5), false, None, None)));
    }
}
