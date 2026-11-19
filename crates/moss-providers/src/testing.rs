//! In-process fake servers for the tests. Nothing here touches the network:
//! IMAP scripts run over in-memory pipes, and the TLS and SMTP fakes listen
//! on 127.0.0.1 only.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// A certificate for `localhost` and an acceptor that serves it.
pub(crate) struct TestTls {
    pub acceptor: TlsAcceptor,
    pub root: Vec<u8>,
}

pub(crate) fn test_tls() -> TestTls {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let der = cert.cert.der().clone();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![der.clone()], key)
    .unwrap();
    TestTls {
        acceptor: TlsAcceptor::from(Arc::new(config)),
        root: der.to_vec(),
    }
}

/// One step of an IMAP conversation: what the client should send (`{tag}`
/// stands for its command tag) and what the server answers (`{tag}` is
/// replaced with that tag).
pub(crate) type Step = (String, String);

pub(crate) fn step(expect: &str, reply: &str) -> Step {
    (expect.to_string(), reply.to_string())
}

/// At the end of an expectation: anything may follow.
pub(crate) const ANY: &str = "<any>";

/// As a reply: the server drops the connection instead of answering.
pub(crate) const HANG_UP: &str = "<hang up>";

/// Plays `steps` against an IMAP client. Literals (`{n}` at the end of a
/// line) get a `+` continuation and are appended to the line. After the
/// script the client may only log out or hang up. Errors say what differed.
pub(crate) async fn serve_imap<S>(
    stream: S,
    greeting: Option<&str>,
    steps: Vec<Step>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    if let Some(greeting) = greeting {
        write
            .write_all(greeting.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut tag = String::new();
    for (expect, reply) in steps {
        let Some(line) = read_command(&mut read, &mut write).await else {
            return Err(format!("client hung up; expected {expect:?}"));
        };
        let actual = match line.split_once(' ') {
            Some((t, rest)) if expect.starts_with("{tag} ") => {
                tag = t.to_string();
                format!("{{tag}} {rest}")
            }
            _ => line,
        };
        let matches = match expect.strip_suffix(ANY) {
            Some(prefix) => actual.starts_with(prefix),
            None => actual == expect,
        };
        if !matches {
            return Err(format!("expected {expect:?}\n     got {actual:?}"));
        }
        let (reply, close) = match reply.strip_suffix(HANG_UP) {
            Some(prefix) => (prefix, true),
            None => (reply.as_str(), false),
        };
        write
            .write_all(reply.replace("{tag}", &tag).as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        if close { return Ok(()); }
    }
    match read_command(&mut read, &mut write).await {
        Some(line) => Err(format!("unexpected after the script: {line:?}")),
        None => Ok(()),
    }
}

async fn read_command<R, W>(read: &mut BufReader<R>, write: &mut W) -> Option<String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut line = read_line(read).await?;
    while let Some(n) = literal_len(&line) {
        write.write_all(b"+ go ahead\r\n").await.ok()?;
        let mut literal = vec![0; n];
        read.read_exact(&mut literal).await.ok()?;
        line.push_str("\r\n");
        line.push_str(&String::from_utf8_lossy(&literal));
        line.push_str(&read_line(read).await?);
    }
    Some(line)
}

async fn read_line<R: AsyncRead + Unpin>(read: &mut BufReader<R>) -> Option<String> {
    let mut line = String::new();
    match read.read_line(&mut line).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
    }
}

fn literal_len(line: &str) -> Option<usize> {
    let open = line.rfind('{')?;
    line.strip_suffix('}')?[open + 1..].parse().ok()
}

/// A signed-in session whose server plays `steps` after LOGIN.
pub(crate) async fn scripted_session(steps: Vec<Step>) -> async_imap::Session<crate::imap::Io> {
    let (client, server) = tokio::io::duplex(1 << 20);
    let mut all = vec![step("{tag} LOGIN \"u\" \"p\"", "{tag} OK\r\n")];
    all.extend(steps);
    tokio::spawn(serve_imap(server, Some("* OK ready\r\n"), all));
    let mut client = async_imap::Client::new(Box::new(client) as crate::imap::Io);
    client.read_response().await.unwrap();
    client.login("u", "p").await.map_err(|(e, _)| e).unwrap()
}

fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

/// Mailbox names as async-imap parses these LIST lines.
pub(crate) fn parse_names(lines: &[&str]) -> Vec<async_imap::types::Name> {
    use futures::TryStreamExt as _;
    block_on(async {
        let reply = format!("{}\r\n{{tag}} OK\r\n", lines.join("\r\n"));
        let mut session = scripted_session(vec![step("{tag} LIST \"\" *", &reply)]).await;
        session
            .list(Some(""), Some("*"))
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    })
}

/// A FETCH response as async-imap parses it.
pub(crate) fn parse_fetch(line: &str) -> async_imap::types::Fetch {
    use futures::TryStreamExt as _;
    block_on(async {
        let reply = format!("{line}\r\n{{tag}} OK\r\n");
        let mut session = scripted_session(vec![step("{tag} UID FETCH 1 X", &reply)]).await;
        let mut all: Vec<_> = session
            .uid_fetch("1", "X")
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        all.remove(0)
    })
}

/// Hands out in-memory connections, one script per connection, in order.
#[derive(Default)]
pub(crate) struct FakeImap {
    scripts: Mutex<std::collections::VecDeque<Vec<Step>>>,
    servers: Mutex<Vec<tokio::task::JoinHandle<Result<(), String>>>>,
}

impl FakeImap {
    pub(crate) fn new(scripts: Vec<Vec<Step>>) -> Arc<Self> {
        Arc::new(FakeImap {
            scripts: Mutex::new(scripts.into()),
            servers: Mutex::default(),
        })
    }

    /// Checks every connection followed its script. Drop the provider first.
    pub(crate) async fn finish(&self) {
        let servers: Vec<_> = self.servers.lock().unwrap().drain(..).collect();
        for server in servers {
            server.await.unwrap().unwrap();
        }
        let left = self.scripts.lock().unwrap().len();
        assert_eq!(left, 0, "{left} scripted connections were never made");
    }
}

#[async_trait::async_trait]
impl crate::imap::Connector for FakeImap {
    async fn connect(&self, watch: &crate::stall::Watch) -> crate::Result<async_imap::Client<crate::imap::Io>> {
        let Some(steps) = self.scripts.lock().unwrap().pop_front() else {
            return Err(crate::Error::Network("no more fake servers".into()));
        };
        let (client, server) = tokio::io::duplex(1 << 20);
        self.servers.lock().unwrap().push(tokio::spawn(serve_imap(
            server,
            Some("* OK fake IMAP\r\n"),
            steps,
        )));
        crate::imap::greeted(Box::new(crate::stall::Stall::new(client, watch.clone()))).await
    }
}

/// What the fake SMTP server saw.
#[derive(Default)]
pub(crate) struct SmtpLog {
    pub commands: Vec<String>,
    pub data: Vec<u8>,
}

pub(crate) enum SmtpMode {
    /// Receives the data, then drops the connection before acknowledging it.
    LoseReply,
    /// Receives the data, then says nothing more, keeping the connection open.
    SilentAfterData,
    /// Takes a while to accept the data, but keeps saying it's working on it.
    SlowAcceptance,
    /// Accepts TLS, then never greets.
    Silent,
    /// TLS from the first byte.
    Tls,
    /// Plain, offering STARTTLS.
    StartTls,
    /// Plain, without STARTTLS (a server Moss must refuse).
    NoTls,
}

/// Starts a one-connection SMTP server on 127.0.0.1. It accepts `auth`
/// (the base64 after `AUTH PLAIN ` or `AUTH XOAUTH2 `) and refuses anything
/// else with 535.
pub(crate) async fn smtp_server(mode: SmtpMode, auth: &str) -> (u16, Vec<u8>, Arc<Mutex<SmtpLog>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let tls = test_tls();
    let log = Arc::new(Mutex::new(SmtpLog::default()));
    let (auth, server_log) = (auth.to_string(), log.clone());
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        match mode {
            SmtpMode::Silent => {
                let mut stream = tls.acceptor.accept(tcp).await.unwrap();
                let _ = stream.read_to_end(&mut Vec::new()).await;
            }
            SmtpMode::Tls | SmtpMode::LoseReply | SmtpMode::SilentAfterData | SmtpMode::SlowAcceptance => {
                let stream = tls.acceptor.accept(tcp).await.unwrap();
                let after = match mode {
                    SmtpMode::LoseReply => AfterData::Lose,
                    SmtpMode::SilentAfterData => AfterData::Silence,
                    SmtpMode::SlowAcceptance => AfterData::Slowly,
                    _ => AfterData::Reply,
                };
                smtp_session(stream, true, false, &auth, &server_log, after).await;
            }
            SmtpMode::StartTls => {
                let mut tcp = tcp;
                assert!(
                    smtp_session(&mut tcp, true, true, &auth, &server_log, AfterData::Reply).await,
                    "no STARTTLS"
                );
                let stream = tls.acceptor.accept(tcp).await.unwrap();
                smtp_session(stream, false, false, &auth, &server_log, AfterData::Reply).await;
            }
            SmtpMode::NoTls => {
                smtp_session(tcp, true, false, &auth, &server_log, AfterData::Reply).await;
            }
        }
    });
    (port, tls.root, log)
}

/// What the fake SMTP server does once it has the message.
#[derive(Clone, Copy, PartialEq)]
enum AfterData {
    Reply,
    Lose,
    Silence,
    Slowly,
}

/// How long the slow server pauses between its lines, [`SLOW_LINES`] times over. Short
/// pauses, many of them: each stays far inside the client's limit even on a busy Mac,
/// while all of them together take well over it.
pub(crate) const SLOW_LINE: std::time::Duration = std::time::Duration::from_millis(50);
const SLOW_LINES: u32 = 15;

/// Returns true when the client asked for STARTTLS.
async fn smtp_session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    greet: bool,
    offer_starttls: bool,
    auth: &str,
    log: &Mutex<SmtpLog>,
    after: AfterData,
) -> bool {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    if greet {
        let _ = write.write_all(b"220 fake ESMTP\r\n").await;
    }
    while let Some(line) = read_line(&mut read).await {
        log.lock().unwrap().commands.push(line.clone());
        let upper = line.to_ascii_uppercase();
        let reply = if upper.starts_with("EHLO") {
            let tls = if offer_starttls {
                "250-STARTTLS\r\n"
            } else {
                ""
            };
            format!("250-fake\r\n{tls}250-AUTH PLAIN LOGIN XOAUTH2\r\n250 8BITMIME\r\n")
        } else if upper == "STARTTLS" {
            let _ = write.write_all(b"220 go ahead\r\n").await;
            return true;
        } else if let Some(given) = line
            .strip_prefix("AUTH PLAIN ")
            .or_else(|| line.strip_prefix("AUTH XOAUTH2 "))
        {
            if given == auth {
                "235 ok\r\n".into()
            } else {
                "535 5.7.8 bad credentials\r\n".into()
            }
        } else if upper.starts_with("RCPT TO") && upper.contains("REJECT") {
            "550 5.1.1 no such user\r\n".into()
        } else if upper == "DATA" {
            let _ = write.write_all(b"354 go\r\n").await;
            let mut data = Vec::new();
            loop {
                let mut chunk = Vec::new();
                read.read_until(b'\n', &mut chunk).await.unwrap();
                if chunk.is_empty() || chunk == b".\r\n" {
                    break;
                }
                data.extend_from_slice(&chunk);
            }
            // The data is every line before the lone dot, the last one's CRLF included (RFC 5321
            // 4.1.1.4).
            log.lock().unwrap().data = data;
            match after {
                AfterData::Lose => return false,
                AfterData::Silence => {
                    let _ = read.read_to_end(&mut Vec::new()).await;
                    return false;
                }
                AfterData::Slowly => {
                    for _ in 0..SLOW_LINES {
                        tokio::time::sleep(SLOW_LINE).await;
                        let _ = write.write_all(b"250-still checking\r\n").await;
                    }
                }
                AfterData::Reply => {}
            }
            "250 queued\r\n".into()
        } else if upper == "QUIT" {
            "221 bye\r\n".into()
        } else {
            "250 ok\r\n".into()
        };
        let _ = write.write_all(reply.as_bytes()).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_literal_lengths() {
        assert_eq!(literal_len("A1 APPEND x {12}"), Some(12));
        assert_eq!(literal_len("A1 NOOP"), None);
        assert_eq!(literal_len("A1 {x}"), None);
    }

    #[tokio::test]
    async fn the_imap_script_can_disconnect_after_partial_response() {
        let (mut client, server) = tokio::io::duplex(1024);
        let fake = tokio::spawn(serve_imap(server, None, vec![step(
            "{tag} SELECT \"INBOX\"", "* 0 EXISTS\r\n<hang up>",
        )]));
        client.write_all(b"A1 SELECT \"INBOX\"\r\n").await.unwrap();
        let mut received = Vec::new();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), client.read_to_end(&mut received)).await;
        assert!(result.is_ok(), "script must close after its partial response");
        assert_eq!(received, b"* 0 EXISTS\r\n");
        fake.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn the_imap_script_reports_a_client_gone_before_it_could_answer() {
        // Gone before the greeting.
        let (client, server) = tokio::io::duplex(1024);
        drop(client);
        let error = serve_imap(server, Some("* OK\r\n"), vec![]).await.unwrap_err();
        assert!(error.contains("broken pipe"), "{error}");
        // Gone after asking, before the reply.
        let (mut client, server) = tokio::io::duplex(1024);
        client.write_all(b"A1 NOOP\r\n").await.unwrap();
        drop(client);
        let error = serve_imap(server, None, vec![step("{tag} NOOP", "{tag} OK\r\n")]).await.unwrap_err();
        assert!(error.contains("broken pipe"), "{error}");
    }

    #[tokio::test]
    async fn the_imap_script_reports_mismatches_and_hangups() {
        let (client, server) = tokio::io::duplex(1024);
        let fake = tokio::spawn(serve_imap(
            server,
            None,
            vec![step("{tag} NOOP", "{tag} OK\r\n")],
        ));
        let (mut r, mut w) = tokio::io::split(client);
        w.write_all(b"A1 CHECK\r\n").await.unwrap();
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).await.unwrap();
        assert!(fake
            .await
            .unwrap()
            .unwrap_err()
            .contains("expected \"{tag} NOOP\""));

        let (client, server) = tokio::io::duplex(1024);
        let fake = tokio::spawn(serve_imap(server, None, vec![step("{tag} NOOP", "")]));
        drop(client);
        assert!(fake.await.unwrap().unwrap_err().contains("hung up"));

        let (client, server) = tokio::io::duplex(1024);
        let fake = tokio::spawn(serve_imap(server, Some("* OK\r\n"), vec![]));
        let (mut r, mut w) = tokio::io::split(client);
        w.write_all(b"A1 NOOP\r\n").await.unwrap();
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).await.unwrap();
        assert!(fake
            .await
            .unwrap()
            .unwrap_err()
            .contains("unexpected after the script"));
    }
}
