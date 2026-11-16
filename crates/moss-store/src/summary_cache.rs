//! The summary kept for each conversation, so opening it again shows it at once: one per
//! conversation, the kind you chose last (writing another kind replaces it). It says which
//! message it read up to: a newer message makes it stale, and it's shown as such until written
//! again. You can discard it, and the automatic one-liner with it.

use moss_core::{SummaryContent, SummaryKind, SummaryRequest, SummaryView, SummaryWhy, ThreadSummaries};
use rusqlite::{params, OptionalExtension, Row};

use crate::codec::{name, named, parse_id, to_json};
use crate::summaries::ToSummarize;
use crate::{Error, Result, Store};

/// SQL: the newest message of conversation `?1`.
const NEWEST: &str = "(SELECT id FROM messages WHERE thread_id = ?1 ORDER BY sent_at DESC, id DESC LIMIT 1)";

const COLUMNS: &str = "kind, instruction, content, why, model, made_at, through";

/// A kept summary, or `None` when its content is in a form this Moss can't read: an older
/// Moss wrote it (Detailed was once a timeline). It's written again when asked for.
fn view(r: &Row, newest: i64) -> rusqlite::Result<Option<SummaryView>> {
    let Ok(content) = serde_json::from_str(&r.get::<_, String>(2)?) else {
        return Ok(None);
    };
    Ok(Some(SummaryView {
        request: SummaryRequest { kind: named(r, 0)?, instruction: r.get(1)? },
        content,
        why: named(r, 3)?,
        model: r.get(4)?,
        made_at: r.get(5)?,
        stale: r.get::<_, i64>(6)? < newest,
    }))
}

impl Store {
    /// The newest message of a conversation, or `NotFound`.
    fn newest(&self, thread_id: &str) -> Result<i64> {
        let id = parse_id(thread_id);
        let newest: Option<i64> = self.conn.query_row(&format!("SELECT {NEWEST}"), [id], |r| r.get(0))?;
        newest.ok_or_else(|| Error::NotFound { kind: "thread", id: thread_id.into() })
    }

    /// A conversation to summarise now, up to its newest message.
    pub fn to_summarize_now(&self, thread_id: &str) -> Result<ToSummarize> {
        let newest = self.newest(thread_id)?;
        self.conversation(parse_id(thread_id), newest, SummaryWhy::Asked)
    }

    /// The summary kept for a conversation, by any model.
    fn kept(&self, thread_id: &str, newest: i64) -> Result<Option<SummaryView>> {
        Ok(self
            .conn
            .query_row(&format!("SELECT {COLUMNS} FROM thread_summaries WHERE thread_id = ?1"), [parse_id(thread_id)], |r| {
                view(r, newest)
            })
            .optional()?
            .flatten())
    }

    /// The summary kept for a conversation when it's this kind by `model`, fresh or stale.
    /// For Short, the automatic one-liner stands in when nothing is kept.
    pub fn kept_summary(&self, thread_id: &str, request: &SummaryRequest, model: &str) -> Result<Option<SummaryView>> {
        let newest = self.newest(thread_id)?;
        match self.kept(thread_id, newest)? {
            Some(kept) => Ok((&kept.request == request && kept.model == model).then_some(kept)),
            None if request.kind == SummaryKind::Short => self.automatic(thread_id, model, newest),
            None => Ok(None),
        }
    }

    /// The automatic one-liner by `model`, as a Short summary.
    fn automatic(&self, thread_id: &str, model: &str, newest: i64) -> Result<Option<SummaryView>> {
        Ok(self
            .conn
            .query_row(
                "SELECT 'short', '', json_object('format', 'text', 'text', text), why, model, made_at, through
                 FROM summaries WHERE thread_id = ?1 AND model = ?2",
                params![parse_id(thread_id), model],
                |r| view(r, newest),
            )
            .optional()?
            .flatten())
    }

    /// Keeps a summary of `c` (up to its `through`), replacing the one kept before, whatever
    /// its kind or model. It was written for the conversation's rule when the rule asks for
    /// this kind; otherwise because you asked.
    pub fn keep_summary(
        &mut self,
        c: &ToSummarize,
        request: &SummaryRequest,
        model: &str,
        content: &SummaryContent,
        now: i64,
    ) -> Result<SummaryView> {
        let thread_id = c.thread_id.to_string();
        let rule = self.summary_rule_for(&thread_id)?;
        let why = if rule.is_some_and(|r| &r.request == request) { SummaryWhy::Rule } else { SummaryWhy::Asked };
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO thread_summaries (thread_id, kind, instruction, model, through, content, why, made_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![c.thread_id, name(&request.kind), request.instruction, model, c.through, to_json(content), why.name(), now],
        )?;
        tx.execute("DELETE FROM discarded_summaries WHERE thread_id = ?1", [c.thread_id])?;
        tx.commit()?;
        let newest = self.newest(&thread_id)?;
        Ok(SummaryView {
            request: request.clone(),
            content: content.clone(),
            why,
            model: model.into(),
            made_at: now,
            stale: c.through < newest,
        })
    }

    /// What a conversation shows as it opens: the summary kept for it, by whichever model (a
    /// relaunch or a rebuild with new prompts keeps it), else its automatic one-liner by
    /// `model`; and the rule for it.
    pub fn thread_summaries(&self, thread_id: &str, model: &str) -> Result<ThreadSummaries> {
        let newest = self.newest(thread_id)?;
        // Kept by another model, or before a rebuild changed the prompts: still worth reading
        // until it's asked for again.
        let summary = match self.kept(thread_id, newest)? {
            kept @ Some(_) => kept,
            None => self.automatic(thread_id, model, newest)?,
        };
        Ok(ThreadSummaries { summary, rule: self.summary_rule_for(thread_id)? })
    }

    /// Discards a conversation's summary and its automatic one-liner. Neither is written
    /// again in the background until a new message comes. Returns its account, whose list
    /// changed.
    pub fn discard_summary(&mut self, thread_id: &str) -> Result<String> {
        let newest = self.newest(thread_id)?;
        let id = parse_id(thread_id);
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM thread_summaries WHERE thread_id = ?1", [id])?;
        tx.execute("DELETE FROM summaries WHERE thread_id = ?1", [id])?;
        tx.execute("INSERT OR REPLACE INTO discarded_summaries (thread_id, through) VALUES (?1, ?2)", [id, newest])?;
        let account: String = tx.query_row("SELECT account_id FROM threads WHERE id = ?1", [id], |r| r.get(0))?;
        tx.commit()?;
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{ActionItem, SummaryKind};

    fn actions(text: &str) -> SummaryContent {
        SummaryContent::Actions { items: vec![ActionItem { text: text.into(), due: None }] }
    }

    fn of(kind: SummaryKind) -> SummaryRequest {
        SummaryRequest::of(kind)
    }

    fn kept_rows(store: &Store) -> i64 {
        store.conn.query_row("SELECT count(*) FROM thread_summaries", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_summary_you_ask_for_is_kept_until_a_new_message() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").body("Can you sign off the budget?").build());
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        assert_eq!((c.why, c.messages.len(), c.you.as_str()), (SummaryWhy::Asked, 1, "maya@northwind.co"));
        let request = of(SummaryKind::Actions);
        assert_eq!(store.kept_summary(&a.thread_id, &request, "e4b").unwrap(), None);
        let kept = store.keep_summary(&c, &request, "e4b", &actions("Sign off the budget"), NOV_1).unwrap();
        assert_eq!((kept.why, kept.stale, kept.made_at), (SummaryWhy::Asked, false, NOV_1));
        assert_eq!(store.kept_summary(&a.thread_id, &request, "e4b").unwrap(), Some(kept.clone()));
        assert_eq!(store.kept_summary(&a.thread_id, &request, "26b").unwrap(), None, "another model");
        assert_eq!(store.kept_summary(&a.thread_id, &of(SummaryKind::Facts), "e4b").unwrap(), None);
        // A new message makes it stale; writing it again makes it fresh.
        ingest(&mut store, "1:2", &mail().id("b@x").subject("Re: Q4 budget").reply_to(&["a@x"]).build());
        assert!(store.kept_summary(&a.thread_id, &request, "e4b").unwrap().unwrap().stale);
        let old = store.keep_summary(&c, &request, "e4b", &actions("Sign"), NOV_1 + 2).unwrap();
        assert!(old.stale, "written from before the new message");
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        let fresh = store.keep_summary(&c, &request, "e4b", &actions("Sign it today"), NOV_1 + 3).unwrap();
        assert!(!fresh.stale);
        assert_eq!(store.kept_summary(&a.thread_id, &request, "e4b").unwrap(), Some(fresh));
        assert!(matches!(store.to_summarize_now("99"), Err(Error::NotFound { .. })));
        assert!(matches!(store.kept_summary("nope", &request, "e4b"), Err(Error::NotFound { .. })));
    }

    #[test]
    fn a_summary_an_older_moss_wrote_in_a_way_this_one_cant_read_counts_as_none() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        let detailed = of(SummaryKind::Detailed);
        store.keep_summary(&c, &detailed, "e4b", &SummaryContent::Text { text: "x".into() }, NOV_1).unwrap();
        // Detailed used to be a timeline, with a date column.
        let timeline = r#"{"format":"timeline","items":[{"when":"Wed","text":"Dana sent the deck."}]}"#;
        store.conn.execute("UPDATE thread_summaries SET content = ?1", [timeline]).unwrap();
        assert_eq!(store.kept_summary(&a.thread_id, &detailed, "e4b").unwrap(), None);
        assert_eq!(store.thread_summaries(&a.thread_id, "e4b").unwrap().summary, None);
        // Writing it again replaces it.
        let text = SummaryContent::Text { text: "Dana sent the deck.".into() };
        store.keep_summary(&c, &detailed, "e4b", &text, NOV_1 + 1).unwrap();
        assert_eq!(store.kept_summary(&a.thread_id, &detailed, "e4b").unwrap().unwrap().content, text);
    }

    #[test]
    fn a_conversation_keeps_one_summary_the_kind_you_chose_last() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        store.keep_summary(&c, &of(SummaryKind::Facts), "e4b", &SummaryContent::Facts { items: vec![] }, NOV_1).unwrap();
        let detailed = store.keep_summary(&c, &of(SummaryKind::Detailed), "e4b", &SummaryContent::Text { text: "Long".into() }, NOV_1 + 1).unwrap();
        assert_eq!(store.kept_summary(&a.thread_id, &of(SummaryKind::Facts), "e4b").unwrap(), None, "replaced");
        assert_eq!(store.thread_summaries(&a.thread_id, "e4b").unwrap().summary, Some(detailed));
        let custom = SummaryRequest::new(SummaryKind::Custom, "In French").unwrap();
        let other = SummaryRequest::new(SummaryKind::Custom, "In Spanish").unwrap();
        store.keep_summary(&c, &custom, "e4b", &SummaryContent::Text { text: "Budget".into() }, NOV_1 + 2).unwrap();
        assert_eq!(store.kept_summary(&a.thread_id, &other, "e4b").unwrap(), None, "each instruction its own");
        // Another writing model's replaces it too: nothing old is kept.
        let short = store.keep_summary(&c, &of(SummaryKind::Short), "26b", &SummaryContent::Text { text: "Short".into() }, NOV_1 + 3).unwrap();
        assert_eq!(kept_rows(&store), 1);
        // A rebuild with new prompts, or another writing model, still shows it as you open the
        // conversation; asking for it writes it again.
        assert_eq!(store.thread_summaries(&a.thread_id, "e4b").unwrap().summary, Some(short));
        assert_eq!(store.kept_summary(&a.thread_id, &of(SummaryKind::Short), "e4b").unwrap(), None);
    }

    #[test]
    fn a_summary_in_the_kind_its_rule_asks_for_says_the_rule_wrote_it() {
        let mut store = store_with_work();
        store.save_summary_rule(None, "northwind.co", &of(SummaryKind::Facts), NOV_1).unwrap();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        let facts = SummaryContent::Facts { items: vec![] };
        let by_rule = store.keep_summary(&c, &of(SummaryKind::Facts), "e4b", &facts, NOV_1).unwrap();
        assert_eq!(by_rule.why, SummaryWhy::Rule);
        let asked = store.keep_summary(&c, &of(SummaryKind::Actions), "e4b", &actions("x"), NOV_1).unwrap();
        assert_eq!(asked.why, SummaryWhy::Asked);
    }

    #[test]
    fn the_summary_you_chose_shows_over_the_one_liner_with_the_rule() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        ingest(&mut store, "1:2", &mail().id("b@x").subject("Re: Q4 budget").reply_to(&["a@x"]).build());
        assert_eq!(store.thread_summaries(&a.thread_id, "e4b").unwrap(), ThreadSummaries::default());
        // The automatic one-liner shows while there's no other, and answers a request for Short.
        let auto = store.to_summarize("e4b", 1).unwrap().remove(0);
        store.set_summary(&auto, "Daniel needs the budget signed off.", "e4b", NOV_1).unwrap();
        let shown = store.thread_summaries(&a.thread_id, "e4b").unwrap().summary.unwrap();
        assert_eq!((shown.request.kind, shown.why, shown.stale), (SummaryKind::Short, SummaryWhy::Important, false));
        assert_eq!(shown.content, SummaryContent::Text { text: "Daniel needs the budget signed off.".into() });
        let short = of(SummaryKind::Short);
        assert_eq!(store.kept_summary(&a.thread_id, &short, "e4b").unwrap(), Some(shown));
        assert_eq!(store.kept_summary(&a.thread_id, &short, "26b").unwrap(), None);
        // The one you chose wins, even when the one-liner is written after it.
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        let detailed = store.keep_summary(&c, &of(SummaryKind::Detailed), "e4b", &SummaryContent::Text { text: "Long".into() }, NOV_1 + 1).unwrap();
        store.set_summary(&auto, "Newer one-liner.", "e4b", NOV_1 + 2).unwrap();
        let rule = store.save_summary_rule(None, "daniel@northwind.co", &of(SummaryKind::Facts), NOV_1).unwrap();
        assert_eq!(store.thread_summaries(&a.thread_id, "e4b").unwrap(), ThreadSummaries { summary: Some(detailed), rule: Some(rule) });
        // Asking for Short then writes one: the one-liner stands in only when nothing is kept.
        assert_eq!(store.kept_summary(&a.thread_id, &short, "e4b").unwrap(), None);
        assert!(matches!(store.thread_summaries("nope", "e4b"), Err(Error::NotFound { .. })));
    }

    #[test]
    fn a_discarded_summary_is_gone_with_the_one_liner_until_a_new_message() {
        let mut store = store_with_work();
        store.save_summary_rule(None, "northwind.co", &of(SummaryKind::Facts), NOV_1 - 1).unwrap();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        ingest(&mut store, "1:2", &mail().id("b@x").subject("Re: Q4 budget").reply_to(&["a@x"]).build());
        let (c, _) = store.to_summarize_by_rule("e4b", 5).unwrap().remove(0);
        store.keep_summary(&c, &of(SummaryKind::Facts), "e4b", &SummaryContent::Facts { items: vec![] }, NOV_1).unwrap();
        let auto = store.to_summarize("e4b", 1).unwrap().remove(0);
        store.set_summary(&auto, "Daniel needs the budget signed off.", "e4b", NOV_1).unwrap();
        assert_eq!(store.discard_summary(&a.thread_id).unwrap(), "work");
        assert_eq!(store.thread_summaries(&a.thread_id, "e4b").unwrap().summary, None);
        assert_eq!(store.thread(&a.thread_id, NOV_1).unwrap().unwrap().summary.ai_summary, None, "gone from the list too");
        // Nothing writes it again in the background…
        assert!(store.to_summarize("e4b", 5).unwrap().is_empty());
        assert!(store.to_summarize_by_rule("e4b", 5).unwrap().is_empty());
        // …until a new message comes.
        ingest(&mut store, "1:3", &mail().id("c@x").subject("Re: Q4 budget").reply_to(&["b@x"]).build());
        assert_eq!(store.to_summarize("e4b", 5).unwrap().len(), 1);
        assert_eq!(store.to_summarize_by_rule("e4b", 5).unwrap().len(), 1);
        // Asking for one keeps it as usual, and forgets the discard.
        store.discard_summary(&a.thread_id).unwrap();
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        let short = store.keep_summary(&c, &of(SummaryKind::Short), "e4b", &SummaryContent::Text { text: "x".into() }, NOV_1).unwrap();
        assert_eq!(store.thread_summaries(&a.thread_id, "e4b").unwrap().summary, Some(short));
        let discards: i64 = store.conn.query_row("SELECT count(*) FROM discarded_summaries", [], |r| r.get(0)).unwrap();
        assert_eq!(discards, 0);
        assert!(matches!(store.discard_summary("nope"), Err(Error::NotFound { .. })));
    }

    #[test]
    fn a_rule_writes_again_only_when_nothing_up_to_date_is_kept() {
        let mut store = store_with_work();
        store.save_summary_rule(None, "northwind.co", &of(SummaryKind::Facts), NOV_1 - 1).unwrap();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        // You chose another kind than the rule's: the rule leaves it alone.
        store.keep_summary(&c, &of(SummaryKind::Detailed), "e4b", &SummaryContent::Text { text: "Long".into() }, NOV_1).unwrap();
        assert!(store.to_summarize_by_rule("e4b", 5).unwrap().is_empty());
        assert_eq!(store.to_summarize_by_rule("26b", 5).unwrap().len(), 1, "another model's isn't kept");
    }

    #[test]
    fn upgrading_keeps_each_conversations_newest_summary() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migrate(&mut conn, &crate::schema::MIGRATIONS[..16]).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts VALUES ('w', 'Work', 'W', 'maya@northwind.co', 'Maya', 'work', 'imap', NULL, NULL, 1);
             INSERT INTO threads VALUES (1, 'w', 'Budget', 'budget', '', 1), (2, 'w', 'Lunch', 'lunch', '', 1);
             INSERT INTO thread_summaries VALUES
                 (1, 'facts', '', 'e4b', 1, '{}', 'asked', 10), (1, 'detailed', '', 'e4b', 1, '{}', 'asked', 30),
                 (1, 'short', '', 'e4b+prompts-2', 1, '{}', 'rule', 20), (2, 'actions', '', 'e4b', 1, '{}', 'asked', 5),
                 (2, 'short', '', 'e4b', 1, '{}', 'asked', 5);",
        )
        .unwrap();
        crate::migrate(&mut conn, crate::schema::MIGRATIONS).unwrap();
        let mut stmt = conn.prepare("SELECT thread_id, kind FROM thread_summaries ORDER BY thread_id").unwrap();
        let kept: Vec<(i64, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(kept, [(1, "detailed".to_string()), (2, "short".to_string())], "the newest; the last written on a tie");
    }

    #[test]
    fn failures_are_errors() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        let request = of(SummaryKind::Short);
        let content = SummaryContent::Text { text: "x".into() };
        read_only(&store);
        assert!(store.keep_summary(&c, &request, "e4b", &content, NOV_1).is_err());
        assert!(store.discard_summary(&a.thread_id).is_err());
        break_schema(&store, "PRAGMA query_only = false; DROP TABLE summaries");
        assert!(store.kept_summary(&a.thread_id, &request, "e4b").is_err());
        assert!(store.thread_summaries(&a.thread_id, "e4b").is_err());
        assert!(store.discard_summary(&a.thread_id).is_err());
        break_schema(&store, "DROP TABLE thread_summaries");
        assert!(store.kept_summary(&a.thread_id, &of(SummaryKind::Facts), "e4b").is_err());
        assert!(store.thread_summaries(&a.thread_id, "e4b").is_err());
        assert!(store.discard_summary(&a.thread_id).is_err());
        // Deleting would take the mail with it, so the columns change instead.
        break_schema(&store, "ALTER TABLE accounts RENAME COLUMN address TO gone");
        assert!(store.to_summarize_now(&a.thread_id).is_err());
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN snippet TO gone");
        assert!(store.to_summarize_now(&a.thread_id).is_err());
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        break_schema(&store, "DROP TABLE discarded_summaries");
        assert!(store.discard_summary(&a.thread_id).is_err());
        let c = store.to_summarize_now(&a.thread_id).unwrap();
        assert!(store.keep_summary(&c, &request, "e4b", &content, NOV_1).is_err());
    }
}
