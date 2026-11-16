//! Sending through SMTP submission (port 465 or 587), for IMAP accounts.

use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials as LettreCredentials, Mechanism};
use lettre::transport::smtp::client::{AsyncSmtpConnection, AsyncTokioStream, Certificate, CertificateStore, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::stall::{Stall, Watch};

use crate::{Credentials, Error, OutgoingMessage, Result, Security, TlsRoots};

/// Where and how to submit outgoing mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub credentials: Credentials,
    pub tls_roots: TlsRoots,
}

/// How long SMTP may take. The app uses the default; tests pass short ones.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Connecting, TLS and signing in, all together. Nothing has been submitted yet, so
    /// running out is a network error, safe to retry.
    pub connect: Duration,
    /// How long any read or write may then wait with nothing moving. A big message that
    /// keeps uploading is never cut. Running out after submission began is an unknown
    /// outcome, never retried on its own.
    pub stall: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        // RFC 5321 4.5.3.2 lets a server take up to 10 minutes to accept the message after
        // its last line. A connection that died fails sooner, through TCP keepalive.
        Limits { connect: Duration::from_secs(60), stall: Duration::from_secs(10 * 60) }
    }
}

/// Submits `wire` (the RFC 822 bytes of `message`) to every recipient,
/// Bcc included.
pub(crate) async fn send(
    config: &SmtpConfig,
    message: &OutgoingMessage,
    wire: &[u8],
) -> Result<()> {
    send_within(config, message, wire, Limits::default()).await
}

pub(crate) async fn send_within(
    config: &SmtpConfig,
    message: &OutgoingMessage,
    wire: &[u8],
    limits: Limits,
) -> Result<()> {
    let address = |email: &str, role: &str| {
        email
            .trim()
            .parse::<lettre::Address>()
            .map_err(|_| Error::InvalidMessage(format!("The {role} address is invalid.")))
    };
    let from = address(&message.from.email, "from")?;
    let to = [("to", &message.to), ("cc", &message.cc), ("bcc", &message.bcc)]
        .into_iter()
        .flat_map(|(role, people)| people.iter().map(move |person| address(&person.email, role)))
        .collect::<Result<_>>()?;
    let envelope =
        Envelope::new(Some(from), to).map_err(|e| Error::InvalidMessage(e.to_string()))?;
    // Connection, TLS and authentication finish before the submission boundary. A failed
    // connection is safe to retry; a lost or stalled response during submission is not.
    let watch = Watch::new(limits.stall);
    let mut connection = tokio::time::timeout(limits.connect, connect(config, &watch)).await
        .map_err(|_| Error::Network("SMTP connection timed out".into()))??;
    connection.send(&envelope, wire).await.map_err(submission_error)?;
    // Acceptance is already known. Closing the transport cannot turn it into a failed send.
    Ok(())
}

/// The connection lettre talks over: TCP, maybe inside TLS, watched for stalls.
#[derive(Debug)]
struct Transport<S> {
    io: S,
    peer: SocketAddr,
}

impl<S: AsyncRead + Unpin> AsyncRead for Transport<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Transport<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + AsyncWrite + Send + Sync + Unpin + std::fmt::Debug> AsyncTokioStream for Transport<S> {
    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.peer)
    }
}

async fn connect(config: &SmtpConfig, watch: &Watch) -> Result<AsyncSmtpConnection> {
    let mut tls = TlsParameters::builder(config.host.clone());
    if let TlsRoots::Custom(certs) = &config.tls_roots {
        tls = tls.certificate_store(CertificateStore::None);
        for der in certs {
            tls = tls.add_root_certificate(Certificate::from_der(der.clone()).map_err(tls_error)?);
        }
    }
    let params = tls.build_rustls().map_err(tls_error)?;
    let hello = ClientId::Ipv4(Ipv4Addr::LOCALHOST);
    let failed = |_| Error::Network("The SMTP connection failed. Check the server and TLS settings.".into());
    let tcp = TcpStream::connect((config.host.as_str(), config.port)).await.map_err(failed)?;
    crate::imap::keep_alive(&tcp).map_err(failed)?;
    let peer = tcp.peer_addr().map_err(failed)?;
    let tcp = Stall::new(tcp, watch.clone());
    let mut connection = match config.security {
        Security::Tls => {
            let name = ServerName::try_from(config.host.clone())
                .map_err(|_| Error::Network("The SMTP server name is invalid.".into()))?;
            let stream = TlsConnector::from(crate::tls::client_config(&config.tls_roots)?)
                .connect(name, tcp)
                .await
                .map_err(|_| Error::Network("TLS handshake failed. Check the server certificate and TLS settings.".into()))?;
            AsyncSmtpConnection::connect_with_transport(Box::new(Transport { io: stream, peer }), &hello).await.map_err(smtp_error)?
        }
        Security::StartTls => {
            let mut plain = AsyncSmtpConnection::connect_with_transport(Box::new(Transport { io: tcp, peer }), &hello)
                .await
                .map_err(smtp_error)?;
            plain.starttls(params, &hello).await.map_err(smtp_error)?;
            plain
        }
    };
    let (secret, mechanisms) = match &config.credentials {
        Credentials::Password { password, .. } => (password, vec![Mechanism::Plain, Mechanism::Login]),
        Credentials::OAuth2 { access_token, .. } => (access_token, vec![Mechanism::Xoauth2]),
    };
    let credentials = LettreCredentials::new(config.credentials.username().to_string(), secret.expose().to_string());
    connection.auth(&mechanisms, &credentials).await.map_err(smtp_error)?;
    Ok(connection)
}

fn submission_error(error: lettre::transport::smtp::Error) -> Error {
    if error.status().is_some() { return smtp_error(error); }
    // Lettre rejects unsupported message/envelope features before issuing MAIL FROM.
    if error.is_client() { return Error::InvalidMessage("The message uses an unsupported SMTP feature.".into()); }
    Error::DeliveryUnknown("SMTP acceptance could not be confirmed.".into())
}

fn tls_error(_: lettre::transport::smtp::Error) -> Error {
    Error::Network("SMTP TLS setup failed.".into())
}

fn smtp_error(e: lettre::transport::smtp::Error) -> Error {
    match e.status().map(|code| code.to_string()) {
        Some(code) if matches!(code.as_str(), "530" | "534" | "535") => Error::Auth(format!("SMTP sign-in was refused ({code}).")),
        Some(code) => Error::Protocol(format!("The SMTP command was refused ({code}).")),
        None => Error::Network("The SMTP connection failed. Check the server and TLS settings.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{smtp_server, SmtpMode};
    use crate::{Address, Secret};
    use base64::Engine as _;

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    fn config(
        port: u16,
        security: Security,
        credentials: Credentials,
        root: Vec<u8>,
    ) -> SmtpConfig {
        SmtpConfig {
            host: "localhost".into(),
            port,
            security,
            credentials,
            tls_roots: TlsRoots::Custom(vec![root]),
        }
    }

    fn password(pw: &str) -> Credentials {
        Credentials::Password {
            username: "ada@moss.dev".into(),
            password: Secret::from(pw),
        }
    }

    fn message() -> OutgoingMessage {
        OutgoingMessage {
            from: Address::named("Ada", "ada@moss.dev"),
            to: vec![Address::new("bob@example.com")],
            cc: vec![Address::new("carol@example.com")],
            bcc: vec![Address::new("dan@example.com")],
            subject: "Hi".into(),
            ..Default::default()
        }
    }

    const WIRE: &[u8] = b"Subject: Hi\r\n\r\nHello\r\n";

    #[tokio::test]
    async fn a_lost_reply_after_data_is_uncertain_but_an_offline_connection_is_not() {
        let (port, root, log) = smtp_server(SmtpMode::LoseReply, &b64("\0ada@moss.dev\0pw")).await;
        let error = send(&config(port, Security::Tls, password("pw"), root), &message(), WIRE).await.unwrap_err();
        assert!(matches!(error, Error::DeliveryUnknown(_)), "{error:?}");
        assert_eq!(log.lock().unwrap().data, WIRE, "the server received the data before the response was lost");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = send(&config(port, Security::Tls, password("pw"), crate::testing::test_tls().root), &message(), WIRE).await.unwrap_err();
        assert!(matches!(error, Error::Network(_)), "connection was refused before submission: {error:?}");
    }

    /// A short limit, so the tests below take well under a second.
    const SHORT: Duration = Duration::from_millis(300);
    /// A limit no test reaches, so only the short one can run out, however busy the Mac is.
    const LONG: Duration = Duration::from_secs(60);
    /// Only a stall can end these tests.
    const STALL: Limits = Limits { connect: LONG, stall: SHORT };

    #[tokio::test]
    async fn a_server_that_goes_quiet_after_the_message_is_an_unknown_outcome() {
        let (port, root, log) = smtp_server(SmtpMode::SilentAfterData, &b64("\0ada@moss.dev\0pw")).await;
        let started = std::time::Instant::now();
        let error = send_within(&config(port, Security::Tls, password("pw"), root), &message(), WIRE, STALL).await.unwrap_err();
        assert!(matches!(error, Error::DeliveryUnknown(_)), "never resent on its own: {error:?}");
        assert_eq!(log.lock().unwrap().data, WIRE);
        assert!(started.elapsed() >= SHORT && started.elapsed() < Duration::from_secs(5));
        // The server's session ends (and lets go of the log) once Moss hangs up.
        while std::sync::Arc::strong_count(&log) > 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[test]
    fn the_transport_says_who_it_talks_to() {
        let peer: SocketAddr = "127.0.0.1:465".parse().unwrap();
        let transport = Transport { io: tokio::io::empty(), peer };
        assert_eq!(AsyncTokioStream::peer_addr(&transport).unwrap(), peer);
    }

    #[tokio::test]
    async fn a_server_that_never_greets_is_a_network_error() {
        // Either limit may end the wait for the greeting: the stall of that one read, or
        // the total for connecting. Each is tried on its own, so neither races the other.
        for (limits, says) in [
            (STALL, "The SMTP connection failed. Check the server and TLS settings."),
            (Limits { connect: SHORT, stall: LONG }, "SMTP connection timed out"),
        ] {
            let (port, root, log) = smtp_server(SmtpMode::Silent, "x").await;
            let started = std::time::Instant::now();
            let error = send_within(&config(port, Security::Tls, password("pw"), root), &message(), WIRE, limits).await.unwrap_err();
            assert!(matches!(&error, Error::Network(m) if m == says), "nothing was sent, so it's safe to retry: {error:?}");
            assert!(started.elapsed() >= SHORT && started.elapsed() < Duration::from_secs(5));
            assert!(log.lock().unwrap().commands.is_empty());
        }
    }

    #[tokio::test]
    async fn a_slow_acceptance_that_keeps_answering_is_not_cut() {
        let (port, root, log) = smtp_server(SmtpMode::SlowAcceptance, &b64("\0ada@moss.dev\0pw")).await;
        let started = std::time::Instant::now();
        send_within(&config(port, Security::Tls, password("pw"), root), &message(), WIRE, STALL).await.unwrap();
        assert!(started.elapsed() > STALL.stall * 2, "took longer than the stall limit in all, twice over");
        assert_eq!(log.lock().unwrap().data, WIRE);
        assert!(crate::testing::SLOW_LINE * 6 <= STALL.stall, "each pause is far inside the limit");
    }

    #[tokio::test]
    async fn a_host_that_cant_be_a_certificate_name_is_refused_before_tls() {
        // "127.1" reaches 127.0.0.1, but it is neither an IP address nor a DNS name to TLS.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            use tokio::io::AsyncReadExt as _;
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut heard = Vec::new();
            tcp.read_to_end(&mut heard).await.unwrap();
            heard
        });
        let cfg = SmtpConfig { host: "127.1".into(), ..config(port, Security::Tls, password("pw"), crate::testing::test_tls().root) };
        let error = send(&cfg, &message(), WIRE).await.unwrap_err();
        assert!(matches!(&error, Error::Network(m) if m == "The SMTP server name is invalid."), "{error:?}");
        assert!(server.await.unwrap().is_empty(), "connected, then hung up without a TLS hello");
    }

    #[test]
    fn real_limits_follow_the_smtp_standard() {
        assert_eq!(Limits::default().connect, Duration::from_secs(60));
        assert_eq!(Limits::default().stall, Duration::from_secs(10 * 60), "RFC 5321 4.5.3.2: up to 10 minutes for the final reply");
    }

    #[tokio::test]
    async fn submits_over_tls_with_a_password_to_every_recipient() {
        let (port, root, log) = smtp_server(SmtpMode::Tls, &b64("\0ada@moss.dev\0app-pw")).await;
        send(
            &config(port, Security::Tls, password("app-pw"), root),
            &message(),
            WIRE,
        )
        .await
        .unwrap();
        let log = log.lock().unwrap();
        assert_eq!(
            log.commands[0], "EHLO [127.0.0.1]",
            "doesn't leak the computer's name"
        );
        assert!(log
            .commands
            .contains(&"MAIL FROM:<ada@moss.dev>".to_string()));
        for to in ["bob@example.com", "carol@example.com", "dan@example.com"] {
            assert!(log.commands.contains(&format!("RCPT TO:<{to}>")), "{to}");
        }
        assert_eq!(log.data, WIRE);
    }

    #[tokio::test]
    async fn upgrades_with_starttls_and_signs_in_with_an_oauth_token() {
        let token = Credentials::OAuth2 {
            username: "ada@moss.dev".into(),
            access_token: Secret::from("ya29.tok"),
        };
        let (port, root, log) = smtp_server(
            SmtpMode::StartTls,
            &b64("user=ada@moss.dev\x01auth=Bearer ya29.tok\x01\x01"),
        )
        .await;
        send(
            &config(port, Security::StartTls, token, root),
            &message(),
            WIRE,
        )
        .await
        .unwrap();
        let log = log.lock().unwrap();
        let starttls = log.commands.iter().position(|c| c == "STARTTLS").unwrap();
        let auth = log
            .commands
            .iter()
            .position(|c| c.starts_with("AUTH XOAUTH2"))
            .unwrap();
        assert!(
            starttls < auth,
            "never signs in before the connection is encrypted"
        );
        assert_eq!(log.data, WIRE);
    }

    #[tokio::test]
    async fn a_wrong_password_is_an_auth_error() {
        let (port, root, _log) = smtp_server(SmtpMode::Tls, "something-else").await;
        let e = send(
            &config(port, Security::Tls, password("wrong"), root),
            &message(),
            WIRE,
        )
        .await
        .unwrap_err();
        assert!(matches!(&e, Error::Auth(_)), "{e:?}");
        assert!(!e.to_string().contains("wrong"));
        assert!(!format!("{e:?} {e}").contains("bad credentials"));
    }

    #[tokio::test]
    async fn a_rejected_recipient_is_a_protocol_error() {
        let (port, root, _log) = smtp_server(SmtpMode::Tls, &b64("\0ada@moss.dev\0pw")).await;
        let msg = OutgoingMessage {
            to: vec![Address::new("reject@example.com")],
            ..message()
        };
        let e = send(
            &config(port, Security::Tls, password("pw"), root),
            &msg,
            WIRE,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&e, Error::Protocol(m) if m.contains("550")),
            "{e:?}"
        );
        assert!(!format!("{e:?} {e}").contains("no such user"));
    }

    #[tokio::test]
    async fn refuses_a_server_that_wont_encrypt() {
        let (port, root, log) = smtp_server(SmtpMode::NoTls, "x").await;
        let e = send(
            &config(port, Security::StartTls, password("pw"), root),
            &message(),
            WIRE,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Network(_) | Error::Protocol(_)), "{e:?}");
        assert!(!log
            .lock()
            .unwrap()
            .commands
            .iter()
            .any(|c| c.starts_with("AUTH")));
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_refused() {
        let (port, _root, _log) = smtp_server(SmtpMode::Tls, "x").await;
        let other = crate::testing::test_tls().root;
        let e = send(
            &config(port, Security::Tls, password("pw"), other),
            &message(),
            WIRE,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Network(_)), "{e:?}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_network_error() {
        let cfg = SmtpConfig {
            tls_roots: TlsRoots::Platform,
            ..config(1, Security::Tls, password("pw"), vec![])
        };
        assert!(matches!(
            send(&cfg, &message(), WIRE).await,
            Err(Error::Network(_))
        ));
    }

    #[tokio::test]
    async fn invalid_address_errors_never_contain_mailbox_data() {
        let cfg = config(1, Security::Tls, password("pw"), crate::testing::test_tls().root);
        for role in ["from", "to", "cc", "bcc"] {
            let mut mail = message();
            let invalid = Address::new("sensitive-fixture@example.test <invalid>");
            match role {
                "from" => mail.from = invalid,
                "to" => mail.to = vec![invalid],
                "cc" => mail.cc = vec![invalid],
                _ => mail.bcc = vec![invalid],
            }
            let error = send(&cfg, &mail, WIRE).await.unwrap_err().to_string();
            assert!(!error.contains("sensitive-fixture"), "mailbox data leaked in an error");
            assert!(error.contains(role), "the invalid field should remain identifiable");
        }
    }

    #[tokio::test]
    async fn a_bad_address_is_an_invalid_message() {
        let cfg = config(
            1,
            Security::Tls,
            password("pw"),
            crate::testing::test_tls().root,
        );
        let bad_from = OutgoingMessage {
            from: Address::new("not an address"),
            ..message()
        };
        assert!(matches!(
            send(&cfg, &bad_from, WIRE).await,
            Err(Error::InvalidMessage(_))
        ));
        let bad_to = OutgoingMessage {
            to: vec![Address::new("nope")],
            ..message()
        };
        assert!(matches!(
            send(&cfg, &bad_to, WIRE).await,
            Err(Error::InvalidMessage(_))
        ));
        let nobody = OutgoingMessage {
            to: vec![],
            cc: vec![],
            bcc: vec![],
            ..message()
        };
        assert!(matches!(
            send(&cfg, &nobody, WIRE).await,
            Err(Error::InvalidMessage(_))
        ));
    }
}
