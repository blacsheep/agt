//! Process lifetime, framed transport and request/response RPC.
use super::*;
use crate::diagnostics::StderrTail;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
pub(super) struct Transport {
    child: Child,
    input: ChildStdin,
    messages: Receiver<Result<Value>>,
    pending: Option<Value>,
    pub(super) next_id: u64,
    pub(super) deadline: Instant,
    stderr: StderrTail,
}
impl Transport {
    pub(super) fn diagnostics(&mut self) -> ProcessDiagnostics {
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

    pub(super) fn connect(
        harness: &HarnessConfig,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Self> {
        harness.validate()?;
        let mut command = Command::new(&harness.executable);
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&harness.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().context("cannot start Codex app-server")?;
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
                    .take(4 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) => break,
                    Ok(_) if line.len() > 4 * 1024 * 1024 => {
                        Err(anyhow::anyhow!("Codex protocol frame exceeds 4 MiB"))
                    }
                    Ok(_) => serde_json::from_slice(&line).context("invalid Codex protocol frame"),
                    Err(e) => Err(e.into()),
                };
                let failed = result.is_err();
                if sender.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            messages,
            pending: None,
            next_id: 0,
            stderr: stderr_tail,
            deadline: Instant::now() + Duration::from_secs(15),
        };
        client.request("initialize", json!({"clientInfo":{"name":"agt","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}), event)?;
        client.send(json!({"method":"initialized","params":{}}))?;
        client.deadline = Instant::now() + Duration::from_secs(harness.timeout_seconds);
        Ok(client)
    }
    pub(super) fn send(&mut self, value: Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, &value)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    pub(super) fn receive(
        &mut self,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        loop {
            if let Some(value) = self.poll(event)? {
                return Ok(value);
            }
        }
    }
    pub(super) fn poll(
        &mut self,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Option<Value>> {
        event(Event::Tick)?;
        if Instant::now() >= self.deadline {
            return Err(Failure::new(FailureCode::Timeout, "Codex execution timed out").into());
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
            Err(_) => {
                Err(Failure::new(FailureCode::Disconnected, "Codex app-server disconnected").into())
            }
        }
    }
    pub(super) fn request(
        &mut self,
        method: &str,
        params: Value,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            let value = self.receive(event)?;
            if value["id"] == id && value.get("method").is_none() {
                if value.get("error").is_some() {
                    bail!(
                        "Codex {method} rejected: {}",
                        value["error"]["message"]
                            .as_str()
                            .unwrap_or("protocol error")
                    );
                }
                return Ok(value["result"].clone());
            }
            if value.get("id").is_some() {
                bail!("Codex requested unsupported interaction during {method}");
            }
        }
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        // Kill the local process group as well as the app-server; tools may own child processes.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
