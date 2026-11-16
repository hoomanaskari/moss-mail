//! The plain rules Moss decides with while the decision model isn't downloaded, and that
//! find the day once the model says an email sets a deadline. Free, instant, and in the
//! email's own words, so the UI can say why: "says “by Friday”".

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Days, FixedOffset, Months, NaiveDate};
use regex::{Captures, Regex};

const MONTHS: &str = "jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?";
const WEEKDAYS: &str = "mon(?:day)?|tue(?:s(?:day)?)?|wed(?:nesday)?|thu(?:r(?:s(?:day)?)?)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?";

/// A day as people write it: 2026-11-13, 13 Nov, Nov 13th, 2026, tomorrow, end of the week,
/// (next) Friday.
fn day_pattern() -> String {
    [
        r"(?:(?P<iso>\d{4}-\d{2}-\d{2})\b".to_string(),
        format!(r"(?:(?:{WEEKDAYS}),?\s+)?(?P<dm_d>\d{{1,2}})(?:st|nd|rd|th)?\s+(?P<dm_m>{MONTHS})\b(?:,?\s+(?P<dm_y>\d{{4}}))?"),
        format!(r"(?:(?:{WEEKDAYS}),?\s+)?(?P<md_m>{MONTHS})\s+(?P<md_d>\d{{1,2}})(?:st|nd|rd|th)?\b(?:,?\s+(?P<md_y>\d{{4}}))?"),
        r"(?P<rel>today|tonight|tomorrow|eod|eow|eom|end\s+of\s+(?:the\s+)?(?:day|week|month))\b".to_string(),
        format!(r"(?P<next>next\s+)?(?P<wd>{WEEKDAYS})\b)"),
    ]
    .join("|")
}

/// A deadline word, then a day: "by Friday", "due 13 Nov", "no later than tomorrow".
static DEADLINE: LazyLock<Regex> = LazyLock::new(|| {
    let cue = r"by|before|due(?:\s+(?:on|by))?|deadline(?:\s+is)?:?|no\s+later\s+than";
    Regex::new(&format!(r"(?i)\b(?:{cue})\s+(?:the\s+)?{}", day_pattern())).expect("the deadline rule compiles")
});

/// Any day.
static DAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\b{}", day_pattern())).expect("the day rule compiles"));

/// The month a name starts like, 1 to 12.
fn month(name: &str) -> Option<u32> {
    let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let start = name.get(..3)?.to_lowercase();
    names.iter().position(|m| *m == start).map(|i| i as u32 + 1)
}

/// Days from Monday of the weekday a name starts like.
fn weekday(name: &str) -> u32 {
    let names = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let start = name.get(..3).unwrap_or_default().to_lowercase();
    names.iter().position(|d| *d == start).unwrap_or_default() as u32
}

/// The day of the week `target` (from Monday) on or after `from`.
fn coming(from: NaiveDate, target: u32) -> NaiveDate {
    let days = (target + 7 - from.weekday().num_days_from_monday()) % 7;
    from + Days::new(days as u64)
}

/// A day and month (and maybe year) as a date: without a year, the next one to come.
fn dated(day: &str, month_name: &str, year: Option<&str>, sent: NaiveDate) -> Option<NaiveDate> {
    let (day, month) = (day.parse().ok()?, month(month_name)?);
    match year {
        Some(year) => NaiveDate::from_ymd_opt(year.parse().ok()?, month, day),
        None => {
            let this = NaiveDate::from_ymd_opt(sent.year(), month, day)?;
            if this < sent { NaiveDate::from_ymd_opt(sent.year() + 1, month, day) } else { Some(this) }
        }
    }
}

/// The day a match names, counting from the day the email was sent.
fn resolve(caps: &Captures, sent: NaiveDate) -> Option<NaiveDate> {
    let text = |name| caps.name(name).map(|m| m.as_str());
    if let Some(iso) = text("iso") {
        return NaiveDate::parse_from_str(iso, "%Y-%m-%d").ok();
    }
    if let (Some(d), Some(m)) = (text("dm_d"), text("dm_m")) {
        return dated(d, m, text("dm_y"), sent);
    }
    if let (Some(m), Some(d)) = (text("md_m"), text("md_d")) {
        return dated(d, m, text("md_y"), sent);
    }
    if let Some(rel) = text("rel") {
        let rel = rel.to_lowercase();
        return Some(if rel == "tomorrow" {
            sent + Days::new(1)
        } else if rel == "eow" || rel.ends_with("week") {
            coming(sent, 4)
        } else if rel == "eom" || rel.ends_with("month") {
            let first = NaiveDate::from_ymd_opt(sent.year(), sent.month(), 1)?;
            first + Months::new(1) - Days::new(1)
        } else {
            sent
        });
    }
    let day = coming(sent, weekday(text("wd")?));
    // "Next Monday", said on a Monday, is a week away.
    Some(if text("next").is_some() && day == sent { day + Days::new(7) } else { day })
}

/// The first match of `rule` in the subject or text that names a real day.
fn find(rule: &Regex, subject: &str, text: &str, sent: DateTime<FixedOffset>) -> Option<(NaiveDate, String)> {
    let all = format!("{subject}\n{text}");
    let sent = sent.date_naive();
    rule.captures_iter(&all).find_map(|caps| Some((resolve(&caps, sent)?, caps[0].trim().to_string())))
}

/// A deadline in an email: the day, and the words that set it.
pub fn deadline(subject: &str, text: &str, sent: DateTime<FixedOffset>) -> Option<(NaiveDate, String)> {
    find(&DEADLINE, subject, text, sent)
}

/// The first day an email names, with or without a deadline word: once the decision model
/// says it sets a deadline, this is most likely its day.
pub fn first_day(subject: &str, text: &str, sent: DateTime<FixedOffset>) -> Option<(NaiveDate, String)> {
    find(&DAY, subject, text, sent)
}

/// Words that ask someone for an answer.
const ASKS: &[&str] = &[
    "let me know",
    "please confirm",
    "please reply",
    "please send",
    "please let me",
    "please advise",
    "can you",
    "could you",
    "would you",
    "get back to me",
    "what do you think",
    "your thoughts",
    "looking forward to hearing",
];
/// Questions nobody needs to answer.
const SMALL_TALK: &[&str] = &["how are you", "hope you", "how's it going", "how is it going", "how have you been"];
/// The longest a reason quotes an email.
const QUOTE: usize = 80;

static SENTENCES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^.!?\n]+[.!?]*").expect("sentences compile"));

/// The words in your email that ask for an answer, if any: a question or a request, but
/// not small talk.
pub fn expects_reply(text: &str) -> Option<String> {
    let asks = SENTENCES.find_iter(text).map(|m| m.as_str().trim()).find(|sentence| {
        let lower = sentence.to_lowercase().replace('’', "'");
        !SMALL_TALK.iter().any(|w| lower.contains(w)) && (sentence.ends_with('?') || ASKS.iter().any(|w| lower.contains(w)))
    })?;
    Some(if asks.chars().count() > QUOTE {
        let cut: String = asks.chars().take(QUOTE - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        asks.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Monday 2 November 2026, 10:00 in London (UTC+0).
    fn monday() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0).unwrap().with_ymd_and_hms(2026, 11, 2, 10, 0, 0).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn due(text: &str) -> Option<(NaiveDate, String)> {
        deadline("", text, monday())
    }

    #[test]
    fn finds_weekdays_and_relative_days_after_a_deadline_word() {
        assert_eq!(due("Can you send it by Friday?"), Some((day(2026, 11, 6), "by Friday".into())));
        assert_eq!(due("Due Wed, please."), Some((day(2026, 11, 4), "Due Wed".into())));
        assert_eq!(due("We need it before tomorrow."), Some((day(2026, 11, 3), "before tomorrow".into())));
        assert_eq!(due("Sign it by today"), Some((day(2026, 11, 2), "by today".into())));
        assert_eq!(due("Send it by EOD"), Some((day(2026, 11, 2), "by EOD".into())));
        assert_eq!(due("by end of the week"), Some((day(2026, 11, 6), "by end of the week".into())));
        assert_eq!(due("no later than end of month"), Some((day(2026, 11, 30), "no later than end of month".into())));
        // Today is Monday: "by Monday" is today, "by next Monday" a week on.
        assert_eq!(due("by Monday").unwrap().0, day(2026, 11, 2));
        assert_eq!(due("by next Monday").unwrap().0, day(2026, 11, 9));
        assert_eq!(due("deadline: Thursday").unwrap().0, day(2026, 11, 5));
    }

    #[test]
    fn finds_written_dates_and_rolls_past_ones_into_next_year() {
        assert_eq!(due("Pay by 2026-11-13."), Some((day(2026, 11, 13), "by 2026-11-13".into())));
        assert_eq!(due("due on 13 November").unwrap().0, day(2026, 11, 13));
        assert_eq!(due("due by Nov 20th, 2027").unwrap().0, day(2027, 11, 20));
        assert_eq!(due("before Friday, 13 Nov").unwrap().0, day(2026, 11, 13));
        assert_eq!(due("by January 5").unwrap().0, day(2027, 1, 5));
        assert_eq!(deadline("Invoice due 30 Nov", "Thanks", monday()).unwrap().0, day(2026, 11, 30));
    }

    #[test]
    fn needs_a_deadline_word_and_a_real_day() {
        assert_eq!(due("See you Friday!"), None);
        assert_eq!(due("Sent by email, by the way."), None);
        assert_eq!(due("by 31 February"), None);
        assert_eq!(due("by 2026-13-40"), None);
        assert_eq!(due(""), None);
    }

    #[test]
    fn the_first_day_named_needs_no_deadline_word() {
        assert_eq!(first_day("", "Signing on Friday, all being well.", monday()), Some((day(2026, 11, 6), "Friday".into())));
        assert_eq!(first_day("Contract 2026-11-20", "", monday()).unwrap().0, day(2026, 11, 20));
        assert_eq!(first_day("", "Soon, all being well.", monday()), None);
    }

    #[test]
    fn end_of_the_week_on_a_weekend_is_next_friday() {
        let saturday = FixedOffset::east_opt(0).unwrap().with_ymd_and_hms(2026, 11, 7, 10, 0, 0).unwrap();
        assert_eq!(deadline("", "by end of week", saturday).unwrap().0, day(2026, 11, 13));
        let december = FixedOffset::east_opt(0).unwrap().with_ymd_and_hms(2026, 12, 2, 10, 0, 0).unwrap();
        assert_eq!(deadline("", "by end of month", december).unwrap().0, day(2026, 12, 31));
    }

    #[test]
    fn a_question_or_a_request_expects_an_answer() {
        assert_eq!(expects_reply("Hi Dana,\n\nCan you send the deck?\n\nMaya").as_deref(), Some("Can you send the deck?"));
        assert_eq!(expects_reply("Here it is. Let me know what you think.").as_deref(), Some("Let me know what you think."));
        assert_eq!(expects_reply("Please confirm the time").as_deref(), Some("Please confirm the time"));
        let long = format!("Could you {}?", "really ".repeat(30));
        let said = expects_reply(&long).unwrap();
        assert!(said.ends_with('…') && said.chars().count() <= 80, "{said}");
    }

    #[test]
    fn thanks_and_small_talk_expect_nothing() {
        assert_eq!(expects_reply("Thanks, got it!"), None);
        assert_eq!(expects_reply("Hi Dana, how are you? Here's the deck."), None);
        assert_eq!(expects_reply("Hope you're well? Attached."), None);
        assert_eq!(expects_reply(""), None);
    }
}
