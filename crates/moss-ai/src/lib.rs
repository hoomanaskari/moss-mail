//! Moss's on-device AI.
//!
//! Three models, three jobs (see `docs/research/ai-split.md`): a small embedding model
//! (EmbeddingGemma) says what each email means, so search finds mail by meaning; a decision
//! model sorts and reads mail; and a writing model (Gemma 4) writes. All run in llama.cpp's
//! `llama-server` on this Mac, except Apple Intelligence, which can write instead of Gemma on a
//! Mac that has it (see `apple`).

pub mod agent;
pub mod apple;
pub mod ask;
pub mod catalog;
pub mod chat;
pub mod decide;
pub mod download;
pub mod embed;
pub mod http;
pub mod machine;
pub mod manager;
pub mod power;
pub mod prompt;
pub mod reply;
pub mod server;
pub mod summaries;
#[cfg(test)]
mod test_mail;
pub mod tools;
pub mod unpack;

pub use machine::Machine;
pub use manager::{Ai, Download, Features, Parts, Plan, Status};

/// Why an AI job didn't run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    /// The model isn't downloaded yet.
    #[error("the AI model isn't downloaded yet")]
    NotReady,
    #[error(transparent)]
    Server(#[from] server::ServerError),
    /// The model answered with an error or something Moss can't read.
    #[error("the AI model failed: {0}")]
    Model(String),
    /// No such writing model, or it needs more memory than this Mac has.
    #[error("That model needs more memory than this Mac has ({0}).")]
    Choice(String),
    #[error("can't save AI files: {0}")]
    Disk(String),
}

/// `{kind, message}` for the UI.
impl serde::Serialize for AiError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let kind = match self {
            AiError::NotReady => "notReady",
            AiError::Server(_) => "server",
            AiError::Model(_) => "model",
            AiError::Choice(_) => "choice",
            AiError::Disk(_) => "disk",
        };
        let mut out = serializer.serialize_struct("AiError", 2)?;
        out.serialize_field("kind", kind)?;
        out.serialize_field("message", &self.to_string())?;
        out.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_reach_the_ui_as_a_kind_and_a_message() {
        let json = |e: AiError| serde_json::to_value(e).unwrap();
        assert_eq!(
            json(AiError::NotReady),
            serde_json::json!({"kind": "notReady", "message": "the AI model isn't downloaded yet"})
        );
        assert_eq!(
            json(AiError::Server(server::ServerError::Timeout))["kind"],
            "server"
        );
        assert_eq!(
            json(AiError::Model("x".into()))["message"],
            "the AI model failed: x"
        );
        assert_eq!(json(AiError::Choice("26b".into()))["kind"], "choice");
        assert_eq!(json(AiError::Disk("full".into()))["kind"], "disk");
    }
}
