//! Best-effort local wake hints. Call only after commit; durable stores remain authoritative.
#[cfg(unix)]
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::UnixDatagram,
    },
    sync::atomic::AtomicU64,
};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[cfg(unix)]
static NEXT: AtomicU64 = AtomicU64::new(0);

static WARNED: AtomicBool = AtomicBool::new(false);

/// Optional notification accelerator. Callers must periodically recheck durable storage.
/// Failed listeners are dropped; degradation is diagnosed once per process.
/// Subsequent waits only sleep.
pub struct Hints {
    listener: Option<Listener>,
}
impl Hints {
    pub fn open(home: &Path) -> Self {
        let listener = match Listener::open(home) {
            Ok(listener) => Some(listener),
            Err(error) => {
                Self::diagnose(&error);
                None
            }
        };
        Self { listener }
    }

    /// Whether this instance can still receive local wake hints.
    /// False on unsupported platforms; callers should use short polling intervals.
    pub fn available(&self) -> bool {
        cfg!(unix) && self.listener.is_some()
    }

    pub fn wait(&mut self, timeout: Duration) -> bool {
        let started = Instant::now();
        if let Some(listener) = &self.listener {
            match listener.wait(timeout) {
                Ok(hinted) => return hinted,
                Err(error) => {
                    Self::diagnose(&error);
                    self.listener = None;
                }
            }
        }
        std::thread::sleep(timeout.saturating_sub(started.elapsed()));
        false
    }

    fn diagnose(error: &io::Error) {
        if WARNED.swap(true, Ordering::Relaxed) {
            return;
        }
        let _ = writeln!(
            io::stderr().lock(),
            "wake hints unavailable; falling back to periodic storage checks: {error}"
        );
    }
}

/// Low-level, fallible socket listener. Runtime callers should use [Hints]
/// so notification failures do not interrupt business execution.
pub struct Listener {
    #[cfg(unix)]
    socket: UnixDatagram,
    #[cfg(unix)]
    path: PathBuf,
}
impl Listener {
    pub fn open(home: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let dir = home.join("runtime/wake");
            crate::ensure_dir_under(home, &dir).map_err(io::Error::other)?;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
            let path = dir.join(format!(
                "{}-{}.sock",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let socket = UnixDatagram::bind(&path)?;
            let listener = Self { socket, path };
            fs::set_permissions(&listener.path, fs::Permissions::from_mode(0o600))?;
            Ok(listener)
        }
        #[cfg(not(unix))]
        {
            let _ = home;
            Ok(Self {})
        }
    }
    /// Coalesces a bounded number of pending hints. False means timeout/unsupported.
    pub fn wait(&self, timeout: Duration) -> io::Result<bool> {
        #[cfg(unix)]
        {
            self.socket
                .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
            match self.socket.recv(&mut [0; 1]) {
                Ok(_) => {
                    self.socket.set_nonblocking(true)?;
                    for _ in 0..256 {
                        if self.socket.recv(&mut [0; 1]).is_err() {
                            break;
                        }
                    }
                    self.socket.set_nonblocking(false)?;
                    Ok(true)
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    Ok(false)
                }
                Err(e) => Err(e),
            }
        }
        #[cfg(not(unix))]
        {
            std::thread::sleep(timeout);
            Ok(false)
        }
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
/// Nonblocking and payload-free: a slow or dead subscriber never delays a commit.
pub fn notify(home: &Path) {
    #[cfg(unix)]
    {
        let dir = home.join("runtime/wake");
        if crate::check_no_symlinks_under(home, &dir).is_err() {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        if socket.set_nonblocking(true).is_err() {
            return;
        }
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_socket()) {
                continue;
            }
            if let Err(e) = socket.send_to(b"1", entry.path())
                && e.kind() == io::ErrorKind::ConnectionRefused
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = home;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn degradation_is_diagnosed_once_per_process() {
        const CHILD: &str = "AGT_WAKE_DIAGNOSTIC_TEST";
        if std::env::var_os(CHILD).is_some() {
            let home = tempfile::tempdir().unwrap();
            fs::create_dir(home.path().join("runtime")).unwrap();
            fs::write(home.path().join("runtime/wake"), b"blocked").unwrap();
            for _ in 0..3 {
                assert!(!Hints::open(home.path()).available());
            }
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "wake::tests::degradation_is_diagnosed_once_per_process",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr)
                .matches("wake hints unavailable")
                .count(),
            1
        );
    }

    #[test]
    fn unavailable_hints_sleep_and_keep_polling() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join("runtime")).unwrap();
        fs::write(home.path().join("runtime/wake"), b"blocked").unwrap();
        let mut hints = Hints::open(home.path());
        assert!(!hints.available());
        for _ in 0..2 {
            let started = Instant::now();
            assert!(!hints.wait(Duration::from_millis(10)));
            assert!(started.elapsed() >= Duration::from_millis(10));
        }
    }

    #[test]
    fn failed_wait_drops_listener_and_falls_back() {
        use std::os::fd::OwnedFd;
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("not-a-socket");
        let fd: OwnedFd = fs::File::create(&path).unwrap().into();
        let mut hints = Hints {
            listener: Some(Listener {
                socket: UnixDatagram::from(fd),
                path: path.clone(),
            }),
        };
        assert!(hints.available());
        let started = Instant::now();
        assert!(!hints.wait(Duration::from_millis(10)));
        assert!(started.elapsed() >= Duration::from_millis(10));
        assert!(!hints.available());
        assert!(!path.exists());
        assert!(!hints.wait(Duration::ZERO));
    }

    #[test]
    fn optional_hints_receive_notifications() {
        let home = tempfile::tempdir().unwrap();
        let mut hints = Hints::open(home.path());
        assert!(hints.available());
        notify(home.path());
        assert!(hints.wait(Duration::from_millis(10)));
    }

    #[test]
    fn hints_fan_out_and_cleanup_without_payloads() {
        let home = tempfile::tempdir().unwrap();
        let a = Listener::open(home.path()).unwrap();
        let b = Listener::open(home.path()).unwrap();
        notify(home.path());
        assert!(a.wait(Duration::from_millis(10)).unwrap());
        assert!(b.wait(Duration::from_millis(10)).unwrap());
        assert!(!a.wait(Duration::from_millis(1)).unwrap());
        let path = a.path.clone();
        drop(a);
        assert!(!path.exists());
    }
    #[test]
    fn rejects_symlinked_listener_directory() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join("runtime")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), home.path().join("runtime/wake")).unwrap();
        assert!(Listener::open(home.path()).is_err());
        notify(home.path());
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    }
}
