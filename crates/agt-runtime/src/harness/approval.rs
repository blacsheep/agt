use super::*;
use crate::approval::{ApprovalStore, Scope};
pub(super) struct Bridge<'a> {
    pub store: ApprovalStore,
    pub scope: Scope,
    pub check: &'a dyn Fn() -> Result<()>,
}
impl agt_harness::ApprovalHandler for Bridge<'_> {
    fn request(&mut self, request: &agt_harness::ApprovalRequest) -> anyhow::Result<String> {
        (self.check)()?;
        let key = format!("codex:{}", request.rpc_id);
        Ok(self
            .store
            .create(
                &self.scope,
                "harness",
                &key,
                serde_json::to_value(request)?,
                600,
            )?
            .id)
    }
    fn poll(&mut self, id: &str) -> anyhow::Result<Option<Value>> {
        (self.check)()?;
        let item = self.store.poll(id)?;
        if matches!(
            item.delivery.as_str(),
            "invalidated" | "unknown" | "sent" | "resolved"
        ) || item.state == "expired"
        {
            anyhow::bail!("approval expired or invalidated")
        }
        Ok(self.store.apply_harness_mode(item)?.decision)
    }
    fn delivery(&mut self, id: &str, state: &str) -> anyhow::Result<()> {
        (self.check)()?;
        self.store.change(id, None, Some(state))
    }
}
impl Drop for Bridge<'_> {
    fn drop(&mut self) {
        let _ = self.store.invalidate_run(&self.scope.run_id);
    }
}
