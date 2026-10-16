//! Bounded host diagnostics. Never sent as public progress or framework tool results.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    Timeout,
    Disconnected,
    Interrupted,
    EngineFailed,
    ResultMissing,
    Protocol,
}
#[derive(Debug)]
pub struct Failure {
    pub code: FailureCode,
    pub message: String,
}
impl Failure {
    pub(crate) fn new(code: FailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Failure {}
#[derive(Clone, Default)]
pub(crate) struct StderrTail(Arc<Mutex<VecDeque<u8>>>);
impl StderrTail {
    pub fn append(&self, bytes: &[u8]) {
        let mut tail = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for byte in bytes {
            if tail.len() == 8192 {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    pub fn redacted(&self) -> String {
        let bytes: Vec<_> = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .copied()
            .collect();
        String::from_utf8_lossy(&bytes)
            .lines()
            .map(|line| {
                let lower = line.to_ascii_lowercase();
                if [
                    "token",
                    "authorization",
                    "api_key",
                    "api-key",
                    "password",
                    "secret",
                    "bearer",
                    "sk-",
                    "cookie",
                ]
                .iter()
                .any(|key| lower.contains(key))
                {
                    "[sensitive diagnostic line omitted]".to_owned()
                } else {
                    line.chars()
                        .filter(|c| !c.is_control())
                        .take(1000)
                        .collect()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProcessDiagnostics {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stderr_tail: String,
}
