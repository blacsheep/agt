//! Scheduled-task transitions. These operations only mutate the in-memory record.
//! Callers retain admission checks, transactions, accounting and persistence.
use crate::TaskState;
use crate::participation::ParticipationDecision;
use crate::schedule_store::{RecordedDecision, now_ms};
use crate::{RunRecord, RunState, ScheduledTask};
use agt_service::InputMode;
use serde_json::Value;

/// Upper bound for a wait whose members never settle; normally a peer finishing wakes it.
pub(crate) const WAIT_WATCHDOG_MS: i64 = 300_000;

/// Reasons to release a claimed task without consuming its reply allowance.
/// The store still owns the atomic refund and epoch check.
pub(crate) enum ClaimDeferral {
    DisabledBinding,
    ResponsePolicy,
    InactiveDiscussion,
    StaleDecision,
    ConversationQuiet { last_message_at: i64 },
    QueueTriage,
}

impl ScheduledTask {
    pub(crate) fn defer_claim(&mut self, cause: ClaimDeferral) {
        self.state = TaskState::Queued;
        match cause {
            ClaimDeferral::DisabledBinding => {
                self.reason = Some("task blocked by disabled binding".into())
            }
            ClaimDeferral::ResponsePolicy => {
                self.reason = Some("task blocked by response policy".into())
            }
            ClaimDeferral::InactiveDiscussion => {}
            ClaimDeferral::StaleDecision => {
                self.due_at = now_ms();
                self.reason = None;
            }
            ClaimDeferral::ConversationQuiet { last_message_at } => {
                self.due_at = self.deadline.min(last_message_at + self.quiet_ms);
            }
            ClaimDeferral::QueueTriage => {
                self.reason = Some("queue triage committed; awaiting priority admission".into());
                self.due_at = now_ms();
            }
        }
        // Keep owner/version until ScheduleStore::update validates the old claim.
    }

    pub(crate) fn skip_ineligible_message(&mut self) {
        self.state = TaskState::Skipped;
        self.reason = Some("response policy or message state".into());
    }

    pub(crate) fn ready_external_reply(&mut self) {
        self.state = TaskState::ReadyToReply;
        self.reason = Some(
            "framework: external engine runs only for direct messages or explicit mentions".into(),
        );
        self.due_at = now_ms();
        self.decision_required = false;
    }

    pub(crate) fn interrupt_publication(&mut self) {
        self.state = TaskState::Interrupted;
        self.reason =
            Some("reply publication stopped or unavailable; inspect run before retrying".into());
    }

    /// Resume a blocked task without changing its claim, wait settings or epoch.
    pub(crate) fn resume_pending(&mut self, now: i64) {
        self.state = TaskState::Queued;
        self.due_at = now;
        self.reason = None;
    }

    pub(crate) fn wake_for_human(&mut self, now: i64) {
        self.resume_pending(now);
        self.quiet = false;
        self.owner = None;
        self.version += 1;
    }

    pub(crate) fn invalidate_ready_snapshot(&mut self, now: i64) {
        self.resume_pending(now);
        self.owner = None;
        self.version += 1;
    }

    /// `retry`: the interrupted reply changed nothing, so it may run again.
    pub(crate) fn recover_expired_lease(&mut self, retry: bool) {
        self.state = if self.state == TaskState::Evaluating || retry {
            TaskState::Queued
        } else {
            TaskState::Interrupted
        };
        self.reason = Some("coordinator lease expired".into());
        self.owner = None;
        self.version += 1;
    }

    pub(crate) fn claim_execution(&mut self, replying: bool, epoch: i64, owner: &str) {
        self.state = if replying {
            TaskState::Responding
        } else {
            TaskState::Evaluating
        };
        self.usage_epoch = epoch;
        self.owner = Some(owner.into());
        self.version += 1;
    }

    /// Cancellation of pending work is distinct from asking a running engine to stop.
    pub(crate) fn cancel_pending(&mut self) -> bool {
        if !matches!(
            self.state,
            TaskState::Queued
                | TaskState::Waiting
                | TaskState::ReadyToReply
                | TaskState::Limited
                | TaskState::Blocked
        ) {
            return false;
        }
        self.state = TaskState::Canceled;
        self.reason = Some("discussion ended, member removed or conversation unavailable".into());
        true
    }

    pub(crate) fn record_decision(
        &mut self,
        decision: &ParticipationDecision,
    ) -> serde_json::Result<()> {
        let value = serde_json::to_value(decision)?;
        self.reason = value["reason_code"].as_str().map(str::to_owned);
        self.last_decision = Some(RecordedDecision {
            decision: decision.clone(),
            decided_at: now_ms(),
            through_seq: self.through_seq,
        });
        Ok(())
    }

    /// A scheduled decision releases its slot; participation never executes inline.
    /// A wait blocks on the members who were actively working when it was decided.
    pub(crate) fn finish_scheduled_decision(
        &mut self,
        decision: &ParticipationDecision,
        active: &[String],
    ) -> serde_json::Result<()> {
        self.record_decision(decision)?;
        match decision {
            ParticipationDecision::Silent { .. } => self.state = TaskState::Silent,
            ParticipationDecision::Participate { .. } => {
                self.state = TaskState::ReadyToReply;
                self.due_at = now_ms();
                self.quiet = false;
            }
            ParticipationDecision::Wait { .. } => {
                self.wait_on(active);
                self.decision_required = true;
            }
        }
        Ok(())
    }

    /// Block until one of `members` finishes its current step or a new message arrives.
    /// The watchdog only covers a member that never settles.
    fn wait_on(&mut self, members: &[String]) {
        self.state = TaskState::Waiting;
        self.blocked_on = members.to_vec();
        self.quiet = false;
        self.due_at = now_ms() + WAIT_WATCHDOG_MS;
        self.deadline = self.due_at;
    }

    pub(crate) fn wait_from_input(&mut self, members: &[String]) {
        self.wait_on(members);
        self.owner = None;
    }

    pub(crate) fn defer_input(&mut self) {
        self.state = TaskState::Queued;
        self.owner = None;
        self.due_at = now_ms();
    }

    pub(crate) fn accept_run_input(
        &mut self,
        parent: &Self,
        run: &str,
        mode: InputMode,
    ) -> serde_json::Result<()> {
        self.state = TaskState::Input;
        self.owner = parent.owner.clone();
        self.version += 1;
        self.run_id = Some(run.into());
        self.reason = Some(format!(
            "{} accepted by active run",
            serde_json::to_value(mode)?
        ));
        Ok(())
    }

    /// Whether this task means its member is working now. A queued task counts only when
    /// it will run without first deciding to participate; deciding and waiting do not.
    pub(crate) fn is_active(&self) -> bool {
        match self.state {
            TaskState::ReadyToReply | TaskState::Responding | TaskState::Input => true,
            TaskState::Queued => !self.decision_required,
            _ => false,
        }
    }

    /// Join a newer peer message to this pending decision. The next decision sees both
    /// messages, and the reply, if any, answers the newest.
    pub(crate) fn absorb(&mut self, newer: &Self) {
        self.triggers.extend(newer.triggers.iter().cloned());
        self.deliveries.extend(newer.deliveries.iter().copied());
        self.message = newer.message.clone();
        if self.state == TaskState::Waiting {
            self.due_at = newer.due_at;
        } else {
            self.due_at = self.due_at.min(newer.due_at);
        }
        self.state = TaskState::Queued;
        self.blocked_on.clear();
        self.quiet = false;
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            TaskState::Completed
                | TaskState::Silent
                | TaskState::Skipped
                | TaskState::Failed
                | TaskState::Canceled
                | TaskState::Interrupted
                | TaskState::OutcomeUnknown
        )
    }

    /// Caller selects only uncompleted accepted inputs; completed turns stay final.
    pub(crate) fn settle_with_parent(&mut self, parent: &Self) {
        self.state = parent.state;
        self.reason = parent.reason.clone();
        // Retain the accepted Run even when a takeover has no Run attached to parent.
        self.owner = None;
    }

    pub(crate) fn complete_input_turn(&mut self, has_reply: bool) {
        self.state = if has_reply {
            TaskState::Completed
        } else {
            TaskState::Silent
        };
        self.owner = None;
    }

    pub(crate) fn finish_work_run(&mut self, record: &RunRecord) {
        self.state = match record.state {
            RunState::Completed => TaskState::Completed,
            RunState::Suspended => TaskState::Suspended,
            RunState::Running | RunState::Interrupted => TaskState::Interrupted,
            RunState::Canceled => TaskState::Canceled,
            RunState::Failed => TaskState::Failed,
            RunState::OutcomeUnknown => TaskState::OutcomeUnknown,
        };
        self.reason = record.error.clone();
    }

    pub(crate) fn finish_unsuccessful_reply(&mut self, record: &RunRecord) {
        self.state = match record.state {
            RunState::Running | RunState::Interrupted => TaskState::Interrupted,
            RunState::Canceled => TaskState::Canceled,
            RunState::OutcomeUnknown => TaskState::OutcomeUnknown,
            _ => TaskState::Failed,
        };
        self.reason = record.error.clone();
    }

    pub(crate) fn finish_published_reply(&mut self, output: &str, obligation: Option<&Value>) {
        self.state = if obligation.is_some_and(|o| o["report_required"] == true) {
            self.reason = Some("task_report_required: execution completed but the result report was not committed; inspect task_get and submit task_report".into());
            TaskState::Failed
        } else if output.trim() == "[[silent]]" {
            if self.work_notification && obligation.is_none() {
                self.reason = Some("task notification was not handled; review task inbox".into());
                TaskState::Failed
            } else if self.work_notification {
                TaskState::Completed
            } else {
                TaskState::Silent
            }
        } else {
            TaskState::Completed
        };
    }
}
