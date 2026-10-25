# Permissions and approvals

Attaching a tool to an agent decides what it *can* call. Approvals decide whether a particular call *may* run. agt has two approval paths: one for native agents' tool calls and one for requests from harness agents (Codex, Claude Code, DeepSeek Harness).

## Native tool calls

Before a native agent runs a tool, the call goes through the workspace policy `permissions.mode`. This covers MCP tools, local commands and built-in tools that change something or reach outside agt. Built-in tools that only read agt's own data, such as `message_history`, `task_list`, `memory_read` or `resource_list`, run without approval in every mode, because they can only see what the agent is already allowed to see.

| Mode | Web label | Behavior |
| --- | --- | --- |
| `model_review` (default) | 模型审核，异常转人工 | A reviewer model approves or denies each call. If no reviewer model is set, or the review fails, times out or is unclear, the call goes to you. |
| `manual` | 人工审批 | Every call waits for you. |
| `auto_allow` | 自动放行 | Every call runs without review. |

::: tip The default asks you
The default mode is `model_review`, but no reviewer model is configured out of the box, so every reviewed call goes to manual approval until you choose a reviewer model.
:::

The reviewer is an ordinary model resource, separate from the agent being reviewed. It gets the tool name, the arguments and the request the agent is working on, and answers allow or deny once. A deny from the reviewer is final; anything else goes to you.

A denied call is not executed. The agent is told it was denied and continues. Calls with invalid arguments are rejected before they reach the approval queue.

MCP tools are always reviewed unless you trust their server's read-only hints; see [MCP](../modules/mcp.md#how-tool-calls-are-checked).

### Always allow a tool for an agent

When you approve a call, you can choose **Always allow this agent to use this tool** (总是允许此 Agent 使用该工具). Later calls of that tool by that agent run without asking, in every mode, and the approval history shows "a saved always-allow rule" as the reason. The rule covers one agent and one tool, with any arguments.

See and remove rules at the top of the **Approvals** (权限审批) panel, or from the CLI:

```bash
agt permission rules
agt permission forget --agent default/lead --tool framework_resource_write
```

## Harness approvals

Harness agents ask for approval themselves when a command, a file change or extra permissions fall outside their sandbox or permission mode: Codex, [Claude Code](./claude-code.md) and [DeepSeek Harness](./deepseek-harness.md) all send these requests to agt. They use `permissions.harness_mode`:

| Mode | Behavior |
| --- | --- |
| `manual` (default) | Each request waits for you. You can approve once, decline, or cancel the turn; Codex also offers approval for the session. |
| `auto_allow` | agt accepts what the harness asks for, for that request or that turn only. |

Neither mode changes the harness's sandbox setting.

## Where to approve

- **In the conversation.** The execution bar above the message box opens automatically when a request is waiting. Click it to see the tool, the arguments and the reason, then approve or deny.
- **Interactions** (交互观察) → **Approvals** (权限审批): all requests in the workspace, with filters.
- **CLI:**

```bash
agt permission list
agt permission info <id>
agt permission decide <id> --expected-revision <rev> --decision '"allow"' --reason 'looks fine'
```

`--decision` takes the JSON `value` of one of the options that `permission info` lists, such as `"allow"`, `"allow_always"`, `"deny"` or `"cancel"` for a native call.

A request that is not decided within `permissions.request_timeout_seconds` (600 by default) cancels the run.

## Changing the policy

In the Web console, set the modes and the reviewer model under **Settings** (运行设置) → **Tool approvals** (工具权限审批), or in the approvals panel. In a file:

```yaml
permissions:
  mode: model_review
  reviewer_model: default/reviewer
  review_timeout_seconds: 30
  request_timeout_seconds: 600
  harness_mode: manual
```

Changes apply to requests that are waiting now and to all later ones. Decisions already made are not undone.

## What approvals do not cover

- Approvals never grant a tool an agent does not have, and they cannot bypass membership or discussion limits.
- `resource_management` deletions use their own two-step confirmation (plan, then confirm the exact list), which the agent completes itself. Approving the tool call approves the delete.
- Once a call is approved and sent to an MCP server or a shell, it cannot be recalled.
