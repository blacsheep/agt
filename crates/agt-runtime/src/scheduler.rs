//! Coordinator and stable scheduler entry points; stages live in focused modules.
use crate::{ChatCompletions, ScheduleStore, schedule_store::now_ms};
use crate::{DiscussionState, TaskState};
use agt_core::CancellationToken;
use agt_im::{
    ConversationId, ConversationState, DeliveryState, ImStore, MessageListOptions, PrincipalId,
};
use agt_service::BindingStore;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

fn im(home: &Path) -> Result<ImStore> {
    Ok(ImStore::open(crate::store::database_path(
        home, "im", "im.db",
    )?)?)
}

mod admission;
mod backend;
mod completion;
mod controls;
mod decision;
mod execution;
mod guard;
mod ingress;
mod lease;

use admission::quiet_ready;
pub use controls::{
    delete_scheduled_conversation, dissolve_scheduled_group, reset_conversation_context,
    start_discussion,
};
pub(crate) use execution::execute;
pub(crate) use guard::{guarded, guarded_store};
pub(crate) use ingress::scan_inputs;
use lease::HomeLease;

pub struct Scheduler {
    home: PathBuf,
    owner: String,
    store: ScheduleStore,
    concurrency: usize,
    jobs: Vec<(String, CancellationToken, JoinHandle<()>)>,
    lease: HomeLease,
}

impl Scheduler {
    pub fn open(home: &Path, concurrency: usize) -> Result<Self> {
        if concurrency == 0 {
            bail!("concurrency must be positive");
        }
        let lease = HomeLease::acquire(home)?;
        let store = ScheduleStore::open(home)?;
        Ok(Self {
            home: home.into(),
            owner: lease.owner.clone(),
            store,
            concurrency,
            jobs: Vec::new(),
            lease,
        })
    }

    pub fn run(&mut self, watch: bool, token: &CancellationToken) -> Result<()> {
        // Subscribe before the first snapshot: commits during tick leave a queued hint.
        let mut listener = agt_storage::wake::Hints::open(&self.home);
        loop {
            if token.is_canceled() {
                for (_, cancel, _) in &self.jobs {
                    cancel.cancel_with_reason("service_shutdown");
                }
            }
            self.tick(!token.is_canceled())?;
            if (token.is_canceled() || !watch) && self.jobs.is_empty() {
                break;
            }
            let waiting = std::time::Instant::now();
            let mut deadline = waiting + self.next_wake_delay()?;
            loop {
                if token.is_canceled() || self.jobs.iter().any(|(_, _, job)| job.is_finished()) {
                    break;
                }
                // Recheck availability during the wait: the socket can fail after open.
                if !listener.available() {
                    deadline = deadline.min(waiting + Duration::from_secs(1));
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() || listener.wait(remaining.min(Duration::from_millis(250))) {
                    break;
                }
            }
        }
        self.store.release(&self.owner)?;
        Ok(())
    }

    fn next_wake_delay(&self) -> Result<Duration> {
        let now = now_ms();
        // Repair missed hints without using full inbox scans as the normal path.
        let mut next = now + 30_000;
        let im = im(&self.home)?;
        for task in self.store.candidates()? {
            let mut due = task.due_at;
            if task.work_id.is_none()
                && task.quiet
                && let Ok(page) = im.list_messages(
                    &ConversationId::new(&task.conversation),
                    &PrincipalId::new(&task.principal),
                    MessageListOptions {
                        limit: 1,
                        before: None,
                        after: None,
                    },
                )
                && let Some(message) = page.messages.last()
            {
                due = due.max(task.deadline.min(message.created_at + task.quiet_ms));
            }
            if due > now {
                next = next.min(due);
            }
        }
        let timer: Option<i64> = self.store.connection.query_row(
            "SELECT MIN(due) FROM (
                SELECT expires AS due FROM discussions WHERE state='active'
                UNION ALL SELECT deadline FROM work_waits WHERE outcome IS NULL
                UNION ALL SELECT created_at+300000 FROM work_reports WHERE body IS NULL AND message_id IS NULL AND attention_at IS NULL
                UNION ALL SELECT json_extract(record,'$.deferred.not_before') FROM work_items WHERE json_extract(record,'$.state')='queued'
                UNION ALL SELECT json_extract(record,'$.deferred.deadline') FROM work_items WHERE json_extract(record,'$.state')='queued'
             ) WHERE due>?1", [now], |r| r.get(0))?;
        if let Some(timer) = timer {
            next = next.min(timer);
        }
        Ok(Duration::from_millis((next - now).max(1) as u64))
    }

    fn tick(&mut self, dispatch: bool) -> Result<()> {
        if !self.store.heartbeat(&self.owner, now_ms())? {
            for (_, token, _) in &self.jobs {
                token.cancel_with_reason("scheduler_lease_lost");
            }
            bail!("scheduler lease lost; active work requires inspection");
        }
        self.poll_jobs()?;
        crate::work::reconcile(&self.home)?;
        self.scan()?;
        let im = im(&self.home)?;
        self.store.expire(now_ms())?;
        self.reconcile_deliveries(&im)?;
        if !dispatch {
            return Ok(());
        }
        self.dispatch_ready(&im)
    }

    fn poll_jobs(&mut self) -> Result<()> {
        let mut i = 0;
        while i < self.jobs.len() {
            if self.jobs[i].2.is_finished() {
                let (id, _, handle) = self.jobs.remove(i);
                if handle.join().is_err() {
                    let mut task = self.store.get(&id)?;
                    task.state = TaskState::Interrupted;
                    task.reason = Some("execution thread interrupted".into());
                    self.store.update(&task, now_ms())?;
                }
            } else {
                i += 1;
            }
        }
        for (id, token, _) in &self.jobs {
            let task = self.store.get(id)?;
            if !task.work_notification
                && let Some(work_id) = &task.work_id
            {
                let item = crate::work::read(&self.store.connection, work_id)?;
                if item.cancellation_requested.is_some()
                    || crate::work::check_task_scope(&self.home, &self.store, work_id).is_err()
                {
                    token.cancel_with_reason(if item.cancellation_requested.is_some() {
                        "task_canceled"
                    } else {
                        "authorization_revoked"
                    });
                }
            }
        }
        Ok(())
    }

    fn reconcile_deliveries(&mut self, im: &ImStore) -> Result<()> {
        for task in self.store.unreconciled()? {
            if task.work_event.as_deref() == Some("input_requested")
                && crate::work::notice_blocked(&self.store.connection, &task)?
            {
                self.store.cancel_pending(&task.id)?;
            }
            let round = self.store.discussion(&task.discussion)?;
            let active = im
                .get_conversation(&ConversationId::new(&task.conversation))
                .is_ok_and(|c| c.state == ConversationState::Active);
            let member = im
                .get_member(
                    &ConversationId::new(&task.conversation),
                    &PrincipalId::new(&task.principal),
                )
                .is_ok_and(|m| m.left_message_seq.is_none());
            let bound = BindingStore::open(&self.home)?
                .get(&PrincipalId::new(&task.principal))?
                .is_some();
            let ended = round.as_ref().is_none_or(|r| {
                r.state == DiscussionState::Ended
                    || !im
                        .get_conversation(&ConversationId::new(&r.conversation))
                        .is_ok_and(|c| c.state == ConversationState::Active)
            });
            if ended || !active || !member || (!bound && task.work_id.is_none()) {
                self.store.cancel_pending(&task.id)?;
                for (id, token, _) in &self.jobs {
                    if id == &task.id {
                        token.cancel_with_reason("authorization_revoked");
                    }
                }
            }
            if !bound && task.work_id.is_some() {
                for (id, token, _) in &self.jobs {
                    if id == &task.id {
                        token.cancel_with_reason("authorization_revoked");
                    }
                }
            }
            // Reconcile the separate IM transaction after crashes or failed writes.
            let state = match task.state.as_str() {
                "completed" => Some(DeliveryState::Completed),
                "silent" | "skipped" | "canceled" => Some(DeliveryState::Skipped),
                "failed" | "interrupted" | "outcome_unknown" => Some(DeliveryState::Failed),
                _ => None,
            };
            if let Some(state) = state {
                for id in &task.deliveries {
                    im.update_delivery(*id, state, task.run_id.as_deref(), task.reason.as_deref())?;
                }
                if task.work_notification && task.state == TaskState::Completed {
                    self.store.connection.execute("UPDATE work_inbox SET state='handled' WHERE recipient=?1 AND event_seq IN (SELECT p.event_seq FROM work_publications p JOIN work_outbox e ON e.seq=p.event_seq WHERE e.work_id=?2 AND p.message_id=?3)",rusqlite::params![task.principal,task.work_id,task.message])?;
                }
                self.store.mark_reconciled(&task.id)?;
            }
        }
        Ok(())
    }

    fn dispatch_ready(&mut self, im: &ImStore) -> Result<()> {
        for candidate in self.store.candidates()? {
            if !crate::work::runnable(&self.home, &self.store, &candidate)? {
                continue;
            }
            if self.jobs.len() >= self.concurrency {
                break;
            }
            let now = now_ms();
            if candidate.due_at > now {
                continue;
            }
            // Read message timestamps, not conversation.updated_at: read receipts
            // and membership events must not keep resetting the quiet window.
            let activity = im.list_messages(
                &ConversationId::new(&candidate.conversation),
                &PrincipalId::new(&candidate.principal),
                MessageListOptions {
                    limit: 1,
                    before: None,
                    after: None,
                },
            );
            let Ok(page) = activity else { continue };
            let Some(latest) = page.messages.last() else {
                continue;
            };
            if !quiet_ready(&candidate, latest.created_at, now) {
                continue;
            }
            if candidate.state == TaskState::ReadyToReply && candidate.through_seq != latest.seq {
                self.store
                    .invalidate_ready(&candidate.id, latest.seq, now)?;
                continue;
            }
            if let Some(task) = self.store.claim(&candidate.id, &self.owner, now)? {
                let home = self.home.clone();
                let token = CancellationToken::new();
                let child_token = token.clone();
                let id = task.id.clone();
                let handle = thread::spawn(move || {
                    let mut model = ChatCompletions;
                    if execute(&home, task.clone(), &mut model, &child_token).is_err()
                        && let Ok(mut store) = ScheduleStore::open(&home)
                        && let Ok(mut current) = store.get(&task.id)
                        && current.version == task.version
                        && matches!(current.state, TaskState::Evaluating | TaskState::Responding)
                    {
                        current.state = TaskState::Failed;
                        current.reason = Some(
                            "Agent execution failed; inspect model, binding or run details".into(),
                        );
                        let _ = store.update(&current, now_ms());
                    }
                    agt_storage::wake::notify(&home);
                });
                self.jobs.push((id, token, handle));
            }
        }
        Ok(())
    }

    fn scan(&mut self) -> Result<()> {
        crate::HarnessSessions::open(&self.home)?.reconcile()?;
        scan_inputs(&self.home, &mut self.store)
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        for (_, token, _) in &self.jobs {
            token.cancel_with_reason("service_shutdown");
        }
        // Do not release the lease while detached threads may still be running.
        self.lease.release = self.jobs.is_empty();
    }
}

#[cfg(test)]
mod tests;
