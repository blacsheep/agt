//! Model I/O for one execution: resolved agent configuration, model protocols and
//! streaming, context budgets and compaction. No persistence or scheduling here.
pub mod context;
mod error;
pub mod model;
mod model_stream;
mod resolve;

pub use context::ContextCheckpoint;
pub use error::{Reason, Result, RuntimeError, StopCode, canonical_agent, fingerprint};
pub use model::{ChatCompletions, ModelClient, ModelEvent};
pub use resolve::ResolvedAgent;
