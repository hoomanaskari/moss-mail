//! Runs llama.cpp's `llama-server` for one model, on this Mac only (or Apple's helper, which
//! takes the same flags: see `apple`).
//!
//! Each run listens on 127.0.0.1, on a free port, behind a random key, so other apps and web
//! pages can't use it. The key goes in a file only you can read, not on the command line where
//! every process can see it. The server stops when the [`Server`] is dropped. Its process id
//! sits beside the key while it runs, so if Moss quits or crashes without stopping it, the next
//! start can: see [`stop_leftovers`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerError {
    #[error("couldn't start the AI runtime: {0}")]
    Spawn(String),
    #[error("the AI runtime stopped while loading the model")]
    Exited,
    #[error("the model took too long to load")]
    Timeout,
}

/// Where a running model answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// e.g. `http://127.0.0.1:52011`.
    pub url: String,
    pub key: String,
}

/// A running `llama-server`. Dropping it stops the process.
pub struct Server {
    endpoint: Endpoint,
    child: tokio::process::Child,
    pid_file: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.pid_file);
    }
}

/// Stops the servers a previous run left running, from the process ids in `run_dir`: only
/// processes that are still one of `binaries`, since an id can be reused by another program once
/// its server has gone. Clears the ids either way. Returns how many it stopped.
/// The caller must exclusively own the data folder before cleanup. Runtime pid files are
/// process-local evidence: never include the run directory in backups or copied test data.
pub fn stop_leftovers(run_dir: &Path, binaries: &[&Path]) -> usize {
    let Ok(entries) = std::fs::read_dir(run_dir) else {
        return 0;
    };
    let binaries: Vec<PathBuf> = binaries.iter().map(|b| b.canonicalize().unwrap_or_else(|_| b.to_path_buf())).collect();
    let mut system = sysinfo::System::new();
    let mut stopped = 0;
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().is_none_or(|e| e != "pid") {
            continue;
        }
        let pid = std::fs::read_to_string(&path).ok().and_then(|t| t.trim().parse::<u32>().ok());
        let _ = std::fs::remove_file(&path);
        let Some(pid) = pid.map(sysinfo::Pid::from_u32) else {
            continue;
        };
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always),
        );
        // macOS may report /var while canonical paths use /private/var. Resolve both
        // identities, and fail closed if the running executable no longer exists.
        let ours = system.process(pid).filter(|p| {
            p.exe().and_then(|exe| exe.canonicalize().ok())
                .is_some_and(|exe| binaries.iter().any(|b| b == &exe))
        });
        if ours.is_some_and(|p| p.kill()) {
            stopped += 1;
        }
    }
    stopped
}

/// A random key for one run.
pub fn new_key() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A port nothing is listening on right now.
pub fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

/// Ports that starts in this process have picked and whose runtime isn't listening on yet.
/// The system can offer the same free port twice; two starts at once must not both take it,
/// and neither should wait for the other.
static PICKED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// How many free ports a start is offered before it gives up because each is another start's.
const PICKS: usize = 64;

/// A port one start has picked, kept from the others until it is dropped.
struct Picked(u16);

impl Drop for Picked {
    fn drop(&mut self) {
        PICKED.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.0);
    }
}

/// A port from `offer` that no other start in this process is still starting on.
fn pick_port(mut offer: impl FnMut() -> std::io::Result<u16>) -> std::io::Result<Picked> {
    for _ in 0..PICKS {
        let port = offer()?;
        if PICKED.lock().unwrap_or_else(PoisonError::into_inner).insert(port) {
            return Ok(Picked(port));
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "every free port offered was another start's"))
}

/// Writes `text` to a file only this user can read.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(path)?, text.as_bytes())
}

impl Server {
    /// Starts `binary` on `model` with `extra` flags and waits until the model has loaded.
    /// The run's key is written to `key_file`.
    pub async fn start(
        binary: &Path,
        model: &Path,
        extra: &[&str],
        key_file: &Path,
        wait: Duration,
    ) -> Result<Server, ServerError> {
        let deadline = tokio::time::Instant::now() + wait;
        let spawn = |e: std::io::Error| ServerError::Spawn(e.to_string());
        // The runtime is on this Mac, so no proxy: looking up the system's can take seconds.
        let client = reqwest::Client::builder().no_proxy().build().map_err(|e| ServerError::Spawn(e.to_string()))?;
        // Other starts skip this port until the runtime answers on it, before its model loads.
        // No start waits for another: a missing runtime is reported as one at once.
        let picked = pick_port(free_port).map_err(spawn)?;
        let port = picked.0;
        let mut picked = Some(picked);
        let key = new_key();
        write_private(key_file, &key).map_err(spawn)?;
        let child = tokio::process::Command::new(binary)
            .arg("-m")
            .arg(model)
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--no-webui",
            ])
            .arg("--api-key-file")
            .arg(key_file)
            .args(extra)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(spawn)?;
        let pid_file = key_file.with_extension("pid");
        let pid = child.id().expect("a process that has just started has an id");
        write_private(&pid_file, &pid.to_string()).map_err(spawn)?;
        let endpoint = Endpoint {
            url: format!("http://127.0.0.1:{port}"),
            key,
        };
        // From here the server stops with `starting`, if it doesn't load.
        let mut starting = Server {
            endpoint,
            child,
            pid_file,
        };
        tokio::time::timeout_at(deadline, async {
            loop {
                if let Ok(Some(_)) = starting.child.try_wait() {
                    return Err(ServerError::Exited);
                }
                let health = client.get(format!("{}/health", starting.endpoint.url)).send().await;
                if health.is_ok() { drop(picked.take()); }
                if health.is_ok_and(|r| r.status().is_success()) {
                    return Ok(starting);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.unwrap_or(Err(ServerError::Timeout))
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    #[test]
    fn two_starts_never_pick_the_same_port() {
        // The system never offers ports this low as free ones, so real starts don't clash.
        let mut offers = [7, 7, 8].into_iter();
        let first = pick_port(|| Ok(offers.next().unwrap())).unwrap();
        let second = pick_port(|| Ok(offers.next().unwrap())).unwrap();
        assert_eq!((first.0, second.0), (7, 8), "the port the first start picked is skipped");
        drop(first);
        let again = pick_port(|| Ok(7)).unwrap();
        assert_eq!(again.0, 7, "once its runtime answers, or its start ends, the port is free again");
        let taken = pick_port(|| Ok(7)).err().unwrap();
        assert_eq!(taken.kind(), ErrorKind::AddrInUse, "only offered picked ports, a start gives up");
        let failed = pick_port(|| Err(Error::other("no ports"))).err().unwrap();
        assert_eq!(failed.to_string(), "no ports");
        drop((second, again));
    }
}
