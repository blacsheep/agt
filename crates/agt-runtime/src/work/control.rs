use super::*;
use agt_im::{ConversationId, ConversationState, ImStore, MessageId, PrincipalId, PrincipalKind};
use agt_service::{Actor, BindingStore, ResponsePolicy, WorkControl};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Fixed host identity. Hosted requests require a live scheduler fence; local human
/// requests require an explicitly authorized human message. No new budget is minted.
pub struct TaskControl {
    home: PathBuf,
    actor: Actor,
    task: Option<crate::ScheduledTask>,
}
impl TaskControl {
    pub fn local(home: &Path, actor: Actor) -> Self {
        Self {
            home: home.into(),
            actor,
            task: None,
        }
    }
    pub(crate) fn hosted(home: &Path, actor: Actor, task: crate::ScheduledTask) -> Self {
        Self {
            home: home.into(),
            actor,
            task: Some(task),
        }
    }
    fn request(&self, args: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            assignee: String,
            conversation: String,
            input: String,
            request_key: String,
            source_message: Option<String>,
        }
        let input: Input = serde_json::from_value(args)?;
        let operation = |store: &ScheduleStore| -> Result<Value> {
            let im = open_im(&self.home)?;
            let (origin, discussion) = if let Some(task) = &self.task {
                if self.actor.principal.as_str() != task.principal
                    || input
                        .source_message
                        .as_ref()
                        .is_some_and(|id| id != &task.message)
                {
                    bail!("invalid hosted task context");
                }
                (task.conversation.clone(), task.discussion.clone())
            } else {
                self.actor.require_manager()?;
                if im.get_principal(&self.actor.principal)?.kind != PrincipalKind::User {
                    bail!("local task requests require a human actor and an authorized message");
                }
                let message = im.get_message(&MessageId::new(
                    input.source_message.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("source_message is required for local task requests")
                    })?,
                ))?;
                if message.sender_id != self.actor.principal
                    || message.deleted_at.is_some()
                    || message.seq <= store.context_cutoff(message.conversation_id.as_str())?
                {
                    bail!("source message is not current human authorization");
                }
                let discussion = store
                    .human_discussion(message.id.as_str())?
                    .ok_or_else(|| {
                        anyhow::anyhow!("authorize source message with schedule start first")
                    })?;
                (message.conversation_id.to_string(), discussion)
            };
            member(&im, &origin, self.actor.principal.as_str())?;
            member(&im, &input.conversation, self.actor.principal.as_str())?;
            member(&im, &input.conversation, &input.assignee)?;
            if self.actor.principal.as_str() == input.assignee {
                bail!("self assignment is not supported");
            }
            if !BindingStore::open(&self.home)?
                .list_with_state()?
                .iter()
                .any(|b| b.binding.principal.as_str() == input.assignee)
            {
                bail!("assignee needs an existing agent binding");
            }
            let round = store
                .discussion(&discussion)?
                .ok_or_else(|| anyhow::anyhow!("discussion missing"))?;
            if !matches!(round.state.as_str(), "active" | "paused")
                || round.expires_at <= crate::schedule_store::now_ms()
            {
                bail!("discussion is ended or expired; renew authorization first");
            }
            let request = WorkRequest {
                requester: self.actor.principal.to_string(),
                assignee: input.assignee.clone(),
                origin_conversation: origin.clone(),
                report_to: input.conversation.clone(),
                discussion,
                context_epoch: store.context_cutoff(&origin)?,
                delivery_epoch: store.context_cutoff(&input.conversation)?,
                input: input.input.clone(),
                request_key: input.request_key.clone(),
            };
            let item = WorkLedger::enqueue_on(&store.connection, request)?;
            view(&self.home, store, &item)
        };
        if let Some(task) = &self.task {
            // Task tools own this transaction, rather than nesting inside the generic tool guard.
            crate::scheduler::guarded_store(&self.home, task, |s| {
                operation(s).map_err(|e| crate::RuntimeError::Invalid(e.to_string()))
            })
            .map_err(Into::into)
        } else {
            let store = ScheduleStore::open(&self.home)?;
            store.connection.execute_batch("BEGIN IMMEDIATE")?;
            let value = operation(&store)?;
            store.connection.execute_batch("COMMIT")?;
            Ok(value)
        }
    }
}
impl WorkControl for TaskControl {
    fn call(&self, name: &str, args: Value) -> Result<Value> {
        if name == "task_request" {
            return self.request(args);
        }
        let operation = |store: &ScheduleStore| -> Result<Value> {
            let im = open_im(&self.home)?;
            let visible = |item: &WorkItem| {
                (item.request.requester == self.actor.principal.as_str()
                    || item.request.assignee == self.actor.principal.as_str())
                    && im
                        .get_member(
                            &ConversationId::new(&item.request.report_to),
                            &self.actor.principal,
                        )
                        .is_ok_and(|m| m.left_message_seq.is_none())
                    && im
                        .get_principal(&self.actor.principal)
                        .is_ok_and(|p| p.deleted_at.is_none())
            };
            match name {
                "task_get" => {
                    let item = read(
                        &store.connection,
                        args["task_id"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("task_id required"))?,
                    )?;
                    if !visible(&item) {
                        bail!("task is not visible to this actor");
                    }
                    view(&self.home, store, &item)
                }
                "task_list" => {
                    let limit = args
                        .get("limit")
                        .map(|v| v.as_u64().ok_or_else(|| anyhow::anyhow!("invalid limit")))
                        .transpose()?
                        .unwrap_or(50);
                    if !(1..=200).contains(&limit) {
                        bail!("limit must be in 1..=200");
                    }
                    let mut stmt = store.connection.prepare("SELECT record FROM work_items WHERE requester=?1 OR json_extract(record,'$.request.assignee')=?1 ORDER BY rowid DESC")?;
                    let mut result = Vec::new();
                    for row in
                        stmt.query_map([self.actor.principal.as_str()], |r| r.get::<_, String>(0))?
                    {
                        let item: WorkItem = serde_json::from_str(&row?)?;
                        if visible(&item) {
                            result.push(view(&self.home, store, &item)?);
                        }
                        if result.len() >= limit as usize {
                            break;
                        }
                    }
                    Ok(json!(result))
                }
                _ => bail!("unknown task operation"),
            }
        };
        if let Some(task) = &self.task {
            crate::scheduler::guarded_store(&self.home, task, |s| {
                operation(s).map_err(|e| crate::RuntimeError::Invalid(e.to_string()))
            })
            .map_err(Into::into)
        } else {
            operation(&ScheduleStore::open(&self.home)?)
        }
    }
}

pub(super) fn open_im(home: &Path) -> Result<ImStore> {
    Ok(ImStore::open(crate::store::database_path(
        home, "im", "im.db",
    )?)?)
}
pub(super) fn member(im: &ImStore, chat: &str, principal: &str) -> Result<()> {
    let chat = ConversationId::new(chat);
    let principal = PrincipalId::new(principal);
    if im.get_conversation(&chat)?.state != ConversationState::Active
        || im.get_principal(&principal)?.deleted_at.is_some()
        || im.get_member(&chat, &principal)?.left_message_seq.is_some()
    {
        bail!("active identity and conversation membership required");
    }
    Ok(())
}
pub(crate) fn check_task_scope(home: &Path, store: &ScheduleStore, id: &str) -> Result<()> {
    check_scope(home, store, &read(&store.connection, id)?)
}
pub(super) fn check_scope(home: &Path, store: &ScheduleStore, item: &WorkItem) -> Result<()> {
    let im = open_im(home)?;
    let r = &item.request;
    member(&im, &r.origin_conversation, &r.requester)?;
    member(&im, &r.report_to, &r.requester)?;
    member(&im, &r.report_to, &r.assignee)?;
    let discussion = store
        .discussion(&r.discussion)?
        .ok_or_else(|| anyhow::anyhow!("discussion missing"))?;
    if discussion.state == "ended"
        || im
            .get_conversation(&ConversationId::new(&discussion.conversation))?
            .state
            != ConversationState::Active
        || store.context_cutoff(&r.origin_conversation)? != r.context_epoch
        || store.context_cutoff(&r.report_to)? != r.delivery_epoch
    {
        bail!("task authorization boundary changed");
    }
    Ok(())
}
pub(super) fn blocked(
    home: &Path,
    store: &ScheduleStore,
    item: &WorkItem,
    recipient: &str,
) -> Result<Option<&'static str>> {
    if check_scope(home, store, item).is_err() {
        return Ok(Some("authorization_changed"));
    }
    let bindings = BindingStore::open(home)?;
    let Some(binding) = bindings.get(&PrincipalId::new(recipient))? else {
        return Ok(Some("disabled_or_unbound"));
    };
    if bindings.policy(&ConversationId::new(&item.request.report_to), &binding)?
        == ResponsePolicy::Manual
    {
        return Ok(Some("manual"));
    }
    let round = store.discussion(&item.request.discussion)?.unwrap();
    if round.state != "active" || round.expires_at <= crate::schedule_store::now_ms() {
        return Ok(Some("discussion_paused_or_expired"));
    }
    Ok(None)
}
fn view(home: &Path, store: &ScheduleStore, item: &WorkItem) -> Result<Value> {
    let mut value = serde_json::to_value(item)?;
    if item.state == WorkState::Queued {
        value["blocked_reason"] = json!(blocked(home, store, item, &item.request.assignee)?);
    }
    let publication: Option<Option<String>> = store.connection.query_row("SELECT p.message_id FROM work_publications p JOIN work_outbox e ON e.seq=p.event_seq WHERE e.work_id=?1", [&item.id], |r| r.get(0)).optional()?;
    let error: Option<String> = store.connection.query_row("SELECT p.error FROM work_publications p JOIN work_outbox e ON e.seq=p.event_seq WHERE e.work_id=?1",[&item.id],|r| r.get(0)).optional()?.flatten();
    value["publication_error"] = json!(error);
    let notification: Option<String> = store.connection.query_row("SELECT i.state FROM work_inbox i JOIN work_outbox e ON i.event_seq=e.seq WHERE e.work_id=?1 AND i.recipient=?2 AND e.kind!='requested'",params![item.id,item.request.requester],|r|r.get(0)).optional()?;
    value["requester_notification"] = json!(notification);
    if notification.as_deref() == Some("pending") {
        value["notification_blocked_reason"] =
            json!(blocked(home, store, item, &item.request.requester)?);
    }
    value["publication"] = json!(match publication {
        None => "not_ready",
        Some(None) =>
            if error.is_none() && check_scope(home, store, item).is_ok() {
                "pending"
            } else {
                "blocked"
            },
        Some(Some(_)) => "published",
    });
    Ok(value)
}
