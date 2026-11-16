use std::time::Duration;

/// Everything that can go wrong talking to a mail server.
///
/// Messages never contain passwords, tokens or API keys.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server refused the password, token or API key.
    #[error("sign-in refused: {0}")]
    Auth(String),
    /// Couldn't reach the server, or the connection dropped. Retrying later may work.
    #[error("connection failed: {0}")]
    Network(String),
    /// Submission started but acceptance could not be established. Never retry automatically.
    #[error("delivery may have succeeded: {0}")]
    DeliveryUnknown(String),
    /// The server said something we didn't expect, or rejected a command.
    #[error("server error: {0}")]
    Protocol(String),
    /// Too many requests. Wait at least this long before trying again.
    #[error("rate limited, retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },
    /// The mailbox or message doesn't exist (any more).
    #[error("not found: {0}")]
    NotFound(String),
    /// The checkpoint belongs to a different provider or is malformed.
    #[error("checkpoint doesn't belong to this provider")]
    InvalidCheckpoint,
    /// This provider can't do that (for example archive with no Archive folder).
    #[error("not supported: {0}")]
    Unsupported(String),
    /// The message to send is incomplete or malformed.
    #[error("invalid message: {0}")]
    InvalidMessage(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        // I/O errors may wrap parser buffers or external response text.
        Error::Network("The connection failed.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_are_network_errors() {
        let e: Error = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "synthetic-private-io-marker").into();
        assert!(matches!(&e, Error::Network(_)));
        assert!(!format!("{e:?} {e}").contains("synthetic-private-io-marker"));
    }

    #[test]
    fn messages_read_well() {
        let e = Error::RateLimited {
            retry_after: Duration::from_secs(2),
        };
        assert_eq!(e.to_string(), "rate limited, retry after 2s");
        assert_eq!(
            Error::InvalidCheckpoint.to_string(),
            "checkpoint doesn't belong to this provider"
        );
    }
}
