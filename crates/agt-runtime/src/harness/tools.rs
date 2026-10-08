//! AGT framework tool policy and audit stages; Codex owns its internal tools.
use super::*;
use agt_service::ToolService;

#[derive(Default)]
pub(super) struct ToolState {
    pub(super) failures: u32,
    pub(super) awaiting: bool,
}
pub(super) struct ToolBridge<'a> {
    pub(super) home: &'a Path,
    pub(super) task: Option<&'a ScheduledTask>,
    pub(super) service: Option<&'a ToolService>,
    pub(super) check: &'a dyn Fn() -> Result<()>,
    pub(super) audit_record: &'a RunRecord,
    pub(super) store: &'a RunStore,
    pub(super) current_chat: Option<String>,
    pub(super) state: ToolState,
}
impl ToolBridge<'_> {
    pub(super) fn call(&mut self, name: &str, args: Value) -> anyhow::Result<Value> {
        let (service, name) = self.prepare_tool_call(name, &args)?;
        let tool_call = self.before_tool_call(name, &args)?;
        self.store.save_tool_detail(
            &tool_call.run_id,
            &format!("call:{}", tool_call.id),
            &[("arguments", &args)],
        )?;
        let result = self.execute_tool_call(service, name, args);
        self.after_tool_call(name, tool_call, result)
    }
    fn prepare_tool_call<'a>(
        &self,
        name: &'a str,
        args: &Value,
    ) -> anyhow::Result<(&ToolService, &'a str)> {
        let service = self.service;
        let current_chat = &self.current_chat;
        (self.check)()?;
        if self.state.awaiting {
            anyhow::bail!("Wait registered. End this turn now; do not call further tools.");
        }
        let service = service.ok_or_else(|| anyhow::anyhow!("framework tools unavailable"))?;
        let name = name
            .strip_prefix("framework_")
            .ok_or_else(|| anyhow::anyhow!("unknown framework tool"))?;
        if name == "message_send"
            && current_chat
                .as_ref()
                .is_some_and(|chat| args["conversation"] == chat.as_str())
        {
            anyhow::bail!(
                "Return your final answer to reply in the current chat; AGT publishes it automatically. message_send may contact other conversations only."
            );
        }
        Ok((service, name))
    }
    fn before_tool_call(&self, name: &str, args: &Value) -> Result<crate::CallRecord> {
        let audit_record = self.audit_record;
        let store = self.store;
        let mut tool_record = crate::CallRecord::new(
            audit_record,
            Some(name),
            crate::audit::argument_summary(args, &[]),
        )
        .with_public_context(args);
        tool_record.kind = "framework".into();
        store.start_call(tool_record)
    }
    fn execute_tool_call(&self, service: &ToolService, name: &str, args: Value) -> Result<Value> {
        let home = self.home;
        let task = self.task;
        let invoke = || service.call(name, args).map_err(error);
        if let Some(t) = task
            && name != "mcp_probe"
            && !name.starts_with("harness_")
            && !matches!(
                name,
                "task_request"
                    | "task_get"
                    | "task_list"
                    | "task_defer"
                    | "task_reprioritize"
                    | "task_execution_policy"
                    | "task_cancel"
                    | "task_retry"
                    | "task_resolve"
                    | "task_await"
                    | "task_yield"
                    | "task_input"
                    | "task_reply"
                    | "task_report"
                    | "task_attention"
            )
        {
            crate::scheduler::guarded(home, t, invoke)
        } else {
            invoke()
        }
    }
    fn after_tool_call(
        &mut self,
        name: &str,
        mut tool_call: crate::CallRecord,
        result: Result<Value>,
    ) -> anyhow::Result<Value> {
        if matches!(name, "task_await" | "task_yield" | "task_input") && result.is_ok() {
            self.state.awaiting = true;
        }
        let state = if result.is_ok() {
            crate::CallState::Succeeded
        } else {
            self.state.failures += 1;
            crate::CallState::Failed
        };
        if let Ok(value) = &result {
            tool_call.summarize_result(value);
        }
        tool_call.finish(
            state,
            result.as_ref().err().map(crate::audit::runtime_error),
        );
        let output = match &result {
            Ok(value) => value.clone(),
            Err(error) => crate::audit::runtime_error(error),
        };
        self.store.save_tool_detail(
            &tool_call.run_id,
            &format!("call:{}", tool_call.id),
            &[("output", &output)],
        )?;
        self.store.finish_call(&tool_call)?;
        result.map_err(Into::into)
    }
}
