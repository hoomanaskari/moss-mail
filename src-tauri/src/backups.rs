//! Own archive jobs independently of any Settings screen or invoke future.
use std::{future::Future, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
use moss_store::{BackupControl, BackupError, BackupInfo, BackupProgress};
use serde::Serialize;
use tokio::sync::watch;

type Progress = Box<dyn FnMut(BackupProgress) + Send>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum Phase {
    Preparing,
    Copying { #[serde(rename = "copiedPages")] copied_pages: u32, #[serde(rename = "totalPages")] total_pages: u32 },
    Checking, Hashing, Publishing,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    Idle,
    Running { id: u32, progress: Phase, cancelling: bool },
    Complete { id: u32, bytes: u64, sha256: String },
    Cancelled { id: u32 },
    Failed { id: u32, message: String },
}
struct Inner { next: u32, closed: bool, active: Option<(u32, Arc<AtomicBool>)> }
#[derive(Clone)]
pub struct Backups { inner: Arc<Mutex<Inner>>, status: watch::Sender<Status> }
impl Default for Backups {
    fn default() -> Self {
        Self {inner: Arc::new(Mutex::new(Inner {next: 0, closed: false, active: None})), status: watch::Sender::new(Status::Idle)}
    }
}
impl Backups {
    pub fn status(&self) -> Status { self.status.borrow().clone() }
    pub fn start(&self, engine: moss_engine::Engine, destination: std::path::PathBuf) -> Result<u32, String> {
        self.start_with(move |control, progress| async move {
            let info = engine.create_backup(destination.clone(), control, progress).await?;
            checked(destination, info).await
        })
    }

    fn start_with<F>(&self, job: impl FnOnce(BackupControl, Progress) -> F + Send + 'static) -> Result<u32, String>
    where F: Future<Output = Result<BackupInfo, BackupError>> + Send + 'static {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed { return Err("Moss is quitting. A backup cannot start.".into()); }
        if inner.active.is_some() { return Err("A backup is already running.".into()); }
        let id = inner.next.checked_add(1).ok_or("Restart Moss before creating another backup.")?;
        inner.next = id;
        let control = BackupControl::default();
        inner.active = Some((id, control.cancelled.clone()));
        self.status.send_replace(Status::Running {id, progress: Phase::Preparing, cancelling: false});
        let updates = self.clone();
        let owner = self.clone();
        tokio::spawn(async move {
            // Supervise unexpected task panics too, so status cannot remain running forever.
            let result = tokio::spawn(async move {
                job(control, Box::new(move |progress| updates.progress(id, progress))).await
            }).await.unwrap_or(Err(BackupError::Invalid));
            owner.finish(id, result);
        });
        Ok(id)
    }

    fn progress(&self, id: u32, progress: BackupProgress) {
        let inner = self.inner.lock().unwrap();
        if let Some((active, cancelled)) = &inner.active {
            if *active == id {
                let progress = match progress {
                    BackupProgress::Preparing => Phase::Preparing,
                    BackupProgress::Copying {copied_pages, total_pages} => Phase::Copying {copied_pages, total_pages},
                    BackupProgress::Checking => Phase::Checking,
                    BackupProgress::Hashing => Phase::Hashing,
                    BackupProgress::Publishing => Phase::Publishing,
                };
                self.status.send_replace(Status::Running {id, progress, cancelling: cancelled.load(Ordering::Relaxed)});
            }
        }
    }

    fn finish(&self, id: u32, result: Result<BackupInfo, BackupError>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.active.as_ref().map(|(active, _)| *active) != Some(id) { return; }
        inner.active = None;
        let status = match result {
            Ok(info) => Status::Complete {id, bytes: info.bytes, sha256: info.sha256},
            Err(BackupError::Cancelled) => Status::Cancelled {id},
            Err(error) => Status::Failed {id, message: error.to_string()},
        };
        self.status.send_replace(status);
    }

    pub fn cancel(&self, id: u32) -> bool {
        let inner = self.inner.lock().unwrap();
        let Some((active, cancelled)) = &inner.active else { return false; };
        if *active != id { return false; }
        cancelled.store(true, Ordering::Relaxed);
        self.status.send_modify(|status| {
            if let Status::Running {cancelling, ..} = status { *cancelling = true; }
        });
        true
    }

    /// Refuse new work, request cancellation, and wait up to the limit. A stalled
    /// filesystem cannot be forcibly interrupted; false means the job is still running.
    pub async fn shutdown(&self, limit: Duration) -> bool {
        let mut status = self.status.subscribe();
        let id = {
            let mut inner = self.inner.lock().unwrap();
            inner.closed = true;
            inner.active.as_ref().map(|(id, _)| *id)
        };
        if let Some(id) = id { self.cancel(id); }
        tokio::time::timeout(limit, async {
            while matches!(*status.borrow_and_update(), Status::Running {..}) {
                // This owner retains the sender throughout the wait.
                status.changed().await.expect("backup status sender remains alive");
            }
        }).await.is_ok()
    }

}

/// Reads a backup back from where it was saved, as a restore would, before it is called done.
/// It is published by then, so cancelling no longer stops it.
async fn checked(destination: std::path::PathBuf, info: BackupInfo) -> Result<BackupInfo, BackupError> {
    tokio::task::spawn_blocking(move || {
        match moss_store::verify_backup(&destination, Some(&info.sha256), Default::default()) {
            Ok(_) => Ok(info),
            Err(_) => Err(BackupError::Unverified),
        }
    }).await.unwrap_or(Err(BackupError::Unverified))
}

#[tauri::command]
pub fn backup_status(jobs: tauri::State<'_, Backups>) -> Status { jobs.status() }

type Selection = Result<Option<std::path::PathBuf>, String>;
type Selected = Box<dyn FnOnce(Selection) + Send>;
/// Only native code supplies this picker. Renderer arguments never grant write access.
pub struct BackupDialog(pub Box<dyn Fn(Selected) + Send + Sync>);
impl BackupDialog {
    async fn choose(&self) -> Selection {
        let (send, receive) = tokio::sync::oneshot::channel();
        (self.0)(Box::new(move |selection| { let _ = send.send(selection); }));
        receive.await.map_err(|_| "The archive destination could not be selected.".to_string())?
    }
}

#[tauri::command]
pub async fn backup_start(jobs: tauri::State<'_, Backups>, engine: tauri::State<'_, moss_engine::Engine>, dialog: tauri::State<'_, BackupDialog>) -> Result<Option<u32>, String> {
    {
        let inner = jobs.inner.lock().unwrap();
        if inner.closed { return Err("Moss is quitting. A backup cannot start.".into()); }
        if inner.active.is_some() { return Err("A backup is already running.".into()); }
    }
    let Some(destination) = dialog.choose().await? else { return Ok(None); };
    match std::fs::symlink_metadata(&destination) {
        Ok(_) => return Err("That file already exists. Choose a new name for the archive.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(_) => return Err("The archive destination could not be checked.".into()),
    }
    jobs.start(engine.inner().clone(), destination).map(Some)
}

#[tauri::command]
pub fn backup_cancel(jobs: tauri::State<'_, Backups>, id: u32) -> bool { jobs.cancel(id) }

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn abandoned_native_picker_reports_failure_instead_of_starting_a_backup() {
        let dialog=BackupDialog(Box::new(drop));
        assert_eq!(dialog.choose().await,Err("The archive destination could not be selected.".into()));
    }

    #[tokio::test]
    async fn owns_a_single_job_and_retains_its_result() {
        let jobs = Backups::default();
        assert_eq!(jobs.status(), Status::Idle);
        let (release, released) = oneshot::channel();
        let id = jobs.start_with(move |_, _| async move {
            released.await.unwrap();
            Ok(BackupInfo {bytes: 42, sha256: "fixture".into()})
        }).unwrap();
        assert_eq!(id, 1);
        assert!(matches!(jobs.status(), Status::Running {id: 1, cancelling: false, ..}));
        let next = |_, _| async { Ok(BackupInfo {bytes: 7, sha256: "next".into()}) };
        assert!(jobs.start_with(next).is_err());
        release.send(()).unwrap();
        let mut status = jobs.status.subscribe();
        while matches!(*status.borrow_and_update(), Status::Running {..}) { status.changed().await.unwrap(); }
        assert_eq!(jobs.status(), Status::Complete {id, bytes:42, sha256:"fixture".into()});
        let next_id = jobs.start_with(next).unwrap();
        assert_eq!(next_id, 2);
        settled(&jobs).await;
        assert_eq!(jobs.status(), Status::Complete {id: next_id, bytes:7, sha256:"next".into()});
    }
    async fn settled(jobs: &Backups) {
        let mut status = jobs.status.subscribe();
        while matches!(*status.borrow_and_update(), Status::Running {..}) { status.changed().await.unwrap(); }
    }

    #[tokio::test]
    async fn cancellation_is_scoped_to_the_job_and_cannot_erase_published_success() {
        let jobs = Backups::default();
        assert!(!jobs.cancel(1));
        let (release, released) = oneshot::channel();
        let (ready, prepared) = oneshot::channel();
        let id = jobs.start_with(move |control, progress| async move {
            ready.send((control.cancelled, progress)).ok().unwrap();
            released.await.unwrap();
            Ok(BackupInfo {bytes:99, sha256:"published".into()})
        }).unwrap();
        let (cancelled, mut progress) = prepared.await.unwrap();
        assert!(!jobs.cancel(id + 1));
        assert!(!cancelled.load(Ordering::Relaxed));
        for (event, expected) in [
            (BackupProgress::Preparing, Phase::Preparing),
            (BackupProgress::Copying {copied_pages:128,total_pages:500}, Phase::Copying {copied_pages:128,total_pages:500}),
            (BackupProgress::Checking, Phase::Checking),
            (BackupProgress::Hashing, Phase::Hashing),
            (BackupProgress::Publishing, Phase::Publishing),
        ] {
            progress(event);
            let wire = serde_json::to_value(jobs.status()).unwrap();
            let phase = match event {
                BackupProgress::Preparing => serde_json::json!({"phase":"preparing"}),
                BackupProgress::Copying {..} => serde_json::json!({"phase":"copying","copiedPages":128,"totalPages":500}),
                BackupProgress::Checking => serde_json::json!({"phase":"checking"}),
                BackupProgress::Hashing => serde_json::json!({"phase":"hashing"}),
                BackupProgress::Publishing => serde_json::json!({"phase":"publishing"}),
            };
            assert_eq!(wire, serde_json::json!({"state":"running","id":id,"progress":phase,"cancelling":false}));
            assert_eq!(jobs.status(), Status::Running {id, progress:expected, cancelling:false});
        }
        assert!(jobs.cancel(id));
        assert!(cancelled.load(Ordering::Relaxed));
        assert_eq!(jobs.status(), Status::Running {id, progress:Phase::Publishing, cancelling:true});
        progress(BackupProgress::Checking);
        assert_eq!(jobs.status(), Status::Running {id, progress:Phase::Checking, cancelling:true});
        let wire = serde_json::to_value(jobs.status()).unwrap();
        assert_eq!(wire, serde_json::json!({"state":"running", "id":id, "progress":{"phase":"checking"}, "cancelling":true}));
        let before = jobs.status();
        jobs.progress(id+1, BackupProgress::Preparing);
        jobs.finish(id+1, Err(BackupError::Cancelled));
        assert_eq!(jobs.status(), before);
        release.send(()).unwrap();
        settled(&jobs).await;
        assert_eq!(jobs.status(), Status::Complete {id,bytes:99,sha256:"published".into()});
        progress(BackupProgress::Preparing);
        jobs.finish(id, Err(BackupError::Cancelled));
        assert!(!jobs.cancel(id));
        assert_eq!(jobs.status(), Status::Complete {id,bytes:99,sha256:"published".into()});
    }

    #[tokio::test]
    async fn records_cancellation_safe_errors_and_unexpected_worker_panics() {
        let jobs = Backups::default();
        let id = jobs.start_with(|_, _| async { Err(BackupError::Cancelled) }).unwrap();
        settled(&jobs).await;
        assert_eq!(jobs.status(), Status::Cancelled {id});
        assert_eq!(serde_json::to_value(jobs.status()).unwrap(), serde_json::json!({"state":"cancelled", "id":id}));
        let id = jobs.start_with(|_, _| async { Err(BackupError::Io(std::io::Error::other("private fixture path and mailbox"))) }).unwrap();
        settled(&jobs).await;
        assert_eq!(jobs.status(), Status::Failed {id,message:"The backup could not be written.".into()});
        assert_eq!(serde_json::to_value(jobs.status()).unwrap(), serde_json::json!({"state":"failed", "id":id, "message":"The backup could not be written."}));
        let id = jobs.start_with(|_, _| async { panic!("fixture worker failure") }).unwrap();
        settled(&jobs).await;
        assert_eq!(jobs.status(), Status::Failed {id,message:"The backup did not pass verification.".into()});
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_cancels_waits_and_permanently_refuses_new_work() {
        let jobs = Backups::default();
        let (release, released) = oneshot::channel();
        let (ready, prepared) = oneshot::channel();
        let id = jobs.start_with(move |control, _| async move {
            ready.send(control.cancelled).unwrap();
            released.await.unwrap();
            Err(BackupError::Cancelled)
        }).unwrap();
        let cancelled = prepared.await.unwrap();
        assert!(!jobs.shutdown(Duration::from_secs(1)).await);
        assert!(cancelled.load(Ordering::Relaxed));
        assert!(matches!(jobs.status(), Status::Running {id:active, cancelling:true, ..} if active==id));
        let job = |_, _| async { Err(BackupError::Cancelled) };
        assert!(jobs.start_with(job).is_err());
        release.send(()).unwrap();
        assert!(jobs.shutdown(Duration::from_secs(1)).await);
        assert_eq!(jobs.status(), Status::Cancelled {id});
        assert!(jobs.shutdown(Duration::ZERO).await);
        assert!(jobs.start_with(job).is_err());
        let other = Backups::default();
        other.start_with(job).unwrap();
        settled(&other).await;
        assert!(matches!(other.status(), Status::Cancelled {..}));
    }

    #[tokio::test]
    async fn a_backup_is_read_back_from_where_it_was_saved_before_it_is_done() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let secrets = std::sync::Arc::new(moss_accounts::MemorySecretStore::new());
        let engine = crate::engine(&data, secrets, std::sync::Arc::new(crate::ask::tests::Quiet), moss_providers::TlsRoots::Platform).unwrap();
        let jobs = Backups::default();
        let destination = dir.path().join("mail.moss-backup");
        let id = jobs.start(engine, destination.clone()).unwrap();
        settled(&jobs).await;
        let Status::Complete {id: done, bytes, sha256} = jobs.status() else { panic!("{:?}", jobs.status()) };
        assert_eq!((done, bytes), (id, std::fs::metadata(&destination).unwrap().len()));
        assert_eq!(moss_store::verify_backup(&destination, Some(&sha256), Default::default()).unwrap().info().sha256, sha256);
    }

    #[tokio::test]
    async fn a_backup_that_does_not_read_back_as_written_is_not_called_done() {
        let dir = tempfile::tempdir().unwrap();
        let saved = dir.path().join("mail.moss-backup");
        std::fs::write(&saved, b"not what was written").unwrap();
        let info = || BackupInfo {bytes: 20, sha256: "fixture".into()};
        let jobs = Backups::default();
        let id = jobs.start_with(move |_, _| async move { checked(saved, info()).await }).unwrap();
        settled(&jobs).await;
        assert_eq!(jobs.status(), Status::Failed {id, message: "The backup was saved, but reading it back failed, so it can't be trusted. Delete it and make a new one.".into()});
        assert!(matches!(checked(dir.path().join("missing"), info()).await, Err(BackupError::Unverified)));
    }

    #[tokio::test]
    async fn refuses_id_wraparound_without_starting_work() {
        let jobs = Backups::default();
        jobs.inner.lock().unwrap().next = u32::MAX;
        let job = |_, _| async { Err(BackupError::Cancelled) };
        assert!(jobs.start_with(job).is_err());
        assert_eq!(jobs.status(), Status::Idle);
        jobs.inner.lock().unwrap().next = 0;
        assert_eq!(jobs.start_with(job).unwrap(), 1);
        settled(&jobs).await;
        assert!(matches!(jobs.status(), Status::Cancelled {id:1}));
    }

}
