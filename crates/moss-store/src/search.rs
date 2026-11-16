//! Keyword search over the new text of messages (quotes and signatures stripped), their
//! subjects, the people on them and the text inside their files. One hit per thread, best
//! first.

use moss_core::{MatchedFile, SearchHit, SnippetPart};
use rusqlite::named_params;

use crate::{text, Result, Store};

/// Characters of text shown around the first match.
const SNIPPET: usize = 120;

/// How much a match inside a file counts, against one in the email's own text: a little
/// less, as a long file matches more words by chance.
const FILE_WEIGHT: f64 = 0.8;

/// A message that matched: where, and how well (lower is better).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Found {
    pub message_id: i64,
    pub thread_id: i64,
    pub rank: f64,
    /// The file whose text matched, or `None` for the email itself.
    pub file: Option<u32>,
}

/// The words someone typed, lower-case, for marking matches.
fn words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Adds plain text to the end of a snippet.
fn plain(parts: &mut Vec<SnippetPart>, text: &str) {
    match parts.last_mut() {
        Some(last) if !last.matched => last.text.push_str(text),
        _ => parts.push(SnippetPart { text: text.into(), matched: false }),
    }
}

/// A snippet of `text` from a few words before the first word that starts with one of
/// `words`, split into plain and matched parts. The index keeps no text, so this reads the
/// body or the preview.
pub(crate) fn snippet_parts(text: &str, words: &[String]) -> Vec<SnippetPart> {
    let all: Vec<&str> = text.split_whitespace().collect();
    let hit = |w: &str| {
        let bare = w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        !bare.is_empty() && words.iter().any(|q| bare.starts_with(q.as_str()))
    };
    let start = all.iter().position(|w| hit(w)).unwrap_or(0).saturating_sub(4);
    let mut parts = Vec::new();
    if start > 0 {
        plain(&mut parts, "…");
    }
    let mut length = 0;
    for (i, w) in all.iter().enumerate().skip(start) {
        if length + w.len() > SNIPPET {
            plain(&mut parts, " …");
            break;
        }
        if i > 0 {
            plain(&mut parts, " ");
        }
        if hit(w) {
            parts.push(SnippetPart { text: (*w).into(), matched: true });
        } else {
            plain(&mut parts, w);
        }
        length += w.len() + 1;
    }
    parts
}

impl Store {
    /// Threads matching every word of `query` (the last word as a prefix), in an account
    /// (`None` for all), best match first. Subject matches count most, then people, then body,
    /// then the text inside a file. Done threads are included.
    pub fn search(
        &self,
        query: &str,
        account: Option<&str>,
        limit: usize,
        now: i64,
    ) -> Result<Vec<SearchHit>> {
        let Some(q) = text::fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "WITH hits AS (
                 SELECT m.id AS message_id, m.thread_id, bm25(message_search, 5.0, 3.0, 1.0) AS rank, NULL AS file
                 FROM message_search JOIN messages m ON m.id = message_search.rowid
                 WHERE message_search MATCH :q AND (:account IS NULL OR m.account_id = :account)
                     AND m.deleted_from IS NULL
                 UNION ALL
                 SELECT m.id, m.thread_id, bm25(attachment_search) * :file_weight, t.position
                 FROM attachment_search JOIN attachment_text t ON t.id = attachment_search.rowid
                     JOIN messages m ON m.id = t.message_id
                 WHERE attachment_search MATCH :q AND (:account IS NULL OR m.account_id = :account)
                     AND m.deleted_from IS NULL
             )
             SELECT message_id, thread_id, rank, file FROM (
                 SELECT *, row_number() OVER (PARTITION BY thread_id ORDER BY rank, message_id DESC, file) AS n FROM hits
             ) WHERE n = 1 ORDER BY rank, message_id DESC LIMIT :limit",
        )?;
        let params = named_params! {":q": q, ":account": account, ":limit": limit as i64, ":file_weight": FILE_WEIGHT};
        let rows = stmt.query_map(params, |r| {
            Ok(Found { message_id: r.get(0)?, thread_id: r.get(1)?, rank: r.get(2)?, file: r.get(3)? })
        })?;
        let found: Vec<Found> = rows.collect::<rusqlite::Result<_>>()?;
        self.hits(&found, &words(query), now)
    }

    /// Search hits for messages found: each thread's summary, and a snippet of where it
    /// matched: the file's text, or the message's body, or its preview when the body isn't
    /// here.
    pub(crate) fn hits(&self, found: &[Found], words: &[String], now: i64) -> Result<Vec<SearchHit>> {
        let mut hits = Vec::new();
        let mut accounts = crate::query::Accounts::default();
        for f in found {
            let Some(thread) = self.thread_summary_in(f.thread_id, now, &mut accounts)? else {
                continue;
            };
            let id = f.message_id.to_string();
            let in_file = match f.file {
                Some(index) => self.attachment_name(&id, index)?.map(|name| (MatchedFile { index, name }, self.attachment_text(&id, index))),
                None => None,
            };
            let (file, text) = match in_file {
                Some((name, text)) => (Some(name), text?.unwrap_or_default()),
                None => {
                    let text = match self.read_body(f.message_id)? {
                        Some(body) => text::strip_quoted(&body.text),
                        None => self.conn.query_row("SELECT snippet FROM messages WHERE id = ?1", [f.message_id], |r| r.get(0))?,
                    };
                    (None, text)
                }
            };
            hits.push(SearchHit { thread, message_id: id, snippet: snippet_parts(&text, words), rank: f.rank, file });
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::MessageSource;

    fn add(store: &mut Store, id: &str, mail: Mail) -> String {
        ingest(store, id, &mail.id(&format!("{id}@x")).build()).thread_id
    }

    #[test]
    fn keyword_previews_do_not_refresh_body_cache_recency() {
        let mut store = store_with_work();
        let thread = add(&mut store, "cache", mail().subject("Budget").body("budget preview"));
        store.conn.execute("UPDATE bodies SET used_at=7", []).unwrap();
        let hits = store.search("budget", None, 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread.id, thread);
        assert!(hits[0].snippet.iter().any(|part| part.text.contains("preview")));
        let used: i64 = store.conn.query_row("SELECT used_at FROM bodies", [], |row| row.get(0)).unwrap();
        assert_eq!(used, 7, "search previews are not opened messages");
        assert!(store.body(hits[0].message_id.parse().unwrap()).unwrap().is_some());
        let opened: i64 = store.conn.query_row("SELECT used_at FROM bodies", [], |row| row.get(0)).unwrap();
        assert!(opened > used, "opening a body must still refresh recency");
    }

    #[test]
    fn a_hit_whose_thread_is_gone_is_left_out() {
        let mut store = store_with_work();
        let thread: i64 = add(&mut store, "a", mail().subject("Budget")).parse().unwrap();
        let message = store.search("budget", None, 10, NOV_1).unwrap()[0].message_id.parse().unwrap();
        let found = |thread_id| Found { message_id: message, thread_id, rank: 0.0, file: None };
        let hits = store.hits(&[found(999), found(thread)], &[], NOV_1).unwrap();
        assert_eq!(hits.iter().map(|h| h.thread.id.clone()).collect::<Vec<_>>(), [thread.to_string()]);
    }

    #[test]
    fn subject_matches_rank_above_body_matches() {
        let mut store = store_with_work();
        let body = add(&mut store, "a", mail().subject("Numbers").body("the budget is attached"));
        let subject = add(&mut store, "b", mail().subject("Budget review").body("see you then"));
        let hits = store.search("budget", None, 10, NOV_1).unwrap();
        let threads: Vec<&str> = hits.iter().map(|h| h.thread.id.as_str()).collect();
        assert_eq!(threads, [subject.as_str(), &body]);
        assert!(hits[0].rank <= hits[1].rank);
        assert!(hits[1].snippet.iter().any(|p| p.matched && p.text == "budget"));
    }

    #[test]
    fn one_hit_per_thread_with_its_best_message() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail().subject("Lunch").body("pineapple pizza?"));
        let reply = mail()
            .id("r@x")
            .subject("Re: Lunch")
            .reply_to(&["a@x"])
            .body("pineapple pineapple pineapple!");
        let best = ingest(&mut store, "r", &reply.build()).message_id;
        let hits = store.search("pineapple", None, 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread.id, thread);
        assert_eq!(hits[0].message_id, best);
    }

    #[test]
    fn finds_people_prefixes_and_words_without_accents() {
        let mut store = store_with_work();
        add(
            &mut store,
            "a",
            mail().subject("Café au lait").from("Ines Duarte <ines@duarte-studio.pt>"),
        );
        assert_eq!(store.search("ines", None, 10, NOV_1).unwrap().len(), 1);
        assert_eq!(store.search("duarte-studio", None, 10, NOV_1).unwrap().len(), 1);
        assert_eq!(store.search("caf", None, 10, NOV_1).unwrap().len(), 1);
        assert_eq!(store.search("cafe au", None, 10, NOV_1).unwrap().len(), 1);
        assert!(store.search("cafe tea", None, 10, NOV_1).unwrap().is_empty());
        assert!(store.search("  ", None, 10, NOV_1).unwrap().is_empty());
    }

    #[test]
    fn searches_one_account_and_includes_done_threads() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya.ross@fastmail.com")).unwrap();
        let work = add(&mut store, "a", mail());
        store.mark_done(&work, true, NOV_1).unwrap();
        store
            .ingest(
                &mail().build(),
                &MessageSource { account_id: "personal".into(), ..source("p") },
            )
            .unwrap();
        assert_eq!(store.search("budget", None, 10, NOV_1).unwrap().len(), 2);
        let hits = store.search("budget", Some("work"), 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].thread.done);
        assert_eq!(store.search("budget", None, 1, NOV_1).unwrap().len(), 1);
    }

    #[test]
    fn leaves_out_deleted_threads() {
        let mut store = store_with_work();
        let thread = add(&mut store, "a", mail());
        store.delete_thread(&thread).unwrap();
        assert!(store.search("budget", None, 10, NOV_1).unwrap().is_empty());
        store.restore_thread(&thread).unwrap();
        assert_eq!(store.search("budget", None, 10, NOV_1).unwrap().len(), 1);
    }

    fn flat(parts: &[SnippetPart]) -> Vec<(&str, bool)> {
        parts.iter().map(|p| (p.text.as_str(), p.matched)).collect()
    }

    #[test]
    fn snippets_split_into_matched_and_plain_parts() {
        let q = words("q4 BUDG");
        assert_eq!(q, ["q4", "budg"]);
        assert_eq!(
            flat(&snippet_parts("the Q4 budget, signed.", &q)),
            [("the ", false), ("Q4", true), (" ", false), ("budget,", true), (" signed.", false)]
        );
        assert_eq!(flat(&snippet_parts("budget", &q)), [("budget", true)]);
        assert!(snippet_parts("", &q).is_empty());
        assert!(words("?! ").is_empty());
    }

    #[test]
    fn snippets_start_near_the_first_match_and_end_when_full() {
        let text = format!("one two three four five six budget {}", "word ".repeat(40));
        let parts = snippet_parts(&text, &words("budget"));
        assert_eq!(parts[0].text, "… three four five six ");
        assert_eq!(parts[1].text, "budget");
        assert!(parts.last().unwrap().text.ends_with(" …"));
        // No match: from the start.
        assert_eq!(snippet_parts("hello there", &words("zebra"))[0].text, "hello there");
    }

    #[test]
    fn snippets_come_from_the_body_or_the_preview_when_the_body_isnt_here() {
        let mut store = store_with_work();
        let long = format!("{} the budget is attached", "filler ".repeat(30));
        add(&mut store, "a", mail().body(&long));
        let hit = &store.search("budget", None, 10, NOV_1).unwrap()[0];
        assert!(hit.snippet.iter().any(|p| p.matched && p.text == "budget"));
        store.conn.execute_batch("DELETE FROM bodies").unwrap();
        let hit = &store.search("budget", None, 10, NOV_1).unwrap()[0];
        assert!(hit.snippet.iter().all(|p| !p.matched), "the preview stops before the match");
        assert!(hit.snippet[0].text.starts_with("filler filler"));
    }

    fn letter(store: &mut Store) -> crate::Ingested {
        let raw = mail()
            .id("permit@x")
            .subject("Letter from the council")
            .body("Please see the attached letter.")
            .attach("P2026-0457.pdf", "application/pdf", "%PDF")
            .build();
        ingest(store, "1:9", &raw)
    }

    #[test]
    fn finds_words_inside_files_and_says_which_file() {
        let mut store = store_with_work();
        let letter = letter(&mut store);
        assert!(store.search("permit", None, 10, NOV_1).unwrap().is_empty(), "not read yet");
        let text = "Notice. Your resident parking permit starts on December 12, 2026 at 10:00.";
        store.keep_attachment_text(&letter.message_id, 0, text).unwrap();
        let hits = store.search("parking perm", None, 10, NOV_1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].thread.id.as_str(), hits[0].message_id.as_str()), (letter.thread_id.as_str(), letter.message_id.as_str()));
        assert_eq!(hits[0].file, Some(MatchedFile { index: 0, name: "P2026-0457.pdf".into() }));
        assert!(hits[0].snippet.iter().any(|p| p.matched && p.text == "parking"), "the snippet is from the file");
        // A match in the email itself names no file.
        let hits = store.search("attached letter", None, 10, NOV_1).unwrap();
        assert_eq!(hits[0].file, None);
        // Every word has to be in the email or in one file.
        assert!(store.search("attached permit", None, 10, NOV_1).unwrap().is_empty());
        assert!(store.search("permit", Some("home"), 10, NOV_1).unwrap().is_empty());
        store.delete_thread(&letter.thread_id).unwrap();
        assert!(store.search("permit", None, 10, NOV_1).unwrap().is_empty());
    }

    #[test]
    fn a_match_in_the_email_ranks_above_the_same_match_in_a_file() {
        let mut store = store_with_work();
        let letter = letter(&mut store);
        store.keep_attachment_text(&letter.message_id, 0, "the venue is the harbour hall").unwrap();
        let email = add(&mut store, "b", mail().subject("Numbers").body("the venue is the harbour hall"));
        let hits = store.search("harbour", None, 10, NOV_1).unwrap();
        let threads: Vec<&str> = hits.iter().map(|h| h.thread.id.as_str()).collect();
        assert_eq!(threads, [email.as_str(), letter.thread_id.as_str()]);
        // In one thread, the better match is the one shown.
        store.keep_attachment_text(&letter.message_id, 0, "Please see").unwrap();
        let hits = store.search("please", None, 10, NOV_1).unwrap();
        assert_eq!((hits.len(), hits[0].file.as_ref().map(|f| f.name.as_str())), (1, None));
    }

    #[test]
    fn a_file_hit_whose_file_is_gone_shows_the_email() {
        let mut store = store_with_work();
        let letter = letter(&mut store);
        let message = letter.message_id.parse().unwrap();
        let thread = letter.thread_id.parse().unwrap();
        let gone = Found { message_id: message, thread_id: thread, rank: 0.0, file: Some(7) };
        let hits = store.hits(&[gone], &words("letter"), NOV_1).unwrap();
        assert_eq!(hits[0].file, None);
        assert!(hits[0].snippet.iter().any(|p| p.matched && p.text == "letter."));
        // Its text can't be read: an empty snippet, still in the file.
        store.keep_attachment_text(&letter.message_id, 0, "x").unwrap();
        store.conn.execute("UPDATE attachment_text SET method = 'brotli'", []).unwrap();
        let hits = store.hits(&[Found { file: Some(0), ..gone }], &[], NOV_1).unwrap();
        assert_eq!((hits[0].file.as_ref().map(|f| f.name.as_str()), hits[0].snippet.len()), (Some("P2026-0457.pdf"), 0));
        break_schema(&store, "DROP TABLE attachment_text");
        assert!(store.hits(&[Found { file: Some(0), ..gone }], &[], NOV_1).is_err());
    }

    #[test]
    fn a_missing_index_is_an_error() {
        let store = store_with_work();
        break_schema(&store, "DROP TABLE message_search");
        assert!(store.search("budget", None, 10, NOV_1).is_err());
    }
}
