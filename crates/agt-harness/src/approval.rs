//! Host bridge for engine-owned approvals. No policy decisions are made here.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub rpc_id: Value,
    pub method: String,
    pub params: Value,
    pub options: Vec<Value>,
    /// Same-turn item details when supplied by the engine; never synthesized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_context: Option<Value>,
}
/// Engine-observed progress of a decision response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryProgress {
    /// About to write the response; a crash after this leaves the outcome unknown.
    Sending,
    Sent,
    /// The engine confirmed the request is no longer pending.
    Resolved,
}
pub trait ApprovalHandler {
    fn request(&mut self, request: &ApprovalRequest) -> Result<String>;
    fn poll(&mut self, id: &str) -> Result<Option<Value>>;
    fn delivery(&mut self, id: &str, progress: DeliveryProgress) -> Result<()>;
}
