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
pub trait ApprovalHandler {
    fn request(&mut self, request: &ApprovalRequest) -> Result<String>;
    fn poll(&mut self, id: &str) -> Result<Option<Value>>;
    fn delivery(&mut self, id: &str, state: &str) -> Result<()>;
}
