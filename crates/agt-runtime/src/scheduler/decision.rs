//! Participation inference and its persisted silent/wait/ready transitions.
use crate::participation::{ParticipationDecision, WakeCondition, decide};
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
                RuntimeError::Canceled => "canceled",
                _ => "failed",
            }
            .into();
            task.reason = Some(error.to_string());
            store.update(task, now_ms())?;
            return Ok(());
        }
    };
    let value = serde_json::to_value(&decision)?;
    let reason = value["reason_code"].as_str().expect("enum string");
    task.reason = Some(reason.into());
    task.last_decision = Some(crate::schedule_store::RecordedDecision {
        decision: decision.clone(),
        decided_at: now_ms(),
        through_seq: task.through_seq,
    });
    match decision {
        ParticipationDecision::Silent { .. } => {
            task.state = "silent".into();
            store.update(task, now_ms())?;
            Ok(())
        }
        ParticipationDecision::Wait { wake, .. } => {
            task.state = "waiting".into();
            task.decision_required = true;
            match wake {
                WakeCondition::After { seconds } => {
                    task.quiet = false;
                    task.due_at = now_ms() + i64::from(seconds) * 1000;
                    task.deadline = task.due_at;
                }
                WakeCondition::ConversationQuiet {
                    quiet_seconds,
                    max_wait_seconds,
                } => {
                    task.quiet = true;
                    task.quiet_ms = i64::from(quiet_seconds) * 1000;
                    task.due_at = now_ms() + task.quiet_ms;
                    task.deadline = now_ms() + i64::from(max_wait_seconds) * 1000;
                }
            }
            store.update(task, now_ms())?;
            Ok(())
        }
        ParticipationDecision::Participate { .. } => {
            task.state = "ready_to_reply".into();
            task.due_at = now_ms();
            task.quiet = false;
            store.update(task, now_ms())?;
            Ok(())
        }
    }
}
