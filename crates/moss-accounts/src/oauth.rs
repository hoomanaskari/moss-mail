//! OAuth 2.0 sign-in for Outlook and Gmail: authorization code + PKCE with a loopback
//! redirect (RFC 8252), so there is no client secret to ship and no Moss server.
//!
//! 1. [`PendingAuthorization::start`] listens on `http://127.0.0.1:<random port>/` and
//!    builds the URL to open in the browser.
//! 2. [`PendingAuthorization::finish`] waits for the browser to come back, checks the
//!    `state`, and swaps the code for tokens.
//! 3. [`refresh`] gets a new access token when the old one expires.

use std::fmt;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use crate::connection::{MailService, OAuthIssuer};
use crate::error::{unreachable, SignInError};
use crate::secret::TokenSet;

/// Set `MOSS_MICROSOFT_CLIENT_ID` to this at compile time to build without Moss's client ID:
/// Outlook then needs the user's own.
pub const MICROSOFT_CLIENT_ID_PLACEHOLDER: &str = "moss-microsoft-client-id-not-configured";

/// Moss's public Microsoft client ID (no secret; PKCE only): the "Moss" app in Microsoft Entra,
/// for personal and work or school accounts, redirecting to `http://127.0.0.1/` (any port).
/// A build can use another one with the `MOSS_MICROSOFT_CLIENT_ID` environment variable.
pub const MOSS_MICROSOFT_CLIENT_ID: &str = match option_env!("MOSS_MICROSOFT_CLIENT_ID") {
    Some(id) => id,
    None => "5d849200-a6d1-4725-a2f6-601c78822972",
};

/// IMAP and SMTP for Outlook.com and Microsoft 365, plus a refresh token, and an
/// `id_token` that says which account signed in.
pub const MICROSOFT_SCOPES: [&str; 6] = [
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "https://outlook.office.com/SMTP.Send",
    "offline_access",
    "openid",
    "email",
    "profile",
];

/// Full IMAP and SMTP access to Gmail, and an `id_token` that says which account signed in.
pub const GOOGLE_SCOPES: [&str; 3] = ["https://mail.google.com/", "openid", "email"];

/// Google drops refresh tokens of OAuth clients in "Testing" status after this long.
pub const GOOGLE_TESTING_TOKEN_LIFETIME: u64 = 7 * 24 * 60 * 60;

/// Where the OAuth providers live. Tests point these at a mock server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthEndpoints {
    pub microsoft_authorize: String,
    pub microsoft_token: String,
    pub google_authorize: String,
    pub google_token: String,
}

impl Default for OAuthEndpoints {
    fn default() -> Self {
        Self {
            // `common` lets both personal accounts and work or school accounts sign in.
            microsoft_authorize: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
                .into(),
            microsoft_token: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
            google_authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            google_token: "https://oauth2.googleapis.com/token".into(),
        }
    }
}

/// One OAuth client: whose it is, its ID and where to send the user.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthConfig {
    pub issuer: OAuthIssuer,
    pub client_id: String,
    /// Google "Desktop app" clients come with a secret that Google says isn't secret but
    /// still wants back. Microsoft public clients have none.
    pub client_secret: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

impl fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthConfig")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("authorize_url", &self.authorize_url)
            .field("token_url", &self.token_url)
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl OAuthConfig {
    /// Outlook / Microsoft 365, with the user's own client ID or Moss's.
    pub fn microsoft(
        own_client_id: Option<&str>,
        endpoints: &OAuthEndpoints,
    ) -> Result<Self, SignInError> {
        Self::microsoft_with_default(own_client_id, MOSS_MICROSOFT_CLIENT_ID, endpoints)
    }

    fn microsoft_with_default(
        own_client_id: Option<&str>,
        moss_client_id: &str,
        endpoints: &OAuthEndpoints,
    ) -> Result<Self, SignInError> {
        let client_id = match own_client_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => id,
            None if moss_client_id == MICROSOFT_CLIENT_ID_PLACEHOLDER => {
                return Err(SignInError::ClientIdNotConfigured)
            }
            None => moss_client_id,
        };
        Ok(Self {
            issuer: OAuthIssuer::Microsoft,
            client_id: client_id.into(),
            client_secret: None,
            authorize_url: endpoints.microsoft_authorize.clone(),
            token_url: endpoints.microsoft_token.clone(),
            scopes: MICROSOFT_SCOPES.map(String::from).to_vec(),
        })
    }

    /// Gmail with the user's own Google OAuth client ("Desktop app" type).
    pub fn google(
        client_id: &str,
        client_secret: Option<&str>,
        endpoints: &OAuthEndpoints,
    ) -> Self {
        Self {
            issuer: OAuthIssuer::Google,
            client_id: client_id.trim().into(),
            client_secret: client_secret
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            authorize_url: endpoints.google_authorize.clone(),
            token_url: endpoints.google_token.clone(),
            scopes: GOOGLE_SCOPES.map(String::from).to_vec(),
        }
    }

    fn service(&self) -> MailService {
        match self.issuer {
            OAuthIssuer::Microsoft => MailService::Outlook,
            OAuthIssuer::Google => MailService::Gmail,
        }
    }

    pub(crate) fn token_host(&self) -> String {
        host_of(&self.token_url)
    }
}

/// A browser sign-in in progress. Open [`authorize_url`](Self::authorize_url), then
/// await [`finish`](Self::finish).
/// How long a browser connection may take to send its request. Browsers open spare
/// connections that may never send one; waiting on such a connection would keep the real
/// redirect, on another connection, from being read.
const REQUEST_WAIT: Duration = Duration::from_secs(2);

pub struct PendingAuthorization {
    config: OAuthConfig,
    listener: TcpListener,
    request_wait: Duration,
    redirect_uri: String,
    verifier: String,
    state: String,
    authorize_url: String,
}

impl fmt::Debug for PendingAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingAuthorization")
            .field("config", &self.config)
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

impl PendingAuthorization {
    /// Starts listening for the redirect and builds the URL to open.
    /// `login_hint` pre-fills the address on the provider's sign-in page.
    pub async fn start(config: OAuthConfig, login_hint: Option<&str>) -> Result<Self, SignInError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| local_error(&e))?;
        let port = listener.local_addr().map_err(|e| local_error(&e))?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/");
        let verifier = random_token(32);
        let state = random_token(16);

        let mut url =
            Url::parse(&config.authorize_url).map_err(|e| SignInError::InvalidOAuthClient {
                detail: format!("bad authorize URL: {e}"),
            })?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("client_id", &config.client_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", &config.scopes.join(" "))
                .append_pair("state", &state)
                .append_pair("code_challenge", &pkce_challenge(&verifier))
                .append_pair("code_challenge_method", "S256");
            match config.issuer {
                // Let people with several Microsoft accounts pick the right one.
                OAuthIssuer::Microsoft => query.append_pair("prompt", "select_account"),
                // Without these Google only returns a refresh token the first time.
                OAuthIssuer::Google => query
                    .append_pair("access_type", "offline")
                    .append_pair("prompt", "consent"),
            };
            if let Some(hint) = login_hint {
                query.append_pair("login_hint", hint);
            }
        }
        let authorize_url = url.into();
        Ok(Self {
            config,
            listener,
            request_wait: REQUEST_WAIT,
            redirect_uri,
            verifier,
            state,
            authorize_url,
        })
    }

    /// The provider's sign-in page. Open it in the user's browser.
    pub fn authorize_url(&self) -> &str {
        &self.authorize_url
    }

    /// Where the browser comes back to: `http://127.0.0.1:<port>/`.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    /// Waits up to `timeout` for the browser to come back, then swaps the code for
    /// tokens.
    pub async fn finish(
        self,
        http: &reqwest::Client,
        timeout: Duration,
    ) -> Result<SignedIn, SignInError> {
        let code = tokio::time::timeout(timeout, self.wait_for_code())
            .await
            .map_err(|_| SignInError::Timeout)??;
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code),
            ("redirect_uri", self.redirect_uri.clone()),
            ("code_verifier", self.verifier.clone()),
        ];
        let now = crate::secret::now_unix();
        let response = token_request(http, &self.config, &mut form, Phase::Exchange).await?;
        let address = response.id_token.as_deref().and_then(address_in);
        Ok(SignedIn {
            tokens: response.into_tokens(now, None),
            address,
        })
    }

    /// Answers browser requests until one carries the code (or an error) for this state.
    async fn wait_for_code(&self) -> Result<String, SignInError> {
        loop {
            let (mut stream, _) = self.listener.accept().await.map_err(|e| local_error(&e))?;
            let Ok(target) = tokio::time::timeout(self.request_wait, read_request_target(&mut stream)).await else {
                continue;
            };
            let Some(target) = target else {
                respond(&mut stream, "400 Bad Request", PAGE_RETRY).await;
                continue;
            };
            let url =
                Url::parse(&format!("http://127.0.0.1{target}")).expect("a path always parses");
            if url.path() != "/" {
                respond(&mut stream, "404 Not Found", PAGE_RETRY).await;
                continue;
            }
            let param = |name: &str| {
                url.query_pairs()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.into_owned())
            };
            if param("state").as_deref() != Some(self.state.as_str()) {
                // Not from this sign-in (an old tab, or someone else's request): ignore it.
                respond(&mut stream, "400 Bad Request", PAGE_RETRY).await;
                continue;
            }
            if let Some(error) = param("error") {
                respond(&mut stream, "200 OK", PAGE_FAILED).await;
                let description = param("error_description").unwrap_or_default();
                return Err(classify(
                    &self.config,
                    &error,
                    &description,
                    &host_of(&self.config.authorize_url),
                    Phase::Authorize,
                ));
            }
            if let Some(code) = param("code") {
                respond(&mut stream, "200 OK", PAGE_DONE).await;
                return Ok(code);
            }
            respond(&mut stream, "400 Bad Request", PAGE_RETRY).await;
        }
    }
}

/// Gets a new access token. Keeps the old refresh token unless the provider sends a new
/// one (Microsoft rotates them; Google doesn't).
pub async fn refresh(
    http: &reqwest::Client,
    config: &OAuthConfig,
    tokens: &TokenSet,
    now: u64,
) -> Result<TokenSet, SignInError> {
    let refresh_token = tokens
        .refresh_token
        .clone()
        .ok_or(SignInError::SignInExpired)?;
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token),
    ];
    let phase = Phase::Refresh {
        refresh_issued_at: tokens.refresh_issued_at,
        now,
    };
    let response = token_request(http, config, &mut form, phase).await?;
    Ok(response.into_tokens(now, Some(tokens)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The browser came back with an error.
    Authorize,
    /// Swapping the code for tokens.
    Exchange,
    /// Refreshing an access token.
    Refresh { refresh_issued_at: u64, now: u64 },
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
}

/// A finished browser sign-in.
#[derive(Debug)]
pub struct SignedIn {
    pub tokens: TokenSet,
    /// The account that signed in, when the provider says.
    pub address: Option<String>,
}

/// Who signed in, from the OpenID `id_token` in the token answer: its `email`, or the
/// `preferred_username` that personal Microsoft accounts send instead. The token came
/// straight from the provider over TLS, so its signature isn't checked.
fn address_in(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let claims: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    ["email", "preferred_username"]
        .into_iter()
        .filter_map(|claim| claims[claim].as_str())
        .map(str::trim)
        .find(|address| address.contains('@'))
        .map(String::from)
}

impl TokenResponse {
    fn into_tokens(self, now: u64, previous: Option<&TokenSet>) -> TokenSet {
        let (refresh_token, refresh_issued_at) = match (self.refresh_token, previous) {
            (Some(new), _) => (Some(new), now),
            (None, Some(old)) => (old.refresh_token.clone(), old.refresh_issued_at),
            (None, None) => (None, now),
        };
        TokenSet {
            access_token: self.access_token,
            refresh_token,
            // Both providers send expires_in; an hour is their usual value if one doesn't.
            expires_at: now + self.expires_in.unwrap_or(3600),
            refresh_issued_at,
        }
    }
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
    #[serde(default)]
    error_description: String,
}

async fn token_request(
    http: &reqwest::Client,
    config: &OAuthConfig,
    form: &mut Vec<(&'static str, String)>,
    phase: Phase,
) -> Result<TokenResponse, SignInError> {
    form.push(("client_id", config.client_id.clone()));
    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    if config.issuer == OAuthIssuer::Microsoft {
        form.push(("scope", config.scopes.join(" ")));
    }
    let host = config.token_host();
    let response = http
        .post(&config.token_url)
        .form(form)
        .send()
        .await
        .map_err(|e| unreachable(&host, e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| unreachable(&host, e))?;
    if status.is_success() {
        return serde_json::from_str(&body).map_err(|_| SignInError::UnexpectedResponse {
            host: host.clone(),
            detail: "The token response could not be read.".into(),
        });
    }
    match serde_json::from_str::<ErrorResponse>(&body) {
        Ok(err) => Err(classify(
            config,
            &err.error,
            &err.error_description,
            &host,
            phase,
        )),
        Err(_) => Err(SignInError::UnexpectedResponse {
            host,
            detail: format!("HTTP {status}"),
        }),
    }
}

/// Microsoft error codes meaning the tenant's admin has to approve the app.
const ADMIN_CONSENT_CODES: [&str; 3] = ["AADSTS65001", "AADSTS90094", "AADSTS900941"];
/// Microsoft error codes meaning the client ID or redirect URI is wrong.
const BAD_CLIENT_CODES: [&str; 4] = [
    "AADSTS700016",
    "AADSTS50011",
    "AADSTS7000218",
    "AADSTS900971",
];

/// Turns a provider's OAuth error into something the UI can explain.
fn classify(
    config: &OAuthConfig,
    error: &str,
    description: &str,
    host: &str,
    phase: Phase,
) -> SignInError {
    let service = config.service();
    // Descriptions and unknown error names can echo tokens or account data.
    // Retain only recognized codes, independently of the UI classification below.
    let contains_code = |code: &str| description
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|token| token == code);
    let known_code = ADMIN_CONSENT_CODES.iter().chain(BAD_CLIENT_CODES.iter())
        .find(|code| contains_code(code)).copied()
        .or(match error {
            "consent_required" | "admin_policy_enforced" | "invalid_client"
            | "unauthorized_client" | "redirect_uri_mismatch" | "access_denied"
            | "invalid_grant" | "invalid_request" | "server_error"
            | "temporarily_unavailable" => Some(error),
            _ => None,
        });
    let detail = known_code.map_or_else(
        || "The provider returned an unrecognized OAuth error.".into(),
        |code| format!("OAuth error: {code}."),
    );
    let has_code = |codes: &[&str]| codes.iter().any(|code| contains_code(code));

    if has_code(&ADMIN_CONSENT_CODES)
        || error == "consent_required"
        || error == "admin_policy_enforced"
    {
        return SignInError::AdminApprovalRequired { service, detail };
    }
    if has_code(&BAD_CLIENT_CODES)
        || matches!(
            error,
            "invalid_client" | "unauthorized_client" | "redirect_uri_mismatch"
        )
    {
        return SignInError::InvalidOAuthClient { detail };
    }
    match (error, phase) {
        ("access_denied", _) => SignInError::AccessDenied { detail },
        (
            "invalid_grant",
            Phase::Refresh {
                refresh_issued_at,
                now,
            },
        ) => {
            let aged_out = now.saturating_sub(refresh_issued_at) >= GOOGLE_TESTING_TOKEN_LIFETIME;
            if config.issuer == OAuthIssuer::Google && aged_out {
                SignInError::GoogleTestingExpired
            } else {
                SignInError::SignInExpired
            }
        }
        // The code expired or was already used: the user has to go through the browser again.
        ("invalid_grant", _) => SignInError::AccessDenied { detail },
        _ => SignInError::UnexpectedResponse {
            host: host.into(),
            detail,
        },
    }
}

/// The PKCE S256 challenge for a verifier (RFC 7636).
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `bytes` random bytes as URL-safe base64.
fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the OS random source is available");
    URL_SAFE_NO_PAD.encode(buf)
}

fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_else(|| "OAuth provider".into())
}

fn local_error(e: &std::io::Error) -> SignInError {
    SignInError::Unreachable {
        host: "127.0.0.1".into(),
        detail: e.to_string(),
    }
}

/// Reads an HTTP request head and returns its target (`/?code=…`), or `None` if it
/// isn't a GET request line.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 8192 {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let mut parts = head.lines().next()?.split(' ');
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Some(target.to_string()),
        _ => None,
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The browser may already have gone; the sign-in result doesn't depend on this.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

const PAGE_DONE: &str = "<!doctype html><meta charset=utf-8><title>Moss</title>\
<body style=\"font:16px system-ui;margin:20vh auto;max-width:28em;text-align:center\">\
<h1>You're signed in</h1><p>You can close this tab and go back to Moss.</p>";
const PAGE_FAILED: &str = "<!doctype html><meta charset=utf-8><title>Moss</title>\
<body style=\"font:16px system-ui;margin:20vh auto;max-width:28em;text-align:center\">\
<h1>Sign-in didn't finish</h1><p>Go back to Moss to see what happened.</p>";
const PAGE_RETRY: &str = "<!doctype html><meta charset=utf-8><title>Moss</title>\
<body style=\"font:16px system-ui;margin:20vh auto;max-width:28em;text-align:center\">\
<h1>This link isn't for this sign-in</h1><p>Go back to Moss and start again.</p>";

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn a_local_listener_failure_is_unreachable_on_this_mac() {
        let e = std::io::Error::new(std::io::ErrorKind::AddrInUse, "port taken");
        let expected = SignInError::Unreachable { host: "127.0.0.1".into(), detail: "port taken".into() };
        assert_eq!(local_error(&e), expected);
    }

    pub fn endpoints(server: &MockServer) -> OAuthEndpoints {
        OAuthEndpoints {
            microsoft_authorize: format!("{}/ms/authorize", server.uri()),
            microsoft_token: format!("{}/ms/token", server.uri()),
            google_authorize: format!("{}/google/auth", server.uri()),
            google_token: format!("{}/google/token", server.uri()),
        }
    }

    fn microsoft(server: &MockServer) -> OAuthConfig {
        OAuthConfig::microsoft(Some("my-client"), &endpoints(server)).unwrap()
    }

    fn google(server: &MockServer) -> OAuthConfig {
        OAuthConfig::google("g-client", Some("g-secret"), &endpoints(server))
    }

    /// Plays the browser: requests `target` on the loopback listener and returns the reply.
    pub async fn browser(redirect_uri: &str, target: &str) -> String {
        let addr = redirect_uri
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        reply
    }

    async fn raw(redirect_uri: &str, bytes: &[u8]) -> String {
        let addr = redirect_uri
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(bytes).await.unwrap();
        stream.shutdown().await.unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        reply
    }

    fn query(url: &str) -> std::collections::HashMap<String, String> {
        Url::parse(url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    /// An OpenID id_token carrying `claims` (header and signature don't matter to Moss).
    pub fn id_token(claims: serde_json::Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    pub fn token_json(access: &str, refresh: Option<&str>) -> serde_json::Value {
        let mut json = serde_json::json!({ "access_token": access, "token_type": "Bearer", "expires_in": 3599 });
        if let Some(refresh) = refresh {
            json["refresh_token"] = refresh.into();
        }
        json
    }

    #[test]
    fn microsoft_uses_moss_client_id_unless_the_user_brings_one() {
        let e = OAuthEndpoints::default();
        let own = OAuthConfig::microsoft_with_default(Some(" mine "), "moss-id", &e).unwrap();
        assert_eq!(own.client_id, "mine");
        let moss = OAuthConfig::microsoft_with_default(Some(""), "moss-id", &e).unwrap();
        assert_eq!(moss.client_id, "moss-id");
        assert_eq!(moss.client_secret, None);
        assert!(moss
            .scopes
            .iter()
            .any(|s| s.ends_with("IMAP.AccessAsUser.All")));
        assert!(moss.scopes.iter().any(|s| s.ends_with("SMTP.Send")));
        assert!(moss.scopes.contains(&"offline_access".to_string()));
        assert_eq!(
            OAuthConfig::microsoft_with_default(None, MICROSOFT_CLIENT_ID_PLACEHOLDER, &e),
            Err(SignInError::ClientIdNotConfigured)
        );
    }

    #[test]
    fn outlook_works_out_of_the_box_with_mosss_own_client_id() {
        let e = OAuthEndpoints::default();
        assert_ne!(MOSS_MICROSOFT_CLIENT_ID, MICROSOFT_CLIENT_ID_PLACEHOLDER);
        let config = OAuthConfig::microsoft(None, &e).unwrap();
        assert_eq!(config.client_id, MOSS_MICROSOFT_CLIENT_ID);
        assert_eq!(config.client_secret, None);
    }

    #[test]
    fn google_trims_and_drops_an_empty_secret() {
        let e = OAuthEndpoints::default();
        let config = OAuthConfig::google(" id ", Some("  "), &e);
        assert_eq!(config.client_id, "id");
        assert_eq!(config.client_secret, None);
        assert_eq!(
            config.scopes,
            vec!["https://mail.google.com/", "openid", "email"]
        );
        assert!(config
            .authorize_url
            .starts_with("https://accounts.google.com/"));
    }

    #[test]
    fn debug_hides_the_client_secret() {
        let config = OAuthConfig::google("id", Some("very-secret"), &OAuthEndpoints::default());
        let shown = format!("{config:?}");
        assert!(!shown.contains("very-secret"));
        assert!(shown.contains("<redacted>"));
    }

    #[test]
    fn pkce_challenge_matches_rfc_7636() {
        // Appendix B of RFC 7636.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let a = random_token(32);
        assert_eq!(a.len(), 43);
        assert_ne!(a, random_token(32));
    }

    #[tokio::test]
    async fn authorize_url_carries_pkce_state_and_loopback_redirect() {
        let server = MockServer::start().await;
        let pending = PendingAuthorization::start(microsoft(&server), Some("maya@outlook.com"))
            .await
            .unwrap();
        assert!(pending
            .authorize_url()
            .starts_with(&format!("{}/ms/authorize?", server.uri())));
        let q = query(pending.authorize_url());
        assert_eq!(q["client_id"], "my-client");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["redirect_uri"], pending.redirect_uri());
        assert!(pending.redirect_uri().starts_with("http://127.0.0.1:"));
        assert_eq!(q["code_challenge"], pkce_challenge(&pending.verifier));
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["state"], pending.state);
        assert_eq!(q["prompt"], "select_account");
        assert_eq!(q["login_hint"], "maya@outlook.com");
        let scopes: Vec<&str> = q["scope"].split(' ').collect();
        for scope in ["offline_access", "openid", "email", "profile"] {
            assert!(scopes.contains(&scope), "{scope} missing from {scopes:?}");
        }
        assert_eq!(pending.config().issuer, OAuthIssuer::Microsoft);
        assert!(!format!("{pending:?}").contains(&pending.verifier));

        let pending = PendingAuthorization::start(google(&server), None)
            .await
            .unwrap();
        let q = query(pending.authorize_url());
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "consent");
        assert!(!q.contains_key("login_hint"));
    }

    #[tokio::test]
    async fn a_bad_authorize_url_is_a_client_problem() {
        let mut config = OAuthConfig::google("id", None, &OAuthEndpoints::default());
        config.authorize_url = "not a url".into();
        let err = PendingAuthorization::start(config, None).await.unwrap_err();
        assert!(matches!(err, SignInError::InvalidOAuthClient { .. }));
    }

    #[tokio::test]
    async fn finishing_swaps_the_code_for_tokens() {
        let server = MockServer::start().await;
        let pending = PendingAuthorization::start(microsoft(&server), None)
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/ms/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .and(body_string_contains(format!(
                "code_verifier={}",
                pending.verifier
            )))
            .and(body_string_contains("client_id=my-client"))
            .and(body_string_contains("offline_access"))
            .respond_with(ResponseTemplate::new(200).set_body_json({
                let mut json = token_json("at", Some("rt"));
                json["id_token"] =
                    id_token(serde_json::json!({ "preferred_username": "maya@outlook.com" }))
                        .into();
                json
            }))
            .expect(1)
            .mount(&server)
            .await;
        let redirect = pending.redirect_uri().to_string();
        let state = pending.state.clone();
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let finish =
            tokio::spawn(async move { pending.finish(&http, Duration::from_secs(5)).await });

        // Noise first: a favicon, a stale tab, a malformed request, an empty reply.
        assert!(browser(&redirect, "/favicon.ico")
            .await
            .starts_with("HTTP/1.1 404"));
        assert!(browser(&redirect, "/?code=old&state=stale")
            .await
            .starts_with("HTTP/1.1 400"));
        assert!(raw(&redirect, b"POST / HTTP/1.1\r\n\r\n")
            .await
            .starts_with("HTTP/1.1 400"));
        assert!(raw(&redirect, b"").await.starts_with("HTTP/1.1 400"));
        assert!(browser(&redirect, &format!("/?state={state}"))
            .await
            .starts_with("HTTP/1.1 400"));

        let reply = browser(&redirect, &format!("/?code=the-code&state={state}&scope=x")).await;
        assert!(reply.starts_with("HTTP/1.1 200"));
        assert!(reply.contains("You're signed in"));

        let signed_in = finish.await.unwrap().unwrap();
        assert_eq!(signed_in.address.as_deref(), Some("maya@outlook.com"));
        let tokens = signed_in.tokens;
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        let now = crate::secret::now_unix();
        assert!(tokens.expires_at > now + 3500 && tokens.expires_at <= now + 3599);
        assert!(tokens.refresh_issued_at <= now);
    }

    #[tokio::test]
    async fn google_sends_its_client_secret() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/google/token"))
            .and(body_string_contains("client_secret=g-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("at", Some("rt"))))
            .expect(1)
            .mount(&server)
            .await;
        let pending = PendingAuthorization::start(google(&server), None)
            .await
            .unwrap();
        let (redirect, state) = (pending.redirect_uri().to_string(), pending.state.clone());
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let finish =
            tokio::spawn(async move { pending.finish(&http, Duration::from_secs(5)).await });
        browser(&redirect, &format!("/?code=c&state={state}")).await;
        let signed_in = finish.await.unwrap().unwrap();
        assert_eq!(signed_in.tokens.access_token, "at");
        assert_eq!(signed_in.address, None);
    }

    #[test]
    fn the_sign_in_answer_names_the_account() {
        let named = |claims| address_in(&id_token(claims));
        // Work and school accounts, and Google, send `email`; it wins.
        assert_eq!(
            named(serde_json::json!({ "email": " maya@northwind.co ", "preferred_username": "m@x.io" }))
                .as_deref(),
            Some("maya@northwind.co")
        );
        // Personal Microsoft accounts often send only `preferred_username`.
        assert_eq!(
            named(serde_json::json!({ "preferred_username": "maya@outlook.com" })).as_deref(),
            Some("maya@outlook.com")
        );
        // A phone number isn't an address.
        assert_eq!(
            named(serde_json::json!({ "preferred_username": "+44 7700 900123" })),
            None
        );
        assert_eq!(named(serde_json::json!({ "name": "Maya" })), None);
        assert_eq!(address_in("no-dots"), None);
        assert_eq!(address_in("e30.!!!.sig"), None);
        assert_eq!(
            address_in(&format!("e30.{}.sig", URL_SAFE_NO_PAD.encode("not json"))),
            None
        );
    }

    #[tokio::test]
    async fn an_error_redirect_is_classified() {
        let server = MockServer::start().await;
        let pending = PendingAuthorization::start(microsoft(&server), None)
            .await
            .unwrap();
        let (redirect, state) = (pending.redirect_uri().to_string(), pending.state.clone());
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let finish =
            tokio::spawn(async move { pending.finish(&http, Duration::from_secs(5)).await });
        let reply = browser(
            &redirect,
            &format!("/?error=consent_required&error_description=AADSTS65001%3A+admin+needed&state={state}"),
        )
        .await;
        assert!(reply.contains("Sign-in didn't finish"));
        assert_eq!(
            finish.await.unwrap().unwrap_err(),
            SignInError::AdminApprovalRequired {
                service: MailService::Outlook,
                detail: "OAuth error: AADSTS65001.".into()
            }
        );
    }

    #[tokio::test]
    async fn a_connection_that_never_asks_for_anything_doesnt_hold_up_the_one_that_does() {
        let server = MockServer::start().await;
        let mut pending = PendingAuthorization::start(microsoft(&server), None).await.unwrap();
        pending.request_wait = Duration::from_millis(100);
        let (redirect, state) = (pending.redirect_uri().to_string(), pending.state.clone());
        // Browsers open spare connections ahead of time and may send nothing on them.
        let _spare = TcpStream::connect(redirect.trim_start_matches("http://").trim_end_matches('/')).await.unwrap();
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let finish = tokio::spawn(async move { pending.finish(&http, Duration::from_secs(5)).await });
        let reply = browser(&redirect, &format!("/?error=access_denied&state={state}")).await;
        assert!(reply.contains("Sign-in didn't finish"));
        assert!(finish.await.unwrap().is_err());
        assert_eq!(REQUEST_WAIT, Duration::from_secs(2));
    }

    #[tokio::test]
    async fn waiting_too_long_times_out() {
        let server = MockServer::start().await;
        let pending = PendingAuthorization::start(microsoft(&server), None)
            .await
            .unwrap();
        let err = pending
            .finish(&reqwest::Client::builder().no_proxy().build().unwrap(), Duration::from_millis(20))
            .await
            .unwrap_err();
        assert_eq!(err, SignInError::Timeout);
    }

    async fn finish_with_token_reply(
        server: &MockServer,
        reply: ResponseTemplate,
    ) -> Result<SignedIn, SignInError> {
        Mock::given(method("POST"))
            .and(path("/ms/token"))
            .respond_with(reply)
            .mount(server)
            .await;
        let pending = PendingAuthorization::start(microsoft(server), None)
            .await
            .unwrap();
        let (redirect, state) = (pending.redirect_uri().to_string(), pending.state.clone());
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let finish =
            tokio::spawn(async move { pending.finish(&http, Duration::from_secs(5)).await });
        browser(&redirect, &format!("/?code=c&state={state}")).await;
        finish.await.unwrap()
    }

    #[tokio::test]
    async fn token_endpoint_errors_are_typed() {
        let server = MockServer::start().await;
        let err = finish_with_token_reply(
            &server,
            ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "AADSTS65001: The user or administrator has not consented.",
                "error_codes": [65001]
            })),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            SignInError::AdminApprovalRequired {
                service: MailService::Outlook,
                ..
            }
        ));

        let server = MockServer::start().await;
        let err = finish_with_token_reply(
            &server,
            ResponseTemplate::new(502).set_body_string("<html>gateway</html>"),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            SignInError::UnexpectedResponse {
                host: "127.0.0.1".into(),
                detail: "HTTP 502 Bad Gateway".into()
            }
        );

        let server = MockServer::start().await;
        let err =
            finish_with_token_reply(&server, ResponseTemplate::new(200).set_body_string("{}"))
                .await
                .unwrap_err();
        assert!(matches!(err, SignInError::UnexpectedResponse { .. }));
    }

    #[tokio::test]
    async fn an_unreachable_token_endpoint_is_reported() {
        let mut config = OAuthConfig::google("id", None, &OAuthEndpoints::default());
        config.token_url = "http://127.0.0.1:1/token".into();
        let tokens = TokenSet {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at: 0,
            refresh_issued_at: 0,
        };
        let err = refresh(&reqwest::Client::builder().no_proxy().build().unwrap(), &config, &tokens, 10)
            .await
            .unwrap_err();
        assert!(matches!(err, SignInError::Unreachable { ref host, .. } if host == "127.0.0.1"));
    }

    #[tokio::test]
    async fn a_body_that_breaks_off_is_unreachable() {
        // Promises more bytes than it sends, so reading the body fails.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                .await
                .unwrap();
        });
        let mut config = OAuthConfig::google("id", None, &OAuthEndpoints::default());
        config.token_url = format!("http://127.0.0.1:{port}/token");
        let tokens = TokenSet {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at: 0,
            refresh_issued_at: 0,
        };
        let err = refresh(&reqwest::Client::builder().no_proxy().build().unwrap(), &config, &tokens, 10)
            .await
            .unwrap_err();
        assert!(matches!(err, SignInError::Unreachable { .. }));
    }

    #[tokio::test]
    async fn refresh_keeps_or_rotates_the_refresh_token() {
        let server = MockServer::start().await;
        let tokens = TokenSet {
            access_token: "old".into(),
            refresh_token: Some("r1".into()),
            expires_at: 100,
            refresh_issued_at: 50,
        };
        Mock::given(method("POST"))
            .and(path("/google/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=r1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("new", None)))
            .mount(&server)
            .await;
        let fresh = refresh(&reqwest::Client::builder().no_proxy().build().unwrap(), &google(&server), &tokens, 1_000)
            .await
            .unwrap();
        assert_eq!(
            fresh,
            TokenSet {
                access_token: "new".into(),
                refresh_token: Some("r1".into()),
                expires_at: 4_599,
                refresh_issued_at: 50
            }
        );

        Mock::given(method("POST"))
            .and(path("/ms/token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"access_token": "new", "refresh_token": "r2"}),
                ),
            )
            .mount(&server)
            .await;
        let fresh = refresh(&reqwest::Client::builder().no_proxy().build().unwrap(), &microsoft(&server), &tokens, 1_000)
            .await
            .unwrap();
        assert_eq!(
            fresh,
            TokenSet {
                access_token: "new".into(),
                refresh_token: Some("r2".into()),
                expires_at: 4_600,
                refresh_issued_at: 1_000
            }
        );

        let none = TokenSet {
            refresh_token: None,
            ..tokens
        };
        assert_eq!(
            refresh(&reqwest::Client::builder().no_proxy().build().unwrap(), &google(&server), &none, 1).await,
            Err(SignInError::SignInExpired)
        );
    }

    #[test]
    fn token_without_refresh_token_on_first_exchange() {
        let response = TokenResponse {
            access_token: "a".into(),
            refresh_token: None,
            expires_in: None,
            id_token: None,
        };
        assert_eq!(
            response.into_tokens(10, None),
            TokenSet {
                access_token: "a".into(),
                refresh_token: None,
                expires_at: 3_610,
                refresh_issued_at: 10
            }
        );
    }

    #[tokio::test]
    async fn refresh_failures_tell_testing_apps_from_revoked_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant", "error_description": "Token has been expired or revoked."
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let issued = 1_000;
        let tokens = TokenSet {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at: 0,
            refresh_issued_at: issued,
        };
        let week = GOOGLE_TESTING_TOKEN_LIFETIME;
        assert_eq!(
            refresh(&http, &google(&server), &tokens, issued + week).await,
            Err(SignInError::GoogleTestingExpired)
        );
        assert_eq!(
            refresh(&http, &google(&server), &tokens, issued + week - 1).await,
            Err(SignInError::SignInExpired)
        );
        assert_eq!(
            refresh(&http, &microsoft(&server), &tokens, issued + week).await,
            Err(SignInError::SignInExpired)
        );
    }

    #[test]
    fn oauth_codes_match_whole_tokens_and_keep_the_exact_known_code() {
        let config = OAuthConfig::microsoft(Some("id"), &OAuthEndpoints::default()).unwrap();
        for (description, expected_kind, expected_detail) in [
            ("AADSTS900941: approval", "adminApprovalRequired", "OAuth error: AADSTS900941."),
            ("[AADSTS90094] approval", "adminApprovalRequired", "OAuth error: AADSTS90094."),
            ("(AADSTS65001), approval", "adminApprovalRequired", "OAuth error: AADSTS65001."),
            ("AADSTS650011", "unexpectedResponse", "OAuth error: invalid_request."),
            ("prefixAADSTS65001", "unexpectedResponse", "OAuth error: invalid_request."),
            ("AADSTS65001suffix", "unexpectedResponse", "OAuth error: invalid_request."),
            ("prefix_AADSTS65001", "unexpectedResponse", "OAuth error: invalid_request."),
            ("AADSTS65001_suffix", "unexpectedResponse", "OAuth error: invalid_request."),
            ("AADSTS50020 synthetic-private-marker", "unexpectedResponse", "OAuth error: invalid_request."),
        ] {
            let value = serde_json::to_value(classify(&config, "invalid_request", description, "provider.test", Phase::Exchange)).unwrap();
            assert_eq!(value["kind"], expected_kind, "{description}");
            assert_eq!(value["detail"], expected_detail, "{description}");
        }
    }

    #[test]
    fn oauth_errors_do_not_serialize_external_details() {
        let config = OAuthConfig::microsoft(Some("id"), &OAuthEndpoints::default()).unwrap();
        let marker = "synthetic-private-oauth-marker";
        let mut leaks = Vec::new();
        for (error, description) in [
            ("access_denied", marker.to_string()),
            ("invalid_grant", marker.to_string()),
            ("invalid_request", format!("AADSTS65001 {marker}")),
            ("invalid_client", format!("AADSTS7000218 {marker}")),
            ("server_error", marker.to_string()),
            (marker, String::new()),
        ] {
            let result = classify(&config, error, &description, "provider.test", Phase::Exchange);
            let shown = format!("{result:?} {result} {}", serde_json::to_string(&result).unwrap());
            if shown.contains(marker) { leaks.push(error); }
        }
        assert!(leaks.is_empty(), "OAuth details exposed through {leaks:?}");
    }

    #[tokio::test]
    async fn malformed_token_values_do_not_enter_errors() {
        let server = MockServer::start().await;
        let marker = "synthetic-private-token-marker";
        let error = finish_with_token_reply(&server, ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token":"synthetic-access-token", "token_type":"Bearer", "expires_in":marker
        }))).await.unwrap_err();
        assert!(matches!(error, SignInError::UnexpectedResponse { .. }));
        assert!(!format!("{error:?} {error} {}", serde_json::to_string(&error).unwrap()).contains(marker));
    }

    #[test]
    fn provider_errors_map_to_screens() {
        let e = OAuthEndpoints::default();
        let ms = OAuthConfig::microsoft(Some("id"), &e).unwrap();
        let g = OAuthConfig::google("id", None, &e);
        let c = |config: &OAuthConfig, error: &str, description: &str, phase| {
            classify(config, error, description, "h", phase)
        };

        for description in [
            "AADSTS90094: admin permission",
            "AADSTS900941: Administrator consent is required",
        ] {
            assert!(matches!(
                c(&ms, "invalid_request", description, Phase::Authorize),
                SignInError::AdminApprovalRequired { .. }
            ));
        }
        assert_eq!(
            c(&g, "admin_policy_enforced", "", Phase::Authorize),
            SignInError::AdminApprovalRequired {
                service: MailService::Gmail,
                detail: "OAuth error: admin_policy_enforced.".into()
            }
        );
        for (error, description) in [
            ("invalid_client", ""),
            ("unauthorized_client", ""),
            ("redirect_uri_mismatch", ""),
            (
                "invalid_request",
                "AADSTS50011: The redirect URI does not match",
            ),
            (
                "invalid_request",
                "AADSTS700016: Application not found in the directory",
            ),
            (
                "invalid_client",
                "AADSTS7000218: client_assertion or client_secret required",
            ),
        ] {
            assert_eq!(
                serde_json::to_value(c(&ms, error, description, Phase::Exchange)).unwrap()["kind"],
                "invalidOAuthClient",
                "{error} {description}",
            );
        }
        assert_eq!(
            c(&ms, "access_denied", "The user cancelled", Phase::Authorize),
            SignInError::AccessDenied {
                detail: "OAuth error: access_denied.".into()
            }
        );
        assert!(matches!(
            c(&g, "invalid_grant", "Bad Request", Phase::Exchange),
            SignInError::AccessDenied { .. }
        ));
        assert_eq!(
            c(&g, "server_error", "oops", Phase::Exchange),
            SignInError::UnexpectedResponse {
                host: "h".into(),
                detail: "OAuth error: server_error.".into()
            }
        );
    }

    #[test]
    fn hosts_come_from_urls() {
        assert_eq!(
            host_of("https://login.microsoftonline.com/common"),
            "login.microsoftonline.com"
        );
        assert_eq!(host_of("malformed synthetic-private-url-marker"), "OAuth provider");
    }
}
