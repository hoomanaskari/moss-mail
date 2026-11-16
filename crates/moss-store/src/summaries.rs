//! What the writing model reads of a conversation (all of it, every message in full, with the
//! clutter of newsletters left out), and the one-liners it writes by itself for the
//! conversations that matter: open ones in Important, and ones you've written in. Each says
//! which message it read up to, so a new message gets a new summary.

use std::sync::LazyLock;

use moss_core::{Person, SummaryWhy};
use regex::Regex;
use rusqlite::params;

use crate::{text, Result, Store};

/// A conversation to summarise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToSummarize {
    pub thread_id: i64,
    pub account_id: String,
    pub subject: String,
    /// The newest message, which the summary will be up to.
    pub through: i64,
    /// Oldest first: who wrote, when (ms), and their new text (or the preview when the body
    /// isn't here).
    pub messages: Vec<(Person, i64, String)>,
    /// The account's address: who "you" are in it.
    pub you: String,
    /// Why the automatic one-liner is wanted: you've written in it, or it's in Important.
    /// Summaries you ask for, or a rule writes, say so themselves.
    pub why: SummaryWhy,
}

static IMAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*((View image|Follow image link):\s*\(.*\)|Caption:)\s*$").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\((https?://[^)\s]+)\)").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s)?#]+[^\s)]*").unwrap());
static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

/// A link without its tracking: no query or fragment.
fn bare(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// An email's text as the writing model reads it: without the clutter newsletters' plain text
/// carries (image placeholders, tracking in every link, divider lines), so its words go to
/// what the email says. Nothing it says is left out.
pub(crate) fn readable(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !IMAGE.is_match(l)).collect();
    let text = lines.join("\n");
    let text = LINK.replace_all(&text, |c: &regex::Captures| match &c[1] {
        "" => String::new(),
        words => format!("{words} ({})", bare(&c[2])),
    });
    let text = URL.replace_all(&text, |c: &regex::Captures| bare(&c[0]).to_string());
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| l.is_empty() || l.chars().any(char::is_alphanumeric)).collect();
    BLANKS.replace_all(&lines.join("\n"), "\n\n").trim().to_string()
}

/// SQL: conversations that matter whose summary from model `?1` is missing or older than
/// their newest message, newest first, up to `?2`.
const TO_SUMMARIZE: &str = "SELECT t.id, lm.id, EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND from_me = 1) FROM threads t
             JOIN messages lm ON lm.id = (SELECT id FROM messages WHERE thread_id = t.id ORDER BY sent_at DESC, id DESC LIMIT 1)
             WHERE (SELECT count(*) FROM messages WHERE thread_id = t.id) > 1
               AND EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND done = 0)
               AND NOT EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND blocked = 1)
               AND (lm.split = 'important' OR EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND from_me = 1))
               AND NOT EXISTS (SELECT 1 FROM summaries s WHERE s.thread_id = t.id AND s.model = ?1 AND s.through >= lm.id)
               AND NOT EXISTS (SELECT 1 FROM discarded_summaries d WHERE d.thread_id = t.id AND d.through >= lm.id)
             ORDER BY t.last_message_at DESC LIMIT ?2";

impl Store {
    /// Up to `limit` conversations that matter whose summary from `model` is missing or older
    /// than their newest message, newest first; not one whose summary you discarded since. A conversation of one message has its
    /// preview, so it needs none.
    pub fn to_summarize(&self, model: &str, limit: usize) -> Result<Vec<ToSummarize>> {
        if !self.automatic_summaries()? {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(TO_SUMMARIZE)?;
        let rows = stmt.query_map(params![model, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let found: Vec<(i64, i64, bool)> = rows.collect::<rusqlite::Result<_>>()?;
        let mut list = Vec::new();
        for (thread_id, through, replied) in found {
            let why = if replied { SummaryWhy::Replied } else { SummaryWhy::Important };
            list.push(self.conversation(thread_id, through, why)?);
        }
        Ok(list)
    }

    /// What the model reads of a conversation: every message, oldest first, without quoted
    /// replies or newsletter clutter.
    pub(crate) fn conversation(&self, thread_id: i64, through: i64, why: SummaryWhy) -> Result<ToSummarize> {
        let (subject, account_id, you): (String, String, String) = self.conn.query_row(
            "SELECT t.subject, t.account_id, a.address FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?1",
            [thread_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, from_name, from_email, snippet, sent_at FROM messages WHERE thread_id = ?1
             ORDER BY sent_at, id",
        )?;
        let rows = stmt.query_map([thread_id], |r| {
            Ok((r.get::<_, i64>(0)?, Person { name: r.get(1)?, email: r.get(2)? }, r.get::<_, String>(3)?, r.get(4)?))
        })?;
        let all: Vec<(i64, Person, String, i64)> = rows.collect::<rusqlite::Result<_>>()?;
        let mut messages = Vec::new();
        for (id, from, preview, at) in all {
            let text = match self.body(id)? {
                Some(body) => readable(&text::strip_quoted(&body.text)),
                None => preview,
            };
            messages.push((from, at, text));
        }
        Ok(ToSummarize { thread_id, account_id, subject, through, messages, you, why })
    }

    /// Saves a conversation's one-liner, written by `model` from its messages up to `through`.
    pub fn set_summary(&self, c: &ToSummarize, text: &str, model: &str, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO summaries (thread_id, text, model, through, made_at, why) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![c.thread_id, text, model, c.through, now, c.why.name()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::*;
    use crate::MessageSource;
    use moss_core::SummaryWhy;

    #[test]
    fn the_summary_queue_reads_the_newest_threads_first_by_index() {
        let store = store_with_work();
        let plan = plan(&store, super::TO_SUMMARIZE);
        assert!(plan.contains("SCAN t USING COVERING INDEX threads_by_last_message") && !plan.contains("TEMP B-TREE"), "{plan}");
    }

    #[test]
    fn conversations_that_matter_wait_for_a_summary_until_a_new_message() {
        let mut store = store_with_work();
        // A conversation in Important, of two messages.
        let a = ingest(&mut store, "1:1", &mail().id("a@x").at(NOV_1 - DAY).body("Can you sign off the budget?").build());
        let b = ingest(&mut store, "1:2", &mail().id("b@x").subject("Re: Q4 budget").reply_to(&["a@x"]).body("Any news?").build());
        // One message only, and a newsletter conversation: no summary needed.
        ingest(&mut store, "1:3", &mail().id("c@x").subject("Lunch").build());
        let news = mail().subject("Weekly digest").header("List-Id", "<news.acme.co>");
        ingest(&mut store, "1:4", &news.clone().id("n1@x").build());
        ingest(&mut store, "1:5", &news.id("n2@x").subject("Re: Weekly digest").reply_to(&["n1@x"]).build());

        let waiting = store.to_summarize("e4b", 10).unwrap();
        assert_eq!(waiting.len(), 1);
        let t = &waiting[0];
        assert_eq!((t.thread_id.to_string(), t.account_id.as_str()), (a.thread_id.clone(), "work"));
        assert_eq!(t.through.to_string(), b.message_id);
        let texts: Vec<&str> = t.messages.iter().map(|(_, _, text)| text.as_str()).collect();
        let sent: Vec<i64> = t.messages.iter().map(|(_, at, _)| *at).collect();
        assert_eq!(sent, [NOV_1 - DAY, NOV_1]);
        assert_eq!(texts, ["Can you sign off the budget?", "Any news?"]);
        assert_eq!(t.messages[0].0.email, "daniel@northwind.co");
        assert_eq!((t.you.as_str(), t.why), ("maya@northwind.co", SummaryWhy::Important));

        store.set_summary(t, "Daniel needs the Q4 budget signed off.", "e4b", NOV_1).unwrap();
        assert!(store.to_summarize("e4b", 10).unwrap().is_empty());
        let summary = store.thread(&a.thread_id, NOV_1).unwrap().unwrap().summary;
        assert_eq!(summary.ai_summary.as_deref(), Some("Daniel needs the Q4 budget signed off."));
        assert_eq!(summary.ai_summary_why, Some(SummaryWhy::Important));
        // Another model, or a new message, asks again.
        assert_eq!(store.to_summarize("26b", 10).unwrap().len(), 1);
        ingest(&mut store, "1:6", &mail().id("d@x").subject("Re: Q4 budget").reply_to(&["b@x"]).build());
        assert_eq!(store.to_summarize("e4b", 10).unwrap().len(), 1);
        // Without bodies, previews stand in.
        store.conn.execute_batch("DELETE FROM bodies").unwrap();
        assert_eq!(store.to_summarize("e4b", 10).unwrap()[0].messages[0].2, "Can you sign off the budget?");
    }

    #[test]
    fn conversations_you_wrote_in_count_and_done_ones_dont() {
        let mut store = store_with_work();
        let news = mail().subject("Order").header("List-Id", "<shop.co>");
        let first = ingest(&mut store, "1:1", &news.clone().id("o@x").build());
        let mine = news.id("m@x").from("maya@northwind.co").reply_to(&["o@x"]).build();
        store.ingest(&mine, &MessageSource { mailbox: "Sent".into(), ..source("2:1") }).unwrap();
        let waiting = store.to_summarize("e4b", 10).unwrap();
        assert_eq!(waiting[0].why, SummaryWhy::Replied);
        store.set_summary(&waiting[0], "Your order.", "e4b", NOV_1).unwrap();
        let listed = store.thread(&first.thread_id, NOV_1).unwrap().unwrap().summary;
        assert_eq!(listed.ai_summary_why, Some(SummaryWhy::Replied));
        store.conn.execute_batch("DELETE FROM summaries").unwrap();
        store.mark_done(&first.thread_id, true, NOV_1).unwrap();
        assert!(store.to_summarize("e4b", 10).unwrap().is_empty());
    }

    #[test]
    fn one_liners_from_before_v14_say_why_they_were_written() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migrate(&mut conn, &crate::schema::MIGRATIONS[..13]).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts VALUES ('w', 'Work', 'W', 'maya@northwind.co', 'Maya', 'work', 'imap', NULL, NULL, 1);
             INSERT INTO threads VALUES (1, 'w', 'Budget', 'budget', '', 1), (2, 'w', 'Lunch', 'lunch', '', 1);",
        )
        .unwrap();
        for (id, thread, from_me) in [(1, 1, 0), (2, 2, 0), (3, 2, 1)] {
            conn.execute(
                "INSERT INTO messages (id, account_id, thread_id, message_id, subject, from_name, from_email, to_json,
                     cc_json, bcc_json, participants, sent_at, received_at, snippet, from_me,
                     split, section)
                 VALUES (?1, 'w', ?2, ?1, 'x', 'Dana', 'dana@acme.co', '[]', '[]', '[]', 'Dana', 1, 1,
                     'Hi', ?3, 'important', 'reply')",
                rusqlite::params![id, thread, from_me],
            )
            .unwrap();
        }
        conn.execute_batch("INSERT INTO summaries VALUES (1, 'a', 'e4b', 1, 1), (2, 'b', 'e4b', 3, 1)").unwrap();
        crate::migrate(&mut conn, crate::schema::MIGRATIONS).unwrap();
        let mut stmt = conn.prepare("SELECT why FROM summaries ORDER BY thread_id").unwrap();
        let why: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(why, ["important", "replied"]);
    }

    #[test]
    fn turning_automatic_summaries_off_writes_none_and_hides_those_written() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        ingest(&mut store, "1:2", &mail().id("b@x").subject("Re: Q4 budget").reply_to(&["a@x"]).build());
        assert!(store.automatic_summaries().unwrap());
        let waiting = store.to_summarize("e4b", 10).unwrap();
        store.set_summary(&waiting[0], "Budget.", "e4b", NOV_1).unwrap();
        store.set_automatic_summaries(false).unwrap();
        assert!(!store.automatic_summaries().unwrap());
        assert_eq!(store.thread(&a.thread_id, NOV_1).unwrap().unwrap().summary.ai_summary, None);
        assert!(store.to_summarize("26b", 10).unwrap().is_empty());
        store.set_automatic_summaries(true).unwrap();
        assert_eq!(store.to_summarize("e4b", 10).unwrap().len(), 1, "written again");
    }

    #[test]
    fn the_model_reads_the_whole_conversation_every_word_of_it() {
        let mut store = store_with_work();
        let long = "word ".repeat(2_000);
        let first = ingest(&mut store, "1:0", &mail().id("m0@x").at(NOV_1 - 9 * DAY).body(&long).build());
        for i in 1..9 {
            let raw = mail().id(&format!("m{i}@x")).at(NOV_1 - (9 - i) * DAY).subject("Re: Q4 budget").reply_to(&[&format!("m{}@x", i - 1)]).body(&long).build();
            ingest(&mut store, &format!("1:{i}"), &raw);
        }
        let c = store.to_summarize_now(&first.thread_id).unwrap();
        assert_eq!(c.messages.len(), 9);
        assert!(c.messages.iter().all(|(_, _, text)| text.trim() == long.trim()));
    }

    #[test]
    fn newsletter_clutter_is_left_out_for_the_model() {
        let text = "View image: (https://media.letters.example/banner.png)\nCaption: \n\n\
                    [SUBSCRIBE](https://letters.example/subscribe?utm=1) | [AI TOOLS](https://aidigest.example/)\n\n\
                    ----------\n### The week in tools\n\
                    Follow image link: (https://builder.example/x?utm_campaign=Y&click_id=12)\n\
                    Describe it and [Buildly](https://builder.example/letters?utm_campaign=Y#top) builds it.\n\n\n\n\
                    * * *\nRead more at https://labnews.example/news/launch?ref=aidigest today. [](https://short.example/x)\n";
        assert_eq!(
            super::readable(text),
            "SUBSCRIBE (https://letters.example/subscribe) | AI TOOLS (https://aidigest.example/)\n\n\
             ### The week in tools\n\
             Describe it and Buildly (https://builder.example/letters) builds it.\n\n\
             Read more at https://labnews.example/news/launch today."
        );
    }

    #[test]
    fn failures_are_errors() {
        let mut store = store_with_work();
        ingest(&mut store, "1:1", &mail().id("a@x").build());
        ingest(&mut store, "1:2", &mail().id("b@x").subject("Re: Q4 budget").reply_to(&["a@x"]).build());
        let waiting = store.to_summarize("e4b", 10).unwrap();
        read_only(&store);
        assert!(store.set_summary(&waiting[0], "x", "e4b", NOV_1).is_err());
        break_schema(&store, "PRAGMA query_only = false; DROP TABLE summaries");
        assert!(store.to_summarize("e4b", 1).is_err());
    }
}

