//! Cancellation fences and ordered engine event persistence.
use super::*;
impl HarnessRun<'_> {
    pub(super) fn check(&self) -> Result<()> {
        let prepared = self.prepared;
        let sessions = &prepared.sessions;
        let scope = &prepared.scope;
        let execution_lock = &self.execution_lock;
        let home = self.home;
        let task = self.task;
        let token = self.token;
        execution_lock
            .check()
            .map_err(|_| RuntimeError::Stopped(crate::StopCode::HarnessLeaseLost))?;
        if let Some(task) = task {
            crate::scheduler::guarded(home, task, || Ok(()))?;
        }
        if sessions.canceled(scope).map_err(error)? {
            execution_lock.canceling().map_err(error)?;
            return Err(RuntimeError::Stopped(
                crate::StopCode::CancellationRequested,
            ));
        }
        if token.is_canceled() {
            execution_lock.canceling().map_err(error)?;
            return Err(RuntimeError::Stopped(crate::StopCode::token(token)));
        }
        Ok(())
    }
    pub(super) fn event(
        &self,
        record: &mut RunRecord,
        store: &RunStore,
        event: Event<'_>,
    ) -> anyhow::Result<()> {
        let prepared = self.prepared;
        let sessions = &prepared.sessions;
        let scope = &prepared.scope;
        let execution_lock = &self.execution_lock;
        let session_db = &sessions.db;
        let history = &prepared.history;
        let task = self.task;
        // Diagnostic events must survive the fence that initiated cancellation.
        if let Event::Stopping(error) = &event {
            record
                .termination
                .get_or_insert_with(|| crate::Termination::from_error(error));
            store.save(record)?;
            return Ok(());
        }
        if matches!(event, Event::InterruptRequested) {
            execution_lock.canceling()?;
            if let Some(termination) = &mut record.termination {
                termination.interrupt_requested_at = Some(crate::schedule_store::now_ms());
            }
            store.save(record)?;
            return Ok(());
        }
        // A cancellation acknowledgement must still be processed after cancellation.
        if matches!(event, Event::Settled(_)) {
            execution_lock.check()?;
        } else {
            self.check()?;
        }
        crate::progress::harness_event(record, store, &event)?;
        match event {
            Event::Text(text) => {
                crate::progress::update(record, store, crate::ModelEvent::Text(text))?
            }
            Event::Tool => crate::progress::update(record, store, crate::ModelEvent::Tool)?,
            Event::Thread(id) => {
                record.execution.as_mut().unwrap()["thread_id"] = json!(id);
                store.save(record)?;
                execution_lock.fenced(|| {
                    session_db.execute(
                        "UPDATE harness_sessions SET thread=?2 WHERE scope=?1",
                        params![scope, id],
                    )?;
                    Ok(())
                })?;
            }
            Event::Turn(id) => {
                execution_lock.running()?;
                record.execution.as_mut().unwrap()["turn_id"] = json!(id);
                store.save(record)?;
            }
            Event::ReasoningEffort(effort) => {
                execution_lock.fenced(|| sessions.resolve_default(scope, None, Some(effort)))?;
                record.execution.as_mut().unwrap()["reasoning_effort"] = json!(effort);
                store.save(record)?;
            }
            Event::Model(model) => {
                execution_lock.fenced(|| sessions.resolve_default(scope, Some(model), None))?;
                record.execution.as_mut().unwrap()["model"] = json!(model);
                store.save(record)?;
            }
            Event::Submitted => {
                record.execution.as_mut().unwrap()["submitted"] = json!(true);
                store.save(record)?;
                execution_lock.submitted()?;
            }
            Event::Settled(status) => {
                if let Some(termination) = &mut record.termination {
                    termination.confirmation = format!("engine_{status}");
                    termination.confirmed_at = Some(crate::schedule_store::now_ms());
                }
                record.execution.as_mut().unwrap()["turn_status"] = json!(status);
                store.save(record)?;
                if matches!(status, "failed" | "interrupted") {
                    // Advance acknowledged input before releasing the execution fence.
                    execution_lock.fenced(|| {
                        session_db.execute("UPDATE harness_sessions SET through_seq=?2,source_hash=?3 WHERE scope=?1",
                            params![scope, task.map(|t| t.through_seq).unwrap_or(0), crate::fingerprint(&serde_json::to_vec(&history)?)])?;
                        Ok(())
                    })?;
                }
                execution_lock.settled(status)?;
            }
            Event::Accepted => execution_lock.running()?,
            Event::ToolDetail { id, item } => store.save_harness_detail(&record.id, id, item)?,
            Event::Protocol | Event::Activity { .. } => {}
            Event::Completed => self.completed.set(true),
            Event::Tick | Event::Stopping(_) | Event::InterruptRequested => {}
        }
        Ok(())
    }
}
