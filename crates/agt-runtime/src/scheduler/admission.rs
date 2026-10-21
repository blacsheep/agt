//! Check a claimed task before context preparation or model execution.
use super::backend;
use crate::engine::EngineKind;
use crate::schedule_state::ClaimDeferral;
use crate::{DiscussionState, TaskState};
use crate::{ScheduleStore, ScheduledTask, schedule_store::now_ms};
use agt_core::CancellationToken;
use agt_im::{MessageId, MessageListOptions, PrincipalId};
use agt_service::{Actor, BindingStore, Messaging, Role};
use anyhow::{Result, bail};
use serde_json::Value;
use std::path::Path;

pub(super) struct Admission {
    pub binding: agt_service::Binding,
    pub messaging: Messaging,
    pub trigger: agt_im::Message,
    pub engine: EngineKind,
    pub batch: Vec<Value>,
}

pub(super) fn prepare(
    home: &Path,
    store: &mut ScheduleStore,
    task: &mut ScheduledTask,
    token: &CancellationToken,
) -> Result<Option<Admission>> {
    let Some(binding) = BindingStore::open(home)?.get(&PrincipalId::new(&task.principal))? else {
        if task.work_id.is_some() {
            task.defer_claim(ClaimDeferral::DisabledBinding);
            store.update(task, now_ms())?;
            return Ok(None);
        }
        bail!("binding removed");
    };
    let messaging = Messaging::open(
        home,
        Actor {
            principal: binding.principal.clone(),
            role: Role::Participant,
            automated_depth: None,
        },
    )?;
    let trigger = messaging.im.get_message(&MessageId::new(&task.message))?;
    let policy = BindingStore::open(home)?.policy(&trigger.conversation_id, &binding)?;
    let engine = EngineKind::resolve(home, &binding.agent)?;
    let zone = crate::clock::zone(&agt_config::RuntimeConfig::load(home)?);
    let mut eligible = false;
    let mut batch = Vec::new();
    let mut batch_bytes = 0;
    for id in &task.triggers {
        let message = messaging.im.get_message(&MessageId::new(id.as_str()))?;
        let effective_policy = backend::response_policy(engine, policy);
        eligible |= messaging.may_respond(&message, effective_policy)?;
        if message.deleted_at.is_none() {
            let item = crate::conversation_context::message_item(&message, &zone);
            batch_bytes += item.to_string().len();
            if batch_bytes > 128 * 1024 {
                bail!("merged message batch exceeds 128 KiB");
            }
            batch.push(item);
        }
    }
    batch.sort_by_key(|m| m["seq"].as_i64().unwrap_or(0));
    batch.dedup_by(|a, b| a["message_id"] == b["message_id"]);
    if !eligible && task.work_id.is_some() {
        task.defer_claim(ClaimDeferral::ResponsePolicy);
        store.update(task, now_ms())?;
        return Ok(None);
    }
    if !eligible {
        task.skip_ineligible_message();
        store.update(task, now_ms())?;
        return Ok(None);
    }
    let round = store
        .discussion(&task.discussion)?
        .ok_or_else(|| anyhow::anyhow!("round missing"))?;
    if round.state != DiscussionState::Active || token.is_canceled() {
        task.defer_claim(ClaimDeferral::InactiveDiscussion);
        store.update(task, now_ms())?;
        return Ok(None);
    }
    // A wait that ends with nothing new is decided again: the members it waited for
    // have settled, so the message may now be this member's to act on.
    let conversation = messaging.im.get_conversation(&trigger.conversation_id)?;
    if task.state == TaskState::Responding
        && task.decision_required
        && task.through_seq != conversation.last_message_seq
    {
        task.defer_claim(ClaimDeferral::StaleDecision);
        store.update(task, now_ms())?;
        return Ok(None);
    }
    let page = messaging.im.list_messages(
        &trigger.conversation_id,
        &binding.principal,
        MessageListOptions {
            limit: 20,
            before: None,
            after: None,
        },
    )?;
    if let Some(latest) = page.messages.last()
        && !quiet_ready(task, latest.created_at, now_ms())
    {
        task.defer_claim(ClaimDeferral::ConversationQuiet {
            last_message_at: latest.created_at,
        });
        store.update(task, now_ms())?;
        return Ok(None);
    }
    if task.decision_required || task.work_id.is_some() {
        task.through_seq = conversation.last_message_seq;
    }
    if backend::admit_participation(engine, store, task)? {
        return Ok(None);
    }
    Ok(Some(Admission {
        binding,
        messaging,
        trigger,
        engine,
        batch,
    }))
}

pub(super) fn quiet_ready(task: &ScheduledTask, last_message_at: i64, now: i64) -> bool {
    if task.work_id.is_some() {
        return now >= task.due_at;
    }
    now >= task.due_at
        && (!task.quiet || now >= task.deadline || now - last_message_at >= task.quiet_ms)
}
