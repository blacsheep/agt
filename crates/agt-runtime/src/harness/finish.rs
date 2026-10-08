//! Confirmed results, audit settlement and resumable work checkpoints.
use super::*;
impl HarnessRun<'_> {
    pub(super) fn finish(
        &self,
        record: &mut RunRecord,
        store: &mut RunStore,
        mut invocation: crate::CallRecord,
        state: &tools::ToolState,
        result: anyhow::Result<String>,
    ) -> Result<String> {
        let prepared = self.prepared;
        let sessions = &prepared.sessions;
        let scope = &prepared.scope;
        let execution_lock = &self.execution_lock;
        let session_db = &sessions.db;
        let history = &prepared.history;
        let task = self.task;
        let home = self.home;
        let session = &self.session;
        let harness = &session.config;
        let thread = &self.thread;
        if let Some(progress) = &record.progress {
            record.execution.as_mut().unwrap()["activity"] = json!({"items":progress.activities,"local_checked_at":progress.local_checked_at,"protocol_at":progress.protocol_at,"meaningful_at":progress.meaningful_at});
        }
        if result.is_ok() && thread.is_none() && !session.title.is_empty() {
            let saved = sessions.get(scope).map_err(error)?;
            if let Some(thread) = saved.session_id {
                let mut metadata_config = harness.clone();
                metadata_config.timeout_seconds = 10;
                if Codex::rename(&metadata_config, &thread, &session.title).is_err() {
                    record.execution.as_mut().unwrap()["title_sync_failed"] = json!(true);
                }
            }
        }
        // A completed result may win the race against a cancellation request.
        let completion_won = result.is_ok()
            && record
                .execution
                .as_ref()
                .is_some_and(|e| e["turn_status"] == "completed");
        let result = (if completion_won {
            execution_lock.check().map_err(error)
        } else {
            self.check()
        })
        .and_then(|_| {
            if state.awaiting && self.completed.get() {
                Ok(String::new())
            } else {
                result.map_err(error)
            }
        });
        invocation.finish(
            if result.is_ok() {
                crate::CallState::Succeeded
            } else if matches!(
                result,
                Err(RuntimeError::Canceled | RuntimeError::Stopped(_))
            ) {
                crate::CallState::Canceled
            } else {
                crate::CallState::Failed
            },
            result.as_ref().err().map(crate::audit::runtime_error),
        );
        store.finish_call(&invocation)?;
        record.tool_failures = state.failures;
        let output = result?;
        let save = || {
            execution_lock
                .fenced(|| {
                    session_db.execute(
                        "UPDATE harness_sessions SET through_seq=?2,source_hash=?3 WHERE scope=?1",
                        params![
                            scope,
                            task.map(|t| t.through_seq).unwrap_or(0),
                            crate::fingerprint(&serde_json::to_vec(&history)?)
                        ],
                    )?;
                    Ok(())
                })
                .map_err(error)
        };
        // Preserve the engine's confirmed result even if publication authorization was revoked.
        save()?;
        if state.awaiting {
            if let Some(t) = task {
                crate::scheduler::guarded(home, t, || Ok(()))?;
            }
            let session = sessions.get(scope).map_err(error)?;
            record.config_fingerprint = prepared.fingerprint(&session.config)?;
            record.state = crate::RunState::Suspended;
            record.progress = None;
            store.save_checkpoint(
                record,
                &crate::work::wait::Checkpoint::Codex {
                    session_record: scope.clone(),
                    thread_id: session
                        .session_id
                        .ok_or_else(|| error("completed turn has no thread"))?,
                    config_fingerprint: record.config_fingerprint.clone(),
                },
            )?;
            execution_lock
                .finish(
                    crate::harness_sessions::ExecutionState::Succeeded,
                    Some(&output),
                )
                .map_err(error)?;
            return Ok(String::new());
        }
        execution_lock
            .finish(
                crate::harness_sessions::ExecutionState::Succeeded,
                Some(&output),
            )
            .map_err(error)?;
        Ok(output)
    }
}
