//! Execution fences and per-attempt model admission/accounting.
use super::im;
use crate::{
    ModelClient, ResolvedAgent, RuntimeError, ScheduleStore, ScheduledTask, schedule_store::now_ms,
};
use agt_core::CancellationToken;
use agt_im::{ConversationId, ConversationState, MessageId, PrincipalId};
use agt_service::BindingStore;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Fence local IM operations against lease takeover and discussion controls.
/// Never hold this transaction across a model request or an external MCP call.
pub(crate) fn guarded<T>(
    home: &Path,
    task: &ScheduledTask,
    operation: impl FnOnce() -> crate::Result<T>,
) -> crate::Result<T> {
    guarded_store(home, task, |_| operation())
}

pub(crate) fn guarded_store<T>(
    home: &Path,
    task: &ScheduledTask,
    operation: impl FnOnce(&ScheduleStore) -> crate::Result<T>,
) -> crate::Result<T> {
    let store = ScheduleStore::open(home)
        .map_err(|_| RuntimeError::Configuration("scheduler unavailable"))?;
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    let cutoff = store
        .context_cutoff(&task.conversation)
        .map_err(|_| RuntimeError::Configuration("context reset state unavailable"))?;
    let valid = store.authorized(task, now_ms()).unwrap_or(false)
        && BindingStore::open(home)
            .and_then(|s| s.get(&PrincipalId::new(&task.principal)))
            .is_ok_and(|b| b.is_some())
        && im(home).is_ok_and(|im| {
            let root_active = store
                .discussion(&task.discussion)
                .ok()
                .flatten()
                .is_some_and(|d| {
                    im.get_conversation(&ConversationId::new(d.conversation))
                        .is_ok_and(|c| c.state == ConversationState::Active)
                });
            root_active
                && crate::run_inputs::visible(&store, &im, task, cutoff)
                && im
                    .get_conversation(&ConversationId::new(&task.conversation))
                    .is_ok_and(|c| c.state == ConversationState::Active)
                && im
                    .get_member(
                        &ConversationId::new(&task.conversation),
                        &PrincipalId::new(&task.principal),
                    )
                    .is_ok_and(|m| m.left_message_seq.is_none())
                && im
                    .get_message(&MessageId::new(&task.message))
                    .is_ok_and(|m| m.deleted_at.is_none() && m.seq > cutoff)
        });
    if !valid {
        return Err(RuntimeError::Stopped(crate::termination::guard_cause(
            &store, task,
        )?));
    }
    if !task.work_notification
        && let Some(id) = &task.work_id
        && crate::work::read(&store.connection, id)
            .map_err(|_| RuntimeError::Canceled)?
            .cancellation_requested
            .is_some()
    {
        return Err(RuntimeError::Stopped(crate::StopCode::TaskCanceled));
    }
    if let Some(id) = &task.work_id
        && crate::work::check_task_scope(home, &store, id).is_err()
    {
        return Err(RuntimeError::Stopped(crate::termination::guard_cause(
            &store, task,
        )?));
    }
    let changes = store.connection.total_changes();
    let result = operation(&store)?;
    store.connection.execute_batch("COMMIT")?;
    if store.connection.total_changes() != changes {
        agt_storage::wake::notify(home);
    }
    Ok(result)
}

pub(super) struct ScheduledModel<'a> {
    pub(super) inner: &'a mut dyn ModelClient,
    pub(super) home: PathBuf,
    pub(super) task: ScheduledTask,
    pub(super) token: CancellationToken,
}
impl ModelClient for ScheduledModel<'_> {
    fn check_canceled(&self) -> crate::Result<()> {
        if self.token.is_canceled() {
            Err(RuntimeError::Stopped(crate::StopCode::token(&self.token)))
        } else {
            Ok(())
        }
    }
    fn complete(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> crate::Result<Value> {
        self.complete_stream(agent, messages, tools, &mut |_| Ok(()))
    }
    fn complete_stream(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
        progress: &mut dyn FnMut(crate::ModelEvent<'_>) -> crate::Result<()>,
    ) -> crate::Result<Value> {
        if self.token.is_canceled() {
            return Err(RuntimeError::Stopped(crate::StopCode::token(&self.token)));
        }
        guarded(&self.home, &self.task, || Ok(()))?;
        let mut store = ScheduleStore::open(&self.home)
            .map_err(|_| RuntimeError::Configuration("scheduler storage unavailable"))?;
        if !store
            .debit(&self.task, now_ms())
            .map_err(|_| RuntimeError::Configuration("scheduler call accounting unavailable"))?
        {
            return Err(RuntimeError::Stopped(crate::StopCode::token(&self.token)));
        }
        let response = self
            .inner
            .complete_stream(agent, messages, tools, &mut |event| {
                if self.token.is_canceled() {
                    return Err(RuntimeError::Stopped(crate::StopCode::token(&self.token)));
                }
                progress(event)
            })?;
        if self.token.is_canceled() {
            return Err(RuntimeError::Stopped(crate::StopCode::token(&self.token)));
        }
        guarded(&self.home, &self.task, || Ok(response))
    }
}
