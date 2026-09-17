//! Best-effort local wake hints. Call only after commit; durable stores remain authoritative.
#[cfg(unix)]
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::UnixDatagram,
    },
    sync::atomic::{AtomicU64, Ordering},
};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
static NEXT: AtomicU64 = AtomicU64::new(0);

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
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            Ok(Self { socket, path })
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
