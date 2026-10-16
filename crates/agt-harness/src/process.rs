//! A local engine process that speaks newline-delimited JSON on stdin/stdout.
//! Process lifetime, bounded frames, deadlines and diagnostics; no protocol.
use crate::diagnostics::StderrTail;
use crate::{Event, Failure, FailureCode, ProcessDiagnostics};
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const MAX_FRAME: usize = 4 * 1024 * 1024;

pub(crate) struct Lines {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<Result<Value>>,
    pending: Option<Value>,
    pub(crate) deadline: Instant,
    stderr: StderrTail,
    /// Engine name for errors, such as "Codex app-server".
    name: &'static str,
}

impl Lines {
    pub(crate) fn spawn(mut command: Command, name: &'static str) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("cannot start {name}"))?;
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let stderr_tail = StderrTail::default();
        let tail = stderr_tail.clone();
        std::thread::spawn(move || {
            let mut bytes = [0; 2048];
            while let Ok(n) = stderr.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                tail.append(&bytes[..n]);
            }
        });
        let (sender, messages) = mpsc::sync_channel(64);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let result = match reader
                    .by_ref()
                    .take(MAX_FRAME as u64 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) => break,
                    Ok(_) if line.len() > MAX_FRAME => {
                        Err(anyhow::anyhow!("{name} protocol frame exceeds 4 MiB"))
                    }
                    // Engines may print blank lines between frames.
                    Ok(_) if line.iter().all(u8::is_ascii_whitespace) => continue,
                    Ok(_) => serde_json::from_slice(&line)
                        .with_context(|| format!("invalid {name} protocol frame")),
                    Err(e) => Err(e.into()),
                };
                let failed = result.is_err();
                if sender.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            messages,
            pending: None,
            deadline: Instant::now() + Duration::from_secs(15),
            stderr: stderr_tail,
            name,
        })
    }

    pub(crate) fn diagnostics(&mut self) -> ProcessDiagnostics {
        let status = self.child.try_wait().ok().flatten();
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.as_ref().and_then(|s| s.signal())
        };
        #[cfg(not(unix))]
        let signal = None;
        ProcessDiagnostics {
            exit_code: status.and_then(|s| s.code()),
            signal,
            stderr_tail: self.stderr.redacted(),
        }
    }

    pub(crate) fn send(&mut self, value: &Value) -> Result<()> {
        let input = self
            .input
            .as_mut()
            .with_context(|| format!("{} input is closed", self.name))?;
        serde_json::to_writer(&mut *input, value)?;
        input.write_all(b"\n")?;
        input.flush()?;
        Ok(())
    }

    /// Signal end of input; engines that run one turn per process then exit.
    pub(crate) fn close_input(&mut self) {
        self.input = None;
    }

    pub(crate) fn receive(
        &mut self,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        loop {
            if let Some(value) = self.poll(event)? {
                return Ok(value);
            }
        }
    }

    /// One frame if one arrives within 50 ms. Reports a tick and every frame to
    /// `event` so the host can cancel; a frame received during cancellation is
    /// kept for the next poll.
    pub(crate) fn poll(
        &mut self,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Option<Value>> {
        event(Event::Tick)?;
        if Instant::now() >= self.deadline {
            return Err(Failure::new(
                FailureCode::Timeout,
                format!("{} execution timed out", self.name),
            )
            .into());
        }
        let received = if let Some(value) = self.pending.take() {
            Ok(Ok(value))
        } else {
            self.messages.recv_timeout(Duration::from_millis(50))
        };
        match received {
            Ok(value) => {
                if let Ok(frame) = &value
                    && let Err(error) = event(Event::Protocol)
                {
                    // Cancellation must not discard an already received start/terminal frame.
                    self.pending = Some(frame.clone());
                    return Err(error);
                }
                value.map(Some)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(Failure::new(
                FailureCode::Disconnected,
                format!("{} disconnected", self.name),
            )
            .into()),
        }
    }
}

impl Drop for Lines {
    fn drop(&mut self) {
        // Kill the local process group as well as the engine; tools may own child processes.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
