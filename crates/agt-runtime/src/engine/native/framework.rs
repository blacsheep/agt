//! Framework-specific publication, idempotency and side-effect fencing.
use super::*;

impl NativeRun<'_, '_> {
    pub(super) fn execute_framework_call(
        &mut self,
        call: &ToolCall,
        arguments: Value,
    ) -> Result<(String, Option<Value>)> {
        let publishes_here = call.function.name == "framework_message_send"
            && self.record.request.trigger.as_ref().is_some_and(|t| {
                arguments["conversation"].as_str() == Some(t.conversation_id.as_str())
            });
        // Within this Run, identical successful sends share the first key even
        // if the model invents a new call ID/key. Keep service validation and
        // membership checks on every invocation rather than returning a cache.
        let mut arguments = arguments;
        let send_identity = if call.function.name == "framework_message_send" {
            serde_json::from_value::<agt_service::MessageInput>(arguments.clone())
                .ok()
                .filter(|input| {
                    !input.request_key.trim().is_empty() && input.request_key.len() <= 256
                })
                .map(|mut input| {
                    input.mentions.sort();
                    input.mentions.dedup();
                    input.reply_to = input.reply_to.filter(|id| !id.as_str().trim().is_empty());
                    let requested_key = std::mem::take(&mut input.request_key);
                    (
                        serde_json::to_string(&input).expect("serializable message input"),
                        requested_key,
                    )
                })
        } else {
            None
        };
        if let Some((identity, _)) = &send_identity
            && let Some(key) = self.sent_messages.get(identity)
        {
            arguments["request_key"] = json!(key);
        }
        if publishes_here
            && self
                .runtime
                .scheduled_task
                .as_ref()
                .is_some_and(|t| t.work_id.is_some())
        {
            return Err(RuntimeError::Tool(
                "Return your task result as the final answer; do not message_send to the current conversation",
            ));
        }
        let operation = || {
            Ok(self
                .communication
                .as_ref()
                .expect("communication service")
                .call(
                    call.function
                        .name
                        .strip_prefix(agt_service::catalog::HOSTED_PREFIX)
                        .expect("tool prefix"),
                    arguments,
                ))
        };
        let fenced = agt_service::catalog::hosted_tool(&call.function.name)
            .expect("validated framework tool")
            .fenced();
        let result = if let Some(task) = &self.runtime.scheduled_task
            && fenced
        {
            crate::scheduler::guarded(&self.runtime.home, task, operation)?
        } else {
            operation()?
        };
        match result {
            Ok(value) => {
                if let Some((identity, key)) = send_identity {
                    self.sent_messages.entry(identity).or_insert(key);
                }
                self.published |= publishes_here;
                Ok((serde_json::to_string(&value)?, None))
            }
            Err(error) => Ok((
                error.as_value().to_string(),
                Some(serde_json::to_value(&error)?),
            )),
        }
    }
}
