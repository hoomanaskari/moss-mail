//! Background archive work, separate from the main mail connection.
use std::path::PathBuf;
use moss_store::{BackupControl, BackupError, BackupInfo, BackupProgress};
use crate::Engine;

impl Engine {
    pub async fn create_backup(&self, destination: PathBuf, control: BackupControl, progress: impl FnMut(BackupProgress) + Send + 'static) -> Result<BackupInfo, BackupError> {
        let engine = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut source = engine.shared.store().backup_source()?;
            source.create(&destination, control, progress)
        }).await.map_err(|_| BackupError::Invalid)?
    }
}
