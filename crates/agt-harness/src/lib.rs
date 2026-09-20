//! Execution engine adapters and reusable harness definitions. No IM or scheduling policy.
mod codex;
mod diagnostics;
pub use diagnostics::{Failure, FailureCode, ProcessDiagnostics};
mod resource;
pub use codex::{Codex, Event, Execution, Tool};
pub use resource::{HarnessConfig, HarnessStore, Sandbox, StoredHarness};
