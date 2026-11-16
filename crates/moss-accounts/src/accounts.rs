//! Signing in: the one entry point the app holds.
//!
//! Every path ends the same way: the secret goes into the keychain and the caller gets
//! an [`AccountConnection`]. A password is only saved after the server accepted it.

use std::sync::Arc;
use std::time::Duration;

use crate::connection::{
    domain_of, AccountConnection, AuthMethod, MailService, OAuthIssuer, ServerSettings,
};
use crate::discovery::{
    discover, hosting_service, BoxFuture, Discovered, DiscoveryEndpoints, DnsMx, MxLookup,
};
use crate::error::{GmailAppPasswordProblem, SignInError};
use crate::oauth::{refresh, OAuthConfig, OAuthEndpoints, PendingAuthorization};
use crate::resend::{check_key, ResendDomain, RESEND_API};
use crate::send_as::{send_as, SendAs, GMAIL_API};
use crate::secret::{now_unix, Secret, SecretRef, SecretStore};

/// Every remote endpoint the sign-in code talks to. Tests point them at mock servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub oauth: OAuthEndpoints,
    pub discovery: DiscoveryEndpoints,
    pub resend_api: String,
    /// Gmail's REST API, for the "Send mail as" list.
    pub gmail_api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            oauth: OAuthEndpoints::default(),
            discovery: DiscoveryEndpoints::default(),
            resend_api: RESEND_API.into(),
            gmail_api: GMAIL_API.into(),
        }
    }
}

/// A password sign-in: Gmail or iCloud app password, Yahoo, Fastmail or any IMAP server.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordSignIn {
    pub service: MailService,
    pub email: String,
    pub password: String,
    /// Settings the user typed in. `None` uses the built-in ones or discovers them.
    pub servers: Option<ServerSettings>,
    /// Who to log in as when that isn't `email`: an iCloud custom domain logs in with
    /// the account's own @icloud.com address. `None` logs in as `email`.
    pub login: Option<String>,
}

impl std::fmt::Debug for PasswordSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordSignIn")
            .field("service", &self.service)
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("servers", &self.servers)
            .field("login", &self.login)
            .finish()
    }
}

/// Why the mail server didn't let a login through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginFailure {
    /// The server said no. `message` is its reply, e.g. Gmail's
    /// `[ALERT] Application-specific password required`.
    Rejected { message: String },
    /// The server couldn't be reached.
    Unreachable { host: String, detail: String },
}

/// Tries a login before anything is saved. The provider crate implements this with a
/// real IMAP login; tests use a fake.
pub trait LoginCheck: Send + Sync {
    fn check<'a>(
        &'a self,
        connection: &'a AccountConnection,
        secret: &'a Secret,
    ) -> BoxFuture<'a, Result<(), LoginFailure>>;
}

/// Turns a failed login into the error screen to show.
pub fn classify_login_failure(service: MailService, failure: LoginFailure) -> SignInError {
    match failure {
        LoginFailure::Unreachable { host, detail } => SignInError::Unreachable { host, detail },
        LoginFailure::Rejected { message } => {
            let message = message.to_ascii_lowercase();
            if service == MailService::Gmail
                && message.contains("application-specific password required")
            {
                SignInError::GmailAppPassword {
                    problem: GmailAppPasswordProblem::AppPasswordRequired,
                }
            } else {
                SignInError::WrongPassword { service }
            }
        }
    }
}

/// A Gmail app password without its spaces, or why it isn't one. Google shows them as
/// four groups of four letters.
pub fn normalize_gmail_app_password(password: &str) -> Result<String, SignInError> {
    let compact: String = password.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() == 16 && compact.chars().all(|c| c.is_ascii_alphabetic()) {
        Ok(compact.to_ascii_lowercase())
    } else {
        Err(SignInError::GmailAppPassword {
            problem: GmailAppPasswordProblem::NotAnAppPassword,
        })
    }
}

/// Reaching a sign-in or settings server; one whole request, answer included. These are
/// small calls, so a slow server fails soon rather than holding up sign-in or sync.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Moss/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("the HTTP client builds with default TLS")
}

/// Signs accounts in and hands out their credentials.
pub struct Accounts {
    /// Built on first use (see [`Accounts::http`]).
    http: tokio::sync::OnceCell<reqwest::Client>,
    store: Arc<dyn SecretStore>,
    endpoints: Endpoints,
    mx: Arc<dyn MxLookup>,
    oauth_timeout: Duration,
}

impl Accounts {
    /// The real thing: public endpoints and the system's DNS.
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self::with(store, Endpoints::default(), Arc::new(DnsMx::system()))
    }

    pub fn with(store: Arc<dyn SecretStore>, endpoints: Endpoints, mx: Arc<dyn MxLookup>) -> Self {
        Self {
            http: tokio::sync::OnceCell::new(),
            store,
            endpoints,
            mx,
            oauth_timeout: Duration::from_secs(10 * 60),
        }
    }

    /// The client for sign-in, token refresh and settings lookups, built the first time one
    /// is needed, on a blocking thread: building it reads the system's proxy settings and
    /// trust store, which can take seconds, so never while the app starts.
    async fn http(&self) -> &reqwest::Client {
        self.http
            .get_or_init(|| async { tokio::task::spawn_blocking(client).await.expect("building the HTTP client never panics") })
            .await
    }

    /// How long to wait for the user to finish in the browser. Ten minutes by default.
    pub fn with_oauth_timeout(mut self, timeout: Duration) -> Self {
        self.oauth_timeout = timeout;
        self
    }

    /// Finds IMAP and SMTP settings for an address.
    pub async fn discover(&self, email: &str) -> Result<Discovered, SignInError> {
        discover(
            self.http().await,
            &self.endpoints.discovery,
            self.mx.as_ref(),
            email,
        )
        .await
    }

    /// Signs in with a password or app password. Checks the login with `check` and only
    /// then saves the password.
    pub async fn sign_in_with_password(
        &self,
        request: PasswordSignIn,
        check: &dyn LoginCheck,
    ) -> Result<AccountConnection, SignInError> {
        let email = request.email.trim().to_string();
        domain_of(&email).ok_or(SignInError::InvalidEmail)?;
        let (service, mut servers) = match (request.servers, request.service.known_servers(&email)) {
            (Some(servers), _) | (None, Some(servers)) => (request.service, servers),
            (None, None) if request.service == MailService::Imap => {
                // A custom domain may turn out to be Google Workspace or Microsoft 365.
                let found = self.discover(&email).await?;
                (found.service, found.servers)
            }
            (None, None) => {
                return Err(SignInError::PasswordNotSupported {
                    service: request.service,
                })
            }
        };
        match request.login.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            Some(login) => {
                servers.imap.username = login.into();
                servers.smtp.username = login.into();
            }
            // iCloud refuses a custom-domain address as the login.
            None if service == MailService::ICloud
                && MailService::for_address(&email) != MailService::ICloud =>
            {
                return Err(SignInError::ICloudAddressNeeded)
            }
            None => {}
        }
        let password = match service {
            MailService::Outlook | MailService::Resend => {
                return Err(SignInError::PasswordNotSupported { service })
            }
            MailService::Gmail => normalize_gmail_app_password(&request.password)?,
            _ if request.password.is_empty() => return Err(SignInError::WrongPassword { service }),
            _ => request.password,
        };
        let connection = AccountConnection {
            secret: SecretRef::fresh_for_account(&email, service.slug())?,
            email,
            kind: service.kind(),
            service,
            servers: Some(servers),
            auth: AuthMethod::Password,
        };
        let secret = Secret::Password { password };
        check
            .check(&connection, &secret)
            .await
            .map_err(|f| classify_login_failure(service, f))?;
        self.store.save(&connection.secret, &secret)?;
        Ok(connection)
    }

    /// Starts the Outlook / Microsoft 365 browser sign-in, with the user's own client ID
    /// or Moss's. Open [`PendingAuthorization::authorize_url`], then call
    /// [`finish_oauth`](Self::finish_oauth). `email`, when known, pre-fills the sign-in page.
    pub async fn start_outlook(
        &self,
        email: Option<&str>,
        own_client_id: Option<&str>,
    ) -> Result<PendingAuthorization, SignInError> {
        let hint = login_hint(email)?;
        let config = OAuthConfig::microsoft(own_client_id, &self.endpoints.oauth)?;
        PendingAuthorization::start(config, hint).await
    }

    /// Starts the Gmail browser sign-in with the user's own Google OAuth client.
    pub async fn start_gmail_oauth(
        &self,
        email: Option<&str>,
        client_id: &str,
        client_secret: Option<&str>,
    ) -> Result<PendingAuthorization, SignInError> {
        let hint = login_hint(email)?;
        if client_id.trim().is_empty() {
            return Err(SignInError::InvalidOAuthClient {
                detail: "the client ID is empty".into(),
            });
        }
        let config = OAuthConfig::google(client_id, client_secret, &self.endpoints.oauth);
        PendingAuthorization::start(config, hint).await
    }

    /// Waits for the browser sign-in to finish and saves the tokens. The account is the one
    /// that signed in, as the provider names it; `email` (what was typed) only when it doesn't.
    pub async fn finish_oauth(
        &self,
        pending: PendingAuthorization,
        email: Option<&str>,
    ) -> Result<AccountConnection, SignInError> {
        let config = pending.config().clone();
        let host = config.token_host();
        let signed_in = pending.finish(self.http().await, self.oauth_timeout).await?;
        let email = signed_in
            .address
            .or_else(|| email.map(|e| e.trim().to_string()))
            .ok_or_else(|| SignInError::UnexpectedResponse {
                host,
                detail: "the sign-in didn't say which account it was".into(),
            })?;
        let tokens = signed_in.tokens;
        let service = match config.issuer {
            OAuthIssuer::Microsoft => MailService::Outlook,
            OAuthIssuer::Google => MailService::Gmail,
        };
        let connection = AccountConnection {
            secret: SecretRef::fresh_for_account(&email, service.slug())?,
            servers: service.known_servers(&email),
            email,
            kind: service.kind(),
            service,
            auth: AuthMethod::OAuth2 {
                issuer: config.issuer,
                client_id: config.client_id,
            },
        };
        self.store.save(
            &connection.secret,
            &Secret::OAuth {
                tokens,
                client_secret: config.client_secret,
            },
        )?;
        Ok(connection)
    }

    /// A working OAuth access token for IMAP/SMTP `XOAUTH2`, refreshed and re-saved
    /// when it has expired.
    pub async fn access_token(
        &self,
        connection: &AccountConnection,
    ) -> Result<String, SignInError> {
        self.access_token_at(connection, now_unix()).await
    }

    async fn access_token_at(
        &self,
        connection: &AccountConnection,
        now: u64,
    ) -> Result<String, SignInError> {
        let AuthMethod::OAuth2 { issuer, client_id } = &connection.auth else {
            return Err(SignInError::Keychain {
                detail: "this account doesn't use OAuth".into(),
            });
        };
        let (tokens, client_secret) = match self.store.load(&connection.secret)? {
            None => return Err(SignInError::SignInExpired),
            Some(Secret::OAuth {
                tokens,
                client_secret,
            }) => (tokens, client_secret),
            Some(_) => {
                return Err(SignInError::Keychain {
                    detail: "the stored secret isn't an OAuth token".into(),
                })
            }
        };
        if !tokens.needs_refresh(now) {
            return Ok(tokens.access_token);
        }
        let config = match issuer {
            OAuthIssuer::Microsoft => {
                OAuthConfig::microsoft(Some(client_id), &self.endpoints.oauth)?
            }
            OAuthIssuer::Google => {
                OAuthConfig::google(client_id, client_secret.as_deref(), &self.endpoints.oauth)
            }
        };
        let fresh = refresh(self.http().await, &config, &tokens, now).await?;
        let access_token = fresh.access_token.clone();
        self.store.save(
            &connection.secret,
            &Secret::OAuth {
                tokens: fresh,
                client_secret,
            },
        )?;
        Ok(access_token)
    }

    /// The other addresses a Gmail account sends from ("Send mail as"), with an OAuth
    /// access token from [`Accounts::access_token`].
    pub async fn gmail_addresses(&self, access_token: &str) -> Result<Vec<SendAs>, SignInError> {
        send_as(self.http().await, &self.endpoints.gmail_api, access_token).await
    }

    /// The mail service a domain's mail goes to, from its MX records. `None` when the
    /// lookup fails or the service isn't one Moss knows.
    pub async fn domain_service(&self, domain: &str) -> Option<MailService> {
        let host = self.mx.mail_hosts(domain).await.into_iter().next()?;
        hosting_service(&host)
    }

    /// Checks a Resend API key and lists the account's domains for the user to pick from.
    pub async fn check_resend_key(&self, key: &str) -> Result<Vec<ResendDomain>, SignInError> {
        check_key(self.http().await, &self.endpoints.resend_api, key).await
    }

    /// Adds a Resend account that sends and receives as `email`, whose domain must be
    /// in the Resend account.
    pub async fn sign_in_resend(
        &self,
        key: &str,
        email: &str,
    ) -> Result<AccountConnection, SignInError> {
        let email = email.trim().to_string();
        let domain = domain_of(&email).ok_or(SignInError::InvalidEmail)?;
        let domains = self.check_resend_key(key).await?;
        if !domains.iter().any(|d| d.name.eq_ignore_ascii_case(&domain)) {
            return Err(SignInError::ResendDomainMissing { domain });
        }
        let connection = AccountConnection {
            secret: SecretRef::fresh_for_account(&email, MailService::Resend.slug())?,
            email,
            kind: MailService::Resend.kind(),
            service: MailService::Resend,
            servers: None,
            auth: AuthMethod::ApiKey,
        };
        self.store.save(
            &connection.secret,
            &Secret::ApiKey {
                key: key.trim().into(),
            },
        )?;
        Ok(connection)
    }

    /// The account's password, tokens or key, for the provider crate.
    pub fn secret(&self, connection: &AccountConnection) -> Result<Option<Secret>, SignInError> {
        Ok(self.store.load(&connection.secret)?)
    }

    /// Removes the account's secret from the keychain.
    pub fn forget(&self, connection: &AccountConnection) -> Result<(), SignInError> {
        Ok(self.store.delete(&connection.secret)?)
    }
}

/// A typed address to pre-fill the provider's sign-in page with, once checked. None is fine:
/// the provider then asks, and says afterwards who signed in.
fn login_hint(email: Option<&str>) -> Result<Option<&str>, SignInError> {
    match email.map(str::trim) {
        Some(email) => {
            domain_of(email).ok_or(SignInError::InvalidEmail)?;
            Ok(Some(email))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{Security, Server};
    use crate::discovery::tests::{FakeMx, CONFIG};
    use crate::oauth::tests::{browser, id_token, token_json};
    use crate::resend::tests::domains_json;
    use crate::secret::{MemorySecretStore, SecretStoreError, TokenSet};
    use std::sync::Mutex;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Accepts or refuses every login, remembering what it was asked.
    struct FakeLogin {
        result: Result<(), LoginFailure>,
        seen: Mutex<Vec<(AccountConnection, Secret)>>,
    }

    impl FakeLogin {
        fn ok() -> Self {
            Self {
                result: Ok(()),
                seen: Mutex::default(),
            }
        }
        fn rejecting(message: &str) -> Self {
            Self {
                result: Err(LoginFailure::Rejected {
                    message: message.into(),
                }),
                seen: Mutex::default(),
            }
        }
    }

    impl LoginCheck for FakeLogin {
        fn check<'a>(
            &'a self,
            connection: &'a AccountConnection,
            secret: &'a Secret,
        ) -> BoxFuture<'a, Result<(), LoginFailure>> {
            self.seen
                .lock()
                .unwrap()
                .push((connection.clone(), secret.clone()));
            Box::pin(async move { self.result.clone() })
        }
    }

    struct BrokenStore;

    impl SecretStore for BrokenStore {
        fn save(&self, _: &SecretRef, _: &Secret) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Unavailable("locked".into()))
        }
        fn load(&self, _: &SecretRef) -> Result<Option<Secret>, SecretStoreError> {
            Err(SecretStoreError::Unavailable("locked".into()))
        }
        fn delete(&self, _: &SecretRef) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Unavailable("locked".into()))
        }
    }

    async fn setup() -> (MockServer, Arc<MemorySecretStore>, Accounts) {
        let server = MockServer::start().await;
        let store = Arc::new(MemorySecretStore::new());
        let accounts = accounts_for(&server, store.clone(), FakeMx::default());
        (server, store, accounts)
    }

    fn accounts_for(server: &MockServer, store: Arc<dyn SecretStore>, mx: FakeMx) -> Accounts {
        let endpoints = Endpoints {
            oauth: crate::oauth::tests::endpoints(server),
            discovery: crate::discovery::tests::endpoints(server),
            resend_api: server.uri(),
            gmail_api: server.uri(),
        };
        Accounts::with(store, endpoints, Arc::new(mx))
    }

    fn password(service: MailService, email: &str, password: &str) -> PasswordSignIn {
        PasswordSignIn {
            service,
            email: email.into(),
            password: password.into(),
            servers: None,
            login: None,
        }
    }

    #[tokio::test]
    async fn the_http_client_is_built_on_first_use_not_at_launch() {
        let (server, _store, accounts) = setup().await;
        // Building one reads the system's proxy settings, which can take seconds: never on
        // the thread that starts the app.
        assert!(accounts.http.get().is_none());
        let _ = accounts.check_resend_key("re_key").await;
        assert!(accounts.http.get().is_some());
        assert!(!server.received_requests().await.unwrap().is_empty());
        let shown = format!("{:?}", accounts.http.get().unwrap());
        assert!(shown.contains("Moss/"), "{shown}");
    }

    #[test]
    fn debug_hides_the_password() {
        let shown = format!("{:?}", password(MailService::Imap, "a@b.co", "hunter2"));
        assert!(!shown.contains("hunter2"));
        assert!(shown.contains("a@b.co"));
    }

    #[test]
    fn endpoints_default_to_the_real_services() {
        let e = Endpoints::default();
        assert_eq!(e.resend_api, "https://api.resend.com");
        assert!(e
            .discovery
            .ispdb
            .starts_with("https://autoconfig.thunderbird.net/"));
        let _ = Accounts::new(Arc::new(MemorySecretStore::new()));
    }

    #[test]
    fn gmail_app_passwords_are_sixteen_letters() {
        assert_eq!(
            normalize_gmail_app_password("abcd EFGH ijkl mnop").unwrap(),
            "abcdefghijklmnop"
        );
        let not = Err(SignInError::GmailAppPassword {
            problem: GmailAppPasswordProblem::NotAnAppPassword,
        });
        assert_eq!(normalize_gmail_app_password("Hunter2-password"), not);
        assert_eq!(normalize_gmail_app_password("abcd efgh ijkl mn0p"), not);
        assert_eq!(normalize_gmail_app_password("abcd efgh ijkl mnopq"), not);
        assert_eq!(normalize_gmail_app_password(""), not);
    }

    #[test]
    fn login_failures_map_to_error_screens() {
        let gmail = classify_login_failure(
            MailService::Gmail,
            LoginFailure::Rejected {
                message: "[ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)".into(),
            },
        );
        assert_eq!(
            gmail,
            SignInError::GmailAppPassword {
                problem: GmailAppPasswordProblem::AppPasswordRequired
            }
        );
        let wrong = classify_login_failure(
            MailService::Gmail,
            LoginFailure::Rejected {
                message: "[AUTHENTICATIONFAILED] Invalid credentials (Failure)".into(),
            },
        );
        assert_eq!(
            wrong,
            SignInError::WrongPassword {
                service: MailService::Gmail
            }
        );
        let icloud = classify_login_failure(
            MailService::ICloud,
            LoginFailure::Rejected {
                message: "Application-specific password required".into(),
            },
        );
        assert_eq!(
            icloud,
            SignInError::WrongPassword {
                service: MailService::ICloud
            }
        );
        let down = classify_login_failure(
            MailService::Imap,
            LoginFailure::Unreachable {
                host: "imap.x.io".into(),
                detail: "refused".into(),
            },
        );
        assert_eq!(
            down,
            SignInError::Unreachable {
                host: "imap.x.io".into(),
                detail: "refused".into()
            }
        );
    }

    #[tokio::test]
    async fn an_app_password_is_checked_then_saved() {
        let (_server, store, accounts) = setup().await;
        let login = FakeLogin::ok();
        let connection = accounts
            .sign_in_with_password(
                password(
                    MailService::Gmail,
                    " Maya@gmail.com ",
                    "abcd efgh ijkl mnop",
                ),
                &login,
            )
            .await
            .unwrap();
        assert_eq!(connection.email, "Maya@gmail.com");
        assert_eq!(connection.kind, moss_core::ProviderKind::Gmail);
        assert_eq!(connection.auth, AuthMethod::Password);
        assert_eq!(
            connection.servers.as_ref().unwrap().imap.host,
            "imap.gmail.com"
        );
        assert_eq!(connection.secret.service, crate::secret::KEYCHAIN_SERVICE);
        assert!(connection.secret.account.starts_with("maya@gmail.com (gmail) ["));
        let saved = Secret::Password {
            password: "abcdefghijklmnop".into(),
        };
        assert_eq!(store.load(&connection.secret).unwrap(), Some(saved.clone()));
        assert_eq!(
            *login.seen.lock().unwrap(),
            vec![(connection.clone(), saved.clone())]
        );
        assert_eq!(accounts.secret(&connection).unwrap(), Some(saved));
        accounts.forget(&connection).unwrap();
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn a_rejected_password_is_not_saved() {
        let (_server, store, accounts) = setup().await;
        let err = accounts
            .sign_in_with_password(
                password(MailService::ICloud, "m@icloud.com", "wrong"),
                &FakeLogin::rejecting("AUTHENTICATIONFAILED"),
            )
            .await
            .unwrap_err();
        assert_eq!(
            err,
            SignInError::WrongPassword {
                service: MailService::ICloud
            }
        );
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn bad_input_is_refused_before_any_login() {
        let (_server, _store, accounts) = setup().await;
        let login = FakeLogin::ok();
        let cases = [
            (
                password(MailService::Imap, "not-an-address", "p"),
                SignInError::InvalidEmail,
            ),
            (
                password(MailService::Gmail, "m@gmail.com", "Hunter2-password"),
                SignInError::GmailAppPassword {
                    problem: GmailAppPasswordProblem::NotAnAppPassword,
                },
            ),
            (
                password(MailService::Yahoo, "m@yahoo.com", ""),
                SignInError::WrongPassword {
                    service: MailService::Yahoo,
                },
            ),
            (
                password(MailService::Outlook, "m@outlook.com", "p"),
                SignInError::PasswordNotSupported {
                    service: MailService::Outlook,
                },
            ),
        ];
        for (request, expected) in cases {
            assert_eq!(
                accounts.sign_in_with_password(request, &login).await,
                Err(expected)
            );
        }
        assert!(login.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_icloud_custom_domain_signs_in_with_the_icloud_address() {
        // iCloud only lets the account's own @icloud.com address log in; the custom
        // address is just what mail is sent from.
        let (_server, store, accounts) = setup().await;
        let login = FakeLogin::ok();
        let request = PasswordSignIn {
            login: Some(" maya.ross@icloud.com ".into()),
            ..password(MailService::ICloud, "info@fernway.co", "pw")
        };
        let connection = accounts.sign_in_with_password(request, &login).await.unwrap();
        let servers = connection.servers.clone().unwrap();
        assert_eq!(
            servers.imap,
            Server::new("imap.mail.me.com", 993, Security::Tls, "maya.ross@icloud.com")
        );
        assert_eq!(servers.smtp.username, "maya.ross@icloud.com");
        assert_eq!(connection.email, "info@fernway.co");
        assert_eq!(login.seen.lock().unwrap()[0].0, connection);
        assert!(!store.is_empty());
    }

    #[tokio::test]
    async fn an_icloud_custom_domain_needs_the_icloud_address() {
        let (_server, _store, accounts) = setup().await;
        let login = FakeLogin::ok();
        for typed in [None, Some("  ".to_string())] {
            let request = PasswordSignIn {
                login: typed,
                ..password(MailService::ICloud, "info@fernway.co", "pw")
            };
            assert_eq!(
                accounts.sign_in_with_password(request, &login).await,
                Err(SignInError::ICloudAddressNeeded)
            );
        }
        assert!(login.seen.lock().unwrap().is_empty());

        // An @icloud.com, @me.com or @mac.com address is its own login.
        for email in ["maya@icloud.com", "maya@me.com", "maya@mac.com"] {
            let connection = accounts
                .sign_in_with_password(password(MailService::ICloud, email, "pw"), &login)
                .await
                .unwrap();
            assert_eq!(connection.servers.unwrap().imap.username, email);
        }
    }

    #[tokio::test]
    async fn a_custom_domain_found_on_icloud_needs_the_icloud_address_too() {
        let server = MockServer::start().await;
        Mock::given(path("/ispdb/icloud.com"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(CONFIG.replace(" imap.%EMAILDOMAIN% ", "imap.mail.me.com")),
            )
            .mount(&server)
            .await;
        let store = Arc::new(MemorySecretStore::new());
        let accounts = accounts_for(&server, store, FakeMx::new(&["mx01.mail.icloud.com"]));
        let err = accounts
            .sign_in_with_password(
                password(MailService::Imap, "info@fernway.co", "pw"),
                &FakeLogin::ok(),
            )
            .await;
        assert_eq!(err, Err(SignInError::ICloudAddressNeeded));

        let request = PasswordSignIn {
            login: Some("maya.ross@icloud.com".into()),
            ..password(MailService::Imap, "info@fernway.co", "pw")
        };
        let connection = accounts
            .sign_in_with_password(request, &FakeLogin::ok())
            .await
            .unwrap();
        assert_eq!(connection.service, MailService::ICloud);
        let servers = connection.servers.unwrap();
        assert_eq!(servers.imap.host, "imap.mail.me.com");
        assert_eq!(servers.imap.username, "maya.ross@icloud.com");
        assert_eq!(servers.smtp.username, "maya.ross@icloud.com");
    }

    #[tokio::test]
    async fn custom_domains_use_discovered_settings() {
        let server = MockServer::start().await;
        Mock::given(path("/ispdb/example.com"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CONFIG))
            .mount(&server)
            .await;
        let store = Arc::new(MemorySecretStore::new());
        let accounts = accounts_for(&server, store.clone(), FakeMx::default());
        let connection = accounts
            .sign_in_with_password(
                password(MailService::Imap, "maya@example.com", "pw"),
                &FakeLogin::ok(),
            )
            .await
            .unwrap();
        assert_eq!(
            connection.servers.unwrap().imap,
            Server::new("imap.example.com", 993, Security::Tls, "maya")
        );
        assert_eq!(connection.service, MailService::Imap);
        assert_eq!(
            accounts
                .discover("maya@example.com")
                .await
                .unwrap()
                .servers
                .smtp
                .port,
            587
        );
    }

    #[tokio::test]
    async fn a_workspace_domain_follows_gmail_rules() {
        let server = MockServer::start().await;
        let store = Arc::new(MemorySecretStore::new());
        let accounts = accounts_for(&server, store.clone(), FakeMx::new(&["aspmx.l.google.com"]));
        let err = accounts
            .sign_in_with_password(
                password(MailService::Imap, "maya@northwind.co", "normal password"),
                &FakeLogin::ok(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            err,
            SignInError::GmailAppPassword {
                problem: GmailAppPasswordProblem::NotAnAppPassword
            }
        );

        let accounts = accounts_for(
            &server,
            store.clone(),
            FakeMx::new(&["x.mail.protection.outlook.com"]),
        );
        let err = accounts
            .sign_in_with_password(
                password(MailService::Imap, "maya@northwind.co", "pw"),
                &FakeLogin::ok(),
            )
            .await;
        assert_eq!(
            err,
            Err(SignInError::PasswordNotSupported {
                service: MailService::Outlook
            })
        );
    }

    #[tokio::test]
    async fn the_service_chosen_by_the_user_wins_over_discovery() {
        // Fastmail with a custom domain: the user picked Fastmail, so its servers are used.
        let (server, _store, accounts) = setup().await;
        let connection = accounts
            .sign_in_with_password(
                password(MailService::Fastmail, "maya@example.com", "pw"),
                &FakeLogin::ok(),
            )
            .await
            .unwrap();
        assert_eq!(connection.service, MailService::Fastmail);
        assert_eq!(connection.servers.unwrap().imap.host, "imap.fastmail.com");
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "nothing discovered"
        );

        let manual = ServerSettings {
            imap: Server::new("mail.example.com", 993, Security::Tls, "maya"),
            smtp: Server::new("mail.example.com", 465, Security::Tls, "maya"),
        };
        let request = PasswordSignIn {
            servers: Some(manual.clone()),
            ..password(MailService::Imap, "maya@example.com", "pw")
        };
        let connection = accounts
            .sign_in_with_password(request, &FakeLogin::ok())
            .await
            .unwrap();
        assert_eq!(connection.servers, Some(manual.clone()));
        assert_eq!(connection.service, MailService::Imap);

        for servers in [None, Some(manual)] {
            let request = PasswordSignIn {
                servers,
                ..password(MailService::Resend, "maya@example.com", "pw")
            };
            let err = accounts
                .sign_in_with_password(request, &FakeLogin::ok())
                .await;
            assert_eq!(
                err,
                Err(SignInError::PasswordNotSupported {
                    service: MailService::Resend
                })
            );
        }
    }

    #[tokio::test]
    async fn settings_not_found_is_passed_on() {
        let (_server, _store, accounts) = setup().await;
        let err = accounts
            .sign_in_with_password(
                password(MailService::Imap, "m@northwind.co", "pw"),
                &FakeLogin::ok(),
            )
            .await;
        assert_eq!(
            err,
            Err(SignInError::SettingsNotFound {
                domain: "northwind.co".into()
            })
        );
    }

    #[tokio::test]
    async fn a_locked_keychain_fails_the_sign_in() {
        let server = MockServer::start().await;
        let accounts = accounts_for(&server, Arc::new(BrokenStore), FakeMx::default());
        let err = accounts
            .sign_in_with_password(
                password(MailService::Yahoo, "m@yahoo.com", "pw"),
                &FakeLogin::ok(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, SignInError::Keychain { .. }));
        let connection = AccountConnection {
            email: "m@yahoo.com".into(),
            kind: moss_core::ProviderKind::Imap,
            service: MailService::Yahoo,
            servers: None,
            auth: AuthMethod::Password,
            secret: SecretRef::for_account("m@yahoo.com", "yahoo"),
        };
        assert!(matches!(
            accounts.secret(&connection),
            Err(SignInError::Keychain { .. })
        ));
        assert!(matches!(
            accounts.forget(&connection),
            Err(SignInError::Keychain { .. })
        ));
    }

    async fn complete_browser(pending: &PendingAuthorization) {
        let state = url::Url::parse(pending.authorize_url())
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .unwrap();
        let redirect = pending.redirect_uri().to_string();
        tokio::spawn(async move { browser(&redirect, &format!("/?code=c&state={state}")).await });
    }

    #[tokio::test]
    async fn outlook_sign_in_saves_tokens_and_builds_a_connection() {
        let (server, store, accounts) = setup().await;
        Mock::given(method("POST"))
            .and(path("/ms/token"))
            .and(body_string_contains("client_id=own-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("at", Some("rt"))))
            .mount(&server)
            .await;
        let pending = accounts
            .start_outlook(Some(" maya@outlook.com "), Some("own-id"))
            .await
            .unwrap();
        assert!(pending
            .authorize_url()
            .contains("login_hint=maya%40outlook.com"));
        complete_browser(&pending).await;
        let connection = accounts
            .finish_oauth(pending, Some(" maya@outlook.com "))
            .await
            .unwrap();
        assert_eq!(connection.email, "maya@outlook.com");
        assert_eq!(connection.kind, moss_core::ProviderKind::Microsoft);
        assert_eq!(
            connection.auth,
            AuthMethod::OAuth2 {
                issuer: OAuthIssuer::Microsoft,
                client_id: "own-id".into()
            }
        );
        assert_eq!(
            connection.servers.as_ref().unwrap().imap.host,
            "outlook.office365.com"
        );
        let Some(Secret::OAuth {
            tokens,
            client_secret: None,
        }) = store.load(&connection.secret).unwrap()
        else {
            panic!("tokens not saved");
        };
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(accounts.access_token(&connection).await.unwrap(), "at");
    }

    #[tokio::test]
    async fn outlook_learns_the_address_from_microsoft() {
        let (server, _store, accounts) = setup().await;
        let mut reply = token_json("at", Some("rt"));
        reply["id_token"] =
            id_token(serde_json::json!({ "preferred_username": "maya@outlook.com" })).into();
        Mock::given(method("POST"))
            .and(path("/ms/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply))
            .mount(&server)
            .await;

        let pending = accounts.start_outlook(None, Some("own-id")).await.unwrap();
        assert!(!pending.authorize_url().contains("login_hint"));
        complete_browser(&pending).await;
        let connection = accounts.finish_oauth(pending, None).await.unwrap();
        assert_eq!(connection.email, "maya@outlook.com");
        assert_eq!(connection.secret.service, crate::secret::KEYCHAIN_SERVICE);
        assert!(connection.secret.account.starts_with("maya@outlook.com (outlook) ["));

        // Signed in to another account than the one typed: the one that signed in wins.
        let pending = accounts
            .start_outlook(Some("typo@outlook.com"), Some("own-id"))
            .await
            .unwrap();
        complete_browser(&pending).await;
        let connection = accounts
            .finish_oauth(pending, Some("typo@outlook.com"))
            .await
            .unwrap();
        assert_eq!(connection.email, "maya@outlook.com");
    }

    #[tokio::test]
    async fn a_sign_in_that_names_no_account_saves_nothing() {
        let (server, store, accounts) = setup().await;
        Mock::given(method("POST"))
            .and(path("/ms/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("at", Some("rt"))))
            .mount(&server)
            .await;
        let pending = accounts.start_outlook(None, Some("own-id")).await.unwrap();
        complete_browser(&pending).await;
        assert!(matches!(
            accounts.finish_oauth(pending, None).await.unwrap_err(),
            SignInError::UnexpectedResponse { .. }
        ));
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn gmail_oauth_keeps_the_client_secret_for_refreshes() {
        let (server, store, accounts) = setup().await;
        Mock::given(method("POST"))
            .and(path("/google/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("at", Some("rt"))))
            .mount(&server)
            .await;
        let pending = accounts
            .start_gmail_oauth(Some("maya@gmail.com"), "g-id", Some("g-secret"))
            .await
            .unwrap();
        complete_browser(&pending).await;
        let connection = accounts
            .finish_oauth(pending, Some("maya@gmail.com"))
            .await
            .unwrap();
        assert_eq!(connection.service, MailService::Gmail);
        assert!(matches!(
            store.load(&connection.secret).unwrap(),
            Some(Secret::OAuth { client_secret: Some(ref s), .. }) if s == "g-secret"
        ));
    }

    #[tokio::test]
    async fn oauth_starts_check_their_input() {
        let (_server, _store, accounts) = setup().await;
        assert_eq!(
            accounts
                .start_outlook(Some("nope"), Some("id"))
                .await
                .unwrap_err(),
            SignInError::InvalidEmail
        );
        assert_eq!(
            accounts
                .start_gmail_oauth(Some("nope"), "id", None)
                .await
                .unwrap_err(),
            SignInError::InvalidEmail
        );
        assert!(matches!(
            accounts
                .start_gmail_oauth(None, "  ", None)
                .await
                .unwrap_err(),
            SignInError::InvalidOAuthClient { .. }
        ));
    }

    #[tokio::test]
    async fn an_abandoned_browser_sign_in_times_out_and_saves_nothing() {
        let (_server, store, accounts) = setup().await;
        let accounts = accounts.with_oauth_timeout(Duration::from_millis(10));
        let pending = accounts
            .start_outlook(Some("m@outlook.com"), Some("id"))
            .await
            .unwrap();
        assert_eq!(
            accounts
                .finish_oauth(pending, Some("m@outlook.com"))
                .await
                .unwrap_err(),
            SignInError::Timeout
        );
        assert!(store.is_empty());
    }

    fn oauth_connection(issuer: OAuthIssuer) -> AccountConnection {
        AccountConnection {
            email: "maya@outlook.com".into(),
            kind: moss_core::ProviderKind::Microsoft,
            service: MailService::Outlook,
            servers: None,
            auth: AuthMethod::OAuth2 {
                issuer,
                client_id: "cid".into(),
            },
            secret: SecretRef::for_account("maya@outlook.com", "outlook"),
        }
    }

    #[tokio::test]
    async fn expired_access_tokens_are_refreshed_and_saved() {
        let (server, store, accounts) = setup().await;
        Mock::given(method("POST"))
            .and(path("/google/token"))
            .and(body_string_contains("client_secret=g-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("fresh", None)))
            .expect(1)
            .mount(&server)
            .await;
        let connection = oauth_connection(OAuthIssuer::Google);
        let old = TokenSet {
            access_token: "stale".into(),
            refresh_token: Some("rt".into()),
            expires_at: 1_000,
            refresh_issued_at: 10,
        };
        store
            .save(
                &connection.secret,
                &Secret::OAuth {
                    tokens: old,
                    client_secret: Some("g-secret".into()),
                },
            )
            .unwrap();

        assert_eq!(
            accounts.access_token_at(&connection, 100).await.unwrap(),
            "stale"
        );
        assert_eq!(
            accounts.access_token_at(&connection, 5_000).await.unwrap(),
            "fresh"
        );
        let Some(Secret::OAuth {
            tokens,
            client_secret,
        }) = store.load(&connection.secret).unwrap()
        else {
            panic!()
        };
        assert_eq!(tokens.expires_at, 5_000 + 3_599);
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(client_secret.as_deref(), Some("g-secret"));
    }

    #[tokio::test]
    async fn microsoft_refreshes_use_the_accounts_client_id() {
        let (server, store, accounts) = setup().await;
        Mock::given(method("POST"))
            .and(path("/ms/token"))
            .and(body_string_contains("client_id=cid"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant", "error_description": "AADSTS65001: consent revoked"
            })))
            .mount(&server)
            .await;
        let connection = oauth_connection(OAuthIssuer::Microsoft);
        let old = TokenSet {
            access_token: "stale".into(),
            refresh_token: Some("rt".into()),
            expires_at: 0,
            refresh_issued_at: 0,
        };
        store
            .save(
                &connection.secret,
                &Secret::OAuth {
                    tokens: old,
                    client_secret: None,
                },
            )
            .unwrap();
        assert!(matches!(
            accounts.access_token(&connection).await,
            Err(SignInError::AdminApprovalRequired { .. })
        ));
    }

    #[tokio::test]
    async fn access_tokens_need_an_oauth_account_and_secret() {
        let (_server, store, accounts) = setup().await;
        let connection = oauth_connection(OAuthIssuer::Microsoft);
        assert_eq!(
            accounts.access_token(&connection).await,
            Err(SignInError::SignInExpired)
        );
        store
            .save(
                &connection.secret,
                &Secret::Password {
                    password: "p".into(),
                },
            )
            .unwrap();
        assert!(matches!(
            accounts.access_token(&connection).await,
            Err(SignInError::Keychain { .. })
        ));
        let password_account = AccountConnection {
            auth: AuthMethod::Password,
            ..connection
        };
        assert!(matches!(
            accounts.access_token(&password_account).await,
            Err(SignInError::Keychain { .. })
        ));
    }

    #[tokio::test]
    async fn resend_sign_in_needs_the_addresses_domain() {
        let (server, store, accounts) = setup().await;
        Mock::given(method("GET"))
            .and(path("/domains"))
            .respond_with(ResponseTemplate::new(200).set_body_json(domains_json()))
            .mount(&server)
            .await;
        assert_eq!(accounts.check_resend_key("re_key").await.unwrap().len(), 2);
        let connection = accounts
            .sign_in_resend(" re_key ", "hello@Studio.Northwind.co")
            .await
            .unwrap();
        assert_eq!(connection.kind, moss_core::ProviderKind::Resend);
        assert_eq!(connection.servers, None);
        assert_eq!(connection.auth, AuthMethod::ApiKey);
        assert_eq!(
            store.load(&connection.secret).unwrap(),
            Some(Secret::ApiKey {
                key: "re_key".into()
            })
        );
        assert_eq!(
            accounts
                .sign_in_resend("re_key", "hello@elsewhere.io")
                .await,
            Err(SignInError::ResendDomainMissing {
                domain: "elsewhere.io".into()
            })
        );
        assert_eq!(
            accounts.sign_in_resend("re_key", "nope").await,
            Err(SignInError::InvalidEmail)
        );
        assert_eq!(
            accounts.sign_in_resend(" ", "hello@northwind.co").await,
            Err(SignInError::InvalidResendKey)
        );
    }

    #[tokio::test]
    async fn gmail_lists_the_addresses_an_account_sends_as() {
        let (server, _, accounts) = setup().await;
        let body = serde_json::json!({"sendAs": [{"sendAsEmail": "hi@rossstudio.art", "displayName": "Ross Studio"}]});
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/settings/sendAs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let found = accounts.gmail_addresses("tok").await.unwrap();
        assert_eq!(found, [SendAs { email: "hi@rossstudio.art".into(), name: Some("Ross Studio".into()) }]);
        assert_eq!(Endpoints::default().gmail_api, "https://gmail.googleapis.com");
    }

    #[tokio::test]
    async fn a_domain_is_hosted_where_its_mail_goes() {
        let server = MockServer::start().await;
        let store = Arc::new(MemorySecretStore::new());
        let mx = FakeMx::new(&["mx01.mail.icloud.com", "mx02.mail.icloud.com"]);
        let accounts = accounts_for(&server, store.clone(), mx);
        assert_eq!(accounts.domain_service("rossstudio.art").await, Some(MailService::ICloud));
        let nowhere = accounts_for(&server, store.clone(), FakeMx::default());
        assert_eq!(nowhere.domain_service("rossstudio.art").await, None);
        let elsewhere = accounts_for(&server, store, FakeMx::new(&["mx.example.com"]));
        assert_eq!(elsewhere.domain_service("example.com").await, None);
    }
    #[tokio::test]
    async fn signing_in_again_never_overwrites_or_reuses_a_password_reference() {
        let (_, store, accounts) = setup().await;
        let legacy = SecretRef::for_account("maya@gmail.com", "gmail");
        let old = Secret::Password { password: "legacy-password".into() };
        store.save(&legacy, &old).unwrap();
        let first = accounts.sign_in_with_password(password(MailService::Gmail, "maya@gmail.com", "abcdefghijklmnop"), &FakeLogin::ok()).await.unwrap();
        let second = accounts.sign_in_with_password(password(MailService::Gmail, "maya@gmail.com", "ponmlkjihgfedcba"), &FakeLogin::ok()).await.unwrap();
        assert_ne!(first.secret, legacy);
        assert_ne!(first.secret, second.secret);
        assert_eq!(store.load(&legacy).unwrap(), Some(old));
        assert_eq!(accounts.secret(&first).unwrap(), Some(Secret::Password { password: "abcdefghijklmnop".into() }));
        accounts.forget(&first).unwrap();
        assert_eq!(accounts.secret(&second).unwrap(), Some(Secret::Password { password: "ponmlkjihgfedcba".into() }));
    }

    #[tokio::test]
    async fn signing_in_again_never_reuses_an_api_key_reference() {
        let (server, _, accounts) = setup().await;
        Mock::given(method("GET")).and(path("/domains")).respond_with(ResponseTemplate::new(200).set_body_json(domains_json())).mount(&server).await;
        let first = accounts.sign_in_resend("re_first_fixture", "hello@studio.northwind.co").await.unwrap();
        let second = accounts.sign_in_resend("re_second_fixture", "hello@studio.northwind.co").await.unwrap();
        assert_ne!(first.secret, second.secret);
        assert_eq!(accounts.secret(&first).unwrap(), Some(Secret::ApiKey { key: "re_first_fixture".into() }));
        accounts.forget(&first).unwrap();
        assert_eq!(accounts.secret(&second).unwrap(), Some(Secret::ApiKey { key: "re_second_fixture".into() }));
    }

    #[tokio::test]
    async fn signing_in_again_never_reuses_an_oauth_reference() {
        let (server, _, accounts) = setup().await;
        Mock::given(method("POST")).and(path("/ms/token")).respond_with(ResponseTemplate::new(200).set_body_json(token_json("at", Some("rt")))).mount(&server).await;
        let pending = accounts.start_outlook(Some("maya@example.com"), Some("own-id")).await.unwrap();
        complete_browser(&pending).await;
        let first = accounts.finish_oauth(pending, Some("maya@example.com")).await.unwrap();
        let pending = accounts.start_outlook(Some("maya@example.com"), Some("own-id")).await.unwrap();
        complete_browser(&pending).await;
        let second = accounts.finish_oauth(pending, Some("maya@example.com")).await.unwrap();
        assert_ne!(first.secret, second.secret);
        accounts.forget(&first).unwrap();
        assert_eq!(accounts.access_token(&second).await.unwrap(), "at");
    }

}
