//! What Moss decided about each message, and who decided it: the decision model, a rule, or
//! you. Each question is asked once per message, the UI can say why ("AI, 91% sure", "says
//! “by Friday”"), and your corrections always win: a message you corrected is never asked
//! again, and nothing overwrites your answer.
//!
//! The questions are the decision model's duties in `docs/research/decision-models.md`:
//! which split, needs your reply, has a deadline, and (for mail you sent) expects a reply.
//! Moss moves mail between splits and sections, shows due days, and brings your sent mail
//! back when no one answers. It never archives, deletes or sends.

use std::collections::hash_map::{Entry, HashMap};

use moss_core::{DecidedBy, Due, Person, Section, SenderStatus, SplitId, Waiting, Why};
use rusqlite::{params, OptionalExtension};

use crate::codec::name;
use crate::senders::Senders;
use crate::{Result, Store};

/// Does it need your reply? Its value is the section: "reply" or "info".
pub const NEEDS_REPLY: &str = "needsReply";
/// Which split tab it belongs in. Its value is the split's name.
pub const SPLIT: &str = "split";
/// Does it set you a deadline? Its value is the day, YYYY-MM-DD.
pub const DEADLINE: &str = "deadline";
/// Does your email expect an answer? Its value is when it comes back if none comes (ms).
pub const EXPECTS_REPLY: &str = "expectsReply";

/// Who gave an answer the sort loop saves. Your answers come from the corrections below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Model,
    Rule,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Model => "model",
            Source::Rule => "rule",
        }
    }
}

/// One answer about a message, as the sort loop saves it.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub source: Source,
    /// How likely the model said yes (or that its pick is right), when it was asked.
    pub yes: Option<f64>,
    /// Whether Moss acts on it.
    pub acted: bool,
    /// The answer itself: see each question.
    pub value: Option<String>,
    /// What a rule went by, in the email's own words.
    pub evidence: Option<String>,
}

/// A message Moss hasn't decided a question about yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Undecided {
    pub message_id: i64,
    pub account_id: String,
    pub from: Person,
    pub to: Vec<Person>,
    pub subject: String,
    pub attachments: Vec<String>,
    /// The new text, without quoted replies or signature, or the preview when the body
    /// isn't on this device.
    pub body: String,
    /// When it was sent (ms): deadlines like "by Friday" count from here.
    pub sent_at: i64,
    /// The split tab it's in now.
    pub split: SplitId,
    /// You know the sender: you wrote to them, accepted them, they wrote in a conversation you
    /// started or are at your company (`classify::sender_status`). Mail from someone you know
    /// is never taken for a sales pitch.
    pub known: bool,
}

/// Which messages each question is about: the latest message in its thread, and for mail
/// from other people, open in the list.
fn about(question: &str) -> Option<&'static str> {
    Some(match question {
        NEEDS_REPLY => "m.split = 'important' AND m.from_me = 0 AND m.done = 0",
        // Newsletters too: list headers only put mail there until the decision model says what
        // it is. Other too: a sales pitch sent from a no-reply or helpdesk address lands there.
        // A thread you moved stays where you put it, so nothing in it is asked.
        SPLIT => "m.split IN ('important', 'other', 'receipts', 'newsletters') AND m.from_me = 0 AND m.done = 0
                  AND (SELECT moved_to FROM threads WHERE id = m.thread_id) IS NULL",
        DEADLINE => "m.split IN ('important', 'receipts') AND m.from_me = 0 AND m.done = 0",
        // Sent mail is stored done; a thread you marked done can still wait on an answer.
        EXPECTS_REPLY => "m.from_me = 1 AND m.to_json <> '[]'",
        _ => return None,
    })
}

/// SQL: the newest messages `which` picks, received since `?2`, that question `?1` hasn't
/// been answered for (with `?4`, nor only by a rule); up to `?3`.
fn undecided_sql(which: &str) -> String {
    format!(
        "SELECT m.id, m.account_id, m.from_name, m.from_email, m.to_json, m.subject, m.snippet, m.sent_at, m.split,
                (SELECT json_group_array(name) FROM (SELECT name FROM attachments
                    WHERE message_id = m.id AND inline = 0 ORDER BY position))
         FROM messages m
         WHERE {which} AND m.received_at >= ?2 AND m.deleted_from IS NULL AND m.blocked = 0
           AND m.id = (SELECT id FROM messages WHERE thread_id = m.thread_id ORDER BY sent_at DESC, id DESC LIMIT 1)
           AND NOT EXISTS (SELECT 1 FROM ai_decisions WHERE message_id = m.id AND question = ?1
               AND (?4 = 0 OR yes IS NOT NULL OR source = 'you'))
         ORDER BY m.received_at DESC, m.id DESC
         LIMIT ?3"
)
}

impl Store {
    /// Newest first: messages received since `since` (ms) that `question` is about and that
    /// haven't been decided. With `model`, that includes the ones only a rule has answered,
    /// so the decision model can look again; your answers are never asked again.
    pub fn undecided(&self, question: &str, since: i64, limit: usize, model: bool) -> Result<Vec<Undecided>> {
        let Some(which) = about(question) else {
            return Ok(vec![]);
        };
        let mut stmt = self.conn.prepare(&undecided_sql(which))?;
        let rows = stmt.query_map(params![question, since, limit as i64, model], |r| {
            Ok(Undecided {
                message_id: r.get(0)?,
                account_id: r.get(1)?,
                from: Person { name: r.get(2)?, email: r.get(3)? },
                to: crate::codec::json(r, 4)?,
                subject: r.get(5)?,
                body: r.get(6)?,
                sent_at: r.get(7)?,
                split: crate::codec::named(r, 8)?,
                attachments: crate::codec::json(r, 9)?,
                known: false,
            })
        })?;
        let mut list: Vec<Undecided> = rows.collect::<rusqlite::Result<_>>()?;
        let mut senders: HashMap<String, Senders> = HashMap::new();
        for email in &mut list {
            // The new text when the body is here; the preview stands in when it isn't.
            if let Some(body) = self.body(email.message_id)? {
                email.body = crate::text::strip_quoted(&body.text);
            }
            let account = match senders.entry(email.account_id.clone()) {
                Entry::Occupied(known) => known.into_mut(),
                Entry::Vacant(new) => new.insert(Senders::load(&self.conn, &email.account_id)?),
            };
            email.known = account.status(&self.conn, &email.from.email)? == SenderStatus::Known;
        }
        Ok(list)
    }

    /// Saves an answer and does what it says, unless you have answered `question` about this
    /// message yourself. Returns whether anything in the list changed.
    pub fn decide(&mut self, message_id: i64, question: &str, decision: &Decision, now: i64) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let saved = tx.execute(
            "INSERT INTO ai_decisions (message_id, question, yes, source, acted, value, evidence, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (message_id, question) DO UPDATE SET yes = excluded.yes, source = excluded.source,
                 acted = excluded.acted, value = excluded.value, evidence = excluded.evidence,
                 decided_at = excluded.decided_at
             WHERE ai_decisions.source <> 'you'",
            params![
                message_id,
                question,
                decision.yes,
                decision.source.name(),
                decision.acted,
                decision.value,
                decision.evidence,
                now
            ],
        )?;
        let value = decision.value.as_deref().filter(|_| saved > 0 && decision.acted);
        let changed = match (question, value) {
            (_, None) => 0,
            (NEEDS_REPLY, Some(section)) => tx.execute(
                "UPDATE messages SET section = ?2 WHERE id = ?1 AND section <> ?2",
                params![message_id, section],
            )?,
            // Only Important has sections: anywhere else is for your information.
            (SPLIT, Some(split)) => tx.execute(
                "UPDATE messages SET split = ?2, section = CASE WHEN ?2 = 'important' THEN section ELSE 'info' END
                 WHERE id = ?1 AND split <> ?2",
                params![message_id, split],
            )?,
            // Back in the list, out of sight until then, like Remind me.
            (EXPECTS_REPLY, Some(until)) => tx.execute(
                "UPDATE messages SET done = 0, done_at = NULL, snoozed_until = ?2 WHERE id = ?1",
                params![message_id, until.parse::<i64>().unwrap_or(now)],
            )?,
            // The due chip reads the answer itself.
            (_, Some(_)) => 1,
        };
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Your answer to `question` about the messages of a thread that `which` picks (a
    /// condition on `messages`), replacing anyone else's.
    pub(crate) fn correct(&self, thread_id: &str, which: &str, question: &str, value: Option<&str>, acted: bool, now: i64) -> Result<()> {
        let id = self.thread_pk(thread_id)?;
        let expected: i64 = self.conn.query_row(
            &format!("SELECT count(*) FROM messages WHERE thread_id=?1 AND {which}"), [id], |row| row.get(0))?;
        let changed = self.conn.execute(
            &format!(
                "INSERT INTO ai_decisions (message_id, question, yes, source, acted, value, decided_at)
                 SELECT id, ?2, NULL, 'you', ?3, ?4, ?5 FROM messages WHERE thread_id = ?1 AND {which}
                 ON CONFLICT (message_id, question) DO UPDATE SET source = 'you', acted = excluded.acted,
                     value = excluded.value, evidence = NULL, decided_at = excluded.decided_at"
            ),
            params![id, question, acted, value, now],
        )?;
        if changed != expected as usize { return Err(crate::Error::Invalid("The conversation preference was not saved completely.".into())); }
        Ok(())
    }

    /// Moves a thread between "needs your reply" and "for your information". Use
    /// [`Store::snooze`] for later. Your choice wins over the decision model's, now and later.
    pub fn set_section(&mut self, thread_id: &str, section: Section, now: i64) -> Result<()> {
        let id = self.thread_pk(thread_id)?;
        self.conn.execute("UPDATE messages SET section = ?2 WHERE thread_id = ?1", params![id, name(&section)])?;
        self.correct(thread_id, "from_me = 0", NEEDS_REPLY, Some(&name(&section)), false, now)
    }

    /// Moves a thread to another split tab, for good.
    pub fn set_split(&mut self, thread_id: &str, split: SplitId, now: i64) -> Result<()> {
        let id = self.thread_pk(thread_id)?;
        self.conn.execute(
            "UPDATE messages SET split = ?2, section = CASE WHEN ?2 = 'important' THEN section ELSE 'info' END
             WHERE thread_id = ?1",
            params![id, name(&split)],
        )?;
        self.correct(thread_id, "from_me = 0", SPLIT, Some(&name(&split)), false, now)
    }

    /// "No deadline": the thread's due chip goes, and stays gone.
    pub fn clear_deadline(&mut self, thread_id: &str, now: i64) -> Result<()> {
        self.correct(thread_id, "from_me = 0", DEADLINE, None, false, now)
    }

    /// "Remind me if no one replies" on the latest email in a thread, which must be yours.
    /// On, the thread is out of sight until `wait` (ms) after you sent it (or after `now`,
    /// when that has passed), like Remind me; it comes back unless someone answers first.
    /// Off, Moss won't bring it back. Returns whether the latest email was yours.
    pub fn remind_if_no_reply(&mut self, thread_id: &str, on: bool, wait: i64, now: i64) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let changed = self.remind_if_no_reply_inner(thread_id, on, wait, now, None)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Called within the caller's transaction. Delivery repair must preserve an explicit choice.
    pub(crate) fn remind_if_no_reply_inner(&self, thread_id: &str, on: bool, wait: i64, now: i64, expected_message: Option<&str>) -> Result<bool> {
        let id = self.thread_pk(thread_id)?;
        let latest: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT id, sent_at FROM messages WHERE thread_id = ?1 AND from_me = 1
                   AND id = (SELECT id FROM messages WHERE thread_id = ?1 ORDER BY sent_at DESC, id DESC LIMIT 1)",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((message, sent_at)) = latest else {
            return Ok(false);
        };
        if let Some(expected) = expected_message {
            if message != crate::codec::parse_id(expected) { return Ok(false); }
            let chosen: bool = self.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM messages WHERE id = ?1 AND
                    (done_at IS NOT NULL OR EXISTS (SELECT 1 FROM ai_decisions
                     WHERE message_id = ?1 AND question = ?2 AND source = 'you')))",
                params![message, EXPECTS_REPLY], |row| row.get(0))?;
            if chosen { return Ok(false); }
        }
        let until = if sent_at + wait > now { sent_at + wait } else { now + wait };
        self.conn.execute(
            "INSERT INTO ai_decisions (message_id, question, yes, source, acted, value, decided_at)
             VALUES (?1, ?2, NULL, 'you', ?3, ?4, ?5)
             ON CONFLICT (message_id, question) DO UPDATE SET source = 'you', acted = excluded.acted,
                 value = excluded.value, evidence = NULL, decided_at = excluded.decided_at",
            params![message, EXPECTS_REPLY, on, on.then(|| until.to_string()), now],
        )?;
        if on {
            self.conn.execute("UPDATE messages SET done = 0, done_at = NULL WHERE id = ?1", [message])?;
            self.conn.execute("UPDATE messages SET snoozed_until = ?2 WHERE thread_id = ?1 AND done = 0", params![id, until])?;
        } else {
            self.conn.execute("UPDATE messages SET snoozed_until = NULL WHERE thread_id = ?1 AND done = 0", [id])?;
            self.conn.execute("UPDATE messages SET done = 1 WHERE id = ?1", [message])?;
        }
        Ok(true)
    }
}

/// Why, from an answer as `json_object('source', …, 'yes', …, 'evidence', …)`.
pub(crate) fn why(source: &str, yes: Option<f64>, evidence: Option<String>) -> Why {
    let by = match source {
        "model" => DecidedBy::Ai,
        "you" => DecidedBy::You,
        _ => DecidedBy::Rule,
    };
    let sure = yes.filter(|_| by == DecidedBy::Ai).map(|y| (100.0 * y.max(1.0 - y)).round() as u8);
    Why { by, sure, evidence }
}

/// An answer read back from the row columns the thread list adds: its JSON object.
#[derive(serde::Deserialize)]
pub(crate) struct Answer {
    source: String,
    yes: Option<f64>,
    value: Option<String>,
    evidence: Option<String>,
}

impl Answer {
    pub(crate) fn why(self) -> Why {
        why(&self.source, self.yes, self.evidence)
    }

    pub(crate) fn due(self) -> Option<Due> {
        let date = self.value.clone()?;
        Some(Due { date, why: self.why() })
    }

    pub(crate) fn waiting(self) -> Option<Waiting> {
        let until = self.value.as_deref()?.parse().ok()?;
        Some(Waiting { until, why: self.why() })
    }
}

/// The SQL for one answer as a JSON object, or NULL: the answer to `question` about the
/// message `message` names, that Moss acts on, from whoever `who` allows.
pub(crate) fn answer_sql(question: &str, message: &str, who: &str) -> String {
    format!(
        "(SELECT json_object('source', source, 'yes', yes, 'value', value, 'evidence', evidence) FROM ai_decisions
          WHERE message_id = {message} AND question = '{question}' AND acted = 1 AND {who})"
    )
}

#[cfg(test)]
mod tests;
