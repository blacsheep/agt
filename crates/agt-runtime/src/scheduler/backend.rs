//! Engine-specific scheduling preparation. The coordinator and settlement stay shared.
use super::decision;
use crate::TaskState;
use crate::engine::EngineKind;
use crate::{
    ModelClient, ResolvedAgent, RuntimeError, ScheduleStore, ScheduledTask, schedule_store::now_ms,
};
use agt_core::CancellationToken;
use agt_im::ImStore;
use agt_service::ResponsePolicy;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn response_policy(
    engine: EngineKind,
    policy: ResponsePolicy,
    peer: bool,
) -> ResponsePolicy {
    if engine == EngineKind::Harness && policy != ResponsePolicy::Manual {
        ResponsePolicy::Mentioned
    } else if peer && policy != ResponsePolicy::Manual {
        ResponsePolicy::All
    } else {
        policy
    }
}

/// Harness only executes direct/mentioned messages; it does not use native triage.
pub(super) fn admit_participation(
    engine: EngineKind,
    store: &mut ScheduleStore,
    task: &mut ScheduledTask,
) -> Result<bool> {
    if engine == EngineKind::Harness && task.state == TaskState::Evaluating {
        task.ready_external_reply();
        store.update(task, now_ms())?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) enum ScheduledEngine {
    Native(Box<ResolvedAgent>),
    Harness,
}

impl ScheduledEngine {
    /// Resolve native model configuration only after admission, as before.
    pub(super) fn load(home: &Path, name: &str, kind: EngineKind) -> Result<Self> {
        Ok(match kind {
            EngineKind::Native => Self::Native(Box::new(ResolvedAgent::load(home, name)?)),
            EngineKind::Harness => Self::Harness,
        })
    }

    pub(super) fn triage(
        &self,
        home: &Path,
        task: &ScheduledTask,
        model: &mut dyn ModelClient,
    ) -> Result<bool> {
        match self {
            Self::Native(agent) => crate::work::triage::run(home, task, agent, model),
            Self::Harness => Ok(false),
        }
    }

    pub(super) fn prepare_context(
        &self,
        home: &Path,
        store: &mut ScheduleStore,
        task: &mut ScheduledTask,
        im: &ImStore,
        model: &mut dyn ModelClient,
        token: &CancellationToken,
    ) -> Result<Option<Value>> {
        match self {
            Self::Harness => {
                let cutoff = store.context_cutoff(&task.conversation)?;
                Ok(Some(
                    json!({"history":crate::conversation_context::visible_history(im,task,cutoff)?,"summary":"","reset_through_seq":cutoff}),
                ))
            }
            Self::Native(agent) => {
                match crate::conversation_context::prepare(home, im, task, agent, model, token) {
                    Ok(context) => Ok(Some(context)),
                    Err(error) => {
                        task.state =
                            if matches!(error, RuntimeError::Canceled | RuntimeError::Stopped(_)) {
                                TaskState::Canceled
                            } else {
                                TaskState::Failed
                            };
                        task.reason = Some(format!("conversation context: {error}"));
                        store.update(task, now_ms())?;
                        Ok(None)
                    }
                }
            }
        }
    }

    pub(super) fn input_history(&self, context: &Value, task: &ScheduledTask) -> Value {
        match self {
            Self::Harness => context["history"].clone(),
            Self::Native(_) => json!(
                context["history"]
                    .as_array()
                    .unwrap_or(&Vec::new())
                    .iter()
                    .filter(|m| !task
                        .triggers
                        .iter()
                        .any(|id| m["message_id"] == id.as_str()))
                    .collect::<Vec<_>>()
            ),
        }
    }

    pub(super) fn evaluate(
        &self,
        store: &mut ScheduleStore,
        task: &mut ScheduledTask,
        model: &mut dyn ModelClient,
        input: &str,
        active: &[String],
    ) -> Result<()> {
        match self {
            Self::Native(agent) => decision::evaluate(store, task, model, agent, input, active),
            Self::Harness => unreachable!("Harness participation is resolved during admission"),
        }
    }

    pub(super) fn validate_input(&self, input: &str) -> Result<()> {
        if let Self::Native(agent) = self
            && crate::context::estimate(input) + crate::context::estimate(&agent.system_prompt)
                > crate::context::input_budget(agent)
        {
            bail!(
                "conversation input exceeds context budget; shorten the message batch or increase model context_window"
            );
        }
        Ok(())
    }
}
