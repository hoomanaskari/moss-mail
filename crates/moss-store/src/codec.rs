//! Moving values in and out of SQLite columns.

use rusqlite::types::Type;
use rusqlite::Row;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// A unit enum as its serde name, e.g. `Section::Reply` → "reply".
pub fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

/// Reads a column written by [`name`] or as JSON text, failing the row if it doesn't parse.
pub fn column<T: DeserializeOwned>(row: &Row, idx: usize, json: bool) -> rusqlite::Result<T> {
    let text: String = row.get(idx)?;
    let parsed = if json {
        serde_json::from_str(&text)
    } else {
        serde_json::from_value(serde_json::Value::String(text))
    };
    parsed.map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(e)))
}

/// A unit enum stored by [`name`].
pub fn named<T: DeserializeOwned>(row: &Row, idx: usize) -> rusqlite::Result<T> {
    column(row, idx, false)
}

/// A value stored as JSON text.
pub fn json<T: DeserializeOwned>(row: &Row, idx: usize) -> rusqlite::Result<T> {
    column(row, idx, true)
}

pub fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Ids are integers in the database and strings in the UI.
pub fn parse_id(id: &str) -> i64 {
    id.parse().unwrap_or(-1)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moss_core::{Person, Section};
    use rusqlite::Connection;

    #[test]
    fn enums_round_trip_by_name_and_values_as_json() {
        let conn = Connection::open_in_memory().unwrap();
        let reply = name(&Section::Reply);
        assert_eq!(reply, "reply");
        let people = vec![Person { name: "Mum".into(), email: "mum@fastmail.com".into() }];
        let (section, back): (Section, Vec<Person>) = conn
            .query_row("SELECT ?1, ?2", [reply, to_json(&people)], |r| {
                Ok((named(r, 0)?, json(r, 1)?))
            })
            .unwrap();
        assert_eq!(section, Section::Reply);
        assert_eq!(back, people);
    }

    #[test]
    fn a_bad_column_fails_the_row() {
        let conn = Connection::open_in_memory().unwrap();
        let err = conn.query_row("SELECT 'sideways'", [], |r| named::<Section>(r, 0)).unwrap_err();
        assert!(matches!(err, rusqlite::Error::FromSqlConversionFailure(0, Type::Text, _)));
        // The same value by name or as JSON text; either fails the row when it doesn't parse.
        let read = |sql: &str, as_json: bool| conn.query_row(sql, [], |r| column::<Section>(r, 0, as_json));
        assert_eq!(read("SELECT 'info'", false).unwrap(), Section::Info);
        assert_eq!(read(r#"SELECT '"info"'"#, true).unwrap(), Section::Info);
        assert!(matches!(read("SELECT 'info'", true), Err(rusqlite::Error::FromSqlConversionFailure(0, Type::Text, _))));
    }

    #[test]
    fn ids_that_are_not_numbers_match_nothing() {
        assert_eq!(parse_id("42"), 42);
        assert_eq!(parse_id("work-1"), -1);
    }

    #[test]
    fn the_clock_is_in_milliseconds() {
        assert!(now_ms() > 1_700_000_000_000);
    }
}
