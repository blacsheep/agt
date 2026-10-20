//! Static metadata for every framework tool. Schemas and descriptions stay with
//! their implementations; hosts consult this table instead of matching names.

/// Hosted engines expose framework tools under this prefix.
pub const HOSTED_PREFIX: &str = "framework_";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolGroup {
    Messaging,
    Identity,
    Resource,
    Execution,
    Memory,
    Harness,
    Task,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    Read,
    Write,
}

#[derive(Debug)]
pub struct ToolSpec {
    pub name: &'static str,
    pub group: ToolGroup,
    pub effect: Effect,
    /// Starts processes or contacts the network; never run inside a database fence.
    pub external: bool,
    /// Saves a checkpoint and suspends the hosted run; must be alone in its batch.
    pub suspends: bool,
}

impl ToolSpec {
    /// Explicit runs have no conversation identity, so only workspace-level tools apply.
    pub fn explicit_run(&self) -> bool {
        !matches!(self.group, ToolGroup::Messaging | ToolGroup::Task)
    }

    /// Whether a hosted call runs inside the short scheduler fence transaction.
    /// Task and Harness tools fence inside their own runtime controls.
    pub fn fenced(&self) -> bool {
        !self.external && !matches!(self.group, ToolGroup::Task | ToolGroup::Harness)
    }

    /// Reads of agt's own data skip native approval: service authorization already
    /// limits them to what the caller may see. Writes and external calls are reviewed.
    pub fn needs_approval(&self) -> bool {
        self.effect == Effect::Write || self.external
    }
}

const fn spec(name: &'static str, group: ToolGroup, effect: Effect) -> ToolSpec {
    ToolSpec {
        name,
        group,
        effect,
        external: false,
        suspends: false,
    }
}
const fn external(name: &'static str, group: ToolGroup, effect: Effect) -> ToolSpec {
    ToolSpec {
        external: true,
        ..spec(name, group, effect)
    }
}
const fn suspending(name: &'static str) -> ToolSpec {
    ToolSpec {
        suspends: true,
        ..spec(name, ToolGroup::Task, Effect::Write)
    }
}

use Effect::{Read, Write};
use ToolGroup::*;

pub static TOOLS: &[ToolSpec] = &[
    spec("message_status", Messaging, Read),
    spec("direct_create", Messaging, Write),
    spec("group_create", Messaging, Write),
    spec("conversation_list", Messaging, Read),
    spec("conversation_hide", Messaging, Write),
    spec("conversation_show", Messaging, Write),
    spec("group_dissolve", Messaging, Write),
    spec("member_list", Messaging, Read),
    spec("member_add", Messaging, Write),
    spec("member_remove", Messaging, Write),
    spec("member_role_set", Messaging, Write),
    spec("response_policy", Messaging, Write),
    spec("message_send", Messaging, Write),
    spec("message_history", Messaging, Read),
    spec("message_inbox", Messaging, Read),
    spec("message_ack", Messaging, Write),
    spec("principal_list", Identity, Read),
    spec("principal_create", Identity, Write),
    spec("agent_bind", Identity, Write),
    spec("agent_set_enabled", Identity, Write),
    spec("agent_unbind", Identity, Write),
    spec("binding_list", Identity, Read),
    spec("resource_list", Resource, Read),
    spec("resource_read", Resource, Read),
    spec("resource_write", Resource, Write),
    spec("resource_delete", Resource, Write),
    external("provider_models", Resource, Read),
    spec("resource_test", Resource, Read),
    external("mcp_probe", Resource, Write),
    spec("agent_mcp_list", Resource, Read),
    spec("resource_category_list", Resource, Read),
    spec("resource_category_create", Resource, Write),
    spec("resource_category_delete", Resource, Write),
    external("local_exec", Execution, Write),
    spec("memory_save", Memory, Write),
    spec("memory_read", Memory, Read),
    spec("memory_forget", Memory, Write),
    spec("memory_search", Memory, Read),
    external("harness_models", Harness, Read),
    spec("harness_session_get", Harness, Read),
    external("harness_session_models", Harness, Read),
    spec("harness_session_configure", Harness, Write),
    spec("task_request", Task, Write),
    spec("task_get", Task, Read),
    spec("task_list", Task, Read),
    spec("task_defer", Task, Write),
    spec("task_reprioritize", Task, Write),
    spec("task_execution_policy", Task, Write),
    spec("task_cancel", Task, Write),
    spec("task_resolve", Task, Write),
    spec("task_retry", Task, Write),
    spec("task_reply", Task, Write),
    spec("task_report", Task, Write),
    spec("task_attention", Task, Read),
    spec("timer_create", Task, Write),
    spec("timer_list", Task, Read),
    spec("timer_cancel", Task, Write),
    spec("goal_create", Task, Write),
    spec("goal_get", Task, Read),
    spec("goal_list", Task, Read),
    spec("goal_complete", Task, Write),
    spec("goal_wait_human", Task, Write),
    spec("goal_abandon", Task, Write),
    suspending("task_await"),
    suspending("task_yield"),
    suspending("task_input"),
];

pub fn tool(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|spec| spec.name == name)
}

/// Resolve a hosted name such as `framework_message_send`.
pub fn hosted_tool(name: &str) -> Option<&'static ToolSpec> {
    name.strip_prefix(HOSTED_PREFIX).and_then(tool)
}

pub fn hosted_name(name: &str) -> String {
    format!("{HOSTED_PREFIX}{name}")
}

/// Rewrite bare framework tool names in model-facing text to the names a hosted
/// engine actually exposes, so descriptions never point at an uncallable tool.
pub fn hosted_text(text: &str) -> String {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(word) {
        let (before, tail) = rest.split_at(start);
        out.push_str(before);
        let end = tail.find(|c: char| !word(c)).unwrap_or(tail.len());
        let (token, after) = tail.split_at(end);
        if tool(token).is_some() {
            out.push_str(HOSTED_PREFIX);
        }
        out.push_str(token);
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_text_prefixes_only_whole_tool_names() {
        assert_eq!(
            hosted_text(
                "Use task_get, then task_report. framework_task_get stays; task_getter and my_task_get do not change."
            ),
            "Use framework_task_get, then framework_task_report. framework_task_get stays; task_getter and my_task_get do not change."
        );
        assert_eq!(
            hosted_text("agent_bind(principal)"),
            "framework_agent_bind(principal)"
        );
        assert_eq!(
            hosted_text("任务用 task_get 查询"),
            "任务用 framework_task_get 查询"
        );
    }
}
