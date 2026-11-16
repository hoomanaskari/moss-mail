//! What older versions of Moss left behind that it no longer uses.

use std::path::Path;

use moss_accounts::{SecretRef, SecretStore, KEYCHAIN_SERVICE};

/// The Keychain items older versions kept API keys for other AIs in.
const OTHER_AI_KEYS: [&str; 5] = [
    "ai (anthropic)",
    "ai (openai)",
    "ai (google)",
    "ai (ollama-cloud)",
    "ai (openrouter)",
];

/// Moss writes with its own models only. A Moss that could write with another AI (Claude
/// Code, Codex, an API key, Ollama, OpenRouter) kept its choices in `<data_dir>/ai/other.json`
/// and its keys in the Keychain: both go, once. Without that file nothing is touched, so the
/// Keychain isn't asked on every start.
pub fn forget_other_ais(data_dir: &Path, secrets: &dyn SecretStore) {
    let file = data_dir.join("ai").join("other.json");
    if !file.exists() {
        return;
    }
    for account in OTHER_AI_KEYS {
        let _ = secrets.delete(&SecretRef {
            service: KEYCHAIN_SERVICE.into(),
            account: account.into(),
        });
    }
    let _ = std::fs::remove_file(file);
}

#[cfg(test)]
mod tests {
    use moss_accounts::{MemorySecretStore, Secret};

    use super::*;

    fn item(account: &str) -> SecretRef {
        SecretRef {
            service: KEYCHAIN_SERVICE.into(),
            account: account.into(),
        }
    }

    fn key(k: &str) -> Secret {
        Secret::ApiKey { key: k.into() }
    }

    #[test]
    fn an_old_other_ai_file_and_its_keys_go_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ai")).unwrap();
        std::fs::write(
            dir.path().join("ai/other.json"),
            r#"{"using":"claude","connections":{"claude":{"model":"sonnet"}}}"#,
        )
        .unwrap();
        let secrets = MemorySecretStore::new();
        for account in OTHER_AI_KEYS {
            secrets.save(&item(account), &key("sk-old")).unwrap();
        }
        // A mail account's key stays.
        secrets
            .save(&item("resend (me@example.com)"), &key("re-key"))
            .unwrap();

        forget_other_ais(dir.path(), &secrets);

        assert!(!dir.path().join("ai/other.json").exists());
        for account in OTHER_AI_KEYS {
            assert_eq!(secrets.load(&item(account)).unwrap(), None, "{account}");
        }
        assert_eq!(
            secrets.load(&item("resend (me@example.com)")).unwrap(),
            Some(key("re-key"))
        );
    }

    #[test]
    fn without_the_file_the_keychain_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecretStore::new();
        secrets.save(&item("ai (anthropic)"), &key("sk")).unwrap();
        forget_other_ais(dir.path(), &secrets);
        assert_eq!(
            secrets.load(&item("ai (anthropic)")).unwrap(),
            Some(key("sk"))
        );
    }
}
