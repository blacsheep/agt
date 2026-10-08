//! Execution engine adapters and reusable harness definitions. No IM or scheduling policy.
mod approval;
mod codex;
pub use approval::{ApprovalHandler, ApprovalRequest};
mod diagnostics;
pub use diagnostics::{Failure, FailureCode, ProcessDiagnostics};
mod resource;
pub use codex::{Codex, Event, Execution, Tool};
pub use resource::{HarnessConfig, HarnessStore, Sandbox, StoredHarness};
