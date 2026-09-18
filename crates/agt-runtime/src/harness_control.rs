//! Fixed-identity bridge shared by hosted tools, local CLI and stdio MCP.
use agt_service::{Actor, HarnessControl};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
pub struct SessionControl {
    home: PathBuf,
    actor: Actor,
    task: Option<crate::ScheduledTask>,
}
impl SessionControl {
    pub fn with_task(mut self, task: Option<crate::ScheduledTask>) -> Self {
        self.task = task;
        self
    }

    pub fn new(home: &Path, actor: Actor) -> Self {
        Self {
            home: home.into(),
            actor,
            task: None,
        }
    }
}
impl HarnessControl for SessionControl {
    fn call(&self, name: &str, args: Value) -> Result<Value> {
        if let Some(task) = &self.task {
            crate::scheduler::guarded(&self.home, task, || Ok(()))?;
        }
        let im = agt_im::ImStore::open(agt_storage::database_path(&self.home, "im", "im.db")?)?;
        if im
            .get_principal(&self.actor.principal)?
            .deleted_at
            .is_some()
        {
            bail!("identity deleted");
        }
        if name == "harness_models" {
            let (category, name) =
                agt_service::named_ref(args["reference"].as_str().context("reference required")?)?;
            let resource = agt_harness::HarnessStore::new(&self.home).read_in(&category, &name)?;
            return agt_harness::Codex::models(&resource.config);
        }
        let conversation = args["conversation"]
            .as_str()
            .context("conversation required")?;
        let target = args["target"].as_str().context("target required")?;
        let chat = agt_im::ConversationId::new(conversation);
        if im.get_conversation(&chat)?.state != agt_im::ConversationState::Active {
            bail!("conversation is not active");
        }
        for id in [self.actor.principal.as_str(), target] {
            let principal = agt_im::PrincipalId::new(id);
            if im.get_principal(&principal)?.deleted_at.is_some()
                || im.get_member(&chat, &principal)?.left_message_seq.is_some()
            {
                bail!("current membership required");
            }
        }
        let sessions = crate::HarnessSessions::open(&self.home)?;
        let session = sessions.current(&self.home, target, conversation)?;
        if name != "harness_session_get" && args["session"].as_str() != Some(&session.id) {
            bail!("session changed; read current session before retrying");
        }
        let session = match name {
            "harness_session_get" => session,
            "harness_session_models" => return agt_harness::Codex::models(&session.config),
            "harness_session_configure" => sessions.configure_for_task(
                &session.id,
                args["model"].as_str().context("model required")?,
                args["effort"].as_str(),
                self.task.as_ref(),
            )?,
            _ => bail!("unknown harness operation"),
        };
        Ok(
            json!({"id":session.id,"session_id":session.session_id,"harness":session.harness,"principal":session.principal,"conversation":session.conversation,"model":session.config.model,"reasoning_effort":session.config.reasoning_effort,"busy":session.busy,"execution":session.execution}),
        )
    }
}
