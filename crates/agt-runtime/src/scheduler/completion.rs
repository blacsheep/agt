//! Persist Run outcomes and publish replies under the existing execution fence.
use super::guarded;
use crate::TaskState;
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
        task.finish_work_run(&record);
        store.update(task, now_ms())?;
        return Ok(());
    }
    if record.state == RunState::Completed {
        let output = record.output.as_deref().unwrap_or("");
        if token.is_canceled() {
            task.state = TaskState::Canceled;
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
                task.interrupt_publication();
                store.update(task, now_ms())?;
                return Ok(());
            }
        }
        let obligation = crate::work::notification_obligation(home, store, task)?;
        task.finish_published_reply(output, obligation.as_ref());
    } else {
        task.finish_unsuccessful_reply(&record);
    }
    store.update(task, now_ms())?;
    Ok(())
}
