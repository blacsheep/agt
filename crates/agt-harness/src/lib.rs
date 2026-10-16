//! Execution engine adapters and reusable harness definitions. No IM or scheduling policy.
mod approval;
mod claude;
mod codex;
mod engine;
pub use approval::{ApprovalHandler, ApprovalRequest, DeliveryProgress};
pub use engine::{
    CLAUDE_CODE, CODEX, ENGINES, Engine, capabilities, compact, connect, inspect, models, rename,
    test,
};
mod diagnostics;
pub use diagnostics::{Failure, FailureCode, ProcessDiagnostics};
mod process;
mod resource;
pub use codex::{Codex, Event, Execution, Tool};
pub use resource::{HarnessConfig, HarnessStore, Sandbox, StoredHarness};
