//! All calls follow batch validation -> prepare -> before -> execute -> after.
//! Framework authorization remains in ToolService; hooks cannot bypass it.
use super::*;
use crate::audit::{argument_summary, runtime_error};
use crate::engine::{Delegation, check_canceled};
use crate::{CallRecord, CallState};

impl NativeRun<'_, '_> {
    /// Validate the complete batch before recording or executing any tool effect.
    pub(super) fn prepare_tool_batch(&mut self, message: &Value) -> Result<Vec<ToolCall>> {
        let calls = validate_message(message)?;
        if calls.is_empty()
            && message["content"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
        {
            return Err(RuntimeError::Model("empty final response".into()));
        }
        if calls.len() > 1 && calls.iter().any(|c| suspends(&c.function.name)) {
            return Err(RuntimeError::Tool(
                "framework_task_await/framework_task_yield/framework_task_input must be the only tool call in its batch".into(),
            ));
        }
        // Validate the entire batch before allowing any side effect.
        for call in &calls {
            if !self.transcript.seen_calls.insert(call.id.clone()) {
                return Err(RuntimeError::Model("duplicate tool call ID".into()));
            }
            if call.function.name == "delegate" && !self.record.request.delegates.is_empty() {
                let args: Value = serde_json::from_str(&call.function.arguments)?;
                if args["agent"].as_str().is_some_and(|agent| {
                    !self
                        .record
                        .request
                        .delegates
                        .iter()
                        .any(|allowed| allowed == agent)
                }) || args["input"]
                    .as_str()
                    .is_some_and(|input| input.trim().is_empty() || input.len() > 256 * 1024)
                {
                    return Err(RuntimeError::Tool(
                        "delegation target or input is not allowed".into(),
                    ));
                }
            } else if !self.tools.targets.contains_key(&call.function.name)
                && !self.is_framework(&call.function.name)
            {
                return Err(RuntimeError::Tool("unknown or unavailable tool".into()));
            }
        }
        Ok(calls)
    }

    pub(super) fn execute_tool_batch(&mut self, calls: Vec<ToolCall>) -> Result<ToolFlow> {
        crate::progress::stage(
            self.record,
            &self.runtime.store,
            self.runtime.scheduled_task.as_ref(),
            "tool_running",
        )?;
        let mut refresh = false;
        for call in calls {
            check_canceled(self.cancellation)?;
            self.runtime.check_schedule()?;
            let (arguments, invocation) = self.prepare_tool_call(&call)?;
            if let Err(error) = self.tools.validators[&call.function.name].validate(&arguments) {
                // Invalid arguments are repairable tool outcomes, never approval requests.
                let invocation = self.before_tool_call(invocation)?;
                let error = if self.is_framework(&call.function.name) {
                    json!(agt_service::ToolError::from_contract(error))
                } else {
                    json!(error)
                };
                let output = json!({"error": error}).to_string();
                self.after_tool_call(&call, invocation, Ok(ToolOutput::failed(output, error)))?;
                continue;
            }
            let scope =
                crate::approval::Scope::new(self.record, self.runtime.scheduled_task.as_ref());
            let (allowed, policy_changed) = self.permissions.authorize(
                crate::permission::Request {
                    scope: &scope,
                    call: &call.id,
                    tool: &call.function.name,
                    arguments: &arguments,
                    input: &self.record.request.input,
                },
                self.cancellation,
                &|| self.runtime.check_schedule(),
            )?;
            if policy_changed {
                // The fingerprint records the policy actually applied, so resume checks
                // compare against what this Run used.
                self.record.config_fingerprint = Some(super::refresh::execution_fingerprint(
                    &self.agent,
                    self.agents,
                    self.record.parent_run_id.is_none(),
                    &self.permissions,
                    &self.tools.schemas,
                )?);
                self.runtime.store.save(self.record)?;
            }
            if !allowed {
                self.transcript.messages.push(json!({"role":"tool","tool_call_id":call.id,"content":"Permission denied. This tool was not executed. Do not repeat the same request without new authorization."}));
                continue;
            }
            let invocation = self.before_tool_call(invocation)?;
            self.runtime.store.save_tool_detail(
                &self.record.id,
                &format!("call:{}", invocation.id),
                &[("arguments", &arguments)],
            )?;
            let affects_capabilities = self.affects_capabilities(&call, &arguments);
            let mut result = self.execute_tool_call(&call, arguments);
            if affects_capabilities
                && let Ok(output) = &mut result
                && output.error.is_none()
            {
                refresh = true;
                let mut value: Value = serde_json::from_str(&output.text)?;
                value["runtime_application"] =
                    json!({"state":"pending","boundary":"after_tool_batch"});
                output.text = value.to_string();
            }
            if let ToolFlow::Suspend = self.after_tool_call(&call, invocation, result)? {
                return Ok(ToolFlow::Suspend);
            }
        }
        if refresh {
            self.refresh_capabilities()?;
        }
        Ok(ToolFlow::Continue)
    }

    fn prepare_tool_call(&self, call: &ToolCall) -> Result<(Value, CallRecord)> {
        let arguments: Value = serde_json::from_str(&call.function.arguments)?;
        let schema = self.framework.as_ref().and_then(|framework| {
            framework.service.schemas().into_iter().find(|s| {
                agt_service::catalog::hosted_name(s["name"].as_str().expect("tool name"))
                    == call.function.name
            })
        });
        let fields = schema
            .as_ref()
            .and_then(|s| s["inputSchema"]["properties"].as_object())
            .map(|p| p.keys().map(String::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let invocation = CallRecord::new(
            self.record,
            Some(&call.function.name),
            argument_summary(&arguments, &fields),
        )
        .with_public_context(&arguments);
        Ok((arguments, invocation))
    }

    fn before_tool_call(&self, invocation: CallRecord) -> Result<CallRecord> {
        check_canceled(self.cancellation)?;
        self.runtime.check_schedule()?;
        self.runtime.store.start_call(invocation)
    }

    fn execute_tool_call(&mut self, call: &ToolCall, arguments: Value) -> Result<ToolOutput> {
        if call.function.name == "delegate" {
            let args: Delegation = serde_json::from_str(&call.function.arguments)?;
            Ok(ToolOutput::ok(self.runtime.delegate(
                self.record,
                args,
                &call.id,
                self.agents,
                self.steps.count,
                self.cancellation,
            )?))
        } else if self.is_framework(&call.function.name) {
            self.execute_framework_call(call, arguments)
        } else {
            let output = self
                .tools
                .call(&call.function.name, arguments, self.cancellation)?;
            let error = (serde_json::from_str::<Value>(&output)?["is_error"] == true)
            .then(|| json!({"code":"mcp_tool_error","message":"External MCP tool reported an error; inspect its result before retrying."}));
            Ok(ToolOutput {
                text: output,
                error,
                diagnostic: None,
            })
        }
    }

    fn after_tool_call(
        &mut self,
        call: &ToolCall,
        mut invocation: CallRecord,
        result: Result<ToolOutput>,
    ) -> Result<ToolFlow> {
        if let Ok(output) = &result
            && output.error.is_none()
            && let Ok(value) = serde_json::from_str(&output.text)
        {
            invocation.summarize_result(&value);
        }
        match &result {
            Ok(ToolOutput { error: None, .. }) => invocation.finish(CallState::Succeeded, None),
            Ok(ToolOutput {
                error: Some(error), ..
            }) => invocation.finish(CallState::Failed, Some(error.clone())),
            Err(error) => invocation.finish(
                if matches!(error, RuntimeError::Canceled | RuntimeError::Stopped(_)) {
                    CallState::Canceled
                } else {
                    CallState::Failed
                },
                Some(runtime_error(error)),
            ),
        }
        let (output, diagnostic) = match &result {
            Ok(output) => (
                serde_json::from_str(&output.text)
                    .unwrap_or_else(|_| Value::String(output.text.clone())),
                output.diagnostic.clone(),
            ),
            Err(error) => (runtime_error(error), error.diagnostic()),
        };
        let diagnostic = diagnostic.map(Value::String);
        let mut fields = vec![("output", &output)];
        fields.extend(diagnostic.as_ref().map(|d| ("diagnostic", d)));
        self.runtime.store.save_tool_detail(
            &self.record.id,
            &format!("call:{}", invocation.id),
            &fields,
        )?;
        self.runtime.store.finish_call(&invocation)?;
        if invocation.state != CallState::Succeeded {
            self.record.tool_failures += 1;
            self.runtime.store.save(self.record)?;
        }
        let output = result?.text;
        let output = crate::context::truncate_output(
            &output,
            (crate::context::input_budget(&self.agent) / 16).max(128),
        );
        self.transcript
            .messages
            .push(json!({"role":"tool", "tool_call_id":call.id, "content":output}));
        Ok(
            if suspends(&call.function.name) && invocation.state == CallState::Succeeded {
                ToolFlow::Suspend
            } else {
                ToolFlow::Continue
            },
        )
    }
}

fn suspends(name: &str) -> bool {
    agt_service::catalog::hosted_tool(name).is_some_and(|t| t.suspends)
}

fn validate_message(message: &Value) -> Result<Vec<ToolCall>> {
    if message["role"] != "assistant"
        || (!message["content"].is_null() && !message["content"].is_string())
    {
        return Err(RuntimeError::Model(
            "expected a text assistant message".into(),
        ));
    }
    let calls: Vec<ToolCall> = if message["tool_calls"].is_null() {
        Vec::new()
    } else {
        serde_json::from_value(message["tool_calls"].clone())
            .map_err(|e| RuntimeError::Model(crate::Reason::because("invalid tool calls", e)))?
    };
    if calls.len() > 32 {
        return Err(RuntimeError::Model(
            "too many tool calls in one response".into(),
        ));
    }
    for call in &calls {
        if call.kind != "function" || call.id.trim().is_empty() || call.id.len() > 256 {
            return Err(RuntimeError::Model("invalid function call identity".into()));
        }
        serde_json::from_str::<Value>(&call.function.arguments).map_err(|e| {
            RuntimeError::Model(crate::Reason::because("tool arguments must be JSON", e))
        })?;
    }
    Ok(calls)
}
