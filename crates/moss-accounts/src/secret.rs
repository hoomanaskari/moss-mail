//! Secrets and where they live.
//!
//! A [`Secret`] is a password, OAuth token set or API key. It only ever goes into a
//! [`SecretStore`] (the OS keychain in the app), never into the database or logs. Its
//! `Debug` output is redacted so a stray `{:?}` can't leak it.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Keychain service name for every Moss item. Matches the app's bundle identifier.
pub const KEYCHAIN_SERVICE: &str = "app.moss.mail";

/// Names a secret in the keychain. Safe to store and show: it holds no secret itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    pub service: String,
    pub account: String,
}

impl SecretRef {
    /// A new sign-in owns a distinct item, so retiring an older connection cannot
    /// delete the replacement. Existing serialized references remain readable.
    pub(crate) fn fresh_for_account(login: &str, kind: &str) -> Result<Self, SecretStoreError> {
        Self::with_random_generation(login, kind, getrandom::fill)
    }

    fn with_random_generation(
        login: &str,
        kind: &str,
        random: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
    ) -> Result<Self, SecretStoreError> {
        let mut bytes = [0u8; 16];
        random(&mut bytes).map_err(|_| SecretStoreError::Unavailable("a unique sign-in reference could not be created".into()))?;
        let generation: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            service: KEYCHAIN_SERVICE.into(),
            account: format!("{} ({kind}) [{generation}]", login.to_ascii_lowercase()),
        })
    }

    /// The legacy keychain item for one mail account, e.g. `maya@northwind.co (imap)`.
    pub fn for_account(login: &str, kind: &str) -> Self {
        Self {
            service: KEYCHAIN_SERVICE.into(),
            account: format!("{} ({kind})", login.to_ascii_lowercase()),
        }
    }
}

/// OAuth tokens for one account. Times are Unix seconds.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// When the access token stops working.
    pub expires_at: u64,
    /// When the current refresh token was issued. Google drops refresh tokens of apps in
    /// "Testing" after 7 days, so this tells a revoked token from an expired one.
    pub refresh_issued_at: u64,
}

impl TokenSet {
    /// True when the access token is expired or will be within a minute.
    pub fn needs_refresh(&self, now: u64) -> bool {
        now + 60 >= self.expires_at
    }
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .field("refresh_issued_at", &self.refresh_issued_at)
            .finish()
    }
}

/// Something that proves who you are to a mail server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Secret {
    /// A password or app password.
    Password { password: String },
    /// OAuth tokens, plus the client secret for Google clients that have one.
    #[serde(rename_all = "camelCase")]
    OAuth {
        tokens: TokenSet,
        client_secret: Option<String>,
    },
    /// An API key (Resend).
    ApiKey { key: String },
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Secret::Password { .. } => f.write_str("Secret::Password(<redacted>)"),
            Secret::OAuth { tokens, .. } => write!(f, "Secret::OAuth({tokens:?})"),
            Secret::ApiKey { .. } => f.write_str("Secret::ApiKey(<redacted>)"),
        }
    }
}

/// Why a secret couldn't be saved or read. Never contains the secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretStoreError {
    /// The keychain is locked, missing or refused access.
    #[error("the keychain is not available: {0}")]
    Unavailable(String),
    /// The stored item exists but couldn't be decoded.
    #[error("the stored secret is damaged: {0}")]
    Corrupt(String),
}

/// Where secrets live. The app uses the OS keychain ([`crate::KeyringSecretStore`]),
/// or an encrypted file on Linux without a Secret Service; tests use
/// [`MemorySecretStore`].
/// A failed mutation may already be visible when its durability could not be confirmed.
/// Callers must not assume an error rolled it back; retries must be idempotent.
pub trait SecretStore: Send + Sync {
    /// Saves `secret`, replacing any secret already at `key`.
    fn save(&self, key: &SecretRef, secret: &Secret) -> Result<(), SecretStoreError>;
    /// Reads the secret at `key`, or `None` when there isn't one.
    fn load(&self, key: &SecretRef) -> Result<Option<Secret>, SecretStoreError>;
    /// Removes the secret at `key`. Removing a missing secret is fine.
    fn delete(&self, key: &SecretRef) -> Result<(), SecretStoreError>;
}

/// Secrets kept in memory only. For tests, here and in other crates.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    items: Mutex<HashMap<SecretRef, Secret>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many secrets are stored.
    pub fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretStore for MemorySecretStore {
    fn save(&self, key: &SecretRef, secret: &Secret) -> Result<(), SecretStoreError> {
        self.items
            .lock()
            .unwrap()
            .insert(key.clone(), secret.clone());
        Ok(())
    }

    fn load(&self, key: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
        Ok(self.items.lock().unwrap().get(key).cloned())
    }

    fn delete(&self, key: &SecretRef) -> Result<(), SecretStoreError> {
        self.items.lock().unwrap().remove(key);
        Ok(())
    }
}

/// Seconds since the Unix epoch.
pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tokens() -> TokenSet {
        TokenSet {
            access_token: "access-123".into(),
            refresh_token: Some("refresh-456".into()),
            expires_at: 1_000,
            refresh_issued_at: 400,
        }
    }

    #[test]
    fn debug_output_never_shows_secrets() {
        let all = [
            Secret::Password {
                password: "hunter2".into(),
            },
            Secret::OAuth {
                tokens: tokens(),
                client_secret: Some("shh".into()),
            },
            Secret::ApiKey {
                key: "re_live_key".into(),
            },
        ];
        for secret in all {
            let shown = format!("{secret:?}");
            for leaked in ["hunter2", "access-123", "refresh-456", "shh", "re_live_key"] {
                assert!(!shown.contains(leaked), "{shown} leaks {leaked}");
            }
            assert!(shown.contains("<redacted>"));
        }
        let no_refresh = TokenSet {
            refresh_token: None,
            ..tokens()
        };
        assert!(format!("{no_refresh:?}").contains("refresh_token: None"));
    }

    #[test]
    fn secrets_round_trip_through_json() {
        let secret = Secret::OAuth {
            tokens: tokens(),
            client_secret: None,
        };
        let json = serde_json::to_value(&secret).unwrap();
        assert_eq!(json["type"], "oAuth");
        assert_eq!(json["tokens"]["refreshIssuedAt"], 400);
        assert_eq!(serde_json::from_value::<Secret>(json).unwrap(), secret);
    }

    #[test]
    fn refresh_is_due_a_minute_before_expiry() {
        let t = tokens();
        assert!(!t.needs_refresh(939));
        assert!(t.needs_refresh(940));
        assert!(t.needs_refresh(2_000));
    }

    #[test]
    fn account_refs_are_lowercase_and_name_the_kind() {
        let key = SecretRef::for_account("Maya@Northwind.co", "imap");
        assert_eq!(key.service, "app.moss.mail");
        assert_eq!(key.account, "maya@northwind.co (imap)");
    }

    #[test]
    fn memory_store_saves_replaces_and_deletes() {
        let store = MemorySecretStore::new();
        let key = SecretRef::for_account("a@b.c", "imap");
        assert!(store.is_empty());
        assert_eq!(store.load(&key).unwrap(), None);
        store
            .save(
                &key,
                &Secret::Password {
                    password: "one".into(),
                },
            )
            .unwrap();
        store
            .save(
                &key,
                &Secret::Password {
                    password: "two".into(),
                },
            )
            .unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.load(&key).unwrap(),
            Some(Secret::Password {
                password: "two".into()
            })
        );
        store.delete(&key).unwrap();
        store.delete(&key).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn now_is_after_2025() {
        assert!(now_unix() > 1_735_689_600);
    }

    #[test]
    fn store_errors_explain_without_secrets() {
        assert_eq!(
            SecretStoreError::Unavailable("locked".into()).to_string(),
            "the keychain is not available: locked"
        );
        assert_eq!(
            SecretStoreError::Corrupt("bad".into()).to_string(),
            "the stored secret is damaged: bad"
        );
    }
    #[test]
    fn new_sign_in_references_keep_all_random_bytes_and_fail_closed_without_entropy() {
        let reference = SecretRef::with_random_generation("MAYA@example.com", "imap", |bytes| {
            for (index, byte) in bytes.iter_mut().enumerate() { *byte = index as u8; }
            Ok(())
        }).unwrap();
        assert_eq!(reference.service, KEYCHAIN_SERVICE);
        assert_eq!(reference.account, "maya@example.com (imap) [000102030405060708090a0b0c0d0e0f]");
        assert!(matches!(SecretRef::with_random_generation("a@example.com", "imap", |_| Err(getrandom::Error::UNSUPPORTED)), Err(SecretStoreError::Unavailable(_))));
        assert_eq!(serde_json::from_str::<SecretRef>(&serde_json::to_string(&reference).unwrap()).unwrap(), reference);
    }

}
