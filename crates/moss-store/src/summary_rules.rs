//! "Always summarise mail from…" rules, and the setting that turns the automatic one-liners
//! off.
//!
//! A rule is for an address, or a whole domain and its subdomains, in every account. The
//! most specific rule wins: an address over a domain, a longer domain over a shorter one. A
//! rule reads the conversation's latest message from someone else, and mail that arrives
//! after it's made is summarised in the background, before you open it.

use moss_core::{SummaryRequest, SummaryRule, SummaryWhy};
use rusqlite::{params, OptionalExtension, Row};

use crate::classify::is_public_domain;
use crate::codec::{name, named, parse_id};
use crate::summaries::ToSummarize;
use crate::{Error, Result, Store};

/// Set to "off" to stop the automatic one-liners.
const AUTOMATIC: &str = "summaries:automatic";

/// SQL: rule `r` applies to the lowercase address `email`.
fn applies(r: &str, email: &str) -> String {
    format!(
        "({r}.sender = {email} OR (instr({r}.sender, '@') = 0
            AND substr({email}, -length({r}.sender) - 1) IN ('@' || {r}.sender, '.' || {r}.sender)))"
    )
}

/// SQL: most specific first. A domain that applies is part of the address, so it's always
/// shorter: the longest is the most specific.
fn specific_first(r: &str) -> String {
    format!("length({r}.sender) DESC")
}

/// The rule's columns, and the name of the person an address rule is for, from the mail you
/// have from them.
const RULE_COLUMNS: &str = "r.id, r.sender, r.kind, r.instruction, r.created_at,
    (SELECT p.name FROM people p WHERE lower(p.email) = r.sender AND p.name != '' ORDER BY p.last_at DESC LIMIT 1)";
/// The same, as `RETURNING` names them.
const RETURNED: &str = "id, sender, kind, instruction, created_at,
    (SELECT p.name FROM people p WHERE lower(p.email) = sender AND p.name != '' ORDER BY p.last_at DESC LIMIT 1)";

fn rule(r: &Row, at: usize) -> rusqlite::Result<SummaryRule> {
    Ok(SummaryRule {
        id: r.get(at)?,
        sender: r.get(at + 1)?,
        request: SummaryRequest { kind: named(r, at + 2)?, instruction: r.get(at + 3)? },
        created_at: r.get(at + 4)?,
        name: r.get(at + 5)?,
    })
}

/// A sender as a rule keeps it: a lowercase address, or a domain without its "@". Public
/// mail services (gmail.com…) can't be a whole domain.
fn sender(text: &str) -> Result<String> {
    let text = text.trim().to_lowercase();
    let text = text.strip_prefix("mailto:").unwrap_or(&text).trim_start_matches('@');
    let bad = || Error::Invalid(format!("“{text}” isn’t an email address or a domain"));
    let (local, domain) = match text.split_once('@') {
        Some((local, domain)) => (Some(local), domain),
        None => (None, text),
    };
    let domain_ok = domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    match local {
        _ if !domain_ok => Err(bad()),
        Some(local) if local.is_empty() || local.contains(char::is_whitespace) => Err(bad()),
        Some(_) => Ok(text.to_string()),
        None if is_public_domain(domain) => {
            Err(Error::Invalid(format!("{domain} is a public mail service: make the rule for one address there")))
        }
        None => Ok(domain.to_string()),
    }
}

impl Store {
    /// Whether Moss writes the one-line summaries of conversations that matter by itself.
    pub fn automatic_summaries(&self) -> Result<bool> {
        Ok(self.setting(AUTOMATIC)?.as_deref() != Some("off"))
    }

    /// Turns the automatic one-liners on or off. Off removes those written, so none show.
    pub fn set_automatic_summaries(&mut self, on: bool) -> Result<()> {
        self.set_setting(AUTOMATIC, (!on).then_some("off"))?;
        if !on {
            self.conn.execute("DELETE FROM summaries", [])?;
        }
        Ok(())
    }

    /// Every rule, by sender.
    pub fn summary_rules(&self) -> Result<Vec<SummaryRule>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {RULE_COLUMNS} FROM summary_rules r ORDER BY r.sender"))?;
        let rows = stmt.query_map([], |r| rule(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Makes a rule, or changes rule `id`. A sender has one rule, so saving one for a sender
    /// that has one replaces it. Returns the rule as saved.
    pub fn save_summary_rule(&mut self, id: Option<i64>, sender_text: &str, request: &SummaryRequest, now: i64) -> Result<SummaryRule> {
        let request = request.clone().checked().map_err(|e| Error::Invalid(e.to_string()))?;
        let sender = sender(sender_text)?;
        let kind = name(&request.kind);
        match id {
            Some(id) => {
                let exists: bool =
                    self.conn.query_row("SELECT EXISTS (SELECT 1 FROM summary_rules WHERE id = ?1)", [id], |r| r.get(0))?;
                if !exists {
                    return Err(Error::NotFound { kind: "summary rule", id: id.to_string() });
                }
                // Another rule for that sender gives way to this one.
                self.conn.execute("DELETE FROM summary_rules WHERE sender = ?2 AND id != ?1", params![id, sender])?;
                Ok(self.conn.query_row(
                    &format!("UPDATE summary_rules SET sender = ?2, kind = ?3, instruction = ?4 WHERE id = ?1 RETURNING {RETURNED}"),
                    params![id, sender, kind, request.instruction],
                    |r| rule(r, 0),
                )?)
            }
            None => Ok(self.conn.query_row(
                &format!(
                    "INSERT INTO summary_rules (sender, kind, instruction, after_message, created_at)
                     VALUES (?1, ?2, ?3, (SELECT coalesce(max(id), 0) FROM messages), ?4)
                     ON CONFLICT (sender) DO UPDATE SET kind = excluded.kind, instruction = excluded.instruction
                     RETURNING {RETURNED}"
                ),
                params![sender, kind, request.instruction, now],
                |r| rule(r, 0),
            )?),
        }
    }

    /// Deletes a rule; false when there was none. What it wrote stays.
    pub fn remove_summary_rule(&mut self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM summary_rules WHERE id = ?1", [id])? > 0)
    }

    /// The rule for a conversation: the most specific one for its latest message from
    /// someone else.
    pub fn summary_rule_for(&self, thread_id: &str) -> Result<Option<SummaryRule>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {RULE_COLUMNS} FROM summary_rules r, messages om
                     WHERE om.id = (SELECT id FROM messages WHERE thread_id = ?1 AND from_me = 0 ORDER BY sent_at DESC, id DESC LIMIT 1)
                       AND {}
                     ORDER BY {} LIMIT 1",
                    applies("r", "lower(om.from_email)"),
                    specific_first("r"),
                ),
                [parse_id(thread_id)],
                |r| rule(r, 0),
            )
            .optional()?)
    }

    /// Up to `limit` open conversations, newest first, that a rule should summarise: mail from
    /// the rule's sender that arrived after the rule, with no summary by `model` up to its newest message in the rule's kind (a kind
    /// you chose stays), and not discarded since.
    pub fn to_summarize_by_rule(&self, model: &str, limit: usize) -> Result<Vec<(ToSummarize, SummaryRule)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT t.id, lm.id, {RULE_COLUMNS} FROM threads t
             JOIN messages lm ON lm.id = (SELECT id FROM messages WHERE thread_id = t.id ORDER BY sent_at DESC, id DESC LIMIT 1)
             JOIN messages om ON om.id = (SELECT id FROM messages WHERE thread_id = t.id AND from_me = 0 ORDER BY sent_at DESC, id DESC LIMIT 1)
             JOIN summary_rules r ON r.id = (SELECT r2.id FROM summary_rules r2 WHERE {} ORDER BY {} LIMIT 1)
             WHERE om.id > r.after_message
               AND EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND done = 0 AND deleted_from IS NULL)
               AND NOT EXISTS (SELECT 1 FROM messages WHERE thread_id = t.id AND blocked = 1)
               AND NOT EXISTS (SELECT 1 FROM thread_summaries s WHERE s.thread_id = t.id AND s.model = ?1 AND s.through >= lm.id
                   AND (s.why = 'asked' OR (s.kind = r.kind AND s.instruction = r.instruction)))
               AND NOT EXISTS (SELECT 1 FROM discarded_summaries d WHERE d.thread_id = t.id AND d.through >= lm.id)
             ORDER BY t.last_message_at DESC LIMIT ?2",
            applies("r2", "lower(om.from_email)"),
            specific_first("r2"),
        ))?;
        let rows = stmt.query_map(params![model, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, rule(r, 2)?)))?;
        let found: Vec<(i64, i64, SummaryRule)> = rows.collect::<rusqlite::Result<_>>()?;
        let mut list = Vec::new();
        for (thread_id, through, rule) in found {
            list.push((self.conversation(thread_id, through, SummaryWhy::Rule)?, rule));
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::MessageSource;
    use moss_core::SummaryKind;

    fn of(kind: SummaryKind) -> SummaryRequest {
        SummaryRequest::of(kind)
    }

    #[test]
    fn senders_are_addresses_or_whole_domains() {
        assert_eq!(sender("  Dana@Acme.co ").unwrap(), "dana@acme.co");
        assert_eq!(sender("mailto:dana@acme.co").unwrap(), "dana@acme.co");
        assert_eq!(sender("@Acme.co").unwrap(), "acme.co");
        assert_eq!(sender("mail.acme.co").unwrap(), "mail.acme.co");
        for bad in ["", "acme", "dana@acme", "@", "dana smith@acme.co", "acme.co.", ".acme.co", "ac me.co", "a/b.co", "@acme.co@"] {
            assert!(matches!(sender(bad), Err(Error::Invalid(_))), "{bad}");
        }
        let Err(public) = sender("gmail.com") else { panic!("public") };
        assert_eq!(public.to_string(), "gmail.com is a public mail service: make the rule for one address there");
        // One person at a public service is fine.
        assert_eq!(sender("sam@gmail.com").unwrap(), "sam@gmail.com");
        let Err(e) = sender("acme") else { panic!("invalid") };
        assert_eq!(e.to_string(), "“acme” isn’t an email address or a domain");
    }

    #[test]
    fn rules_are_made_listed_changed_and_removed() {
        let mut store = store_with_work();
        let facts = store.save_summary_rule(None, "Paydesk.example", &of(SummaryKind::Facts), NOV_1).unwrap();
        assert_eq!((facts.sender.as_str(), facts.request.kind, facts.created_at), ("paydesk.example", SummaryKind::Facts, NOV_1));
        let custom = SummaryRequest::new(SummaryKind::Custom, "Only prices").unwrap();
        let dana = store.save_summary_rule(None, "dana@acme.co", &custom, NOV_1).unwrap();
        assert_eq!(store.summary_rules().unwrap(), [dana.clone(), facts.clone()]);
        // The same sender again changes its rule, keeping when it was made.
        let again = store.save_summary_rule(None, "paydesk.example", &of(SummaryKind::Actions), NOV_1 + DAY).unwrap();
        assert_eq!((again.id, again.request.kind, again.created_at), (facts.id, SummaryKind::Actions, NOV_1));
        // Changing a rule by id, to another sender.
        let moved = store.save_summary_rule(Some(dana.id), "acme.co", &of(SummaryKind::Short), NOV_1).unwrap();
        assert_eq!((moved.id, moved.sender.as_str(), moved.request.instruction.as_str()), (dana.id, "acme.co", ""));
        // To a sender with its own rule: that one goes.
        let took = store.save_summary_rule(Some(dana.id), "paydesk.example", &of(SummaryKind::Detailed), NOV_1).unwrap();
        assert_eq!(took.id, dana.id);
        assert_eq!(store.summary_rules().unwrap(), [took]);
        assert!(matches!(store.save_summary_rule(Some(99), "x.co", &of(SummaryKind::Short), NOV_1), Err(Error::NotFound { .. })));
        assert!(matches!(store.save_summary_rule(Some(99), "paydesk.example", &of(SummaryKind::Short), NOV_1), Err(Error::NotFound { .. })));
        let no_instruction = SummaryRequest { kind: SummaryKind::Custom, instruction: " ".into() };
        assert!(matches!(store.save_summary_rule(None, "x.co", &no_instruction, NOV_1), Err(Error::Invalid(m)) if m == "say how to summarise it"));
        assert!(matches!(store.save_summary_rule(None, "gmail.com", &of(SummaryKind::Short), NOV_1), Err(Error::Invalid(_))));
        assert!(store.remove_summary_rule(dana.id).unwrap());
        assert!(!store.remove_summary_rule(dana.id).unwrap());
        assert!(store.summary_rules().unwrap().is_empty());
    }

    #[test]
    fn a_rule_for_someone_you_have_mail_from_has_their_name() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").build());
        let daniel = store.save_summary_rule(None, "Daniel@Northwind.co", &of(SummaryKind::Short), NOV_1).unwrap();
        assert_eq!(daniel.name.as_deref(), Some("Daniel Okafor"));
        let domain = store.save_summary_rule(None, "northwind.co", &of(SummaryKind::Short), NOV_1).unwrap();
        let stranger = store.save_summary_rule(None, "sam@acme.co", &of(SummaryKind::Short), NOV_1).unwrap();
        assert_eq!((domain.name, stranger.name), (None, None));
        let names: Vec<Option<String>> = store.summary_rules().unwrap().into_iter().map(|r| r.name).collect();
        assert_eq!(names, [Some("Daniel Okafor".into()), None, None]);
        assert_eq!(store.summary_rule_for(&a.thread_id).unwrap().unwrap().name.as_deref(), Some("Daniel Okafor"));
    }

    #[test]
    fn the_most_specific_rule_for_the_latest_sender_applies() {
        let mut store = store_with_work();
        let a = ingest(&mut store, "1:1", &mail().id("a@x").from("Dana <dana@mail.Acme.co>").build());
        assert_eq!(store.summary_rule_for(&a.thread_id).unwrap(), None);
        store.save_summary_rule(None, "co", &of(SummaryKind::Short), NOV_1).unwrap_err();
        store.save_summary_rule(None, "acme.co", &of(SummaryKind::Facts), NOV_1).unwrap();
        store.save_summary_rule(None, "notacme.co", &of(SummaryKind::Short), NOV_1).unwrap();
        assert_eq!(store.summary_rule_for(&a.thread_id).unwrap().unwrap().sender, "acme.co", "a subdomain");
        store.save_summary_rule(None, "mail.acme.co", &of(SummaryKind::Detailed), NOV_1).unwrap();
        assert_eq!(store.summary_rule_for(&a.thread_id).unwrap().unwrap().sender, "mail.acme.co");
        store.save_summary_rule(None, "dana@mail.acme.co", &of(SummaryKind::Actions), NOV_1).unwrap();
        assert_eq!(store.summary_rule_for(&a.thread_id).unwrap().unwrap().sender, "dana@mail.acme.co");
        // Your own reply doesn't change whose rule it is.
        let mine = mail().id("b@x").from("maya@northwind.co").subject("Re: Q4 budget").reply_to(&["a@x"]).build();
        store.ingest(&mine, &MessageSource { mailbox: "Sent".into(), ..source("2:1") }).unwrap();
        assert_eq!(store.summary_rule_for(&a.thread_id).unwrap().unwrap().request.kind, SummaryKind::Actions);
        // An address rule doesn't catch another address that ends the same way.
        let b = ingest(&mut store, "1:3", &mail().id("c@x").subject("Other").from("x.dana@mail.acme.co").build());
        assert_eq!(store.summary_rule_for(&b.thread_id).unwrap().unwrap().sender, "mail.acme.co");
        assert_eq!(store.summary_rule_for("nope").unwrap(), None);
    }

    #[test]
    fn new_mail_from_a_rules_sender_waits_for_its_summary() {
        let mut store = store_with_work();
        let old = ingest(&mut store, "1:1", &mail().id("old@x").from("billing@paydesk.example").subject("Old invoice").build());
        let rule = store.save_summary_rule(None, "paydesk.example", &of(SummaryKind::Facts), NOV_1).unwrap();
        assert!(store.to_summarize_by_rule("e4b", 10).unwrap().is_empty(), "mail from before the rule");
        let new = ingest(&mut store, "1:2", &mail().id("new@x").from("billing@paydesk.example").subject("Invoice").build());
        ingest(&mut store, "1:3", &mail().id("other@x").subject("Lunch").build());
        let waiting = store.to_summarize_by_rule("e4b", 10).unwrap();
        assert_eq!(waiting.len(), 1);
        let (c, found) = &waiting[0];
        assert_eq!((c.thread_id.to_string(), c.why, found), (new.thread_id.clone(), SummaryWhy::Rule, &rule));
        assert_eq!(c.through.to_string(), new.message_id);
        assert_eq!(c.messages.len(), 1);
        // Once written by this model, it waits no more, until a new message.
        let content = moss_core::SummaryContent::Facts { items: vec![] };
        store.keep_summary(c, &rule.request, "e4b", &content, NOV_1).unwrap();
        assert!(store.to_summarize_by_rule("e4b", 10).unwrap().is_empty());
        assert_eq!(store.to_summarize_by_rule("26b", 10).unwrap().len(), 1);
        // Changing the rule's kind asks again.
        store.save_summary_rule(None, "paydesk.example", &of(SummaryKind::Actions), NOV_1).unwrap();
        assert_eq!(store.to_summarize_by_rule("e4b", 10).unwrap().len(), 1);
        // Done or blocked mail isn't summarised.
        store.mark_done(&new.thread_id, true, NOV_1).unwrap();
        assert!(store.to_summarize_by_rule("e4b", 10).unwrap().is_empty());
        store.mark_done(&new.thread_id, false, NOV_1).unwrap();
        store.conn.execute("UPDATE messages SET blocked = 1", []).unwrap();
        assert!(store.to_summarize_by_rule("e4b", 10).unwrap().is_empty());
        let _ = old;
    }

    #[test]
    fn failures_are_errors() {
        let mut store = store_with_work();
        ingest(&mut store, "1:1", &mail().id("a@x").build());
        break_schema(&store, "DROP TABLE settings");
        assert!(store.automatic_summaries().is_err());
        assert!(store.set_automatic_summaries(false).is_err());
        let mut store = store_with_work();
        read_only(&store);
        assert!(store.save_summary_rule(None, "x.co", &of(SummaryKind::Short), NOV_1).is_err());
        assert!(store.save_summary_rule(Some(1), "x.co", &of(SummaryKind::Short), NOV_1).is_err());
        assert!(store.remove_summary_rule(1).is_err());
        assert!(store.set_automatic_summaries(true).is_err());
        break_schema(&store, "PRAGMA query_only = false");
        let rule = store.save_summary_rule(None, "x.co", &of(SummaryKind::Short), NOV_1).unwrap();
        break_schema(&store, "CREATE TRIGGER stuck BEFORE UPDATE ON summary_rules BEGIN SELECT RAISE(FAIL, 'disk full'); END");
        assert!(store.save_summary_rule(Some(rule.id), "x.co", &of(SummaryKind::Facts), NOV_1).is_err());
        break_schema(&store, "DROP TABLE summary_rules");
        assert!(store.summary_rules().is_err());
        assert!(store.summary_rule_for("1").is_err());
        assert!(store.to_summarize_by_rule("e4b", 1).is_err());
    }
}
