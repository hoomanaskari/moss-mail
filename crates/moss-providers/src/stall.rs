//! A connection whose server stops answering fails, instead of waiting for ever.
//!
//! [`Stall`] wraps a stream and fails a read (or write) that has made no progress for the
//! [`Watch`]'s limit. Only waiting with nothing arriving counts: a big download that keeps
//! coming, however long it takes, is never cut. A connection that sits unused between
//! commands isn't waiting either, so it never times out. IMAP's IDLE, which waits for mail on
//! purpose, turns the watch off while it waits.

use std::future::Future as _;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Instant, Sleep};

/// How long a [`Stall`] lets a read or write wait for the server. Cheap to clone: every
/// clone sets the same limit.
#[derive(Clone, Debug)]
pub(crate) struct Watch(Arc<AtomicU64>);

impl Watch {
    pub(crate) fn new(limit: Duration) -> Self {
        Watch(Arc::new(AtomicU64::new(millis(limit))))
    }

    /// From now on, a wait longer than `limit` fails.
    pub(crate) fn set(&self, limit: Duration) {
        self.0.store(millis(limit), Ordering::SeqCst);
    }

    /// Waits as long as it takes, until the next [`Watch::set`].
    pub(crate) fn off(&self) {
        self.0.store(0, Ordering::SeqCst);
    }

    fn limit(&self) -> Option<Duration> {
        Some(self.0.load(Ordering::SeqCst)).filter(|ms| *ms > 0).map(Duration::from_millis)
    }
}

fn millis(limit: Duration) -> u64 {
    u64::try_from(limit.as_millis()).unwrap_or(u64::MAX).max(1)
}

/// What a stalled read or write fails with.
pub(crate) fn stalled() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "the server stopped answering")
}

/// Whether `e` is a [`Stall`] giving up.
pub(crate) fn is_stall(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::TimedOut
}

#[derive(Debug)]
pub(crate) struct Stall<S> {
    inner: S,
    watch: Watch,
    read: Option<Pin<Box<Sleep>>>,
    write: Option<Pin<Box<Sleep>>>,
}

impl<S> Stall<S> {
    pub(crate) fn new(inner: S, watch: Watch) -> Self {
        Stall { inner, watch, read: None, write: None }
    }
}

/// `Ready` with a stall error once `timer` (started on the first pending poll) runs out.
fn wait(timer: &mut Option<Pin<Box<Sleep>>>, limit: Option<Duration>, cx: &mut Context<'_>) -> Poll<io::Error> {
    let Some(limit) = limit else {
        *timer = None;
        return Poll::Pending;
    };
    let sleep = timer.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(Instant::now() + limit)));
    match sleep.as_mut().poll(cx) {
        Poll::Ready(()) => {
            *timer = None;
            Poll::Ready(stalled())
        }
        Poll::Pending => Poll::Pending,
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Stall<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(done) => {
                this.read = None;
                Poll::Ready(done)
            }
            Poll::Pending => wait(&mut this.read, this.watch.limit(), cx).map(Err),
        }
    }
}

impl<S: AsyncWrite + Unpin> Stall<S> {
    /// Polls one write-side step, failing it once it has waited too long. Any write also
    /// starts the wait for an answer afresh: it's a new command.
    fn step<T>(&mut self, cx: &mut Context<'_>, poll: impl FnOnce(Pin<&mut S>, &mut Context<'_>) -> Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        match poll(Pin::new(&mut self.inner), cx) {
            Poll::Ready(done) => {
                self.write = None;
                self.read = None;
                Poll::Ready(done)
            }
            Poll::Pending => wait(&mut self.write, self.watch.limit(), cx).map(Err),
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Stall<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        self.step(cx, |inner, cx| inner.poll_write(cx, buf))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.step(cx, |inner, cx| inner.poll_flush(cx))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.step(cx, |inner, cx| inner.poll_shutdown(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const LIMIT: Duration = Duration::from_secs(60);
    /// Longer than any test should take, so a missing timeout fails instead of hanging.
    const NEVER: Duration = Duration::from_secs(24 * 3600);

    async fn read_one(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<u8> {
        let mut byte = [0];
        tokio::time::timeout(NEVER, stream.read_exact(&mut byte)).await.expect("the read should end by itself")?;
        Ok(byte[0])
    }

    #[tokio::test(start_paused = true)]
    async fn a_read_that_waits_too_long_fails() {
        let (client, _server) = tokio::io::duplex(64);
        let mut stream = Stall::new(client, Watch::new(LIMIT));
        let started = Instant::now();
        let error = read_one(&mut stream).await.unwrap_err();
        assert!(is_stall(&error), "{error:?}");
        assert_eq!(error.to_string(), "the server stopped answering");
        assert_eq!(started.elapsed(), LIMIT);
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_answer_that_keeps_coming_is_never_cut() {
        let (client, mut server) = tokio::io::duplex(64);
        let mut stream = Stall::new(client, Watch::new(LIMIT));
        let feed = tokio::spawn(async move {
            for byte in 0..5u8 {
                tokio::time::sleep(LIMIT - Duration::from_secs(1)).await;
                server.write_all(&[byte]).await.unwrap();
            }
            server
        });
        let started = Instant::now();
        for byte in 0..5u8 {
            assert_eq!(read_one(&mut stream).await.unwrap(), byte);
        }
        assert!(started.elapsed() > LIMIT * 4, "five slow bytes take longer than the limit in all");
        drop(feed.await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_is_counted_per_command() {
        let (client, mut server) = tokio::io::duplex(64);
        let mut stream = Stall::new(client, Watch::new(LIMIT));
        // A read given up on (as IDLE's own timer does) leaves no deadline behind once the
        // next command is written.
        assert!(tokio::time::timeout(LIMIT / 2, read_one(&mut stream)).await.is_err());
        tokio::time::sleep(LIMIT).await;
        stream.write_all(b"A2 NOOP\r\n").await.unwrap();
        let answer = tokio::spawn(async move {
            tokio::time::sleep(LIMIT - Duration::from_secs(1)).await;
            server.write_all(b"!").await.unwrap();
            server
        });
        assert_eq!(read_one(&mut stream).await.unwrap(), b'!');
        drop(answer.await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn with_the_watch_off_a_read_waits_as_long_as_it_takes() {
        let (client, mut server) = tokio::io::duplex(64);
        let watch = Watch::new(LIMIT);
        let mut stream = Stall::new(client, watch.clone());
        watch.off();
        let mail = tokio::spawn(async move {
            tokio::time::sleep(LIMIT * 30).await;
            server.write_all(b"*").await.unwrap();
            server
        });
        assert_eq!(read_one(&mut stream).await.unwrap(), b'*');
        watch.set(LIMIT);
        let started = Instant::now();
        assert!(is_stall(&read_one(&mut stream).await.unwrap_err()));
        assert_eq!(started.elapsed(), LIMIT, "on again, the limit counts from the next wait");
        drop(mail.await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_stops_reading_fails_the_write() {
        let (client, server) = tokio::io::duplex(8);
        let mut stream = Stall::new(client, Watch::new(LIMIT));
        let started = Instant::now();
        let error = tokio::time::timeout(NEVER, stream.write_all(&[0; 64])).await.unwrap().unwrap_err();
        assert!(is_stall(&error), "{error:?}");
        assert_eq!(started.elapsed(), LIMIT);
        stream.flush().await.unwrap();
        drop(server);
        stream.shutdown().await.unwrap();
    }

    #[test]
    fn limits_are_whole_milliseconds_and_never_zero() {
        assert_eq!(Watch::new(Duration::from_micros(10)).limit(), Some(Duration::from_millis(1)));
        assert_eq!(Watch::new(Duration::MAX).limit(), Some(Duration::from_millis(u64::MAX)));
        assert!(!is_stall(&io::Error::other("x")));
    }
}
