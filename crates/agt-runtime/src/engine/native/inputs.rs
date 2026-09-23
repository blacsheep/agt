//! Poll only at safe boundaries; model retries and individual tools never drain inputs.
use super::*;
use agt_service::InputMode;

impl NativeRun<'_, '_> {
    pub(super) fn next_input(
        &mut self,
        mode: InputMode,
    ) -> Result<Option<crate::run_inputs::RunInput>> {
        if self.record.parent_run_id.is_some() {
            return Ok(None);
        }
        let Some(task) = self.runtime.scheduled_task.clone() else {
            return Ok(None);
        };
        loop {
            check_canceled(self.cancellation)?;
            let Some(input) =
                crate::run_inputs::take(&self.runtime.home, &task, &self.record.id, mode)?
            else {
                return Ok(None);
            };
            if input.task.decision_required {
                // This private execution context stays inside this principal/conversation.
                let decision_input =
                    json!({"event":input.content,"execution_context":self.messages}).to_string();
                let mut audit =
                    crate::CallRecord::new(self.record, None, json!({"input_task":input.task.id}));
                audit.kind = "participation".into();
                let mut audit = self.runtime.store.start_call(audit)?;
                let result =
                    crate::participation::decide(self.runtime.model, self.agent, &decision_input);
                audit.finish(
                    match &result {
                        Ok(_) => crate::CallState::Succeeded,
                        Err(RuntimeError::Canceled | RuntimeError::Stopped(_)) => {
                            crate::CallState::Canceled
                        }
                        Err(_) => crate::CallState::Failed,
                    },
                    result.as_ref().err().map(crate::audit::runtime_error),
                );
                self.runtime.store.finish_call(&audit)?;
                let decision = match result {
                    Ok(decision) => decision,
                    Err(error @ (RuntimeError::Canceled | RuntimeError::Stopped(_))) => {
                        return Err(error);
                    }
                    Err(error) => {
                        crate::run_inputs::fail_decision(
                            &self.runtime.home,
                            &task,
                            &input,
                            &error,
                        )?;
                        continue;
                    }
                };
                check_canceled(self.cancellation)?;
                if !crate::run_inputs::decide(&self.runtime.home, &task, &input, &decision)? {
                    continue;
                }
            }
            return Ok(Some(input));
        }
    }

    pub(super) fn accept_input(&mut self, input: crate::run_inputs::RunInput) {
        self.reply = Some((
            agt_im::MessageId::new(input.task.message),
            input.reply_depth,
        ));
        if input.mode == InputMode::FollowUp {
            // A new task may legitimately send the same text as the previous one.
            self.sent_messages.clear();
        }
        let message = json!({"role":"user","content":input.content.to_string()});
        // Pin additional requests through compaction just like the initial request.
        self.input_messages.push(message.clone());
        self.messages.push(message);
        // A reply to an earlier input must not suppress a later input's final reply.
        self.published = false;
    }

    pub(super) fn publish_completed_turn(
        &mut self,
        output: String,
        next_input: &str,
    ) -> Result<()> {
        check_canceled(self.cancellation)?;
        let reply = self.finish_turn(output);
        let body = match &reply {
            FinalReply::Text(body) if body.trim() != "[[silent]]" => Some(body.as_str()),
            _ => None,
        };
        let task = self
            .runtime
            .scheduled_task
            .as_ref()
            .expect("hosted input queue");
        crate::run_inputs::complete_turn(
            &self.runtime.home,
            task,
            &self.record.id,
            self.completed_turns,
            body,
            self.reply.as_ref(),
            next_input,
        )?;
        self.completed_turns += 1;
        Ok(())
    }
}
