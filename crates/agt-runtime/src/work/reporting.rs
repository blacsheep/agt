//! Report obligations are independent of result delivery and notification Runs.
use super::{
    control::{check_scope, member, open_im, view},
    *,
};
use agt_im::{ConversationId, PrincipalId, PrincipalKind, SendMessageRequest};
use serde_json::{Value, json};

pub(super) fn status(c: &Connection, id: &str) -> Result<Value> {
    let row=c.query_row("SELECT event_seq,body IS NOT NULL,message_id,error,created_at FROM work_reports WHERE work_id=?1 ORDER BY event_seq DESC LIMIT 1",[id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,i64>(4)?))).optional()?;
    Ok(match row {
        None => json!({"state":"not_ready"}),
        Some((seq, body, message, error, at)) => {
            json!({"event_seq":seq,"state":if message.is_some(){"published"}else if error.is_some(){"blocked"}else if body{"pending"}else{"awaiting_report"},"message_id":message,"error":error,"created_at":at})
        }
    })
}
/// Only the report attached to this authorized result notification is exposed.
/// Tool reads and natural-language answers are not report acknowledgements.
pub(crate) fn notification_obligation(
    home: &Path,
    store: &ScheduleStore,
    task: &crate::ScheduledTask,
) -> Result<Option<Value>> {
    if !task.work_notification {
        return Ok(None);
    }
    let Some(id) = &task.work_id else {
        return Ok(None);
    };
    let item = read(&store.connection, id)?;
    if item.request.requester != task.principal
        || item.request.discussion != task.discussion
        || item.request.report_to != task.conversation
    {
        bail!("notification does not belong to requester");
    }
    let im = open_im(home)?;
    member(&im, &item.request.report_to, &task.principal)?;
    member(&im, &item.request.origin_conversation, &task.principal)?;
    let report: Option<(i64, bool, bool)> = store
        .connection
        .query_row(
            "SELECT r.event_seq,r.body IS NOT NULL,r.message_id IS NOT NULL
         FROM work_reports r JOIN work_publications p ON p.event_seq=r.event_seq
         WHERE r.work_id=?1 AND p.message_id=?2",
            params![id, task.message],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(report.map(|(seq, submitted, published)| {
        json!({
            "task_id": id,
            "event_seq": seq,
            "expected_revision": item.revision,
            "origin_conversation": item.request.origin_conversation,
            "report_required": !submitted && !published,
            "report_submitted": submitted,
            "report_published": published
        })
    }))
}

impl TaskControl {
    pub(super) fn report(&self, args: Value) -> Result<Value> {
        let operation = |store: &ScheduleStore| -> Result<Value> {
            let id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("task_id required"))?;
            let mut item = read(&store.connection, id)?;
            let im = open_im(&self.home)?;
            let human = self.human_manager(&im)?;
            if (!human && self.task.is_none())
                || (!human && item.request.requester != self.actor.principal.as_str())
            {
                bail!("only hosted requester or human Manager can report");
            }
            if self
                .task
                .as_ref()
                .is_some_and(|t| t.discussion != item.request.discussion)
            {
                bail!("report belongs to another discussion");
            }
            check_scope(&self.home, store, &item)?;
            member(&im, &item.request.report_to, self.actor.principal.as_str())?;
            member(
                &im,
                &item.request.origin_conversation,
                self.actor.principal.as_str(),
            )?;
            let body = args["body"]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 64000)
                .ok_or_else(|| anyhow::anyhow!("report requires 1..64000 bytes"))?;
            let (seq,previous,actor,message):(i64,Option<String>,Option<String>,Option<String>)=store.connection.query_row("SELECT event_seq,body,actor,message_id FROM work_reports WHERE work_id=?1 ORDER BY event_seq DESC LIMIT 1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            if previous.as_deref() == Some(body)
                && actor.as_deref() == Some(self.actor.principal.as_str())
            {
                return view(&self.home, store, &item);
            }
            if previous.is_some() || message.is_some() {
                bail!("report already submitted; cannot replace it");
            }
            if args["expected_revision"].as_i64() != Some(item.revision) {
                bail!("stale revision; inspect task_get");
            }
            store.connection.execute(
                "UPDATE work_reports SET body=?2,actor=?3 WHERE event_seq=?1",
                params![seq, body, self.actor.principal.as_str()],
            )?;
            super::actions::save_event(&store.connection, &mut item, "report_requested", false)?;
            view(&self.home, store, &item)
        };
        if let Some(task) = &self.task {
            crate::scheduler::guarded_store(&self.home, task, |s| {
                operation(s).map_err(|e| crate::RuntimeError::Invalid(e.to_string()))
            })
            .map_err(Into::into)
        } else {
            let store = ScheduleStore::open(&self.home)?;
            let tx_db = store.connection.clone();
            let tx = agt_storage::sqlite::write(&tx_db)?;
            let result = operation(&store)?;
            tx.commit()?;
            agt_storage::wake::notify(&self.home);
            Ok(result)
        }
    }
    pub(super) fn attention(&self, args: Value) -> Result<Value> {
        let im = open_im(&self.home)?;
        if !self.human_manager(&im)?
            || im.workspace_owner()?.as_ref() != Some(&self.actor.principal)
        {
            bail!("only workspace owner can inspect minimal incident metadata");
        }
        let limit = args
            .get("limit")
            .map(|v| {
                v.as_u64()
                    .filter(|n| (1..=200).contains(n))
                    .ok_or_else(|| anyhow::anyhow!("invalid limit"))
            })
            .transpose()?
            .unwrap_or(50);
        let before = args
            .get("before")
            .map(|v| {
                v.as_i64()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| anyhow::anyhow!("invalid cursor"))
            })
            .transpose()?;
        let store = ScheduleStore::open(&self.home)?;
        let mut stmt=store.connection.prepare("SELECT rowid,record FROM work_items WHERE (?1 IS NULL OR rowid<?1) ORDER BY rowid DESC")?;
        let mut result = Vec::new();
        for row in stmt.query_map([before], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (cursor, record) = row?;
            let item: WorkItem = serde_json::from_str(&record)?;
            let detail = view(&self.home, &store, &item)?;
            let mut reasons = Vec::new();
            if matches!(item.state, WorkState::OutcomeUnknown { .. }) && item.resolution.is_none() {
                reasons.push("outcome_unknown");
            }
            if detail["publication"] == "blocked" {
                reasons.push("publication_blocked");
            }
            if detail["report"]["state"] == "blocked" {
                reasons.push("report_blocked");
            }
            if detail["report"]["state"] == "awaiting_report"
                && detail["report"]["created_at"]
                    .as_i64()
                    .is_some_and(|at| crate::schedule_store::now_ms() - at >= 300_000)
            {
                reasons.push("report_overdue");
            }
            if detail["blocked_reason"] == "dependency_deadline_exceeded" {
                reasons.push("dependency_deadline_exceeded");
            }
            if !im
                .get_principal(&PrincipalId::new(&item.request.requester))
                .is_ok_and(|p| p.deleted_at.is_none())
                && detail["report"]["state"] != "published"
            {
                reasons.push("requester_unavailable");
            }
            if !reasons.is_empty() {
                let visible = im
                    .get_member(
                        &ConversationId::new(&item.request.report_to),
                        &self.actor.principal,
                    )
                    .is_ok_and(|m| m.left_message_seq.is_none());
                result.push(json!({"id":item.id,"requester":item.request.requester,"assignee":item.request.assignee,"reasons":reasons,"updated_at":item.updated_at,"list_cursor":cursor,"can_inspect":visible}));
                if result.len() >= limit as usize {
                    break;
                }
            }
        }
        Ok(json!(result))
    }
}

pub(super) fn publish(home: &Path, store: &ScheduleStore) -> Result<()> {
    let rows = {
        let mut stmt=store.connection.prepare("SELECT event_seq,work_id FROM work_reports WHERE message_id IS NULL ORDER BY event_seq")?;
        stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let im = open_im(home)?;
    for (seq, id) in rows {
        let tx_db = store.connection.clone();
        let tx = agt_storage::sqlite::write(&tx_db)?;
        let result = (|| -> Result<bool> {
            let item = read(&store.connection, &id)?;
            let (body, actor, message): (Option<String>, Option<String>, Option<String>) =
                store.connection.query_row(
                    "SELECT body,actor,message_id FROM work_reports WHERE event_seq=?1",
                    [seq],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
            if message.is_some() {
                return Ok(false);
            }
            let direct = im
                .get_principal(&PrincipalId::new(&item.request.requester))
                .is_ok_and(|p| p.kind == PrincipalKind::User)
                && item.request.origin_conversation == item.request.report_to;
            let published: Option<String> = if direct && body.is_none() {
                store.connection.query_row(
                    "SELECT message_id FROM work_publications WHERE event_seq=?1",
                    [seq],
                    |r| r.get(0),
                )?
            } else if let (Some(body), Some(actor)) = (body, actor) {
                check_scope(home, store, &item)?;
                member(&im, &item.request.origin_conversation, &actor)?;
                member(&im, &item.request.report_to, &actor)?;
                Some(im.send_message(SendMessageRequest{conversation_id:ConversationId::new(&item.request.origin_conversation),sender_id:PrincipalId::new(actor),body,client_message_id:Some(format!("work-report:{id}:{seq}")),reply_to_id:None,metadata:json!({"work_report":id,"mentions":[],"discussion_id":item.request.discussion}),idempotency_key:None})?.message.id.to_string())
            } else {
                let changed=store.connection.execute("UPDATE work_reports SET attention_at=?2 WHERE event_seq=?1 AND attention_at IS NULL AND created_at+300000<=?2",params![seq,crate::schedule_store::now_ms()])?>0;
                return Ok(changed);
            };
            if let Some(message) = published {
                store.connection.execute(
                    "UPDATE work_reports SET message_id=?2,error=NULL WHERE event_seq=?1",
                    params![seq, message],
                )?;
                store.connection.execute(
                    "UPDATE work_inbox SET state='handled' WHERE event_seq=?1 AND recipient=?2",
                    params![seq, item.request.requester],
                )?;
                return Ok(true);
            }
            Ok(false)
        })();
        if result.is_ok() {
            tx.commit()?;
        } else {
            drop(tx);
        }
        let changed=match result {Ok(changed)=>changed,Err(_)=>store.connection.execute("UPDATE work_reports SET error='report_unavailable' WHERE event_seq=?1 AND error IS NOT 'report_unavailable'",[seq])?>0};
        if changed {
            agt_storage::wake::notify(home);
        }
    }
    Ok(())
}

/// A queue barrier expiring is a decision request, never permission to skip it.
pub(super) fn expired_barrier(
    home: &Path,
    store: &ScheduleStore,
    item: WorkItem,
) -> Result<WorkItem> {
    if queue::blocker(&store.connection, &item, crate::schedule_store::now_ms())?
        != Some("dependency_deadline_exceeded")
    {
        return Ok(item);
    }
    let tx_db = store.connection.clone();
    let tx = agt_storage::sqlite::write(&tx_db)?;
    let operation = (|| -> Result<WorkItem> {
        let mut current = read(&store.connection, &item.id)?;
        if queue::blocker(&store.connection, &current, crate::schedule_store::now_ms())?
            != Some("dependency_deadline_exceeded")
        {
            return Ok(current);
        }
        let deadline = current.deferred.as_ref().and_then(|d| d.deadline);
        let exists:bool=store.connection.query_row("SELECT EXISTS(SELECT 1 FROM work_outbox WHERE work_id=?1 AND kind='dependency_expired' AND json_extract(payload,'$.deferred.deadline')=?2)",params![current.id,deadline],|r|r.get(0))?;
        if !exists {
            super::actions::save_event(
                &store.connection,
                &mut current,
                "dependency_expired",
                true,
            )?;
        }
        Ok(current)
    })();
    if operation.is_ok() {
        tx.commit()?;
    } else {
        drop(tx);
    }
    let current = operation?;
    if current.revision != item.revision {
        agt_storage::wake::notify(home);
    }
    Ok(current)
}
