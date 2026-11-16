//! Resend: mail received on the user's domains through Resend's receiving
//! API, and sending through `POST /emails`.
//!
//! Resend keeps received mail for about 30 days and has no read or folder
//! state, so Moss's store is the only archive and `apply` changes nothing on
//! the server.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::CheckpointState;
use crate::{
    Capabilities, Change, Checkpoint, Error, Flags, Mailbox, MailboxRole, OutgoingMessage,
    Provider, RawMessage, Result, Secret, SentMessage, SyncBatch, Wakeup,
};

/// Where received mail is listed.
pub const INBOX: &str = "INBOX";
/// Sent copies live only in the local store; syncing it returns nothing.
pub const SENT: &str = "SENT";

const API: &str = "https://api.resend.com";

/// A domain on the Resend account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResendDomain {
    pub id: String,
    pub name: String,
    /// `verified`, `pending`, `not_started`, `failed`…
    pub status: String,
    pub sending: bool,
    pub receiving: bool,
}

/// Resend sync position. The list is newest first, so a run walks down from
/// the top until it reaches mail it already has.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResendCheckpoint {
    /// The newest email from the last finished run.
    high_water: Option<Mark>,
    /// A run that stopped part way down the list.
    run: Option<Run>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Mark {
    id: String,
    created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Run {
    /// The newest email seen in this run; becomes the high water when it ends.
    top: Mark,
    /// The last email fetched; the next page starts after it.
    cursor: String,
}

pub struct ResendProvider {
    http: reqwest::Client,
    base: String,
    key: Secret,
    limits: Limits,
}

/// How long Resend may take. The app uses the default; tests pass short ones.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Reaching Resend. Nothing was sent yet, so running out is safe to retry.
    pub connect: Duration,
    /// Any one read waiting with nothing arriving.
    pub read: Duration,
    /// A whole API call. Downloads have no total: they may take as long as they keep coming.
    pub api: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { connect: Duration::from_secs(10), read: Duration::from_secs(60), api: Duration::from_secs(60) }
    }
}

/// What a request is, for how long it may take and what a failure means.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Api,
    /// Sending: once the request went out, a failure is an unknown outcome.
    Submission,
    Download,
}

/// Resend allows at most 100 items a page.
const MAX_PAGE: usize = 100;
/// Requests a rate limit may hold up before Moss gives up.
const ATTEMPTS: usize = 3;
/// Longest rate-limit wait worth sitting through; longer ones go to the caller.
const MAX_WAIT: Duration = Duration::from_secs(5);

const QUOTA_NOTE: &str =
    "Resend didn't keep this email's content: the account was over its quota when it arrived.";

impl ResendProvider {
    pub fn new(api_key: Secret) -> Result<Self> {
        Self::with_base_url(api_key, API)
    }

    /// Talks to another address than `api.resend.com` (tests, proxies).
    pub fn with_base_url(api_key: Secret, base_url: &str) -> Result<Self> {
        Self::with_limits(api_key, base_url, Limits::default())
    }

    pub(crate) fn with_limits(api_key: Secret, base_url: &str, limits: Limits) -> Result<Self> {
        let tls = crate::tls::client_config(&crate::TlsRoots::Platform)?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("Moss/", env!("CARGO_PKG_VERSION")))
            .tls_backend_preconfigured((*tls).clone())
            .connect_timeout(limits.connect)
            .read_timeout(limits.read)
            .build()
            .map_err(network)?;
        Ok(ResendProvider {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            key: api_key,
            limits,
        })
    }

    /// Every domain on the account, with whether it can send and receive.
    pub async fn domains(&self) -> Result<Vec<ResendDomain>> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page: Page<DomainJson> = self.list("/domains", after.as_deref(), MAX_PAGE).await?;
            after = page.data.last().map(|d| d.id.clone());
            out.extend(page.data.into_iter().map(|d| ResendDomain {
                sending: d.capabilities.sending == "enabled",
                receiving: d.capabilities.receiving == "enabled",
                id: d.id,
                name: d.name,
                status: d.status,
            }));
            if !page.has_more || after.is_none() {
                return Ok(out);
            }
        }
    }

    async fn list<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Page<T>> {
        let mut query = vec![("limit", limit.clamp(1, MAX_PAGE).to_string())];
        if let Some(after) = after {
            query.push(("after", after.to_string()));
        }
        let url = format!("{}{path}", self.base);
        let response = self
            .call(|| {
                self.http
                    .get(&url)
                    .query(&query)
                    .bearer_auth(self.key.expose())
            })
            .await?;
        json(response).await
    }

    async fn received(&self, id: &str) -> std::result::Result<Received, (Error, Option<String>)> {
        let url = format!("{}/emails/receiving/{id}", self.base);
        let response = self
            .call_named(|| {
                self.http
                    .get(&url)
                    .query(&[("html_format", "cid")])
                    .bearer_auth(self.key.expose())
            })
            .await?;
        json(response).await.map_err(|e| (e, None))
    }

    /// A signed download link. Never send the API key there.
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.request_named(|| self.http.get(url), Kind::Download).await.map_err(|(e, _)| e)?;
        let bytes = response.bytes().await.map_err(network)?;
        Ok(bytes.to_vec())
    }

    /// Sends a request, sitting out short rate limits, and maps error statuses.
    async fn call(
        &self,
        request: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        self.call_named(request).await.map_err(|(e, _)| e)
    }

    /// Like `call`, but a failure also carries Resend's error name.
    async fn call_named(
        &self,
        request: impl Fn() -> reqwest::RequestBuilder,
    ) -> std::result::Result<reqwest::Response, (Error, Option<String>)> {
        self.request_named(request, Kind::Api).await
    }

    async fn request_named(
        &self,
        request: impl Fn() -> reqwest::RequestBuilder,
        kind: Kind,
    ) -> std::result::Result<reqwest::Response, (Error, Option<String>)> {
        let submission = kind == Kind::Submission;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut request = request();
            if kind != Kind::Download {
                request = request.timeout(self.limits.api);
            }
            let response = request.send().await.map_err(|e| {
                let uncertain = submission && !e.is_connect() && !e.is_builder();
                let error = if uncertain { Error::DeliveryUnknown("Resend acceptance could not be confirmed.".into()) } else { network(e) };
                (error, None)
            })?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait = retry_after(response.headers());
                if attempt < ATTEMPTS && wait <= MAX_WAIT {
                    tokio::time::sleep(wait).await;
                    continue;
                }
                return Err((Error::RateLimited { retry_after: wait }, None));
            }
            let body = read_api_error(response).await;
            let message = format!("The Resend request failed (HTTP {}).", status.as_u16());
            let error = match (status.as_u16(), body.name.as_deref()) {
                (401, _)
                | (403, Some("restricted_api_key" | "invalid_permission" | "suspended_api_key")) => {
                    Error::Auth(message)
                }
                (404, _) => Error::NotFound(message),
                (409 | 500.., _) if submission => Error::DeliveryUnknown(message),
                (500.., _) => Error::Network(message),
                _ => Error::Protocol(message),
            };
            return Err((error, body.name));
        }
    }

    async fn message(&self, id: &str) -> Result<RawMessage> {
        let email = self.received(id).await.map_err(|(e, _)| e)?;
        self.complete(email).await
    }

    /// Fetches one listed email; one Resend dropped for quota becomes a note.
    async fn listed_message(&self, item: &Listed) -> Result<RawMessage> {
        match self.received(&item.id).await {
            Ok(email) => self.complete(email).await,
            Err((_, Some(name))) if name == "email_above_quota" => Ok(raw_message(
                item,
                rebuild(item, None, Some(QUOTA_NOTE), None, &[], Some("above-quota")),
            )),
            Err((e, _)) => Err(e),
        }
    }

    async fn complete(&self, email: Received) -> Result<RawMessage> {
        let raw = match &email.raw {
            Some(link) => self.download(&link.download_url).await?,
            None => {
                let attachments = self.attachments(&email.meta.id).await?;
                rebuild(
                    &email.meta,
                    email.headers.as_ref(),
                    email.text.as_deref(),
                    email.html.as_deref(),
                    &attachments,
                    None,
                )
            }
        };
        Ok(raw_message(&email.meta, raw))
    }

    async fn attachments(&self, id: &str) -> Result<Vec<crate::Attachment>> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page: Page<AttachmentJson> = self
                .list(
                    &format!("/emails/receiving/{id}/attachments"),
                    after.as_deref(),
                    MAX_PAGE,
                )
                .await?;
            after = page.data.last().map(|a| a.id.clone());
            for a in page.data {
                let data = self.download(&a.download_url).await?;
                let inline = a.content_disposition.as_deref() != Some("attachment");
                out.push(crate::Attachment {
                    filename: a.filename.unwrap_or_else(|| "attachment".into()),
                    content_type: a.content_type.unwrap_or_default(),
                    content_id: a.content_id.filter(|_| inline),
                    data,
                });
            }
            if !page.has_more || after.is_none() {
                return Ok(out);
            }
        }
    }
}

#[async_trait]
impl Provider for ResendProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            server_flags: false,
        }
    }

    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
        let mailbox = |id: &str, name: &str, role| Mailbox {
            id: id.into(),
            name: name.into(),
            delimiter: None,
            role: Some(role),
            selectable: true,
        };
        Ok(vec![
            mailbox(INBOX, "Inbox", MailboxRole::Inbox),
            mailbox(SENT, "Sent", MailboxRole::Sent),
        ])
    }

    async fn fetch_since(
        &self,
        mailbox: &str,
        checkpoint: Option<&Checkpoint>,
        limit: usize,
    ) -> Result<SyncBatch> {
        let mut cp = match checkpoint.map(|c| &c.0) {
            None => ResendCheckpoint::default(),
            Some(CheckpointState::Resend(cp)) => cp.clone(),
            Some(_) => return Err(Error::InvalidCheckpoint),
        };
        let batch = |messages, cp, more| SyncBatch {
            reset: false,
            messages,
            flag_updates: Vec::new(),
            vanished: Vec::new(),
            checkpoint: Checkpoint(CheckpointState::Resend(cp)),
            more,
        };
        match mailbox {
            INBOX => {}
            SENT => return Ok(batch(Vec::new(), cp, false)),
            other => return Err(Error::NotFound(format!("Resend has no mailbox {other:?}"))),
        }

        let cursor = cp.run.as_ref().map(|r| r.cursor.clone());
        let page: Page<Listed> = self
            .list("/emails/receiving", cursor.as_deref(), limit)
            .await?;
        let mut messages = Vec::new();
        let mut reached_known = false;
        for item in &page.data {
            if cp
                .high_water
                .as_ref()
                .is_some_and(|hw| item.id == hw.id || item.created_at < hw.created_at)
            {
                reached_known = true;
                break;
            }
            messages.push(self.listed_message(item).await?);
        }

        let top = cp.run.take().map(|r| r.top).or_else(|| {
            page.data
                .first()
                .filter(|_| !messages.is_empty())
                .map(|i| Mark {
                    id: i.id.clone(),
                    created_at: i.created_at,
                })
        });
        let last = messages.last().map(|m: &RawMessage| m.provider_id.clone());
        let more = !reached_known && page.has_more && last.is_some();
        if more {
            cp.run = top.zip(last).map(|(top, cursor)| Run { top, cursor });
        } else if top.is_some() {
            cp.high_water = top;
        }
        Ok(batch(messages, cp, more))
    }

    async fn fetch_message(&self, mailbox: &str, provider_id: &str) -> Result<RawMessage> {
        if mailbox != INBOX {
            return Err(Error::NotFound(format!(
                "Resend keeps no messages in {mailbox:?}"
            )));
        }
        self.message(provider_id).await
    }

    async fn apply(
        &self,
        _mailbox: &str,
        _provider_ids: &[String],
        _change: &Change,
    ) -> Result<()> {
        // Resend has no read or folder state: the local store is the only copy.
        Ok(())
    }

    async fn send(&self, message: &OutgoingMessage) -> Result<SentMessage> {
        let built = crate::mime::build(message, Utc::now(), &mut crate::mime::new_boundary)?;
        let addresses = |list: &[crate::Address]| -> Result<Vec<String>> {
            list.iter().map(crate::mime::format_address).collect()
        };
        let mut headers = serde_json::Map::new();
        headers.insert("Message-ID".into(), built.message_id.clone().into());
        if let Some(parent) = &message.in_reply_to {
            headers.insert("In-Reply-To".into(), crate::mime::angle(parent).into());
        }
        if !message.references.is_empty() {
            let refs: Vec<String> = message
                .references
                .iter()
                .map(|r| crate::mime::angle(r))
                .collect();
            headers.insert("References".into(), refs.join(" ").into());
        }
        for (name, value) in &message.headers {
            headers.insert(name.clone(), value.clone().into());
        }
        let body = SendJson {
            from: crate::mime::format_address(&message.from)?,
            to: addresses(&message.to)?,
            cc: addresses(&message.cc)?,
            bcc: addresses(&message.bcc)?,
            reply_to: addresses(&message.reply_to)?,
            subject: message.subject.clone(),
            // An empty text stops Resend from making one up from the HTML.
            text: message
                .text
                .clone()
                .or_else(|| message.html.is_none().then(String::new)),
            html: message.html.clone(),
            headers,
            attachments: message
                .attachments
                .iter()
                .map(|a| AttachmentOut {
                    filename: a.filename.clone(),
                    content: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &a.data,
                    ),
                    content_type: (!a.content_type.is_empty()).then(|| a.content_type.clone()),
                    content_id: a.content_id.clone(),
                })
                .collect(),
        };
        let url = format!("{}/emails", self.base);
        let response = self
            .request_named(|| {
                self.http
                    .post(&url)
                    .bearer_auth(self.key.expose())
                    .header("Idempotency-Key", &built.message_id)
                    .json(&body)
            }, Kind::Submission)
            .await.map_err(|(error, _)| error)?;
        let sent: SentJson = json(response).await.map_err(|_| Error::DeliveryUnknown("Resend returned success without a readable receipt".into()))?;
        if sent.id.trim().is_empty() {
            return Err(Error::DeliveryUnknown("Resend returned success without a delivery identifier".into()));
        }
        Ok(SentMessage {
            provider_id: Some(sent.id),
            message_id: built.message_id,
            raw: built.archive,
            saved_on_server: false,
        })
    }

    async fn wait_for_change(&self, _mailbox: &str, timeout: Duration) -> Result<Wakeup> {
        tokio::time::sleep(timeout).await;
        Ok(Wakeup::Timeout)
    }
}

fn raw_message(meta: &Listed, raw: Vec<u8>) -> RawMessage {
    RawMessage {
        provider_id: meta.id.clone(),
        mailbox: INBOX.into(),
        flags: Flags::default(),
        received_at: Some(meta.created_at),
        raw,
        labels: Vec::new(),
        gmail_msg_id: None,
        headers_only: false,
    }
}

/// An RFC 822 message from Resend's JSON, for mail it has no raw copy of.
fn rebuild(
    meta: &Listed,
    original: Option<&serde_json::Map<String, serde_json::Value>>,
    text: Option<&str>,
    html: Option<&str>,
    attachments: &[crate::Attachment],
    missing: Option<&str>,
) -> Vec<u8> {
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in original.into_iter().flatten() {
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "content-type" | "content-transfer-encoding" | "mime-version"
        ) {
            continue;
        }
        let values = match value {
            serde_json::Value::String(v) => vec![v.as_str()],
            serde_json::Value::Array(vs) => vs.iter().filter_map(|v| v.as_str()).collect(),
            _ => Vec::new(),
        };
        for v in values {
            headers.push((name.clone(), unfold_safely(v)));
        }
    }
    let has = |headers: &[(String, String)], name: &str| {
        headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    };
    let mut add = |name: &str, value: String| {
        if !value.is_empty() && !has(&headers, name) {
            headers.push((name.into(), value));
        }
    };
    add("From", meta.from.clone());
    add("To", meta.to.join(", "));
    add("Cc", meta.cc.join(", "));
    add("Reply-To", meta.reply_to.join(", "));
    add("Subject", crate::mime::encode_header(&meta.subject));
    add("Date", meta.created_at.to_rfc2822());
    add("Message-ID", meta.message_id.clone().unwrap_or_default());
    headers.push(("X-Moss-Rebuilt".into(), "resend-json".into()));
    if let Some(reason) = missing {
        headers.push(("X-Moss-Content-Missing".into(), reason.into()));
    }
    let body = crate::mime::Body {
        text,
        html,
        attachments,
    };
    crate::mime::write_message(&headers, &body, &mut crate::mime::new_boundary)
}

/// Header values with their line breaks made into proper CRLF folds.
fn unfold_safely(value: &str) -> String {
    value
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .enumerate()
        .map(|(i, line)| {
            if i == 0 || line.starts_with([' ', '\t']) {
                line.to_string()
            } else {
                format!(" {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Duration {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(1))
}

fn network(_: reqwest::Error) -> Error {
    Error::Network("The connection to Resend failed.".into())
}

async fn json<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let bytes = response.bytes().await.map_err(network)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::Protocol("Resend returned an unreadable response.".into()))
}

/// Treats `null` like a missing field.
fn nullable<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct Page<T> {
    data: Vec<T>,
    #[serde(default)]
    has_more: bool,
}

// Error classification needs only a small JSON object. Never collect an
// unbounded server error body, even when Content-Length is absent or false.
const ERROR_BODY_LIMIT: usize = 16 * 1024;

async fn read_api_error(mut response: reqwest::Response) -> ApiError {
    if response.content_length().is_some_and(|length| length > ERROR_BODY_LIMIT as u64) {
        return ApiError::default();
    }
    let mut bytes = Vec::with_capacity(ERROR_BODY_LIMIT);
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if chunk.len() > ERROR_BODY_LIMIT - bytes.len() { return ApiError::default(); }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => return ApiError::default(),
        }
    }
    serde_json::from_slice(&bytes).unwrap_or_default()
}

#[derive(Default, Deserialize)]
struct ApiError {
    name: Option<String>,
}

#[derive(Clone, Deserialize)]
struct Listed {
    id: String,
    #[serde(default, deserialize_with = "nullable")]
    from: String,
    #[serde(default, deserialize_with = "nullable")]
    to: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    cc: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    reply_to: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    subject: String,
    created_at: DateTime<Utc>,
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Deserialize)]
struct Received {
    #[serde(flatten)]
    meta: Listed,
    #[serde(default)]
    html: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    headers: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    raw: Option<RawLink>,
}

#[derive(Deserialize)]
struct RawLink {
    download_url: String,
}

#[derive(Deserialize)]
struct AttachmentJson {
    id: String,
    filename: Option<String>,
    content_type: Option<String>,
    content_disposition: Option<String>,
    content_id: Option<String>,
    download_url: String,
}

#[derive(Deserialize)]
struct DomainJson {
    id: String,
    name: String,
    status: String,
    #[serde(default)]
    capabilities: DomainCapabilities,
}

#[derive(Default, Deserialize)]
struct DomainCapabilities {
    #[serde(default)]
    sending: String,
    #[serde(default)]
    receiving: String,
}

#[derive(Serialize)]
struct SendJson {
    from: String,
    to: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reply_to: Vec<String>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    headers: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<AttachmentOut>,
}

#[derive(Serialize)]
struct AttachmentOut {
    filename: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

#[derive(Deserialize)]
struct SentJson {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Address, Attachment};
    use serde_json::json;
    use wiremock::matchers::{
        body_json, header, method, path, query_param, query_param_is_missing,
    };
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const KEY: &str = "re_test_123";

    async fn server() -> (MockServer, ResendProvider) {
        let server = MockServer::start().await;
        let provider = ResendProvider::with_base_url(Secret::from(KEY), &server.uri()).unwrap();
        (server, provider)
    }

    fn listed(id: &str, at: &str) -> serde_json::Value {
        json!({
            "id": id, "to": ["me@moss.dev"], "from": "Bob <bob@example.com>", "created_at": at,
            "subject": format!("Mail {id}"), "bcc": [], "cc": [], "reply_to": [],
            "message_id": format!("<{id}@example.com>"), "attachments": []
        })
    }

    fn page(items: Vec<serde_json::Value>, has_more: bool) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_json(json!({ "object": "list", "has_more": has_more, "data": items }))
    }

    /// Mounts the detail and raw download for one email.
    async fn mount_email(server: &MockServer, id: &str, at: &str) {
        let mut detail = listed(id, at);
        detail["object"] = json!("email");
        detail["raw"] = json!({
            "download_url": format!("{}/raw/{id}?Signature=abc", server.uri()),
            "expires_at": "2026-11-01T10:00:00.000Z"
        });
        Mock::given(method("GET"))
            .and(path(format!("/emails/receiving/{id}")))
            .and(query_param("html_format", "cid"))
            .respond_with(ResponseTemplate::new(200).set_body_json(detail))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/raw/{id}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(format!("Subject: {id}\r\n\r\nbody {id}\r\n")),
            )
            .mount(server)
            .await;
    }

    fn ids(batch: &SyncBatch) -> Vec<&str> {
        batch
            .messages
            .iter()
            .map(|m| m.provider_id.as_str())
            .collect()
    }

    fn round_trip(cp: &Checkpoint) -> Checkpoint {
        serde_json::from_str(&serde_json::to_string(cp).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn syncs_page_by_page_then_only_new_mail() {
        let (server, provider) = server().await;
        for (id, at) in [
            ("d", "2026-10-31T12:00:00.000Z"),
            ("c", "2026-10-30T12:00:00.000Z"),
            ("b", "2026-10-29T12:00:00.000Z"),
            ("a", "2026-10-28T12:00:00.000Z"),
        ] {
            mount_email(&server, id, at).await;
        }
        // First run: c and b on page one, a on page two.
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .and(query_param("limit", "2"))
            .and(query_param_is_missing("after"))
            .respond_with(page(
                vec![
                    listed("c", "2026-10-30T12:00:00.000Z"),
                    listed("b", "2026-10-29T12:00:00.000Z"),
                ],
                true,
            ))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .and(query_param("after", "b"))
            .respond_with(page(vec![listed("a", "2026-10-28T12:00:00.000Z")], false))
            .mount(&server)
            .await;

        let first = provider.fetch_since(INBOX, None, 2).await.unwrap();
        assert_eq!(ids(&first), ["c", "b"]);
        assert!(first.more && !first.reset);
        let m = &first.messages[0];
        assert_eq!(m.mailbox, INBOX);
        assert_eq!(m.raw, b"Subject: c\r\n\r\nbody c\r\n");
        assert_eq!(m.flags, Flags::default());
        assert_eq!(
            m.received_at.unwrap().to_rfc3339(),
            "2026-10-30T12:00:00+00:00"
        );

        let second = provider
            .fetch_since(INBOX, Some(&round_trip(&first.checkpoint)), 2)
            .await
            .unwrap();
        assert_eq!(ids(&second), ["a"]);
        assert!(!second.more);

        // Later: d arrives on top of c.
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .and(query_param_is_missing("after"))
            .respond_with(page(
                vec![
                    listed("d", "2026-10-31T12:00:00.000Z"),
                    listed("c", "2026-10-30T12:00:00.000Z"),
                ],
                true,
            ))
            .mount(&server)
            .await;
        let third = provider
            .fetch_since(INBOX, Some(&round_trip(&second.checkpoint)), 2)
            .await
            .unwrap();
        assert_eq!(ids(&third), ["d"]);
        assert!(!third.more);

        // Nothing new: stops at d straight away.
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .and(query_param_is_missing("after"))
            .respond_with(page(vec![listed("d", "2026-10-31T12:00:00.000Z")], true))
            .with_priority(1)
            .mount(&server)
            .await;
        let fourth = provider
            .fetch_since(INBOX, Some(&third.checkpoint), 2)
            .await
            .unwrap();
        assert!(fourth.messages.is_empty() && !fourth.more);
        assert_eq!(fourth.checkpoint, third.checkpoint);
    }

    #[tokio::test]
    async fn a_run_interrupted_by_new_mail_still_reaches_the_old_high_water() {
        let (server, provider) = server().await;
        for (id, at) in [
            ("e", "2026-10-31T13:00:00.000Z"),
            ("d", "2026-10-31T12:00:00.000Z"),
        ] {
            mount_email(&server, id, at).await;
        }
        // High water is c; a run fetched e and stopped at cursor e.
        let cp = Checkpoint(CheckpointState::Resend(ResendCheckpoint {
            high_water: Some(Mark {
                id: "c".into(),
                created_at: "2026-10-30T12:00:00Z".parse().unwrap(),
            }),
            run: Some(Run {
                top: Mark {
                    id: "e".into(),
                    created_at: "2026-10-31T13:00:00Z".parse().unwrap(),
                },
                cursor: "e".into(),
            }),
        }));
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .and(query_param("after", "e"))
            .respond_with(page(
                vec![
                    listed("d", "2026-10-31T12:00:00.000Z"),
                    listed("c", "2026-10-30T12:00:00.000Z"),
                ],
                true,
            ))
            .mount(&server)
            .await;
        let batch = provider.fetch_since(INBOX, Some(&cp), 50).await.unwrap();
        assert_eq!(ids(&batch), ["d"]);
        assert!(!batch.more);
        let done: ResendCheckpoint =
            serde_json::from_value(serde_json::to_value(&batch.checkpoint).unwrap()).unwrap();
        assert_eq!(done.high_water.unwrap().id, "e");
        assert!(done.run.is_none());
    }

    #[tokio::test]
    async fn stops_at_older_mail_when_the_high_water_email_is_gone() {
        let (server, provider) = server().await;
        mount_email(&server, "d", "2026-10-31T12:00:00.000Z").await;
        let cp = Checkpoint(CheckpointState::Resend(ResendCheckpoint {
            high_water: Some(Mark {
                id: "deleted".into(),
                created_at: "2026-10-30T12:00:00Z".parse().unwrap(),
            }),
            run: None,
        }));
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .respond_with(page(
                vec![
                    listed("d", "2026-10-31T12:00:00.000Z"),
                    listed("b", "2026-10-29T12:00:00.000Z"),
                ],
                true,
            ))
            .mount(&server)
            .await;
        let batch = provider.fetch_since(INBOX, Some(&cp), 50).await.unwrap();
        assert_eq!(ids(&batch), ["d"]);
        assert!(!batch.more);
        // Resend has no headers-only fetch: the same, whole.
        let shallow = provider.fetch_since_shallow(INBOX, Some(&cp), 50).await.unwrap();
        assert_eq!(ids(&shallow), ["d"]);
        assert!(!shallow.messages[0].headers_only);
    }

    #[tokio::test]
    async fn an_attachment_comes_out_of_the_whole_message() {
        let (server, provider) = server().await;
        mount_email(&server, "d", "2026-10-31T12:00:00.000Z").await;
        assert_eq!(provider.fetch_part(INBOX, "d", "1").await.unwrap(), b"body d\r\n");
        assert!(matches!(provider.fetch_part(INBOX, "d", "2").await, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn an_empty_inbox_finishes_without_a_high_water() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .respond_with(page(vec![], false))
            .mount(&server)
            .await;
        let batch = provider.fetch_since(INBOX, None, 500).await.unwrap();
        assert!(batch.messages.is_empty() && !batch.more);
        assert_eq!(
            batch.checkpoint,
            Checkpoint(CheckpointState::Resend(ResendCheckpoint::default()))
        );
        let limit = &server.received_requests().await.unwrap()[0];
        assert_eq!(
            limit.url.query(),
            Some("limit=100"),
            "page size is capped at Resend's maximum"
        );
    }

    #[tokio::test]
    async fn sends_the_key_to_the_api_but_never_to_download_links() {
        let (server, provider) = server().await;
        mount_email(&server, "a", "2026-10-28T12:00:00.000Z").await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .and(header("authorization", format!("Bearer {KEY}").as_str()))
            // Names the app and its version, and nothing that points elsewhere.
            .and(header("user-agent", concat!("Moss/", env!("CARGO_PKG_VERSION"))))
            .respond_with(page(vec![listed("a", "2026-10-28T12:00:00.000Z")], false))
            .mount(&server)
            .await;
        provider.fetch_since(INBOX, None, 10).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let download: &Request = requests.iter().find(|r| r.url.path() == "/raw/a").unwrap();
        assert!(!download.headers.contains_key("authorization"));
        assert!(requests
            .iter()
            .filter(|r| r.url.path() != "/raw/a")
            .all(|r| r.headers.contains_key("authorization")));
    }

    #[tokio::test]
    async fn rebuilds_the_message_when_resend_has_no_raw_copy() {
        let (server, provider) = server().await;
        let detail = json!({
            "object": "email", "id": "x", "to": ["me@moss.dev"], "from": "Bob <bob@example.com>",
            "created_at": "2026-10-28T12:00:00.000Z", "subject": "Fallback", "bcc": null, "cc": ["cc@example.com"],
            "reply_to": [], "message_id": "<x@example.com>", "raw": null,
            "html": "<p>Hi <img src=\"cid:img1\"></p>", "text": "Hi",
            "headers": { "from": "Bob <bob@example.com>", "x-mailer": "Thing", "content-type": "multipart/mixed; boundary=zzz",
                         "received": ["by a", "by b"], "x-odd": 5, "x-folded": "one\n two", "x-bare": "a\nb" },
            "attachments": []
        });
        Mock::given(method("GET"))
            .and(path("/emails/receiving/x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(detail))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving/x/attachments"))
            .and(query_param_is_missing("after"))
            .respond_with(page(vec![json!({
                "id": "att1", "filename": "logo.png", "size": 3, "content_type": "image/png",
                "content_disposition": "inline", "content_id": "img1",
                "download_url": format!("{}/files/att1", server.uri()), "expires_at": "2026-11-01T10:00:00.000Z"
            })], true))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving/x/attachments"))
            .and(query_param("after", "att1"))
            .respond_with(page(vec![json!({
                "id": "att2", "filename": null, "size": 1, "content_type": null, "content_disposition": "attachment",
                "content_id": "ignored", "download_url": format!("{}/files/att2", server.uri()), "expires_at": "x"
            })], false))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/files/att1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3]))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/files/att2"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9]))
            .mount(&server)
            .await;

        let m = provider.fetch_message(INBOX, "x").await.unwrap();
        // Several at once is the same, one by one.
        let several = provider.fetch_messages(INBOX, &["x".into()]).await.unwrap();
        let got: Vec<_> = several
            .iter()
            .map(|s| (s.provider_id.as_str(), s.received_at))
            .collect();
        assert_eq!(got, [("x", m.received_at)]);
        // Older mail: only its header block, marked as such.
        let heads = provider.fetch_headers(INBOX, &["x".into()]).await.unwrap();
        let full = String::from_utf8(several[0].raw.clone()).unwrap();
        let head = String::from_utf8(heads[0].raw.clone()).unwrap();
        assert!(heads[0].headers_only && !several[0].headers_only);
        // Each rebuild draws a new MIME boundary, so compare what comes before it.
        let before_boundary = &head[..head.find("Content-Type").unwrap()];
        assert!(head.ends_with("\r\n\r\n") && full.starts_with(before_boundary) && head.len() < full.len(), "{head}");
        let raw = String::from_utf8(m.raw).unwrap();
        let (head, body) = raw.split_once("MIME-Version: 1.0\r\n").unwrap();
        // The original headers, minus the MIME ones Moss writes itself, and the missing basics filled in.
        assert_eq!(
            head,
            "from: Bob <bob@example.com>\r\n\
             received: by a\r\n\
             received: by b\r\n\
             x-bare: a\r\n b\r\n\
             x-folded: one\r\n two\r\n\
             x-mailer: Thing\r\n\
             To: me@moss.dev\r\n\
             Cc: cc@example.com\r\n\
             Subject: Fallback\r\n\
             Date: Wed, 28 Oct 2026 12:00:00 +0000\r\n\
             Message-ID: <x@example.com>\r\n\
             X-Moss-Rebuilt: resend-json\r\n"
        );
        assert!(body.starts_with("Content-Type: multipart/mixed;"));
        assert!(body.contains("multipart/related"));
        assert!(body.contains("Content-ID: <img1>"));
        assert!(body.contains("Content-Type: application/octet-stream; name=\"attachment\""));
        assert!(body.contains("<p>Hi <img src=\"cid:img1\"></p>"));
    }

    #[tokio::test]
    async fn rebuilds_from_fields_when_there_are_no_headers() {
        let (server, provider) = server().await;
        let detail = json!({
            "id": "y", "to": ["me@moss.dev"], "from": "bob@example.com", "created_at": "2026-10-28T12:00:00.000Z",
            "subject": "Café", "reply_to": ["r@example.com"], "message_id": null, "raw": null, "headers": null,
            "text": "plain", "html": null
        });
        Mock::given(method("GET"))
            .and(path("/emails/receiving/y"))
            .respond_with(ResponseTemplate::new(200).set_body_json(detail))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving/y/attachments"))
            .respond_with(page(vec![], false))
            .mount(&server)
            .await;
        let raw = String::from_utf8(provider.fetch_message(INBOX, "y").await.unwrap().raw).unwrap();
        assert!(raw.starts_with(
            "From: bob@example.com\r\nTo: me@moss.dev\r\nReply-To: r@example.com\r\nSubject: =?UTF-8?B?Q2Fmw6k=?=\r\n\
             Date: Wed, 28 Oct 2026 12:00:00 +0000\r\nX-Moss-Rebuilt: resend-json\r\n"
        ));
        assert!(raw.ends_with("\r\n\r\nplain\r\n"));
    }

    #[tokio::test]
    async fn an_error_fetching_an_email_fails_the_batch() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .respond_with(page(vec![listed("a", "2026-10-28T12:00:00.000Z")], false))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving/a"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        assert!(matches!(
            provider.fetch_since(INBOX, None, 10).await,
            Err(Error::Network(_))
        ));
    }

    #[tokio::test]
    async fn mail_resend_dropped_for_quota_still_shows_up_with_a_note() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving"))
            .respond_with(page(vec![listed("q", "2026-10-28T12:00:00.000Z")], false))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/emails/receiving/q")).respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "name": "email_above_quota", "statusCode": 403,
            "message": "You can't retrieve this email's content because it was above quota when received."
        }))).mount(&server).await;
        let batch = provider.fetch_since(INBOX, None, 10).await.unwrap();
        let raw = String::from_utf8(batch.messages[0].raw.clone()).unwrap();
        assert!(raw.contains("Subject: Mail q\r\n"));
        assert!(raw.contains("X-Moss-Content-Missing: above-quota\r\n"));
        assert!(raw.contains("over its quota"));
    }

    async fn error_for(status: u16, body: serde_json::Value) -> Error {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        provider.fetch_message(INBOX, "z").await.unwrap_err()
    }

    #[tokio::test]
    async fn oversized_error_responses_do_not_wait_for_the_body_end() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut failed = Vec::new();
        for (chunked, truncated) in [(false, false), (true, false), (false, true)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (release, finish) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                }
                assert!(request.starts_with(b"GET "));
                if truncated {
                    socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 100\r\n\r\n").await.unwrap();
                    return;
                }
                if chunked {
                    socket.write_all(b"HTTP/1.1 403 Forbidden\r\nTransfer-Encoding: chunked\r\n\r\n4001\r\n").await.unwrap();
                    let _ = socket.write_all(&vec![b' '; 16 * 1024 + 1]).await;
                    let _ = socket.write_all(b"\r\n").await;
                } else {
                    socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 16385\r\n\r\n").await.unwrap();
                }
                // No terminating chunk/body: the client must stop at its budget.
                let _ = finish.await;
            });
            let provider = ResendProvider::with_base_url(Secret::from(KEY), &format!("http://{address}")).unwrap();
            let result = tokio::time::timeout(Duration::from_secs(2), provider.fetch_message(INBOX, "a")).await;
            let _ = release.send(());
            server.await.unwrap();
            if !matches!(result, Ok(Err(Error::Protocol(_)))) { failed.push(chunked); }
        }
        assert!(failed.is_empty(), "did not stop oversized responses, chunked={failed:?}");
    }

    #[tokio::test]
    async fn the_error_body_limit_is_inclusive() {
        let (server, provider) = server().await;
        let mut body = r#"{"name":"restricted_api_key"}"#.to_string();
        body.extend(std::iter::repeat_n(' ', 16 * 1024 - body.len()));
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(403).set_body_string(body)).mount(&server).await;
        assert!(matches!(provider.fetch_message(INBOX, "a").await, Err(Error::Auth(_))));
    }

    #[tokio::test]
    async fn external_diagnostics_keep_status_without_response_content() {
        let marker = "synthetic-private-resend-marker";
        let mut leaks = Vec::new();
        for status in [401, 403, 404, 409, 422, 503] {
            let error = error_for(status, json!({"message":marker,"name":marker})).await;
            assert!(error.to_string().contains(&format!("HTTP {status}")));
            if format!("{error:?} {error}").contains(marker) { leaks.push(format!("read {status}")); }
        }
        let message = OutgoingMessage { from: Address::new("sender@example.test"), to: vec![Address::new("recipient@example.test")], ..Default::default() };
        for status in [409, 422, 503] {
            let (server, provider) = server().await;
            Mock::given(method("POST")).and(path("/emails"))
                .respond_with(ResponseTemplate::new(status).set_body_json(json!({"message":marker,"name":marker}))).mount(&server).await;
            let error = provider.send(&message).await.unwrap_err();
            assert_eq!(matches!(error, Error::DeliveryUnknown(_)), status != 422);
            assert!(error.to_string().contains(&format!("HTTP {status}")));
            if format!("{error:?} {error}").contains(marker) { leaks.push(format!("send {status}")); }
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
        let (server, provider) = server().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":marker}))).mount(&server).await;
        let error = provider.domains().await.unwrap_err();
        assert!(matches!(error, Error::Protocol(_)));
        if format!("{error:?} {error}").contains(marker) { leaks.push("decoded field".into()); }
        assert!(leaks.is_empty(), "external text leaked through {leaks:?}");
    }

    #[tokio::test]
    async fn maps_api_errors() {
        let e = error_for(
            401,
            json!({"name": "validation_error", "message": "API key is invalid"}),
        )
        .await;
        assert!(matches!(e, Error::Auth(m) if m == "The Resend request failed (HTTP 401)."));
        let e = error_for(
            403,
            json!({"name": "restricted_api_key", "message": "This key can only send"}),
        )
        .await;
        assert!(matches!(e, Error::Auth(_)));
        let e = error_for(403, json!({"name": "invalid_permission", "message": "no"})).await;
        assert!(matches!(e, Error::Auth(_)));
        let e = error_for(403, json!({"name": "email_above_quota", "message": "over"})).await;
        assert!(matches!(e, Error::Protocol(m) if m == "The Resend request failed (HTTP 403)."));
        let e = error_for(
            404,
            json!({"name": "not_found", "message": "Email not found"}),
        )
        .await;
        assert!(matches!(e, Error::NotFound(_)));
        let e = error_for(422, json!({"name": "invalid_parameter", "message": "bad"})).await;
        assert!(matches!(e, Error::Protocol(m) if m == "The Resend request failed (HTTP 422)."));
        let e = error_for(503, json!({})).await;
        assert!(matches!(e, Error::Network(m) if m.contains("503")));
    }

    #[tokio::test]
    async fn an_error_without_a_json_body_still_says_what_happened() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string("nope"))
            .mount(&server)
            .await;
        let e = provider.fetch_message(INBOX, "z").await.unwrap_err();
        assert!(matches!(e, Error::Protocol(m) if m.contains("400")));
    }

    #[tokio::test]
    async fn retries_after_a_short_rate_limit() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
            .up_to_n_times(2)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/domains"))
            .respond_with(page(vec![], false))
            .mount(&server)
            .await;
        assert!(provider.domains().await.unwrap().is_empty());
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn gives_up_after_three_rate_limits() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
            .mount(&server)
            .await;
        let e = provider.domains().await.unwrap_err();
        assert!(matches!(e, Error::RateLimited { retry_after } if retry_after == Duration::ZERO));
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn fetching_an_email_sits_out_short_rate_limits_then_says_it_is_rate_limited() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving/z"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
            .mount(&server)
            .await;
        let e = provider.fetch_message(INBOX, "z").await.unwrap_err();
        assert!(matches!(e, Error::RateLimited { retry_after } if retry_after == Duration::ZERO), "{e:?}");
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_long_rate_limit_is_returned_straight_away() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "3600"))
            .mount(&server)
            .await;
        let e = provider.domains().await.unwrap_err();
        assert!(
            matches!(e, Error::RateLimited { retry_after } if retry_after == Duration::from_secs(3600))
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn reads_retry_after_or_waits_a_second() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after(&headers), Duration::from_secs(1));
        headers.insert("retry-after", "7".parse().unwrap());
        assert_eq!(retry_after(&headers), Duration::from_secs(7));
        headers.insert("retry-after", "soon".parse().unwrap());
        assert_eq!(retry_after(&headers), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_network_error() {
        let provider =
            ResendProvider::with_base_url(Secret::from(KEY), "http://127.0.0.1:1").unwrap();
        let e = provider.domains().await.unwrap_err();
        assert!(matches!(&e, Error::Network(_)));
        assert!(!e.to_string().contains(KEY));
    }

    #[tokio::test]
    async fn malformed_json_is_a_protocol_error() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{"))
            .mount(&server)
            .await;
        assert!(matches!(
            provider.domains().await.unwrap_err(),
            Error::Protocol(_)
        ));
    }

    #[tokio::test]
    async fn a_malformed_email_is_a_protocol_error() {
        let (server, provider) = server().await;
        Mock::given(method("GET"))
            .and(path("/emails/receiving/a"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;
        assert!(matches!(
            provider.fetch_message(INBOX, "a").await,
            Err(Error::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn a_failed_download_is_an_error() {
        let (server, provider) = server().await;
        let mut detail = listed("a", "2026-10-28T12:00:00.000Z");
        detail["raw"] = json!({ "download_url": format!("{}/raw/a", server.uri()) });
        Mock::given(method("GET"))
            .and(path("/emails/receiving/a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(detail))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/raw/a"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        assert!(matches!(
            provider.fetch_message(INBOX, "a").await.unwrap_err(),
            Error::Protocol(_)
        ));
    }

    #[tokio::test]
    async fn lists_domains_across_pages() {
        let (server, provider) = server().await;
        let domain = |id: &str, name: &str, receiving: &str| {
            json!({
                "id": id, "name": name, "status": "verified", "created_at": "2026-04-26 20:21:26.347412+00",
                "region": "us-east-1", "capabilities": { "sending": "enabled", "receiving": receiving }
            })
        };
        Mock::given(method("GET"))
            .and(path("/domains"))
            .and(query_param_is_missing("after"))
            .respond_with(page(vec![domain("d1", "moss.dev", "enabled")], true))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/domains"))
            .and(query_param("after", "d1"))
            .respond_with(page(
                vec![json!({ "id": "d2", "name": "old.dev", "status": "not_started" })],
                false,
            ))
            .mount(&server)
            .await;
        assert_eq!(
            provider.domains().await.unwrap(),
            vec![
                ResendDomain {
                    id: "d1".into(),
                    name: "moss.dev".into(),
                    status: "verified".into(),
                    sending: true,
                    receiving: true
                },
                ResendDomain {
                    id: "d2".into(),
                    name: "old.dev".into(),
                    status: "not_started".into(),
                    sending: false,
                    receiving: false
                },
            ]
        );
    }

    #[tokio::test]
    async fn sends_with_threading_headers_and_an_idempotency_key() {
        let (server, provider) = server().await;
        Mock::given(method("POST"))
            .and(path("/emails"))
            .and(header("idempotency-key", "<r1@moss.dev>"))
            .and(body_json(json!({
                "from": "Ada <ada@moss.dev>",
                "to": ["bob@example.com"],
                "cc": ["c@example.com"],
                "bcc": ["d@example.com"],
                "reply_to": ["r@moss.dev"],
                "subject": "Re: Plans",
                "text": "Yes",
                "html": "<p>Yes</p>",
                "headers": {
                    "Message-ID": "<r1@moss.dev>",
                    "In-Reply-To": "<p1@example.com>",
                    "References": "<p0@example.com> <p1@example.com>",
                    "X-Moss-Mode": "reply"
                },
                "attachments": [
                    { "filename": "a.txt", "content": "aGk=", "content_type": "text/plain" },
                    { "filename": "i.png", "content": "AQ==", "content_type": "image/png", "content_id": "i1" }
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "sent-1" })))
            .mount(&server)
            .await;
        let msg = OutgoingMessage {
            date: None,
            from: Address::named("Ada", "ada@moss.dev"),
            to: vec![Address::new("bob@example.com")],
            cc: vec![Address::new("c@example.com")],
            bcc: vec![Address::new("d@example.com")],
            reply_to: vec![Address::new("r@moss.dev")],
            subject: "Re: Plans".into(),
            text: Some("Yes".into()),
            html: Some("<p>Yes</p>".into()),
            in_reply_to: Some("p1@example.com".into()),
            references: vec!["p0@example.com".into(), "p1@example.com".into()],
            message_id: Some("r1@moss.dev".into()),
            headers: vec![("X-Moss-Mode".into(), "reply".into())],
            attachments: vec![
                Attachment {
                    filename: "a.txt".into(),
                    content_type: "text/plain".into(),
                    content_id: None,
                    data: b"hi".to_vec(),
                },
                Attachment {
                    filename: "i.png".into(),
                    content_type: "image/png".into(),
                    content_id: Some("i1".into()),
                    data: vec![1],
                },
            ],
        };
        let sent = provider.send(&msg).await.unwrap();
        assert_eq!(sent.provider_id.as_deref(), Some("sent-1"));
        assert_eq!(sent.message_id, "<r1@moss.dev>");
        let raw = String::from_utf8(sent.raw).unwrap();
        assert!(
            raw.contains("Bcc: d@example.com\r\n"),
            "the archive copy keeps Bcc"
        );
        assert!(raw.contains("In-Reply-To: <p1@example.com>\r\n"));
    }

    #[tokio::test]
    async fn a_dropped_http_response_after_submission_is_uncertain() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let received = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut partial_bodies = 0;
            loop {
                let mut chunk = [0; 64];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap().trim().parse().unwrap();
                    if bytes.len() >= end + 4 + length { return (length, partial_bodies); }
                    partial_bodies += 1;
                }
            }
        });
        let provider = ResendProvider::with_base_url(Secret::from(KEY), &format!("http://{address}")).unwrap();
        let message = OutgoingMessage { from: Address::new("sender@example.test"), to: vec![Address::new("recipient@example.test")], ..Default::default() };
        let error = provider.send(&message).await.unwrap_err();
        let (length, partial_bodies) = received.await.unwrap();
        assert!(length > 64);
        assert!(partial_bodies > 0, "the fixture must read a fragmented body before disconnecting");
        assert!(matches!(error, Error::DeliveryUnknown(_)), "{error:?}");
    }

    #[tokio::test]
    async fn submission_errors_distinguish_rejection_from_uncertain_acceptance() {
        let message = OutgoingMessage { from: Address::new("sender@example.test"), to: vec![Address::new("recipient@example.test")], ..Default::default() };
        for (status, body, uncertain) in [(500, "{}", true), (409, "{}", true), (200, "broken receipt", true), (200, r#"{"id":"  "}"#, true), (422, "{}", false)] {
            let (server, provider) = server().await;
            Mock::given(method("POST")).and(path("/emails"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body)).mount(&server).await;
            let error = provider.send(&message).await.unwrap_err();
            assert_eq!(matches!(error, Error::DeliveryUnknown(_)), uncertain, "{status}: {error:?}");
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let provider = ResendProvider::with_base_url(Secret::from(KEY), &format!("http://{address}")).unwrap();
        assert!(matches!(provider.send(&message).await, Err(Error::Network(_))));
    }

    #[tokio::test]
    async fn a_minimal_send_leaves_out_empty_fields() {
        let (server, provider) = server().await;
        Mock::given(method("POST"))
            .and(path("/emails"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "sent-2" })))
            .mount(&server)
            .await;
        let msg = OutgoingMessage {
            from: Address::new("ada@moss.dev"),
            to: vec![Address::new("bob@example.com")],
            subject: "Hi".into(),
            ..Default::default()
        };
        let sent = provider.send(&msg).await.unwrap();
        let request = &server.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({ "from": "ada@moss.dev", "to": ["bob@example.com"], "subject": "Hi", "text": "",
                    "headers": { "Message-ID": sent.message_id } })
        );
    }

    #[tokio::test]
    async fn refuses_to_send_an_invalid_message() {
        let (_server, provider) = server().await;
        let e = provider
            .send(&OutgoingMessage::default())
            .await
            .unwrap_err();
        assert!(matches!(e, Error::InvalidMessage(_)));
    }

    #[tokio::test]
    async fn has_an_inbox_and_a_local_sent_folder() {
        let (_server, provider) = server().await;
        let boxes = provider.list_mailboxes().await.unwrap();
        assert_eq!(
            boxes
                .iter()
                .map(|b| (b.id.as_str(), b.role))
                .collect::<Vec<_>>(),
            [
                (INBOX, Some(MailboxRole::Inbox)),
                (SENT, Some(MailboxRole::Sent))
            ]
        );
        assert!(!provider.capabilities().server_flags);
        assert!(provider
            .apply(INBOX, &["a".into()], &Change::Archive)
            .await
            .is_ok());
        // No folders, so nothing was archived and there is nothing to bring back.
        assert!(provider.unarchive(&["a@x".into()]).await.is_ok());
        // Nor any Trash: deleted mail is only hidden here, and undo shows it again.
        assert!(provider.trash(&["a@x".into()]).await.is_ok());
        assert!(provider.untrash(&["a@x".into()], true).await.is_ok());
        assert!(provider.create_mailbox("Blocked").await.is_ok());
        // Nor Drafts: drafts stay on this device.
        let draft = OutgoingMessage { from: Address::new("ada@moss.dev"), ..Default::default() };
        assert_eq!(provider.save_draft("Drafts", &draft).await.unwrap(), None);
        assert!(provider.delete_drafts("Drafts", &["old@x".into()]).await.is_ok());
    }

    #[tokio::test]
    async fn syncing_sent_returns_nothing_and_other_mailboxes_dont_exist() {
        let (_server, provider) = server().await;
        let batch = provider.fetch_since(SENT, None, 10).await.unwrap();
        assert!(batch.messages.is_empty() && !batch.more);
        assert!(matches!(
            provider.fetch_since("Spam", None, 10).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            provider.fetch_message(SENT, "a").await,
            Err(Error::NotFound(_))
        ));
        // Messages that aren't there are left out of a batch.
        assert!(provider
            .fetch_messages(SENT, &["a".into()])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn refuses_a_checkpoint_from_another_provider() {
        let (_server, provider) = server().await;
        let imap = Checkpoint(CheckpointState::Imap(
            crate::imap::ImapCheckpoint::for_tests(),
        ));
        assert!(matches!(
            provider.fetch_since(INBOX, Some(&imap), 10).await,
            Err(Error::InvalidCheckpoint)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_for_change_just_waits_out_the_timeout() {
        let provider =
            ResendProvider::with_base_url(Secret::from(KEY), "http://127.0.0.1:1").unwrap();
        let started = tokio::time::Instant::now();
        let wakeup = provider
            .wait_for_change(INBOX, Duration::from_secs(300))
            .await
            .unwrap();
        assert_eq!(wakeup, Wakeup::Timeout);
        assert!(started.elapsed() >= Duration::from_secs(300));
        // Resend reports deletions itself, so there's no id list to compare.
        assert_eq!(provider.all_ids(INBOX).await.unwrap(), None);
        assert_eq!(provider.list_drafts("Drafts").await.unwrap(), None);
        // Nor are there Gmail conversations to fill in.
        let asked = ["a".to_string()];
        assert_eq!(
            provider.conversations_of(INBOX, &asked).await.unwrap(),
            None
        );
        assert!(provider
            .conversation_messages(INBOX, &[1])
            .await
            .unwrap()
            .is_empty());
        // A failure other than "not there" fails the batch.
        assert!(matches!(
            provider.fetch_messages(INBOX, &asked).await,
            Err(Error::Network(_))
        ));
    }

    /// A server that reads one request, then answers with `answer`: each item after a pause.
    /// Its task ends once the client hangs up.
    async fn raw_server(answer: Vec<(Duration, &'static [u8])>) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 64 * 1024];
            let _ = socket.read(&mut request).await;
            for (pause, bytes) in answer {
                tokio::time::sleep(pause).await;
                let _ = socket.write_all(bytes).await;
            }
            // Then silence, with the connection open, until the client gives up.
            let _ = socket.read_to_end(&mut Vec::new()).await;
        });
        (format!("http://{address}"), server)
    }

    /// A short limit, so the tests below take well under a second.
    const SHORT: Duration = Duration::from_millis(300);
    /// A limit no test reaches, so only the short one can run out, however busy the Mac is.
    const LONG: Duration = Duration::from_secs(30);

    /// Made before a test starts its clock: loading the system's certificates can be slow.
    fn quick(base: &str, limits: Limits) -> ResendProvider {
        ResendProvider::with_limits(Secret::from(KEY), base, limits).unwrap()
    }

    /// Only the total for an API call is short: a read that waits is never what ends it.
    const API_TOTAL: Limits = Limits { connect: LONG, read: LONG, api: SHORT };

    #[tokio::test]
    async fn a_send_resend_never_answers_is_an_unknown_outcome() {
        let message = OutgoingMessage { from: Address::new("sender@example.test"), to: vec![Address::new("recipient@example.test")], ..Default::default() };
        let (base, server) = raw_server(vec![]).await;
        let provider = quick(&base, API_TOTAL);
        let started = std::time::Instant::now();
        let error = provider.send(&message).await.unwrap_err();
        assert!(matches!(error, Error::DeliveryUnknown(_)), "never resent on its own: {error:?}");
        assert!(started.elapsed() >= SHORT && started.elapsed() < LONG, "an API call has a total limit");
        drop(provider);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn an_api_call_resend_never_answers_is_a_network_error() {
        let (base, server) = raw_server(vec![]).await;
        let provider = quick(&base, API_TOTAL);
        let started = std::time::Instant::now();
        let error = provider.domains().await.unwrap_err();
        assert!(matches!(error, Error::Network(_)), "{error:?}");
        assert!(started.elapsed() >= SHORT && started.elapsed() < LONG, "an API call has a total limit");
        drop(provider);
        server.await.unwrap();
    }

    const HEAD: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\n";
    /// The pause before each piece of the download: far inside the read limit.
    const STEP: Duration = Duration::from_millis(150);

    #[tokio::test]
    async fn a_download_that_keeps_coming_may_take_longer_than_an_api_call() {
        let (base, server) = raw_server(vec![(STEP, HEAD), (STEP, b"a"), (STEP, b"b"), (STEP, b"c"), (STEP, b"d"), (STEP, b"e")]).await;
        let provider = quick(&base, API_TOTAL);
        let started = std::time::Instant::now();
        assert_eq!(provider.download(&format!("{base}/file")).await.unwrap(), b"abcde");
        assert!(started.elapsed() > API_TOTAL.api * 2, "longer than an API call may take");
        drop(provider);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_download_that_stops_coming_fails() {
        let (base, server) = raw_server(vec![(Duration::ZERO, HEAD), (Duration::ZERO, b"ab")]).await;
        let provider = quick(&base, Limits { connect: LONG, read: SHORT, api: LONG });
        let started = std::time::Instant::now();
        assert!(matches!(provider.download(&format!("{base}/file")).await, Err(Error::Network(_))));
        assert!(started.elapsed() >= SHORT && started.elapsed() < LONG);
        drop(provider);
        server.await.unwrap();
    }

    #[test]
    fn real_limits_are_short_for_the_api_and_open_ended_for_downloads() {
        let limits = Limits::default();
        assert_eq!((limits.connect, limits.read, limits.api), (Duration::from_secs(10), Duration::from_secs(60), Duration::from_secs(60)));
    }

    #[test]
    fn the_default_provider_talks_to_resend() {
        assert_eq!(
            ResendProvider::new(Secret::from(KEY)).unwrap().base,
            "https://api.resend.com"
        );
    }
}
