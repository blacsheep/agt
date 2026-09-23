//! Coordinator lease and independent heartbeat lifecycle.
use crate::{ScheduleStore, schedule_store::now_ms};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

/// Separate heartbeat: inbox scans and slow model calls must not expire ownership.
pub(crate) struct HomeLease {
    home: PathBuf,
    pub(super) owner: String,
    stop: mpsc::Sender<()>,
    heartbeat: Option<JoinHandle<()>>,
    pub(super) release: bool,
}
impl HomeLease {
    pub(crate) fn acquire(home: &Path) -> Result<Self> {
        let owner = format!("scheduler:{}", uuid::Uuid::new_v4());
        if !ScheduleStore::open(home)?.acquire(&owner, now_ms())? {
            bail!("another scheduler owns this home");
        }
        let (stop, rx) = mpsc::channel();
        let path = home.to_path_buf();
        let name = owner.clone();
        let heartbeat = thread::spawn(move || {
            while rx.recv_timeout(Duration::from_secs(2)) == Err(mpsc::RecvTimeoutError::Timeout) {
                if !ScheduleStore::open(&path)
                    .and_then(|s| s.heartbeat(&name, now_ms()))
                    .unwrap_or(false)
                {
                    break;
                }
            }
        });
        Ok(Self {
            home: home.into(),
            owner,
            stop,
            heartbeat: Some(heartbeat),
            release: true,
        })
    }
}
impl Drop for HomeLease {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.join();
        }
        if self.release {
            let _ = ScheduleStore::open(&self.home).and_then(|s| s.release(&self.owner));
        }
    }
}
