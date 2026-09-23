//! Check a claimed task before context preparation or model execution.
use crate::{ScheduleStore, ScheduledTask, schedule_store::now_ms};
use agt_core::CancellationToken;
use agt_im::{MessageId, MessageListOptions, PrincipalId, PrincipalKind};
use agt_service::{Actor, BindingStore, Messaging, ResponsePolicy, Role};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::Path;

pub(super) struct Admission {
    pub binding: agt_service::Binding,
    pub messaging: Messaging,
    pub trigger: agt_im::Message,
    pub round: crate::Discussion,
    pub external: bool,
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
            task.state = "queued".into();
            task.reason = Some("task blocked by disabled binding".into());
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
    let external = crate::harness::agent_config(home, &binding.agent)?
        .harness
        .is_some();
    let mut eligible = false;
    let mut batch = Vec::new();
    let mut batch_bytes = 0;
    for id in &task.triggers {
        let message = messaging.im.get_message(&MessageId::new(id.as_str()))?;
        let peer = messaging.im.get_principal(&message.sender_id)?.kind != PrincipalKind::User;
        let effective_policy = if external && policy != ResponsePolicy::Manual {
            ResponsePolicy::Mentioned
        } else if peer && policy != ResponsePolicy::Manual {
            ResponsePolicy::All
        } else {
            policy
        };
        eligible |= messaging.may_respond(&message, effective_policy)?;
        if message.deleted_at.is_none() {
            let item = json!({"seq":message.seq,"message_id":message.id,"sender":message.sender_id,"body":message.body});
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
        task.state = "queued".into();
        task.reason = Some("task blocked by response policy".into());
        store.update(task, now_ms())?;
        return Ok(None);
    }
    if !eligible {
        task.state = "skipped".into();
        task.reason = Some("response policy or message state".into());
        store.update(task, now_ms())?;
        return Ok(None);
    }
    let round = store
        .discussion(&task.discussion)?
        .ok_or_else(|| anyhow::anyhow!("round missing"))?;
    if round.state != "active" || token.is_canceled() {
        task.state = "queued".into();
        store.update(task, now_ms())?;
        return Ok(None);
    }
    let conversation = messaging.im.get_conversation(&trigger.conversation_id)?;
    if task.state == "responding"
        && task.decision_required
        && task.through_seq != conversation.last_message_seq
    {
        task.state = "queued".into();
        task.due_at = now_ms();
        task.reason = None;
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
        task.state = "queued".into();
        task.due_at = task.deadline.min(latest.created_at + task.quiet_ms);
        store.update(task, now_ms())?;
        return Ok(None);
    }
    if task.decision_required || task.work_id.is_some() {
        task.through_seq = conversation.last_message_seq;
    }
    if external && task.state == "evaluating" {
        task.state = "ready_to_reply".into();
        task.reason = Some(
            "framework: external engine runs only for direct messages or explicit mentions".into(),
        );
        task.due_at = now_ms();
        task.decision_required = false;
        store.update(task, now_ms())?;
        return Ok(None);
    }
    Ok(Some(Admission {
        binding,
        messaging,
        trigger,
        round,
        external,
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
