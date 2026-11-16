//! Settings → AI → "Your own agents": the MCP server that lets Claude Code, Codex, Claude
//! Desktop, Cursor and others read your mail and write drafts. The work is in `moss_mcp`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use moss_accounts::SecretStore;
use moss_engine::{Engine, Events};
use moss_mcp::{Mcp, McpError, McpParts, McpStatus};
use serde::Serialize;
use tauri::State;

/// The connection as the AI page shows it, with the program agents that only take a
/// command (Claude Desktop) run as the bridge: this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpView {
    #[serde(flatten)]
    pub status: McpStatus,
    pub bridge: String,
}

/// The server over `engine`, with its settings beside the database in `data_dir` and the
/// token in `secrets`. Off until it's turned on.
pub fn mcp(
    engine: &Engine,
    events: Arc<dyn Events>,
    secrets: Arc<dyn SecretStore>,
    data_dir: &Path,
    port: u16,
) -> Mcp {
    Mcp::new(McpParts {
        engine: engine.clone(),
        events,
        secrets,
        dir: data_dir.to_path_buf(),
        port,
        clock: crate::now_ms,
    })
}

fn view(status: McpStatus) -> McpView {
    let bridge = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("Moss"));
    McpView {
        status,
        bridge: bridge.display().to_string(),
    }
}

/// Turns the server back on in the background if it was on when Moss quit.
pub fn start(mcp: Mcp) -> Mcp {
    let restored = mcp.clone();
    tauri::async_runtime::spawn(async move { restored.restore().await });
    mcp
}

/// What Moss runs as: the bridge for `Moss mcp <url>` (then it exits with its code), or
/// the app.
pub fn bridge_or(app: fn()) {
    let args: Vec<String> = std::env::args().collect();
    let token = std::env::var(moss_mcp::bridge::TOKEN_VAR).ok();
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    match bridge(&args, token, input, tokio::io::stdout()) {
        Some(code) => std::process::exit(code),
        None => app(),
    }
}

/// `Moss mcp <url>`, as Claude Desktop starts it: passes `input` to the running server and its
/// answers to `output`, and gives the exit code. `None` when Moss was started as the app.
pub fn bridge(
    args: &[String],
    token: Option<String>,
    input: impl tokio::io::AsyncBufRead + Unpin,
    output: impl tokio::io::AsyncWrite + Unpin,
) -> Option<i32> {
    let (url, token) = moss_mcp::bridge::wanted(args, token)?;
    let done = tauri::async_runtime::block_on(moss_mcp::bridge::run(&url, &token, input, output));
    Some(if done.is_ok() { 0 } else { 1 })
}

#[tauri::command]
pub fn mcp_status(mcp: State<'_, Mcp>) -> Result<McpView, McpError> {
    Ok(view(mcp.status()))
}

#[tauri::command]
pub async fn mcp_set_on(mcp: State<'_, Mcp>, on: bool) -> Result<McpView, McpError> {
    mcp.set_on(on).await.map(view)
}

#[tauri::command]
pub async fn mcp_new_token(mcp: State<'_, Mcp>) -> Result<McpView, McpError> {
    mcp.new_token().await.map(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
    use tauri::Manager;

    /// The app with its real commands, and the server over `dir` with settings in `settings`.
    fn app_with(dir: &Path, settings: &Path) -> tauri::WebviewWindow<MockRuntime> {
        let app = crate::with_commands(mock_builder())
            .build(mock_context(noop_assets()))
            .unwrap();
        let events: Arc<dyn Events> = Arc::new(crate::WindowEvents(app.handle().clone()));
        let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
        let engine =
            crate::engine(dir, secrets.clone(), events.clone(), Default::default()).unwrap();
        app.manage(mcp(&engine, events, secrets, settings, 0));
        tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap()
    }

    fn call(
        webview: &tauri::WebviewWindow<MockRuntime>,
        cmd: &str,
        args: Value,
    ) -> Result<Value, Value> {
        let request = tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(webview, request).map(|b| b.deserialize::<Value>().unwrap())
    }

    #[test]
    fn the_ai_page_turns_it_on_and_off_and_makes_new_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let webview = app_with(&data, &data);

        let off = call(&webview, "mcp_status", json!({})).unwrap();
        assert_eq!(off["on"], false);
        assert_eq!(off["url"], Value::Null);
        assert!(off["bridge"].as_str().unwrap().len() > 1, "{off}");
        let on = call(&webview, "mcp_set_on", json!({"on": true})).unwrap();
        assert_eq!(on["on"], true);
        assert!(on["url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
        let next = call(&webview, "mcp_new_token", json!({})).unwrap();
        assert_ne!(next["token"], on["token"]);
        assert_eq!(
            call(&webview, "mcp_set_on", json!({"on": false})).unwrap()["on"],
            false
        );
        assert!(dir
            .path()
            .join("data")
            .join(moss_mcp::SETTINGS_FILE)
            .exists());
    }

    #[test]
    fn problems_reach_the_window_as_a_sentence() {
        let dir = tempfile::tempdir().unwrap();
        // The settings can't be written where a file is in the way.
        let file = dir.path().join("file");
        std::fs::write(&file, "").unwrap();
        let webview = app_with(&dir.path().join("data"), &file);
        let e = call(&webview, "mcp_set_on", json!({"on": true})).unwrap_err();
        assert!(
            e.as_str().unwrap().starts_with("couldn't save the setting"),
            "{e}"
        );
    }

    static RAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[test]
    fn started_any_other_way_it_runs_the_app() {
        // The test runner's own arguments aren't `mcp <url>`.
        bridge_or(|| RAN.store(true, std::sync::atomic::Ordering::SeqCst));
        assert!(RAN.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn the_server_comes_back_on_if_it_was_on() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
        let quiet: Arc<dyn Events> = Arc::new(crate::WindowEvents(
            mock_builder()
                .build(mock_context(noop_assets()))
                .unwrap()
                .handle()
                .clone(),
        ));
        let engine = crate::engine(
            dir.path(),
            secrets.clone(),
            quiet.clone(),
            Default::default(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join(moss_mcp::SETTINGS_FILE),
            r#"{"on":true,"port":0}"#,
        )
        .unwrap();
        let agents = start(mcp(&engine, quiet, secrets, dir.path(), 0));
        let waiting = async {
            while !agents.status().on {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let came_back = tauri::async_runtime::block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), waiting).await
        });
        assert!(came_back.is_ok());
    }

    #[test]
    fn moss_mcp_runs_as_the_bridge_and_anything_else_as_the_app() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(bridge(&args(&["Moss"]), None, &b""[..], Vec::new()), None);
        // main makes stdin before it knows which: that must not need a runtime.
        let _ = tokio::io::BufReader::new(tokio::io::stdin());
        // Nothing listens there: each request hears why, and the bridge ends with its input.
        let mut out = Vec::new();
        let ping = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
        let code = bridge(
            &args(&["Moss", "mcp", "http://127.0.0.1:9/mcp"]),
            Some("t".into()),
            &ping[..],
            &mut out,
        );
        assert_eq!(code, Some(0));
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("Moss isn't running"));
    }
}
