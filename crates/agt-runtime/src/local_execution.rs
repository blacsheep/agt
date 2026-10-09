//! Explicit host execution. A working directory is not a security sandbox.
use agt_core::CancellationToken;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct LocalExecution {
    pub home: PathBuf,
    pub directory: PathBuf,
    pub cancellation: CancellationToken,
    pub task: Option<crate::ScheduledTask>,
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn drain(
    mut reader: impl Read + Send + 'static,
    buffer: Arc<Mutex<(Vec<u8>, bool)>>,
    limit: usize,
    stopped: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let mut output = buffer.lock().unwrap();
                    let accepted = n.min(limit.saturating_sub(output.0.len()));
                    output.0.extend_from_slice(&chunk[..accepted]);
                    output.1 |= accepted < n;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    })
}
#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<()> {
    let fd = pipe.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}
impl agt_service::ExecutionControl for LocalExecution {
    fn call(&self, args: Value) -> Result<Value> {
        let policy = agt_config::RuntimeConfig::load(&self.home)?.execution;
        if !policy.enabled {
            bail!("local execution disabled by workspace owner");
        }
        #[cfg(not(unix))]
        bail!("local execution currently requires Unix process groups");
        let command = args["command"].as_str().context("command required")?;
        if command.trim().is_empty() || command.len() > 65536 {
            bail!("command must contain 1..65536 bytes");
        }
        let directory = args["directory"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or(self.directory.clone());
        if !directory.is_dir() {
            bail!("working directory does not exist");
        }
        let check = || -> crate::Result<()> {
            if self.cancellation.is_canceled() {
                return Err(crate::RuntimeError::Canceled);
            }
            if let Some(task) = &self.task {
                crate::scheduler::guarded(&self.home, task, || Ok(()))?;
            }
            Ok(())
        };
        check()?;
        let spawn = || -> crate::Result<Child> {
            let mut process = Command::new("/bin/bash");
            process
                .args(["--noprofile", "--norc", "-c", command])
                .current_dir(&directory)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                process.process_group(0);
            }
            process.spawn().map_err(|e| {
                crate::RuntimeError::Invalid(format!("cannot start local command: {e}"))
            })
        };
        let child = if let Some(task) = &self.task {
            crate::scheduler::guarded(&self.home, task, spawn)?
        } else {
            spawn()?
        };
        let mut process = Process(child);
        let output = Arc::new(Mutex::new((Vec::new(), false)));
        let stopped = Arc::new(AtomicBool::new(false));
        let stdout_pipe = process.0.stdout.take().unwrap();
        let stderr_pipe = process.0.stderr.take().unwrap();
        #[cfg(unix)]
        {
            nonblocking(&stdout_pipe)?;
            nonblocking(&stderr_pipe)?;
        }
        let stdout = drain(
            stdout_pipe,
            output.clone(),
            policy.output_bytes,
            stopped.clone(),
        );
        let stderr = drain(
            stderr_pipe,
            output.clone(),
            policy.output_bytes,
            stopped.clone(),
        );
        let started = Instant::now();
        let result = (|| -> Result<_> {
            loop {
                check()?;
                if let Some(status) = process.0.try_wait()? {
                    return Ok(status);
                }
                if policy
                    .timeout_seconds
                    .is_some_and(|s| started.elapsed() >= Duration::from_secs(s.into()))
                {
                    bail!(
                        "local command timed out; process group terminated; inspect side effects before retrying"
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })();
        drop(process);
        stopped.store(true, Ordering::Relaxed);
        let _ = stdout.join();
        let _ = stderr.join();
        let status = result?;
        let captured = output.lock().unwrap();
        Ok(
            json!({"exit_code":status.code(),"success":status.success(),"output":String::from_utf8_lossy(&captured.0),"truncated":captured.1,"duration_ms":started.elapsed().as_millis(),"streams":"combined stdout/stderr; order is arrival order"}),
        )
    }
}

#[cfg(all(test, unix))]
mod tests;
