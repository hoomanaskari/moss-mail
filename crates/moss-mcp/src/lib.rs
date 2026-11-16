//! Moss's MCP server, so your own AI agents (Claude Code, Codex, Claude Desktop, Cursor…)
//! can read, manage and send mail when authorized by the user. Local paths are never read
//! on an agent's behalf; files must already be attached in Moss or supplied as contents.
//!
//! - Streamable HTTP on `127.0.0.1` only, at `http://127.0.0.1:<port>/mcp`. Requests from
//!   a browser page (with an `Origin`) or for another host are refused, so a web page can't
//!   reach it.
//! - Every request needs `Authorization: Bearer <token>`. The token is random, kept in the
//!   Keychain, and [`Mcp::new_token`] replaces it: the old one stops working at once.
//! - Off until you turn it on. Off stops the server, so connections are refused.
//! - Stateless, with plain JSON answers, so the stdio [`bridge`] (for Claude Desktop) is a
//!   few lines.
//! - Nothing is logged, so no mail ever lands in a log.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use moss_accounts::{Secret, SecretRef, SecretStore, KEYCHAIN_SERVICE};
use moss_engine::{Engine, Events};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub mod bridge;
pub mod tools;

pub use tools::MossTools;

/// The port Moss asks for first, so the address you paste into an agent stays the same.
pub const PORT: u16 = 47_217;
/// The path the server answers on.
pub const PATH: &str = "/mcp";
/// The Keychain item the token is kept in.
pub const TOKEN_ITEM: &str = "mcp";
/// The file beside the database that says whether it's on, and on which port.
pub const SETTINGS_FILE: &str = "mcp.json";

/// What the server runs on. The app passes the real ones; tests pass their own.
pub struct McpParts {
    pub engine: Engine,
    pub events: Arc<dyn Events>,
    pub secrets: Arc<dyn SecretStore>,
    /// Where [`SETTINGS_FILE`] goes: Moss's data folder.
    pub dir: PathBuf,
    /// The port to ask for first; `0` takes any free one.
    pub port: u16,
    /// Milliseconds since the epoch.
    pub clock: fn() -> i64,
}

/// The connection, as Settings → AI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub on: bool,
    /// Where agents connect, while it's on.
    pub url: Option<String>,
    /// What they connect with, while it's on.
    pub token: Option<String>,
    /// When an agent last used it, in milliseconds since the epoch; since Moss started.
    pub last_used: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("the keychain failed: {0}")]
    Keychain(String),
    #[error("couldn't start the server: {0}")]
    Start(String),
    #[error("couldn't save the setting: {0}")]
    Settings(String),
}

impl Serialize for McpError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct Settings {
    on: bool,
    port: Option<u16>,
}

struct Running {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

/// The token requests are checked against, shared with the running server. `None` while
/// off, so a request that got in as it stopped is refused too.
#[derive(Clone)]
struct Gate {
    token: Arc<RwLock<Option<String>>>,
    last_used: Arc<AtomicI64>,
    clock: fn() -> i64,
}

/// The MCP server: on or off, its address and token. Cheap to clone.
#[derive(Clone)]
pub struct Mcp(Arc<Inner>);

struct Inner {
    parts: McpParts,
    gate: Gate,
    /// The running server's port, for [`Mcp::status`] to read without waiting.
    port: Mutex<Option<u16>>,
    running: tokio::sync::Mutex<Option<Running>>,
    settings: Mutex<Settings>,
}

impl Mcp {
    pub fn new(parts: McpParts) -> Self {
        let settings = std::fs::read(parts.dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let gate = Gate {
            token: Default::default(),
            last_used: Arc::new(AtomicI64::new(0)),
            clock: parts.clock,
        };
        Mcp(Arc::new(Inner {
            parts,
            gate,
            running: Default::default(),
            port: Mutex::new(None),
            settings: Mutex::new(settings),
        }))
    }

    /// Starts the server if it was on when Moss last quit.
    pub async fn restore(&self) -> Result<McpStatus, McpError> {
        let on = self.0.settings.lock().unwrap().on;
        if on {
            self.start().await?;
        }
        Ok(self.status())
    }

    pub fn status(&self) -> McpStatus {
        let port = *self.0.port.lock().unwrap();
        let last = self.0.gate.last_used.load(Ordering::Relaxed);
        let last_used = (last > 0).then_some(last);
        match port {
            Some(port) => McpStatus {
                on: true,
                url: Some(url(port)),
                token: self.0.gate.token.read().unwrap().clone(),
                last_used,
            },
            None => McpStatus {
                on: false,
                url: None,
                token: None,
                last_used,
            },
        }
    }

    /// Turns the server on (making a token the first time) or off, and remembers it.
    pub async fn set_on(&self, on: bool) -> Result<McpStatus, McpError> {
        if on {
            self.start().await?;
        } else {
            self.stop().await;
        }
        self.save(|s| s.on = on)?;
        Ok(self.status())
    }

    /// A new token: the old one stops working at once, so agents need the new one.
    pub async fn new_token(&self) -> Result<McpStatus, McpError> {
        let token = self.make_token()?;
        let running = self.0.running.lock().await;
        if running.is_some() {
            *self.0.gate.token.write().unwrap() = Some(token);
        }
        drop(running);
        Ok(self.status())
    }

    /// Stops the server, as Moss quits.
    pub async fn stop(&self) {
        let running = self.0.running.lock().await.take();
        *self.0.gate.token.write().unwrap() = None;
        *self.0.port.lock().unwrap() = None;
        if let Some(r) = running {
            r.stop.cancel();
            // Wait for the port to be free, so turning it on again gets the same one.
            let _ = tokio::time::timeout(Duration::from_secs(2), r.task).await;
        }
    }

    async fn start(&self) -> Result<(), McpError> {
        let mut running = self.0.running.lock().await;
        if running.is_some() {
            return Ok(());
        }
        let token = match self.token()? {
            Some(t) => t,
            None => self.make_token()?,
        };
        let asked = self
            .0
            .settings
            .lock()
            .unwrap()
            .port
            .unwrap_or(self.0.parts.port);
        let listener = match TcpListener::bind(("127.0.0.1", asked)).await {
            Ok(l) => l,
            // Taken by something else: any free port, kept from now on.
            Err(_) => TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| McpError::Start(e.to_string()))?,
        };
        let port = listener
            .local_addr()
            .map_err(|e| McpError::Start(e.to_string()))?
            .port();
        self.save(|s| s.port = Some(port))?;
        *self.0.gate.token.write().unwrap() = Some(token);
        let stop = CancellationToken::new();
        let router = self.router(stop.child_token());
        let until = stop.clone();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(until.cancelled_owned())
                .await;
        });
        *running = Some(Running { stop, task });
        *self.0.port.lock().unwrap() = Some(port);
        Ok(())
    }

    fn router(&self, stop: CancellationToken) -> axum::Router {
        let p = &self.0.parts;
        let (engine, events, clock) = (p.engine.clone(), p.events.clone(), p.clock);
        // Files from mail an agent asks for; the engine keeps only the newest few.
        let files = p.dir.join("attachments");
        let mut config = StreamableHttpServerConfig::default().enforce_origin_validation();
        config.legacy_session_mode = false;
        config.json_response = true;
        config.cancellation_token = stop;
        let service: StreamableHttpService<MossTools, LocalSessionManager> =
            StreamableHttpService::new(
                move || {
                    Ok(MossTools::new(
                        engine.clone(),
                        events.clone(),
                        clock,
                        files.clone(),
                    ))
                },
                Default::default(),
                config,
            );
        axum::Router::new()
            .nest_service(PATH, service)
            .layer(middleware::from_fn_with_state(self.0.gate.clone(), check))
    }

    fn item() -> SecretRef {
        SecretRef {
            service: KEYCHAIN_SERVICE.into(),
            account: TOKEN_ITEM.into(),
        }
    }

    fn token(&self) -> Result<Option<String>, McpError> {
        match self
            .0
            .parts
            .secrets
            .load(&Self::item())
            .map_err(|e| McpError::Keychain(e.to_string()))?
        {
            Some(Secret::ApiKey { key }) => Ok(Some(key)),
            _ => Ok(None),
        }
    }

    fn make_token(&self) -> Result<String, McpError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| McpError::Keychain(e.to_string()))?;
        let token = format!("moss_{}", hex(&bytes));
        let secret = Secret::ApiKey { key: token.clone() };
        self.0
            .parts
            .secrets
            .save(&Self::item(), &secret)
            .map_err(|e| McpError::Keychain(e.to_string()))?;
        Ok(token)
    }

    fn save(&self, change: impl FnOnce(&mut Settings)) -> Result<(), McpError> {
        let mut settings = self.0.settings.lock().unwrap();
        let mut next = *settings;
        change(&mut next);
        let json = serde_json::to_vec(&next).expect("settings are plain JSON");
        std::fs::create_dir_all(&self.0.parts.dir)
            .and_then(|_| std::fs::write(self.0.parts.dir.join(SETTINGS_FILE), json))
            .map_err(|e| McpError::Settings(e.to_string()))?;
        *settings = next;
        Ok(())
    }
}

/// Where agents connect on `port`.
pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The same length and bytes, taking as long whatever they are.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Lets a request in only with the token, and notes when an agent last used it.
async fn check(State(gate): State<Gate>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let token = gate.token.read().unwrap().clone();
    let ok =
        matches!((given, token.as_deref()), (Some(g), Some(t)) if same(g.as_bytes(), t.as_bytes()));
    if !ok {
        return (
            StatusCode::UNAUTHORIZED,
            "Moss needs its token: copy the connection again from Settings, AI.",
        )
            .into_response();
    }
    gate.last_used.store((gate.clock)(), Ordering::Relaxed);
    next.run(request).await
}

#[cfg(test)]
mod tests;
