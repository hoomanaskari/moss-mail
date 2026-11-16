//! Key details found by simple rules when a message arrives: amounts, dates, tracking and
//! order numbers. Kept as short values, so they're there to search and sort by after the
//! text itself is compressed or gone.

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{params, Transaction};
use serde::Serialize;

use crate::Result;

/// What kind of detail a value is, as the `details.kind` column stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Amount,
    Date,
    Tracking,
    Order,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Amount => "amount",
            Kind::Date => "date",
            Kind::Tracking => "tracking",
            Kind::Order => "order",
        }
    }
}

/// Each message keeps at most this many of each kind: a long statement lists hundreds.
const MAX_EACH: usize = 5;

static RULES: LazyLock<Vec<(Kind, Regex)>> = LazyLock::new(|| {
    let months = "jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?";
    let rules = [
        (Kind::Amount, r"(?:[$€£¥]\s?\d[\d,]*(?:\.\d{1,2})?)|(?:\b\d[\d,]*(?:\.\d{1,2})?\s?(?:USD|EUR|GBP|CAD|AUD|CHF|JPY)\b)".to_string()),
        (Kind::Date, format!(r"(?i)\b(?:\d{{4}}-\d{{2}}-\d{{2}}|\d{{1,2}} (?:{months})\b(?: \d{{4}})?|(?:{months}) \d{{1,2}}(?:st|nd|rd|th)?\b(?:,? \d{{4}})?)")),
        // UPS, then a number after the word "tracking".
        (Kind::Tracking, r"\b1Z[0-9A-Z]{16}\b|(?i:tracking(?: number| no\.?| #)?:?\s*)([A-Z0-9]{10,30})\b".to_string()),
        (Kind::Order, r"(?i:order(?: number| no\.?| #| id)?:?\s*#?)([A-Z0-9][A-Z0-9-]{3,24})\b".to_string()),
    ];
    rules.into_iter().map(|(k, r)| (k, Regex::new(&r).expect("detail rules compile"))).collect()
});

/// The details in a subject and text, by kind, each at most [`MAX_EACH`] times.
pub fn extract(subject: &str, text: &str) -> Vec<(Kind, String)> {
    let all = format!("{subject}\n{text}");
    let mut found: Vec<(Kind, String)> = Vec::new();
    for (kind, rule) in RULES.iter() {
        let mut values: Vec<String> = Vec::new();
        for caps in rule.captures_iter(&all) {
            let value = caps.get(1).unwrap_or_else(|| caps.get(0).expect("a match has group 0"));
            let value = value.as_str().trim().to_string();
            // Order "numbers" have digits; "Order confirmed" doesn't count.
            let is_number = value.chars().any(|c| c.is_ascii_digit());
            if is_number && !values.contains(&value) && values.len() < MAX_EACH {
                values.push(value);
            }
        }
        found.extend(values.into_iter().map(|v| (*kind, v)));
    }
    found
}

/// Stores the details of message `id`, replacing any it had.
pub(crate) fn find(tx: &Transaction, id: i64, subject: &str, text: &str) -> Result<()> {
    tx.execute("DELETE FROM details WHERE message_id = ?1", [id])?;
    for (kind, value) in extract(subject, text) {
        tx.execute(
            "INSERT OR IGNORE INTO details (message_id, kind, value) VALUES (?1, ?2, ?3)",
            params![id, kind.name(), value],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(kind: Kind, subject: &str, text: &str) -> Vec<String> {
        extract(subject, text).into_iter().filter(|(k, _)| *k == kind).map(|(_, v)| v).collect()
    }

    #[test]
    fn finds_amounts_in_symbols_and_codes() {
        let text = "Total: $1,240.50 (was €99). Refund of 12.00 GBP, and 30 USD later. 3 items.";
        assert_eq!(of(Kind::Amount, "", text), ["$1,240.50", "€99", "12.00 GBP", "30 USD"]);
    }

    #[test]
    fn finds_dates_written_in_the_usual_ways() {
        let text = "Due 2026-11-13. Flight on 13 Nov, back November 20th, 2026. Call May 4 2027.";
        assert_eq!(of(Kind::Date, "", text), ["2026-11-13", "13 Nov", "November 20th, 2026", "May 4 2027"]);
        assert!(of(Kind::Date, "", "No dates in here, maybe.").is_empty());
    }

    #[test]
    fn finds_tracking_and_order_numbers_but_not_words() {
        let text = "Your order #A12-3456 shipped. UPS 1Z999AA10123456784. Tracking number: 9400111899562537866371";
        assert_eq!(of(Kind::Order, "Order confirmed", text), ["A12-3456"]);
        assert_eq!(of(Kind::Tracking, "", text), ["1Z999AA10123456784", "9400111899562537866371"]);
    }

    #[test]
    fn keeps_a_few_of_each() {
        let text = (1..=20).map(|n| format!("${n}")).collect::<Vec<_>>().join(" ");
        assert_eq!(of(Kind::Amount, "", &text).len(), MAX_EACH);
        assert_eq!(of(Kind::Amount, "$5 and $5", "").len(), 1);
    }

    #[test]
    fn kinds_have_column_names() {
        let names: Vec<&str> = [Kind::Amount, Kind::Date, Kind::Tracking, Kind::Order].map(Kind::name).to_vec();
        assert_eq!(names, ["amount", "date", "tracking", "order"]);
    }
}
