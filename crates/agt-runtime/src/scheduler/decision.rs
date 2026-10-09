//! Participation inference and its persisted silent/wait/ready transitions.
use crate::TaskState;
use crate::participation::decide;
use crate::{
    ModelClient, ResolvedAgent, RuntimeError, ScheduleStore, ScheduledTask, schedule_store::now_ms,
};
use anyhow::Result;

pub(super) fn evaluate(
    store: &mut ScheduleStore,
    task: &mut ScheduledTask,
    model: &mut dyn ModelClient,
    agent: &ResolvedAgent,
    input: &str,
) -> Result<()> {
    let result = decide(model, agent, input);
    let decision = match result {
        Ok(value) => value,
        Err(error) => {
            task.state = match error {
                RuntimeError::Canceled => TaskState::Canceled,
                _ => TaskState::Failed,
            };
            task.reason = Some(error.to_string());
            store.update(task, now_ms())?;
            return Ok(());
        }
    };
    task.finish_scheduled_decision(&decision)?;
    store.update(task, now_ms())?;
    Ok(())
}
