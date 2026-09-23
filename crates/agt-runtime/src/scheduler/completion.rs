//! Persist Run outcomes and publish replies under the existing execution fence.
use super::guarded;
use crate::{RunState, RuntimeError, ScheduleStore, ScheduledTask, schedule_store::now_ms};
use agt_core::CancellationToken;
use agt_im::{ConversationId, MessageId, PrincipalId};
use agt_service::{Actor, MessageInput, Messaging, Role};
use anyhow::Result;
use std::path::Path;

pub(super) fn settle(
    home: &Path,
    store: &mut ScheduleStore,
    task: &mut ScheduledTask,
    record: crate::RunRecord,
    messaging: &Messaging,
    scope: (PrincipalId, ConversationId),
    token: &CancellationToken,
) -> Result<()> {
    let (principal, conversation) = scope;
    task.run_id = Some(record.id.clone());
    if task.work_id.is_some() && !task.work_notification {
        task.state = match record.state {
            RunState::Completed => "completed",
            RunState::Suspended => "suspended",
            RunState::Running | RunState::Interrupted => "interrupted",
            RunState::Canceled => "canceled",
            RunState::Failed => "failed",
            RunState::OutcomeUnknown => "outcome_unknown",
        }
        .into();
        task.reason = record.error.clone();
        store.update(task, now_ms())?;
        return Ok(());
    }
    if record.state == RunState::Completed {
        let output = record.output.as_deref().unwrap_or("");
        if token.is_canceled() {
            task.state = "canceled".into();
            store.update(task, now_ms())?;
            return Ok(());
        }
        if output.trim() != "[[silent]]" {
            // Lease fencing precedes publication; no stale task may publish.
            if !store.update(task, now_ms())? {
                return Ok(());
            }
            let reply_message =
                messaging
                    .im
                    .get_message(&MessageId::new(crate::run_inputs::reply_target(
                        store, task, &record.id,
                    )?))?;
            let reply_depth = reply_message.metadata["automated_depth"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(1)
                .min(u64::from(u32::MAX)) as u32;
            let publisher = Messaging::open(
                home,
                Actor {
                    principal,
                    role: Role::Participant,
                    automated_depth: Some(reply_depth),
                },
            )?
            .with_discussion(Some(task.discussion.clone()));
            if guarded(home, task, || {
                publisher
                    .send(MessageInput {
                        input_mode: None,
                        conversation,
                        body: output.into(),
                        request_key: format!("run-result:{}", record.id),
                        mentions: Vec::new(),
                        reply_to: Some(reply_message.id),
                    })
                    .map_err(|_| RuntimeError::Im("cannot publish scheduled reply"))
            })
            .is_err()
            {
                task.state = "interrupted".into();
                task.reason = Some(
                    "reply publication stopped or unavailable; inspect run before retrying".into(),
                );
                store.update(task, now_ms())?;
                return Ok(());
            }
        }
        let obligation = crate::work::notification_obligation(home, store, task)?;
        task.state = if obligation.as_ref().is_some_and(|o| o["report_required"] == true) {
            task.reason = Some("task_report_required: execution completed but the result report was not committed; inspect task_get and submit task_report".into());
            "failed"
        } else if output.trim() == "[[silent]]" {
            if task.work_notification && obligation.is_none() {
                task.reason = Some("task notification was not handled; review task inbox".into());
                "failed"
            } else if task.work_notification {
                "completed"
            } else {
                "silent"
            }
        } else {
            "completed"
        }.into();
    } else {
        task.state = match record.state {
            RunState::Running | RunState::Interrupted => "interrupted",
            RunState::Canceled => "canceled",
            RunState::OutcomeUnknown => "outcome_unknown",
            _ => "failed",
        }
        .into();
        task.reason = record.error;
    }
    store.update(task, now_ms())?;
    Ok(())
}
