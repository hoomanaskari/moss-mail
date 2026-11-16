//! Building a provider for a saved account connection, and checking a password works
//! before it is saved.

use std::sync::Arc;

use moss_accounts::{
    AccountConnection, BoxFuture, LoginCheck, LoginFailure, MailService, Secret, Security,
};
use moss_providers::{
    Credentials, ImapConfig, ImapProvider, Provider, ResendProvider, SmtpConfig, TlsRoots,
};

use crate::Error;

/// The secret a provider signs in with, ready to use (OAuth tokens already refreshed).
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    Password(String),
    Token(String),
    ApiKey(String),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Credential::Password(_) => "Credential::Password(<redacted>)",
            Credential::Token(_) => "Credential::Token(<redacted>)",
            Credential::ApiKey(_) => "Credential::ApiKey(<redacted>)",
        })
    }
}

/// Makes providers. The app uses [`NetConnector`]; tests use fakes.
pub trait Connector: Send + Sync {
    /// Builds the provider for a connection. Doesn't touch the network yet.
    fn provider(
        &self,
        connection: &AccountConnection,
        credential: &Credential,
    ) -> Result<Arc<dyn Provider>, Error>;
}

/// Real mail servers: IMAP and SMTP over TLS, or Resend's API.
#[derive(Debug, Clone, Default)]
pub struct NetConnector {
    /// Which certificates to trust. The OS store, except in end-to-end tests.
    pub roots: TlsRoots,
}

fn security(s: Security) -> moss_providers::Security {
    match s {
        Security::Tls => moss_providers::Security::Tls,
        Security::StartTls => moss_providers::Security::StartTls,
    }
}

impl Connector for NetConnector {
    fn provider(
        &self,
        connection: &AccountConnection,
        credential: &Credential,
    ) -> Result<Arc<dyn Provider>, Error> {
        if let Credential::ApiKey(key) = credential {
            let provider = ResendProvider::new(key.as_str().into())
                .map_err(|e| Error::Server(e.to_string()))?;
            return Ok(Arc::new(provider));
        }
        let servers = connection
            .servers
            .as_ref()
            .ok_or_else(|| Error::Server("this account has no server settings".into()))?;
        let login = |username: &str| match credential {
            Credential::Token(token) => Credentials::OAuth2 {
                username: username.into(),
                access_token: token.as_str().into(),
            },
            Credential::Password(password) | Credential::ApiKey(password) => {
                Credentials::Password {
                    username: username.into(),
                    password: password.as_str().into(),
                }
            }
        };
        let imap = ImapConfig {
            host: servers.imap.host.clone(),
            port: servers.imap.port,
            security: security(servers.imap.security),
            credentials: login(&servers.imap.username),
            tls_roots: self.roots.clone(),
        };
        let smtp = SmtpConfig {
            host: servers.smtp.host.clone(),
            port: servers.smtp.port,
            security: security(servers.smtp.security),
            credentials: login(&servers.smtp.username),
            tls_roots: self.roots.clone(),
        };
        // Gmail files sent mail itself; appending a copy would show it twice.
        let save_sent = connection.service != MailService::Gmail;
        Ok(Arc::new(ImapProvider::new(imap, smtp).save_sent_copies(save_sent)))
    }
}

/// Signs in to the server (by listing its folders) before a password is saved.
pub struct ProviderLogin {
    pub connector: Arc<dyn Connector>,
}

impl LoginCheck for ProviderLogin {
    fn check<'a>(
        &'a self,
        connection: &'a AccountConnection,
        secret: &'a Secret,
    ) -> BoxFuture<'a, Result<(), LoginFailure>> {
        Box::pin(async move {
            let host = connection.servers.as_ref().map(|s| s.imap.host.clone()).unwrap_or_default();
            let credential = match secret {
                Secret::Password { password } => Credential::Password(password.clone()),
                Secret::ApiKey { key } => Credential::ApiKey(key.clone()),
                Secret::OAuth { tokens, .. } => Credential::Token(tokens.access_token.clone()),
            };
            let provider = self.connector.provider(connection, &credential).map_err(|e| {
                LoginFailure::Unreachable { host: host.clone(), detail: e.to_string() }
            })?;
            match provider.list_mailboxes().await {
                Ok(_) => Ok(()),
                Err(moss_providers::Error::Auth(message)) => {
                    Err(LoginFailure::Rejected { message })
                }
                Err(e) => Err(LoginFailure::Unreachable { host, detail: e.to_string() }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeConnector, FakeProvider};
    use moss_accounts::{AuthMethod, SecretRef, Server, ServerSettings, TokenSet};
    use moss_core::ProviderKind;

    fn connection(service: MailService) -> AccountConnection {
        AccountConnection {
            email: "maya@gmail.com".into(),
            kind: ProviderKind::Gmail,
            service,
            servers: service.known_servers("maya@gmail.com"),
            auth: AuthMethod::Password,
            secret: SecretRef::for_account("maya@gmail.com", "gmail"),
        }
    }

    #[test]
    fn credentials_never_show_their_secret() {
        let all = [
            Credential::Password("p1".into()),
            Credential::Token("t1".into()),
            Credential::ApiKey("k1".into()),
        ];
        let debug = format!("{all:?}");
        assert!(!debug.contains("p1") && !debug.contains("t1") && !debug.contains("k1"), "{debug}");
        assert!(debug.contains("Token"));
    }

    #[tokio::test]
    async fn builds_imap_and_resend_providers_without_connecting() {
        let net = NetConnector::default();
        let gmail =
            net.provider(&connection(MailService::Gmail), &Credential::Password("x".into()));
        assert!(gmail.unwrap().capabilities().server_flags);
        let mut icloud = connection(MailService::ICloud);
        icloud.servers = Some(ServerSettings {
            imap: Server::new("imap.mail.me.com", 993, Security::Tls, "m"),
            smtp: Server::new("smtp.mail.me.com", 587, Security::StartTls, "m"),
        });
        assert!(net.provider(&icloud, &Credential::Token("t".into())).is_ok());
        let resend =
            net.provider(&connection(MailService::Resend), &Credential::ApiKey("re_x".into()));
        assert!(!resend.unwrap().capabilities().server_flags);
        let mut bare = connection(MailService::Imap);
        bare.servers = None;
        let err = net.provider(&bare, &Credential::Password("x".into())).err().unwrap();
        assert!(matches!(err, Error::Server(_)));
    }

    #[tokio::test]
    async fn an_unreachable_server_fails_the_login_check_without_a_panic() {
        let login = ProviderLogin { connector: Arc::new(NetConnector::default()) };
        let mut conn = connection(MailService::Imap);
        conn.servers = Some(ServerSettings {
            imap: Server::new("127.0.0.1", 1, Security::Tls, "m"),
            smtp: Server::new("127.0.0.1", 1, Security::Tls, "m"),
        });
        let failure =
            login.check(&conn, &Secret::Password { password: "x".into() }).await.unwrap_err();
        assert!(
            matches!(failure, LoginFailure::Unreachable { ref host, .. } if host == "127.0.0.1")
        );
    }

    #[tokio::test]
    async fn the_login_check_reports_what_the_server_said() {
        let fake = FakeProvider::new();
        let connector = Arc::new(FakeConnector::new(fake.clone()));
        let login = ProviderLogin { connector: connector.clone() };
        let conn = connection(MailService::Gmail);
        let password = Secret::Password { password: "pw".into() };
        assert_eq!(login.check(&conn, &password).await, Ok(()));
        assert_eq!(connector.last_credential(), Some(Credential::Password("pw".into())));

        fake.fail_next(moss_providers::Error::Auth(
            "[ALERT] Application-specific password required".into(),
        ));
        let failure = login.check(&conn, &password).await.unwrap_err();
        assert_eq!(
            failure,
            LoginFailure::Rejected {
                message: "[ALERT] Application-specific password required".into()
            }
        );

        fake.fail_next(moss_providers::Error::Network("refused".into()));
        let failure = login.check(&conn, &Secret::ApiKey { key: "k".into() }).await.unwrap_err();
        assert!(
            matches!(failure, LoginFailure::Unreachable { ref host, .. } if host == "imap.gmail.com")
        );
        assert_eq!(connector.last_credential(), Some(Credential::ApiKey("k".into())));

        let tokens = TokenSet {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: 0,
            refresh_issued_at: 0,
        };
        let mut no_servers = conn.clone();
        no_servers.servers = None;
        assert_eq!(
            login.check(&no_servers, &Secret::OAuth { tokens, client_secret: None }).await,
            Ok(())
        );
        assert_eq!(connector.last_credential(), Some(Credential::Token("tok".into())));

        connector.refuse();
        let failure = login.check(&conn, &password).await.unwrap_err();
        assert!(matches!(failure, LoginFailure::Unreachable { .. }));
    }
}
