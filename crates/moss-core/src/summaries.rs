//! Summaries of a conversation, in the kind you pick, and the rules that write them for you.
//!
//! The writing model writes each kind with its own prompt (`moss_ai::summaries`). What comes
//! back is [`SummaryContent`]: plain text for Short, Detailed and Custom, a list of items for
//! Action points and Key facts, so the UI renders each its own way.

use serde::{Deserialize, Serialize};

/// What kind of summary to write.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SummaryKind {
    /// One or two lines: what it's about and where it stands.
    Short,
    /// The main points, in order, in about a fifth of the words (see `moss_ai::summaries`).
    Detailed,
    /// What you're asked to do, each with its date when there is one.
    Actions,
    /// People, dates, amounts and links.
    Facts,
    /// Your own instruction.
    Custom,
}

/// The longest custom instruction, in characters.
pub const MAX_INSTRUCTION: usize = 500;

/// A kind, and for Custom your instruction. Every other kind has none (an empty string).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct SummaryRequest {
    pub kind: SummaryKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instruction: String,
}

/// Why a request can't be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadRequest {
    /// Custom with nothing to go on.
    NoInstruction,
    /// Longer than [`MAX_INSTRUCTION`].
    TooLong,
}

impl std::fmt::Display for BadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BadRequest::NoInstruction => f.write_str("say how to summarise it"),
            BadRequest::TooLong => write!(f, "keep the instruction under {MAX_INSTRUCTION} characters"),
        }
    }
}

impl SummaryRequest {
    /// A request as the UI sends it: Custom needs an instruction, which is trimmed; other
    /// kinds drop theirs.
    pub fn new(kind: SummaryKind, instruction: &str) -> Result<Self, BadRequest> {
        let instruction = instruction.trim();
        if kind != SummaryKind::Custom {
            return Ok(Self { kind, instruction: String::new() });
        }
        if instruction.is_empty() {
            return Err(BadRequest::NoInstruction);
        }
        if instruction.chars().count() > MAX_INSTRUCTION {
            return Err(BadRequest::TooLong);
        }
        Ok(Self { kind, instruction: instruction.to_string() })
    }

    /// The kind with no instruction.
    pub fn of(kind: SummaryKind) -> Self {
        Self { kind, instruction: String::new() }
    }

    /// Checks one that came from outside (the UI, the database) as [`SummaryRequest::new`] does.
    pub fn checked(self) -> Result<Self, BadRequest> {
        Self::new(self.kind, &self.instruction)
    }
}

impl SummaryKind {
    /// The name the database and the UI use: "short", "detailed", "actions", "facts", "custom".
    pub fn name(self) -> &'static str {
        match self {
            SummaryKind::Short => "short",
            SummaryKind::Detailed => "detailed",
            SummaryKind::Actions => "actions",
            SummaryKind::Facts => "facts",
            SummaryKind::Custom => "custom",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Short, Self::Detailed, Self::Actions, Self::Facts, Self::Custom]
            .into_iter()
            .find(|k| k.name() == name)
    }
}

/// A summary as the UI renders it, tagged by `format`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "format", rename_all = "camelCase")]
pub enum SummaryContent {
    /// Short, Detailed and Custom: text to read. Paragraphs are split by a blank line; separate
    /// lines are separate points.
    Text { text: String },
    /// Action points, most pressing first. None means nothing is asked of you.
    Actions { items: Vec<ActionItem> },
    /// Key facts, in the order the model found them.
    Facts { items: Vec<Fact> },
}

/// One thing to do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    /// Starts with a verb: "Send Daniel the testing plan".
    pub text: String,
    /// When by, as the email says it: "Friday, 13 November".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
}

/// One fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub kind: FactKind,
    /// The fact itself: "Daniel Okafor", "€18,400", "https://…".
    pub value: String,
    /// What it is: "Northwind's project lead", "new Q4 budget total".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactKind {
    Person,
    Date,
    Amount,
    Link,
    Other,
}

/// Why a summary is there, for its label.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SummaryWhy {
    /// You asked for it.
    Asked,
    /// A rule for its sender wrote it.
    Rule,
    /// Automatic: the conversation is in Important.
    Important,
    /// Automatic: you've written in the conversation.
    Replied,
}

impl SummaryWhy {
    pub fn name(self) -> &'static str {
        match self {
            SummaryWhy::Asked => "asked",
            SummaryWhy::Rule => "rule",
            SummaryWhy::Important => "important",
            SummaryWhy::Replied => "replied",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Asked, Self::Rule, Self::Important, Self::Replied].into_iter().find(|w| w.name() == name)
    }
}

/// A summary of a conversation, written or kept.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryView {
    #[serde(flatten)]
    pub request: SummaryRequest,
    pub content: SummaryContent,
    pub why: SummaryWhy,
    /// The writing model that wrote it.
    pub model: String,
    /// When (ms).
    pub made_at: i64,
    /// A message came after it was written: show it, and offer to write it again.
    pub stale: bool,
}

/// "Always summarise mail from `sender` as `kind`": an address or a whole domain (and its
/// subdomains), in every account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryRule {
    pub id: i64,
    /// Lower-case: "dana@acme.co" or "acme.co".
    pub sender: String,
    #[serde(flatten)]
    pub request: SummaryRequest,
    pub created_at: i64,
    /// The person's name, for an address Moss has seen mail from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The summary kept for a conversation, and the rule that applies to it, for the thread
/// view: shown at once, no model needed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummaries {
    /// By the current writing model: the kind you chose last, else the automatic one-liner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SummaryView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<SummaryRule>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn custom_needs_an_instruction_and_other_kinds_drop_theirs() {
        assert_eq!(
            SummaryRequest::new(SummaryKind::Facts, "ignored").unwrap(),
            SummaryRequest::of(SummaryKind::Facts)
        );
        assert_eq!(
            SummaryRequest::new(SummaryKind::Custom, "  Only the prices ").unwrap().instruction,
            "Only the prices"
        );
        assert_eq!(SummaryRequest::new(SummaryKind::Custom, "  "), Err(BadRequest::NoInstruction));
        let long = "é".repeat(MAX_INSTRUCTION + 1);
        assert_eq!(SummaryRequest::new(SummaryKind::Custom, &long), Err(BadRequest::TooLong));
        assert!(SummaryRequest::new(SummaryKind::Custom, &"é".repeat(MAX_INSTRUCTION)).is_ok());
        let sent = SummaryRequest { kind: SummaryKind::Custom, instruction: String::new() };
        assert_eq!(sent.checked(), Err(BadRequest::NoInstruction));
        assert_eq!(BadRequest::NoInstruction.to_string(), "say how to summarise it");
        assert_eq!(BadRequest::TooLong.to_string(), "keep the instruction under 500 characters");
    }

    #[test]
    fn kinds_and_reasons_have_names_both_ways() {
        for kind in [SummaryKind::Short, SummaryKind::Detailed, SummaryKind::Actions, SummaryKind::Facts, SummaryKind::Custom] {
            assert_eq!(SummaryKind::from_name(kind.name()), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.name()));
        }
        assert_eq!(SummaryKind::from_name("long"), None);
        for why in [SummaryWhy::Asked, SummaryWhy::Rule, SummaryWhy::Important, SummaryWhy::Replied] {
            assert_eq!(SummaryWhy::from_name(why.name()), Some(why));
            assert_eq!(serde_json::to_value(why).unwrap(), json!(why.name()));
        }
        assert_eq!(SummaryWhy::from_name("because"), None);
    }

    #[test]
    fn summaries_reach_the_ui_flat_and_tagged_by_format() {
        let view = SummaryView {
            request: SummaryRequest::of(SummaryKind::Actions),
            content: SummaryContent::Actions {
                items: vec![
                    ActionItem { text: "Send the testing plan".into(), due: Some("Friday, 13 November".into()) },
                    ActionItem { text: "Approve the budget".into(), due: None },
                ],
            },
            why: SummaryWhy::Rule,
            model: "e4b".into(),
            made_at: 5,
            stale: false,
        };
        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(
            value,
            json!({
                "kind": "actions",
                "content": {"format": "actions", "items": [
                    {"text": "Send the testing plan", "due": "Friday, 13 November"},
                    {"text": "Approve the budget"}
                ]},
                "why": "rule", "model": "e4b", "madeAt": 5, "stale": false
            })
        );
        assert_eq!(serde_json::from_value::<SummaryView>(value).unwrap(), view);
        let facts = SummaryContent::Facts {
            items: vec![Fact { kind: FactKind::Amount, value: "€18,400".into(), detail: Some("new total".into()) }],
        };
        assert_eq!(
            serde_json::to_value(&facts).unwrap(),
            json!({"format": "facts", "items": [{"kind": "amount", "value": "€18,400", "detail": "new total"}]})
        );
        let text = SummaryContent::Text { text: "Hi".into() };
        assert_eq!(serde_json::to_value(&text).unwrap(), json!({"format": "text", "text": "Hi"}));

    }

    #[test]
    fn rules_and_custom_requests_carry_their_instruction() {
        let rule = SummaryRule {
            id: 3,
            sender: "acme.co".into(),
            request: SummaryRequest::new(SummaryKind::Custom, "Only prices").unwrap(),
            created_at: 9,
            name: None,
        };
        let value = serde_json::to_value(&rule).unwrap();
        assert_eq!(
            value,
            json!({"id": 3, "sender": "acme.co", "kind": "custom", "instruction": "Only prices", "createdAt": 9})
        );
        let named = SummaryRule { name: Some("Dana Whitfield".into()), ..rule.clone() };
        assert_eq!(serde_json::to_value(&named).unwrap()["name"], "Dana Whitfield");
        assert_eq!(serde_json::from_value::<SummaryRule>(value).unwrap(), rule);
        let none = ThreadSummaries::default();
        assert_eq!(serde_json::to_value(&none).unwrap(), json!({}));
        let sent: SummaryRequest = serde_json::from_value(json!({"kind": "short"})).unwrap();
        assert_eq!(sent, SummaryRequest::of(SummaryKind::Short));
    }
}
