//! Apply workspace auto-allow only to options actually offered by the engine.
use super::*;
impl ApprovalStore {
    pub(crate) fn native_policy(
        &self,
        id: &str,
        config: &agt_config::runtime::PermissionConfig,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        let old = self.poll(id)?;
        if old.decision.is_some() || old.delivery != "not_sent" {
            return Ok(());
        }
        let state = if config.mode == agt_config::runtime::PermissionMode::ModelReview {
            "reviewing"
        } else {
            "awaiting_human"
        };
        let policy = serde_json::to_value(config)?;
        if old.state == state
            && old.request["policy"] == policy
            && old.request["reviewer_fingerprint"] == json!(fingerprint)
        {
            return Ok(());
        }
        let mut next = old.clone();
        next.revision += 1;
        next.state = state.into();
        next.reason.clear();
        next.request["policy"] = policy;
        next.request["reviewer_fingerprint"] = json!(fingerprint);
        match self.update(&old, &next) {
            Err(_) if self.raw(id)?.decision.is_some() => Ok(()),
            result => result,
        }
    }

    pub(crate) fn apply_harness_mode(&self, item: Approval) -> Result<Approval> {
        if item.source != "harness"
            || item.decision.is_some()
            || item.delivery != "not_sent"
            || !matches!(item.state.as_str(), "reviewing" | "awaiting_human")
        {
            return Ok(item);
        }
        let config = agt_config::runtime::RuntimeConfig::load(&self.home)?;
        if config.permissions.harness_mode != agt_config::runtime::HarnessPermissionMode::AutoAllow
        {
            return Ok(item);
        }
        let options = item.request["options"].as_array();
        let decision = options
            .and_then(|options| {
                options.iter().find(|o| {
                    o["value"] == "accept"
                        || (item.request["method"] == "item/permissions/requestApproval"
                            && item.request["params"]["permissions"].is_object()
                            && o["value"]["scope"] == "turn"
                            && o["value"]["permissions"] == item.request["params"]["permissions"])
                })
            })
            .map(|o| o["value"].clone());
        let Some(decision) = decision else {
            return Ok(item);
        };
        // Preserve the same revision, run and scheduler checks as a human decision.
        match self.decide_as(
            &item.id,
            item.revision,
            decision,
            "工作空间模式：自动放行",
            None,
            Some("auto_allow"),
        ) {
            Ok(updated) => Ok(updated),
            Err(error) => {
                let current = self.poll(&item.id)?;
                if current.decision.is_some() {
                    Ok(current)
                } else {
                    Err(error)
                }
            }
        }
    }
}
