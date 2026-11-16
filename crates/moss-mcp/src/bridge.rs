//! For agents that only start servers as a program on stdio (Claude Desktop): Moss's own app,
//! run as `Moss mcp <url>` with the token in `MOSS_MCP_TOKEN`, passes each line it reads to
//! the running server and writes back what it answers. Nothing goes anywhere but this Mac.

use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// The environment variable the bridge reads the token from.
pub const TOKEN_VAR: &str = "MOSS_MCP_TOKEN";

/// What `Moss mcp` needs: the address and the token. `None` when run another way.
pub fn wanted(args: &[String], token: Option<String>) -> Option<(String, String)> {
    match args {
        [_, mcp, url] if mcp == "mcp" && url.starts_with("http") => {
            Some((url.clone(), token.unwrap_or_default()))
        }
        _ => None,
    }
}

/// Passes JSON-RPC lines from `input` to the server at `url`, and its answers to `output`,
/// until `input` ends. A line the server can't take gets a JSON-RPC error back, so the
/// agent hears why instead of waiting.
pub async fn run(
    url: &str,
    token: &str,
    input: impl AsyncBufRead + Unpin,
    output: impl AsyncWrite + Unpin,
) -> std::io::Result<()> {
    run_within(url, token, input, output, ANSWER_WAIT).await
}

/// How long Moss may take to answer one request: a tool may first have to start a model, and
/// a send may wait out a slow mail server, which has a minute to answer and then up to 10 minutes
/// to accept the email (RFC 5321). Longer than both, with room to spare.
pub const ANSWER_WAIT: Duration = Duration::from_secs(15 * 60);

/// The client for Moss's own server on this Mac: no proxy (looking up the system's can take
/// seconds, and none applies to 127.0.0.1), a quick connect, and `answer` for the rest.
fn client(answer: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(answer)
        .build()
        .expect("a client without TLS or a proxy always builds")
}

async fn run_within(
    url: &str,
    token: &str,
    input: impl AsyncBufRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
    answer: Duration,
) -> std::io::Result<()> {
    let client = client(answer);
    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        for answer in forward(&client, url, token, &line).await {
            output.write_all(answer.as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
    }
    Ok(())
}

async fn forward(client: &reqwest::Client, url: &str, token: &str, line: &str) -> Vec<String> {
    let sent = client
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(line.to_string())
        .send()
        .await;
    let response = match sent {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return failed(line, late(line)),
        Err(_) => return failed(line, "Moss isn't running, or MCP is off in Settings, AI."),
    };
    let status = response.status();
    let sse = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return failed(
            line,
            "Moss refused the token: copy the connection again from Settings, AI.",
        );
    }
    if sse {
        return events(&body);
    }
    if status.is_success() {
        return if body.trim().is_empty() {
            vec![]
        } else {
            vec![body.trim().to_string()]
        };
    }
    // A JSON-RPC error from the server comes back as is.
    if serde_json::from_str::<serde_json::Value>(&body).is_ok_and(|v| v.get("jsonrpc").is_some()) {
        return vec![body.trim().to_string()];
    }
    failed(line, &format!("Moss answered {status}."))
}

/// What a request that ran out of time hears. Moss may still be doing it: a send may go.
fn late(line: &str) -> &'static str {
    let request = serde_json::from_str::<serde_json::Value>(line).unwrap_or_default();
    match request["method"] == "tools/call" && request["params"]["name"] == "send" {
        true => "Moss didn't answer in time. The email may have been sent. Check Sent before trying again. A retry with the same request_id never sends it twice.",
        false => "Moss didn't answer in time.",
    }
}

/// The messages in a server-sent event stream.
fn events(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect()
}

/// A JSON-RPC error answering `line`'s request; nothing for a notification, which has no id.
fn failed(line: &str, why: &str) -> Vec<String> {
    let id = serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("id").cloned());
    match id {
        Some(id) => vec![serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": why } }).to_string()],
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_moss_mcp_with_a_url_is_the_bridge() {
        let url = "http://127.0.0.1:1/mcp";
        assert_eq!(
            wanted(&args(&["Moss", "mcp", url]), Some("t".into())),
            Some((url.into(), "t".into()))
        );
        assert_eq!(
            wanted(&args(&["Moss", "mcp", url]), None),
            Some((url.into(), "".into()))
        );
        assert_eq!(wanted(&args(&["Moss"]), None), None);
        assert_eq!(wanted(&args(&["Moss", "-psn_0_1"]), None), None);
        assert_eq!(wanted(&args(&["Moss", "other", url]), None), None);
        // A test run filtered by "mcp" isn't the bridge.
        assert_eq!(
            wanted(&args(&["moss-test", "mcp", "--nocapture"]), None),
            None
        );
    }

    #[test]
    fn event_streams_give_their_data_lines() {
        let body = "id: 1\ndata: {\"a\":1}\n\ndata:\n\ndata: {\"b\":2}\n";
        assert_eq!(events(body), vec!["{\"a\":1}", "{\"b\":2}"]);
    }

    #[test]
    fn failures_answer_requests_but_not_notifications() {
        let out = failed(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#, "why");
        let v: serde_json::Value = serde_json::from_str(&out[0]).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["error"]["message"], "why");
        assert!(failed(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "why"
        )
        .is_empty());
        assert!(failed("not json", "why").is_empty());
    }

    #[tokio::test]
    async fn a_moss_that_stops_answering_fails_the_request_instead_of_hanging() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = socket.read_to_end(&mut Vec::new()).await;
        });
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n".as_slice();
        let mut out = Vec::new();
        run_within(&url, "t", input, &mut out, Duration::from_millis(200)).await.unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Moss didn't answer in time."), "{text}");
        assert!(!text.contains("email"), "{text}");
        server.await.unwrap();
    }

    #[test]
    fn a_send_that_runs_out_of_time_may_have_gone() {
        let send = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send","arguments":{}}}"#;
        assert_eq!(late(send), "Moss didn't answer in time. The email may have been sent. Check Sent before trying again. A retry with the same request_id never sends it twice.");
        let read = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_thread","arguments":{}}}"#;
        for line in [read, r#"{"jsonrpc":"2.0","id":4,"method":"send"}"#, "not json"] {
            assert_eq!(late(line), "Moss didn't answer in time.", "{line}");
        }
    }

    #[test]
    fn the_bridge_never_asks_the_system_for_a_proxy() {
        let shown = format!("{:?}", client(ANSWER_WAIT));
        assert!(!shown.contains("proxies"), "{shown}");
        // Past a slow mail server's 60 seconds to connect and 10 minutes for its final reply.
        assert!(ANSWER_WAIT > Duration::from_secs(60 + 10 * 60), "{ANSWER_WAIT:?}");
    }

    #[tokio::test]
    async fn with_moss_closed_each_request_hears_why() {
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n".as_slice();
        let mut out = Vec::new();
        run("http://127.0.0.1:9/mcp", "t", input, &mut out)
            .await
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Moss isn't running"), "{text}");
        assert_eq!(text.lines().count(), 1);
    }
}
