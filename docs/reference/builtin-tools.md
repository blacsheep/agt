# Built-in tools

These are the tools agt provides to agents. The same tools are available to people and
scripts through `agt tool call` and `agt serve`. Inside an agent run their names carry a
`framework_` prefix, for example `framework_message_send`.

Parameters marked `?` are optional. Run `agt tool list --principal <id> --manager` for the full
JSON schemas.

## Who gets which tools

| Group | Hosted agents | `agt tool` / `agt serve` |
| --- | --- | --- |
| Conversations, messages | Always | Always |
| Memory | Always (unless `memory.enabled: false`) | Always |
| Resources, identities, bindings, Codex sessions | With `resource_management` | With `--manager` |
| Tasks | With `task_collaboration` | Always (limited to the identity's own tasks) |
| Waiting inside a task | During a task run | — |
| Local commands | With `local_execution` and `execution.enabled` (native only) | — |

## Conversations and messages

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `principal_list` | `include_deleted?` | List people and agents |
| `direct_create` | `principal` | Open (or find) the direct chat with someone |
| `group_create` | `title`, `members` | Create a group; the caller becomes admin, the workspace owner becomes owner |
| `conversation_list` | `groups_only?`, `include_hidden?` | List your conversations |
| `conversation_hide`, `conversation_show` | `conversation` | Hide or restore your own entry |
| `member_list` | `conversation` | List members |
| `member_add`, `member_remove` | `conversation`, `principal` | Invite or remove (owner/admin), or leave yourself |
| `group_dissolve` | `conversation` | Dissolve a group you own |
| `response_policy` | `conversation`, `policy` | Set your own policy here: `all`, `mentioned` or `manual` |
| `message_send` | `conversation`, `body`, `request_key`, `mentions?`, `reply_to?` | Send a message; repeating a `request_key` with the same content does not send twice |
| `message_history` | `conversation`, `before?`, `limit?` | Read messages; pass `next_cursor` as `before` for older ones |
| `message_inbox`, `message_ack` | | Deliveries for external clients |
| `message_status` | `message_id` | Whether each recipient has processed a message |

## Memory

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `memory_search` | `kind?` (`messages` or `notes`), `query?`, `conversation?`, `day?`, `since?`, `until?`, `before?`, `limit?` | Search your visible history or your notes |
| `memory_save` | `id`, `topic`, `summary`, `source_messages`, `expected_revision` | Create (`0`) or update a note |
| `memory_read` | `id` | Read a note with its sources |
| `memory_forget` | `id`, `expected_revision` | Delete a note |

See [Memory](../guide/memory.md).

## Resource management

`kind` is one of `provider`, `model`, `prompt`, `skill`, `mcp`, `harness`, `agent`.
`reference` is `name` or `category/name`.

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `resource_list` | `kind` | List resources (secrets redacted) |
| `resource_read` | `kind`, `reference` | Read one resource, including its `revision` |
| `resource_write` | `kind`, `reference`, `create`, `data`, `expected_revision?`, `add?`, `remove?`, `clear?` | Create or update a resource |
| `resource_delete` | `kind`, `reference`, `recursive?`, `confirm?` | Without `confirm`: get the deletion plan. With `confirm` set to the exact plan: delete |
| `resource_test` | `kind` (`provider` or `mcp`), `reference` | Validate the configuration only |
| `provider_models` | `reference` | List the provider's models |
| `mcp_probe` | `reference` | Start or connect to an MCP server, list and check its tools |
| `agent_mcp_list` | `agent` | List an agent's MCP servers |
| `resource_category_list`, `resource_category_create`, `resource_category_delete` | `kind`, `category?` | Manage categories |

`data` uses the same fields as the resource files. Examples:

```json
{"kind":"prompt","reference":"critic","create":true,"data":{"text":"You challenge every claim..."}}
{"kind":"agent","reference":"critic","create":true,
 "data":{"model":"fast","prompt":"critic","builtin_tools":[],"description":"Devil's advocate"}}
{"kind":"agent","reference":"critic","create":false,"data":{},"add":{"mcps":["search"]}}
{"kind":"provider","reference":"main","create":true,
 "data":{"base_url":"https://api.openai.com/v1","api_key":"sk-..."}}
```

Pass `expected_revision` from `resource_read` to make an update fail instead of overwriting
someone else's newer change.

### Identities and bindings

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `principal_create` | `id`, `name`, `kind` | Create an identity. Agents with `resource_management` can create only `kind: agent` |
| `agent_bind` | `principal`, `agent`, `working_directory?`, `policy?` | Connect an identity to a definition and enable it |
| `agent_set_enabled` | `principal`, `enabled` | Enable or disable an agent everywhere |
| `agent_unbind` | `principal` | Remove the binding (needed before deleting the agent) |
| `binding_list` | | List bindings and whether they are enabled |

`resource_write` for an agent returns an `activation` summary: whether the agent is configured,
bound and enabled, and the next steps if not. Creating the definition alone does not make the
agent chat.

### Codex sessions

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `harness_models` | `reference` | Models and reasoning levels a harness reports |
| `harness_session_get` | `conversation`, `target` | The Codex session of an agent in a conversation |
| `harness_session_models` | `conversation`, `target`, `session` | Choices available to that session |
| `harness_session_configure` | `conversation`, `target`, `session`, `model`, `effort?` | Change the session's model and reasoning level |

The caller and the target must both be members of the conversation.

## Task collaboration

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `task_request` | `assignee`, `conversation`, `input`, `request_key`, `priority?`, `execution_policy?` | Queue a task for an agent; both must be in `conversation` |
| `task_get` | `task_id` | State, result, report state and blocking reason |
| `task_list` | `conversation?`, `queued_only?`, `limit?`, `before?` | Tasks you requested or were assigned |
| `task_report` | `task_id`, `expected_revision`, `body` | Report a result to the original conversation |
| `task_reprioritize` | `task_id`, `expected_revision`, `priority`, `reason` | `low`, `normal`, `high` (`urgent` for people) |
| `task_defer` | `task_id`, `expected_revision`, `reason`, `not_before?` or `after_task?` + `deadline?` | Delay a queued task |
| `task_execution_policy` | `task_id`, `expected_revision`, `reason`, `execution_policy` | Parallelism and exclusive resources |
| `task_cancel` | `task_id`, `expected_revision`, `reason` | Cancel a task and its own subtasks |
| `task_retry` | `task_id`, `expected_revision`, `request_key`, `reason` | New task from a failed or canceled one |
| `task_reply` | `task_id`, `input_request_id`, `expected_revision`, `answer`, `request_key` | Answer a task's question |
| `task_resolve` | `task_id`, `expected_revision`, `outcome`, `evidence` | People only: record the real outcome of an `outcome_unknown` task |
| `task_attention` | `limit?`, `before?` | Workspace owner only: tasks that need a human |

Available only inside a task run:

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `task_await` | `task_id` or `task_ids`, `deadline?` | Wait for subtasks; must be the only call in its step |
| `task_yield` | `not_before`, `reason` | Pause until a time |
| `task_input` | `question`, `deadline` | Ask the requester a question and wait |

Times are Unix milliseconds. See [Task collaboration](../guide/tasks.md).

## Local commands

| Tool | Parameters | Purpose |
| --- | --- | --- |
| `local_exec` | `command`, `directory?` | Run a Bash command on the host |

See [Local commands](../guide/local-execution.md).
