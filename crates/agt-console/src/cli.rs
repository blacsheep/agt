//! The console acts through the same `agt` CLI a person would use, one JSON
//! command per operation, so it has no write path of its own.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use super::error::ApiError;

pub(crate) type Reply<'a> = Pin<Box<dyn Future<Output = Result<Value, ApiError>> + Send + 'a>>;

/// Runs one CLI operation: `args` after `--home <home> --output json`, plus
/// extra environment variables for the child only.
pub(crate) trait Backend: Send + Sync {
    fn call(&self, args: Vec<String>, env: Vec<(String, String)>) -> Reply<'_>;
}

const MAX_OUTPUT: u64 = 8 * 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const COMPACT_TIMEOUT: Duration = Duration::from_secs(86_400);

pub(crate) struct Cli {
    binary: PathBuf,
    /// Arguments before agt's own, for a host binary that also acts as the CLI.
    prefix: Vec<String>,
    home: PathBuf,
    scheduler: Mutex<Option<Child>>,
}

impl Cli {
    pub fn new(binary: PathBuf, home: PathBuf) -> Arc<Self> {
        Self::with_prefix(binary, Vec::new(), home)
    }

    pub fn with_prefix(binary: PathBuf, prefix: Vec<String>, home: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            binary,
            prefix,
            home,
            scheduler: Mutex::new(None),
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args(&self.prefix)
            .arg("--home")
            .arg(&self.home)
            .stdin(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    }

    async fn run(&self, args: Vec<String>, env: Vec<(String, String)>) -> Result<Value, ApiError> {
        let compact = args.len() >= 3 && args[..3] == ["harness", "session", "compact"];
        let mut command = self.command();
        command
            .args(["--output", "json"])
            .args(&args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| cli_failed())?;
        let mut stdout = child.stdout.take().ok_or_else(cli_failed)?;
        let work = async {
            let mut bytes = Vec::new();
            (&mut stdout)
                .take(MAX_OUTPUT + 1)
                .read_to_end(&mut bytes)
                .await?;
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((bytes, status))
        };
        let limit = if compact {
            COMPACT_TIMEOUT
        } else {
            CALL_TIMEOUT
        };
        // On timeout the child is dropped, which kills it.
        let Ok(Ok((bytes, status))) = tokio::time::timeout(limit, work).await else {
            return Err(cli_failed());
        };
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err(cli_failed());
        }
        // Never echo raw stderr or unparsed output: it may contain configuration.
        let value: Option<Value> = serde_json::from_slice(&bytes).ok();
        if !status.success() {
            let safe = value.as_ref().and_then(|v| v.get("error"));
            let text = |key| safe.and_then(|e| e.get(key)).and_then(Value::as_str);
            let mut error = cli_failed();
            if let Some(code) = text("code") {
                error.code = code.to_owned();
            }
            if let Some(message) = text("message") {
                error.message = message.to_owned();
            }
            if let Some(field) = safe.and_then(|e| e.get("field")) {
                error.field = Some(field.clone());
            }
            return Err(error);
        }
        value.ok_or_else(|| ApiError::new(502, "invalid_response", "CLI 返回了无法解析的响应。"))
    }

    /// One operation whose extra environment reaches only that child process.
    pub async fn call_with_env(
        &self,
        args: Vec<String>,
        env: Vec<(String, String)>,
    ) -> Result<Value, ApiError> {
        self.run(args, env).await
    }

    /// Starts `schedule run --watch` and waits until the scheduler reports
    /// online. A standby coordinator takes over when an old process exits or
    /// its lease expires, so a stale online snapshot cannot block startup.
    pub async fn start_scheduler(&self) -> Result<(), ApiError> {
        let mut child = self
            .command()
            .args(["--output", "json", "schedule", "run", "--watch"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| scheduler_offline())?;
        for _ in 0..50 {
            let status = match self.run(strings(&["schedule", "status"]), Vec::new()).await {
                Ok(status) => status,
                Err(error) => {
                    stop(&mut child).await;
                    return Err(error);
                }
            };
            if status.get("online").and_then(Value::as_bool) == Some(true) {
                *self.scheduler.lock().expect("scheduler lock") = Some(child);
                return Ok(());
            }
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        stop(&mut child).await;
        Err(scheduler_offline())
    }

    /// `events watch` for one identity, as JSON lines on the child's stdout.
    pub fn watch_events(&self, principal: &str, after: Option<&str>) -> std::io::Result<Child> {
        let mut command = self.command();
        command
            .args(["events", "watch", "--principal", principal])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(after) = after {
            command.args(["--after", after]);
        }
        command.spawn()
    }

    /// Stops the scheduler this console started, letting it finish cleanly.
    pub async fn close(&self) {
        let child = self.scheduler.lock().expect("scheduler lock").take();
        if let Some(mut child) = child {
            stop(&mut child).await;
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }
}

impl Backend for Cli {
    fn call(&self, args: Vec<String>, env: Vec<(String, String)>) -> Reply<'_> {
        Box::pin(self.run(args, env))
    }
}

/// Asks a child to stop (SIGTERM on Unix), then kills it if it lingers.
pub(crate) async fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id().and_then(|id| i32::try_from(id).ok()) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        if tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .is_ok()
        {
            return;
        }
    }
    let _ = child.kill().await;
}

fn cli_failed() -> ApiError {
    ApiError::new(
        400,
        "cli_failed",
        "操作未完成。请检查资源配置、权限或命令超时；重试前先刷新当前状态。",
    )
}

fn scheduler_offline() -> ApiError {
    ApiError::new(
        503,
        "scheduler_offline",
        "回复服务未能启动，请检查调度进程状态和数据目录权限。",
    )
}

pub(crate) fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// CLI operations as the console's fixed operator, or as an observed identity.
#[derive(Clone, Copy)]
pub(crate) struct Ops<'a> {
    pub backend: &'a dyn Backend,
    pub operator: &'a str,
}

impl Ops<'_> {
    pub async fn run(&self, args: Vec<String>) -> Result<Value, ApiError> {
        self.backend.call(args, Vec::new()).await
    }

    /// `tool call` as `viewer`; only the operator acts with manager rights.
    pub async fn tool(&self, name: &str, args: &Value, viewer: &str) -> Result<Value, ApiError> {
        let mut command = strings(&["tool", "call", "--principal", viewer]);
        if viewer == self.operator {
            command.push("--manager".into());
        }
        command.extend([name.to_owned(), "--args".into(), args.to_string()]);
        self.run(command).await
    }

    pub async fn members(&self, conversation: &str, viewer: &str) -> Result<Value, ApiError> {
        self.tool(
            "member_list",
            &serde_json::json!({ "conversation": conversation }),
            viewer,
        )
        .await
    }
}

/// Whether `principal` is a current (not departed) member in a `member_list` result.
pub(crate) fn is_member(members: &Value, principal: &str) -> bool {
    members.as_array().is_some_and(|list| {
        list.iter().any(|m| {
            super::js::is(m.get("principal_id"), principal)
                && super::js::nullish(m.get("left_message_seq"))
        })
    })
}
