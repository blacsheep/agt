use super::*;
use agt_im::{ConversationId, ConversationState, ImStore, MessageId, PrincipalId, PrincipalKind};
use agt_service::{Actor, BindingStore, ResponsePolicy, WorkControl};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Fixed host identity. Hosted requests require a live scheduler fence; local human
/// requests require an explicitly authorized human message. No new budget is minted.
pub struct TaskControl {
    pub(super) home: PathBuf,
    pub(super) actor: Actor,
    pub(super) task: Option<crate::ScheduledTask>,
    resumable_wait: bool,
}
impl TaskControl {
    pub fn local(home: &Path, actor: Actor) -> Self {
        Self {
            home: home.into(),
            actor,
            task: None,
            resumable_wait: false,
        }
    }
    pub(crate) fn hosted(home: &Path, actor: Actor, task: crate::ScheduledTask) -> Self {
        Self {
            home: home.into(),
            actor,
            task: Some(task),
            resumable_wait: false,
        }
    }
    pub(crate) fn with_resumable_wait(mut self) -> Self {
        self.resumable_wait = true;
        self
    }
    pub(super) fn human_manager(&self, im: &ImStore) -> Result<bool> {
        Ok(self.actor.role == agt_service::Role::Manager
            && im.get_principal(&self.actor.principal)?.kind == PrincipalKind::User)
    }
    fn edit_queue(&self, name: &str, args: Value) -> Result<Value> {
        let operation = |store: &ScheduleStore| -> Result<Value> {
            let id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("task_id required"))?;
            let mut item = read(&store.connection, id)?;
            let im = open_im(&self.home)?;
            check_scope(&self.home, store, &item)?;
            member(&im, &item.request.report_to, self.actor.principal.as_str())?;
            let human = self.human_manager(&im)?;
            if self.task.is_none() && !human {
                bail!("queue changes require a hosted task or human Manager");
            }
            if self.actor.principal.as_str() != item.request.assignee
                && !(human && self.actor.principal.as_str() == item.request.requester)
            {
                bail!("only assignee or requesting human Manager can manage this queue");
            }
            if let Some(task) = &self.task
                && (task.principal != self.actor.principal.as_str()
                    || task.discussion != item.request.discussion)
            {
                bail!("task belongs to another authorization scope");
            }
            let revision = args["expected_revision"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("expected_revision required"))?;
            let reason = args["reason"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("reason required"))?;
            if reason.trim().is_empty() || reason.len() > 2000 {
                bail!("reason must contain 1..2000 bytes");
            }
            queue::ensure_queued(&store.connection, &item, revision)?;
            match name {
                "task_reprioritize" => {
                    let priority: Priority = serde_json::from_value(args["priority"].clone())?;
                    if !human && (priority == Priority::Urgent || item.priority == Priority::Urgent)
                    {
                        bail!("urgent priority is controlled by a human Manager");
                    }
                    item.priority = priority;
                    item.priority_reason = Some(reason.into());
                }
                "task_defer" => {
                    let not_before = args
                        .get("not_before")
                        .map(|v| {
                            v.as_i64()
                                .ok_or_else(|| anyhow::anyhow!("invalid not_before"))
                        })
                        .transpose()?;
                    let after = args
                        .get("after_task")
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| anyhow::anyhow!("invalid after_task"))
                        })
                        .transpose()?;
                    let deadline = args
                        .get("deadline")
                        .map(|v| {
                            v.as_i64()
                                .ok_or_else(|| anyhow::anyhow!("invalid deadline"))
                        })
                        .transpose()?;
                    let now = crate::schedule_store::now_ms();
                    if not_before.is_some() == after.is_some() {
                        bail!("provide exactly one of not_before or after_task");
                    }
                    if let Some(at) = not_before
                        && (at <= now || deadline.is_some())
                    {
                        bail!("not_before must be future; deadline is only for after_task");
                    }
                    if let Some(target) = &after {
                        let other = read(&store.connection, target)?;
                        check_scope(&self.home, store, &other)?;
                        member(&im, &other.request.report_to, self.actor.principal.as_str())?;
                        if self.actor.principal.as_str() != other.request.assignee
                            && self.actor.principal.as_str() != other.request.requester
                        {
                            bail!("dependency task is not visible to this actor");
                        }
                        if other.request.assignee != item.request.assignee
                            || other.request.discussion != item.request.discussion
                        {
                            bail!("barrier must reference the same assignee and discussion");
                        }
                        if deadline.is_none_or(|at| at <= now) {
                            bail!("after_task requires a future deadline");
                        }
                        queue::set_dependency(&store.connection, &item.id, target)?;
                    } else {
                        store.connection.execute(
                            "DELETE FROM work_dependencies WHERE work_id=?1",
                            [&item.id],
                        )?;
                    }
                    item.deferred = Some(Deferred {
                        reason: reason.into(),
                        not_before,
                        after_task: after,
                        deadline,
                    });
                }
                _ => bail!("unknown queue action"),
            }
            queue::save(
                &store.connection,
                &mut item,
                if name == "task_defer" {
                    "deferred"
                } else {
                    "reprioritized"
                },
            )?;
            view(&self.home, store, &item)
        };
        if let Some(task) = &self.task {
            crate::scheduler::guarded_store(&self.home, task, |s| {
                operation(s).map_err(|e| crate::RuntimeError::Invalid(e.to_string()))
            })
            .map_err(Into::into)
        } else {
            let store = ScheduleStore::open(&self.home)?;
            store.connection.execute_batch("BEGIN IMMEDIATE")?;
            let result = operation(&store)?;
            store.connection.execute_batch("COMMIT")?;
            agt_storage::wake::notify(&self.home);
            Ok(result)
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
            #[serde(default)]
            priority: Priority,
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
            if input.priority == Priority::Urgent && !self.human_manager(&im)? {
                bail!("urgent priority requires a human Manager");
            }
            let request = WorkRequest {
                parent_id: self
                    .task
                    .as_ref()
                    .filter(|t| !t.work_notification)
                    .and_then(|t| t.work_id.clone()),
                retry_of: None,
                requester: self.actor.principal.to_string(),
                assignee: input.assignee.clone(),
                origin_conversation: origin.clone(),
                report_to: input.conversation.clone(),
                discussion,
                context_epoch: store.context_cutoff(&origin)?,
                delivery_epoch: store.context_cutoff(&input.conversation)?,
                input: input.input.clone(),
                request_key: input.request_key.clone(),
                priority: input.priority,
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
            agt_storage::wake::notify(&self.home);
            Ok(value)
        }
    }
}
impl WorkControl for TaskControl {
    fn supports_await(&self) -> bool {
        self.resumable_wait
            && self
                .task
                .as_ref()
                .is_some_and(|t| t.work_id.is_some() && !t.work_notification)
    }
    fn call(&self, name: &str, args: Value) -> Result<Value> {
        if matches!(name, "task_cancel" | "task_retry" | "task_resolve") {
            return self.lifecycle(name, args);
        }
        if name == "task_report" {
            return self.report(args);
        }
        if name == "task_attention" {
            return self.attention(args);
        }
        if name == "task_reply" {
            return self.reply(args);
        }
        if matches!(name, "task_await" | "task_yield" | "task_input") {
            if !self.supports_await() {
                bail!("this execution host cannot suspend work");
            }
            return super::wait::register(
                &self.home,
                self.task.as_ref().unwrap(),
                args,
                match name {
                    "task_yield" => "timer",
                    "task_input" => "input",
                    _ => "dependency",
                },
            );
        }
        if matches!(name, "task_reprioritize" | "task_defer") {
            return self.edit_queue(name, args);
        }
        if name == "task_request" {
            return self.request(args);
        }
        let operation = |store: &ScheduleStore| -> Result<Value> {
            let im = open_im(&self.home)?;
            let human = self.human_manager(&im)?;
            let visible = |item: &WorkItem| {
                (item.request.requester == self.actor.principal.as_str()
                    || item.request.assignee == self.actor.principal.as_str()
                    || human)
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
                    let queued_only = args
                        .get("queued_only")
                        .map(|v| {
                            v.as_bool()
                                .ok_or_else(|| anyhow::anyhow!("queued_only must be boolean"))
                        })
                        .transpose()?
                        .unwrap_or(false);
                    let conversation = args
                        .get("conversation")
                        .map(|v| {
                            v.as_str()
                                .ok_or_else(|| anyhow::anyhow!("invalid conversation"))
                        })
                        .transpose()?;
                    let before = args
                        .get("before")
                        .map(|v| {
                            v.as_i64()
                                .filter(|n| *n > 0)
                                .ok_or_else(|| anyhow::anyhow!("invalid cursor"))
                        })
                        .transpose()?;
                    let mut stmt = store.connection.prepare("SELECT rowid,record FROM work_items WHERE (?3 OR requester=?1 OR json_extract(record,'$.request.assignee')=?1) AND (NOT ?2 OR json_extract(record,'$.state')='queued') AND (?4 IS NULL OR json_extract(record,'$.request.report_to')=?4) AND (?5 IS NULL OR rowid<?5) ORDER BY rowid DESC")?;
                    let mut result = Vec::new();
                    for row in stmt.query_map(
                        params![
                            self.actor.principal.as_str(),
                            queued_only,
                            human,
                            conversation,
                            before
                        ],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                    )? {
                        let (cursor, record) = row?;
                        let item: WorkItem = serde_json::from_str(&record)?;
                        if visible(&item) {
                            let mut value = view(&self.home, store, &item)?;
                            value["list_cursor"] = json!(cursor);
                            result.push(value);
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
    let mut parent = item.request.parent_id.clone();
    while let Some(id) = parent {
        let ancestor = read(&store.connection, &id)?;
        if ancestor.cancellation_requested.is_some()
            || matches!(ancestor.state, WorkState::Canceled { .. })
        {
            bail!("parent task canceled");
        }
        parent = ancestor.request.parent_id;
    }
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
pub(super) fn view(home: &Path, store: &ScheduleStore, item: &WorkItem) -> Result<Value> {
    let mut value = serde_json::to_value(item)?;
    value["report"] = super::reporting::status(&store.connection, &item.id)?;
    if item.state == WorkState::Queued {
        value["blocked_reason"] = json!(blocked(home, store, item, &item.request.assignee)?.or(
            queue::blocker(&store.connection, item, crate::schedule_store::now_ms())?
        ));
    }
    if let WorkState::WaitingDependency { dependency, .. } = &item.state {
        value["waiting_for_task"] = json!(dependency);
        value["blocked_reason"] =
            json!(blocked(home, store, item, &item.request.assignee)?.or(Some("waiting_for_task")));
    }
    if matches!(
        item.state,
        WorkState::WaitingInput { .. } | WorkState::WaitingUntil { .. }
    ) {
        value["blocked_reason"] = json!(blocked(home, store, item, &item.request.assignee)?.or(
            Some(if matches!(item.state, WorkState::WaitingInput { .. }) {
                "waiting_input"
            } else {
                "deferred_until"
            })
        ));
    }
    if let Some(deferred) = &item.deferred {
        // Both sides see the same durable condition; no private dependency payload is exposed.
        value["waiting_for_task"] = json!(deferred.after_task);
    }
    let publication: Option<Option<String>> = store.connection.query_row("SELECT p.message_id FROM work_publications p JOIN work_outbox e ON e.seq=p.event_seq WHERE e.work_id=?1 ORDER BY e.seq DESC LIMIT 1", [&item.id], |r| r.get(0)).optional()?;
    let error: Option<String> = store.connection.query_row("SELECT p.error FROM work_publications p JOIN work_outbox e ON e.seq=p.event_seq WHERE e.work_id=?1 ORDER BY e.seq DESC LIMIT 1",[&item.id],|r| r.get(0)).optional()?.flatten();
    value["publication_error"] = json!(error);
    let notification: Option<String> = store.connection.query_row("SELECT i.state FROM work_inbox i JOIN work_outbox e ON i.event_seq=e.seq WHERE e.work_id=?1 AND i.recipient=?2 AND e.kind IN ('succeeded','failed','canceled','outcome_unknown','resolved') ORDER BY e.seq DESC LIMIT 1",params![item.id,item.request.requester],|r|r.get(0)).optional()?;
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
    if let Some(run_id) = value["run_id"].as_str()
        && let Ok(run) = crate::RunStore::open(home).and_then(|s| s.get(run_id))
    {
        if let Some(progress) = &run.progress {
            value["activity"] = json!(progress.activities);
        } else if let Some(execution) = &run.execution {
            value["activity"] = execution["activity"]["items"].clone();
        }
    }
    let mut history = store.connection.prepare(
        "SELECT kind,created_at FROM work_outbox WHERE work_id=?1 ORDER BY seq DESC LIMIT 40",
    )?;
    value["timeline"] = json!(
        history
            .query_map([&item.id], |r| Ok(
                json!({"kind":r.get::<_,String>(0)?,"at":r.get::<_,i64>(1)?})
            ))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    );
    Ok(value)
}
