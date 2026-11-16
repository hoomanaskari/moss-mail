//! The two HTTP clients the AI uses: one for the models on this Mac, one for downloading them.
//!
//! Neither has a limit on a whole request: a model may write for minutes, and a model file
//! takes as long as the connection needs. Each limits how long one read may wait with nothing
//! arriving instead, so a runtime or download server that stops answering fails rather than
//! hanging for ever.

use std::time::Duration;

/// Reaching a model on this Mac: it's listening already, or it isn't there.
pub const LOCAL_CONNECT: Duration = Duration::from_secs(5);
/// How long a model on this Mac may go quiet: reading a long prompt before its first word can
/// take a few minutes on a slower Mac.
pub const LOCAL_READ: Duration = Duration::from_secs(5 * 60);
/// Reaching the download server.
pub const DOWNLOAD_CONNECT: Duration = Duration::from_secs(15);
/// How long a download may wait with nothing arriving.
pub const DOWNLOAD_READ: Duration = Duration::from_secs(60);

/// For the models on this Mac (llama-server and Apple's helper, on 127.0.0.1). Never asks the
/// system for a proxy: none applies, and looking it up can take seconds.
pub fn local() -> reqwest::Client {
    local_within(LOCAL_READ)
}

pub(crate) fn local_within(read: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(LOCAL_CONNECT)
        .read_timeout(read)
        .build()
        .expect("a client without a proxy always builds")
}

/// For downloading models. Building it reads the system's proxy settings and trust store,
/// which can take seconds: build it off the main thread, when a download starts.
pub fn internet() -> reqwest::Client {
    internet_within(DOWNLOAD_CONNECT, DOWNLOAD_READ)
}

pub(crate) fn internet_within(connect: Duration, read: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .read_timeout(read)
        .build()
        .expect("the HTTP client builds with default TLS")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A server that reads one request, then sends `answer` piece by piece, `pause` apart, and
    /// then nothing, with the connection open.
    async fn server(pause: Duration, answer: Vec<&'static [u8]>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = socket.read(&mut [0; 4096]).await;
            for piece in answer {
                tokio::time::sleep(pause).await;
                let _ = socket.write_all(piece).await;
            }
            let _ = socket.read_to_end(&mut Vec::new()).await;
        });
        format!("http://{address}/")
    }

    const READ: Duration = Duration::from_millis(300);
    const HEAD: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 4\r\n\r\n";

    #[tokio::test]
    async fn a_model_that_goes_quiet_fails_instead_of_hanging() {
        let url = server(Duration::ZERO, vec![HEAD, b"ab"]).await;
        let started = std::time::Instant::now();
        let response = local_within(READ).get(&url).send().await.unwrap();
        assert!(response.bytes().await.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_long_answer_that_keeps_coming_is_never_cut() {
        for client in [local_within(READ), internet_within(DOWNLOAD_CONNECT, READ)] {
            let url = server(READ / 2, vec![HEAD, b"a", b"b", b"c", b"d"]).await;
            let started = std::time::Instant::now();
            let body = client.get(&url).send().await.unwrap().bytes().await.unwrap();
            assert_eq!(&body[..], b"abcd");
            assert!(started.elapsed() > READ * 2, "longer than one read may wait, in all");
        }
    }

    #[test]
    fn the_local_client_never_asks_the_system_for_a_proxy() {
        assert!(!format!("{:?}", local()).contains("proxies"));
        let _ = internet();
        assert_eq!((LOCAL_CONNECT, LOCAL_READ), (Duration::from_secs(5), Duration::from_secs(300)));
        assert_eq!((DOWNLOAD_CONNECT, DOWNLOAD_READ), (Duration::from_secs(15), Duration::from_secs(60)));
    }
}
