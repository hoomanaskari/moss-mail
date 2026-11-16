//! Opening an IMAP connection: TCP, then TLS (straight away or after
//! STARTTLS), then the server's greeting.

use std::fmt::Debug;
use std::time::Duration;

use async_imap::Client;
use async_trait::async_trait;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::stall::{Stall, Watch};
use crate::tls::client_config;
use crate::{Error, Result, Security, TlsRoots};

pub(crate) trait Stream: AsyncRead + AsyncWrite + Unpin + Send + Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Debug> Stream for T {}

/// Any encrypted (or, in tests, in-memory) connection to an IMAP server.
pub(crate) type Io = Box<dyn Stream>;

/// A connection quiet this long (an IDLE waiting for mail) is asked whether it's still
/// there. Routers forget quiet connections, and a Mac that slept leaves them dead, and
/// neither end hears about it: without asking, IDLE would wait for mail that never comes.
pub(crate) const KEEPALIVE_AFTER: Duration = Duration::from_secs(60);
pub(crate) const KEEPALIVE_EVERY: Duration = Duration::from_secs(15);
/// Unanswered checks before the connection counts as dead, about two minutes in all.
pub(crate) const KEEPALIVE_TRIES: u32 = 4;

pub(crate) fn keep_alive(tcp: &TcpStream) -> std::io::Result<()> {
    let checks = socket2::TcpKeepalive::new()
        .with_time(KEEPALIVE_AFTER)
        .with_interval(KEEPALIVE_EVERY)
        .with_retries(KEEPALIVE_TRIES);
    socket2::SockRef::from(tcp).set_tcp_keepalive(&checks)
}

/// Opens a connection ready to sign in, whose reads and writes fail once they wait longer
/// than `watch` allows. Tests swap in a scripted server.
#[async_trait]
pub(crate) trait Connector: Send + Sync {
    async fn connect(&self, watch: &Watch) -> Result<Client<Io>>;
}

pub(crate) struct TcpConnector {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub roots: TlsRoots,
    pub timeout: Duration,
}

#[async_trait]
impl Connector for TcpConnector {
    async fn connect(&self, watch: &Watch) -> Result<Client<Io>> {
        tokio::time::timeout(self.timeout, self.open(watch))
            .await
            .map_err(|_| {
                Error::Network(format!(
                    "timed out connecting to {}:{}",
                    self.host, self.port
                ))
            })?
    }
}

impl TcpConnector {
    async fn open(&self, watch: &Watch) -> Result<Client<Io>> {
        let name = ServerName::try_from(self.host.clone())
            .map_err(|_| Error::Network(format!("not a host name: {:?}", self.host)))?;
        let tls = TlsConnector::from(client_config(&self.roots)?);
        let tcp = TcpStream::connect((self.host.as_str(), self.port)).await?;
        keep_alive(&tcp)?;
        let tcp = Stall::new(tcp, watch.clone());
        match self.security {
            Security::Tls => greeted(Box::new(tls.connect(name, tcp).await.map_err(|_| Error::Network("TLS handshake failed. Check the server certificate and TLS settings.".into()))?)).await,
            Security::StartTls => {
                let mut plain = greeted(Box::new(tcp)).await?;
                plain
                    .run_command_and_check_ok("STARTTLS", None)
                    .await
                    .map_err(super::imap_error)?;
                let stream = tls.connect(name, plain.into_inner()).await.map_err(|_| Error::Network("TLS handshake failed. Check the server certificate and TLS settings.".into()))?;
                // No second greeting after STARTTLS.
                Ok(Client::new(Box::new(stream)))
            }
        }
    }
}

/// A client whose server has said hello.
pub(crate) async fn greeted(stream: Io) -> Result<Client<Io>> {
    let mut client = Client::new(stream);
    match client.read_response().await.map_err(super::imap_io_error)? {
        Some(_) => Ok(client),
        None => Err(Error::Network("the server closed the connection".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{serve_imap, step, test_tls};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn malformed_greetings_do_not_echo_server_content() {
        let (client, mut server) = tokio::io::duplex(1024);
        server.write_all(b"!! synthetic-private-greeting-marker\r\n").await.unwrap();
        drop(server);
        let error = greeted(Box::new(client)).await.unwrap_err();
        assert!(matches!(error, Error::Network(_)));
        assert!(!format!("{error:?} {error}").contains("synthetic-private-greeting-marker"));
    }


    fn watch() -> Watch {
        Watch::new(Duration::from_secs(5))
    }

    fn connector(port: u16, security: Security, root: Vec<u8>) -> TcpConnector {
        TcpConnector {
            host: "localhost".into(),
            port,
            security,
            roots: TlsRoots::Custom(vec![root]),
            timeout: Duration::from_secs(5),
        }
    }

    async fn listener() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    const LOGIN: &str = "{tag} LOGIN \"u\" \"p\"";

    async fn login(client: Client<Io>) {
        client.login("u", "p").await.map_err(|(e, _)| e).unwrap();
    }

    #[tokio::test]
    async fn connects_with_tls_from_the_start() {
        let (listener, port) = listener().await;
        let tls = test_tls();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let stream = tls.acceptor.accept(tcp).await.unwrap();
            serve_imap(
                stream,
                Some("* OK ready\r\n"),
                vec![step(LOGIN, "{tag} OK\r\n")],
            )
            .await
        });
        login(
            connector(port, Security::Tls, tls.root)
                .connect(&watch())
                .await
                .unwrap(),
        )
        .await;
        server.await.unwrap().unwrap();
    }

    /// Greets in the clear, answers STARTTLS with `reply`, then speaks TLS.
    async fn starttls_server(
        reply: &'static str,
    ) -> (u16, Vec<u8>, tokio::task::JoinHandle<Result<(), String>>) {
        let (listener, port) = listener().await;
        let tls = test_tls();
        let root = tls.root.clone();
        let server = tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            tcp.write_all(b"* OK [CAPABILITY IMAP4rev1 STARTTLS] ready\r\n")
                .await
                .unwrap();
            let mut line = String::new();
            BufReader::new(&mut tcp).read_line(&mut line).await.unwrap();
            let (tag, command) = line.trim_end().split_once(' ').unwrap();
            assert_eq!(command, "STARTTLS");
            tcp.write_all(format!("{tag} {reply}\r\n").as_bytes())
                .await
                .unwrap();
            if !reply.starts_with("OK") {
                return Ok(());
            }
            let stream = tls.acceptor.accept(tcp).await.unwrap();
            serve_imap(stream, None, vec![step(LOGIN, "{tag} OK\r\n")]).await
        });
        (port, root, server)
    }

    #[tokio::test]
    async fn a_quiet_connection_is_checked_so_a_dead_one_fails_instead_of_waiting_forever() {
        let (listener, port) = listener().await;
        let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let _server = listener.accept().await.unwrap();
        keep_alive(&tcp).unwrap();
        let socket = socket2::SockRef::from(&tcp);
        assert!(socket.keepalive().unwrap());
        assert_eq!(socket.tcp_keepalive_time().unwrap(), KEEPALIVE_AFTER);
        assert_eq!(socket.tcp_keepalive_interval().unwrap(), KEEPALIVE_EVERY);
        assert_eq!(socket.tcp_keepalive_retries().unwrap(), KEEPALIVE_TRIES);
    }

    #[tokio::test]
    async fn upgrades_with_starttls_before_signing_in() {
        let (port, root, server) = starttls_server("OK begin TLS").await;
        login(
            connector(port, Security::StartTls, root)
                .connect(&watch())
                .await
                .unwrap(),
        )
        .await;
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn stops_when_the_server_refuses_starttls() {
        let (port, root, server) = starttls_server("NO not today").await;
        let e = connector(port, Security::StartTls, root)
            .connect(&watch())
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Protocol(_)), "{e:?}");
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn refuses_an_untrusted_certificate() {
        for security in [Security::Tls, Security::StartTls] {
            let (listener, port) = listener().await;
            let tls = test_tls();
            let server = tokio::spawn(async move {
                let (mut tcp, _) = listener.accept().await.unwrap();
                if security == Security::StartTls {
                    tcp.write_all(b"* OK ready\r\n").await.unwrap();
                    let mut line = String::new();
                    BufReader::new(&mut tcp).read_line(&mut line).await.unwrap();
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    assert_eq!(command, "STARTTLS");
                    tcp.write_all(format!("{tag} OK begin TLS\r\n").as_bytes()).await.unwrap();
                }
                tls.acceptor.accept(tcp).await.is_err()
            });
            let e = connector(port, security, test_tls().root).connect(&watch()).await.unwrap_err();
            assert!(matches!(&e, Error::Network(m) if m == "TLS handshake failed. Check the server certificate and TLS settings."), "{e:?}");
            assert!(server.await.unwrap(), "the handshake never completes");
        }
    }

    #[tokio::test]
    async fn a_server_that_hangs_up_or_goes_quiet_fails_cleanly() {
        let (listener, port) = listener().await;
        let hang_up = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            drop(tcp);
            listener
        });
        let plain = connector(port, Security::StartTls, test_tls().root);
        assert!(matches!(plain.connect(&watch()).await, Err(Error::Network(m)) if m.contains("closed")));
        // Still listening but never accepting: the connection opens and nothing is said.
        let _listener = hang_up.await.unwrap();
        let quiet = TcpConnector {
            timeout: Duration::from_millis(50),
            ..plain
        };
        assert!(matches!(quiet.connect(&watch()).await, Err(Error::Network(m)) if m.contains("timed out")));
    }

    #[tokio::test]
    async fn an_unreachable_or_unnamed_host_is_a_network_error() {
        let refused = connector(1, Security::Tls, test_tls().root);
        assert!(matches!(refused.connect(&watch()).await, Err(Error::Network(_))));
        let unnamed = TcpConnector {
            host: "not a host".into(),
            ..connector(1, Security::Tls, test_tls().root)
        };
        assert!(
            matches!(unnamed.connect(&watch()).await, Err(Error::Network(m)) if m.contains("not a host name"))
        );
        let bad_roots = TcpConnector {
            roots: TlsRoots::Custom(vec![vec![0]]),
            ..connector(1, Security::Tls, vec![])
        };
        assert!(
            matches!(bad_roots.connect(&watch()).await, Err(Error::Network(m)) if m.contains("TLS setup"))
        );
    }
}
