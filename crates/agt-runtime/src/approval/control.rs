use super::*;
use agt_im::{ConversationId, PrincipalId, PrincipalKind};
impl ApprovalStore {
    pub(super) fn visible(
        &self,
        item: &Approval,
        viewer: Option<&str>,
        write: bool,
    ) -> Result<bool> {
        let Some(viewer) = viewer else {
            return Ok(true);
        }; // local CLI administrator
        let im = agt_im::ImStore::open(agt_storage::database_path(&self.home, "im", "im.db")?)?;
        let actor = im.get_principal(&PrincipalId::new(viewer))?;
        if actor.deleted_at.is_some() || (write && actor.kind != PrincipalKind::User) {
            return Ok(false);
        }
        if write && im.workspace_owner()?.as_ref() != Some(&actor.id) {
            return Ok(false);
        }
        let Some(chat) = &item.conversation else {
            return Ok(false);
        };
        if !im
            .get_member(&ConversationId::new(chat), &actor.id)
            .is_ok_and(|m| m.left_message_seq.is_none())
        {
            return Ok(false);
        }
        if actor.kind == PrincipalKind::User || item.principal.as_deref() == Some(viewer) {
            return Ok(true);
        }
        Ok(item.work_id.as_ref().is_some_and(|id| {
            crate::work::WorkLedger::open(&self.home)
                .and_then(|l| l.get(id))
                .is_ok_and(|w| w.request.requester == viewer)
        }))
    }
    pub fn list(&self, viewer: Option<&str>, before: Option<&str>) -> Result<Vec<Approval>> {
        self.reconcile()?;
        let mut result = Vec::new();
        let mut reached = before.is_none();
        for item in self.all()? {
            if !reached {
                reached = Some(item.id.as_str()) == before;
                continue;
            }
            if self.visible(&item, viewer, false)? {
                result.push(item);
                if result.len() == 100 {
                    break;
                }
            }
        }
        Ok(result)
    }
    pub fn get(&self, id: &str, viewer: Option<&str>) -> Result<Approval> {
        self.reconcile()?;
        let item = self.raw(id)?;
        if !self.visible(&item, viewer, false)? {
            bail!("approval is not visible")
        };
        Ok(item)
    }
    pub fn decide(
        &self,
        id: &str,
        revision: i64,
        decision: Value,
        reason: &str,
        viewer: Option<&str>,
    ) -> Result<Approval> {
        self.decide_as(id, revision, decision, reason, viewer, None)
    }
    pub(crate) fn decide_as(
        &self,
        id: &str,
        revision: i64,
        decision: Value,
        reason: &str,
        viewer: Option<&str>,
        origin: Option<DecisionOrigin>,
    ) -> Result<Approval> {
        self.reconcile()?;
        let old = self.raw(id)?;
        if !self.visible(&old, viewer, true)? {
            bail!("human approval requires an authorized human operator")
        }
        if old.revision != revision
            || old.decision.is_some()
            || !old.state.is_pending()
            || old.delivery != Delivery::NotSent
        {
            bail!("approval is no longer pending; refresh")
        }
        let scope: String =
            self.db
                .query_row("SELECT scope FROM approvals WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
        let scope: Scope = serde_json::from_str(&scope)?;
        if let Some(task) = scope.task {
            crate::scheduler::guarded(&self.home, &task, || Ok(()))?;
        }
        let run = crate::RunStore::open(&self.home)?.get(&old.run_id)?;
        if run.state != crate::RunState::Running {
            bail!("execution is no longer running")
        }
        let options = old.request["options"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing approval choices"))?;
        if !options.iter().any(|o| o["value"] == decision) {
            bail!("decision is not an offered option")
        }
        if reason.len() > 2000 {
            bail!("reason exceeds 2000 bytes")
        }
        let mut next = old.clone();
        next.revision += 1;
        next.decision = Some(decision.clone());
        next.reason = reason.into();
        next.actor = Some(
            origin
                .map(DecisionOrigin::as_str)
                .or(viewer)
                .unwrap_or("local-admin")
                .into(),
        );
        next.decision_origin = Some(origin.unwrap_or(if viewer.is_some() {
            DecisionOrigin::Human
        } else {
            DecisionOrigin::LocalAdmin
        }));
        next.state = if decision == "cancel" {
            ApprovalState::Canceled
        } else if decision == "deny"
            || decision == "decline"
            || decision.get("permissions") == Some(&json!({}))
        {
            ApprovalState::Denied
        } else {
            ApprovalState::Approved
        };
        let tx = agt_storage::sqlite::write(&self.db)?;
        self.update(&old, &next)?;
        if decision == "allow_always" {
            // Saved with the decision, so the rule exists exactly when the choice does.
            let (Some(agent), Some(tool)) =
                (old.request["agent"].as_str(), old.request["tool"].as_str())
            else {
                bail!("this approval cannot save a rule");
            };
            self.db.execute(
                "INSERT OR IGNORE INTO permission_rules(agent,tool,actor,created_at) VALUES(?1,?2,?3,?4)",
                params![agent, tool, next.actor.as_deref().unwrap_or("local-admin"), now()],
            )?;
        }
        tx.commit()?;
        // `update` hinted inside this transaction; hint again now that it is visible.
        agt_storage::wake::notify(&self.home);
        Ok(next)
    }
}
