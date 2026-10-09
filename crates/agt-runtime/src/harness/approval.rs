use super::*;
use crate::approval::{ApprovalStore, Scope};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The engine polls between protocol frames (about every 50 ms). Storage is consulted
/// only after a wake hint or this fallback, which still renews leases and observes expiry.
const RECHECK: Duration = Duration::from_secs(1);

pub(super) struct Bridge<'a> {
    pub store: ApprovalStore,
    pub scope: Scope,
    pub check: &'a dyn Fn() -> Result<()>,
    pub wake: Wake,
}

/// Tracks when each pending approval last read storage and the latest wake hint.
pub(super) struct Wake {
    listener: Option<agt_storage::wake::Listener>,
    hinted_at: Instant,
    checked: HashMap<String, Instant>,
}
impl Wake {
    pub(super) fn open(home: &std::path::Path) -> Self {
        Self {
            listener: agt_storage::wake::Listener::open(home).ok(),
            hinted_at: Instant::now(),
            checked: HashMap::new(),
        }
    }
    fn due(&mut self, id: &str) -> bool {
        // Without a listener every poll reads storage, as before.
        let Some(listener) = &self.listener else {
            return true;
        };
        if listener.wait(Duration::ZERO).unwrap_or(true) {
            self.hinted_at = Instant::now();
        }
        let due = self
            .checked
            .get(id)
            .is_none_or(|at| *at <= self.hinted_at || at.elapsed() >= RECHECK);
        if due {
            self.checked.insert(id.to_owned(), Instant::now());
        }
        due
    }
}
impl agt_harness::ApprovalHandler for Bridge<'_> {
    fn request(&mut self, request: &agt_harness::ApprovalRequest) -> anyhow::Result<String> {
        (self.check)()?;
        let key = format!("codex:{}", request.rpc_id);
        Ok(self
            .store
            .create(
                &self.scope,
                crate::approval::ApprovalSource::Harness,
                &key,
                serde_json::to_value(request)?,
                600,
            )?
            .id)
    }
    fn poll(&mut self, id: &str) -> anyhow::Result<Option<Value>> {
        if !self.wake.due(id) {
            return Ok(None);
        }
        (self.check)()?;
        let item = self.store.poll(id)?;
        if item.delivery.is_final()
            || item.delivery == crate::approval::Delivery::Sent
            || item.state == crate::approval::ApprovalState::Expired
        {
            anyhow::bail!("approval expired or invalidated")
        }
        Ok(self.store.apply_harness_mode(item)?.decision)
    }
    fn delivery(
        &mut self,
        id: &str,
        progress: agt_harness::DeliveryProgress,
    ) -> anyhow::Result<()> {
        (self.check)()?;
        self.store.change(id, None, Some(progress.into()))
    }
}
impl Drop for Bridge<'_> {
    fn drop(&mut self) {
        let _ = self.store.invalidate_run(&self.scope.run_id);
    }
}
