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
                // Only a refused renewal means the lease is gone. Errors (a busy database,
                // a configuration file being replaced) are retried on the next beat; the
                // lease expires on its own if they persist.
                if let Ok(false) =
                    ScheduleStore::open(&path).and_then(|s| s.heartbeat(&name, now_ms()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn expires(home: &Path) -> i64 {
        ScheduleStore::open(home)
            .unwrap()
            .connection
            .query_row("SELECT expires FROM coordinator", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn heartbeat_survives_a_transient_error() {
        let home = tempfile::tempdir().unwrap();
        agt_config::initialize_home(Some(home.path().to_path_buf())).unwrap();
        let _lease = HomeLease::acquire(home.path()).unwrap();
        let acquired = expires(home.path());
        // An unreadable configuration makes the first beat (after 2 s) fail.
        let config = home.path().join("config.yaml");
        let saved = std::fs::read(&config).unwrap();
        std::fs::write(&config, "scheduler: [unclosed").unwrap();
        thread::sleep(Duration::from_millis(3_000));
        std::fs::write(&config, saved).unwrap();
        // The beat after it must renew the lease again.
        thread::sleep(Duration::from_millis(2_500));
        assert!(expires(home.path()) > acquired + 3_000);
    }
}
