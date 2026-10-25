# Tool service

| Overview | |
| --- | --- |
| **What it is** | agt's tools, available to scripts and external MCP clients |
| **CLI** | `agt tool`, `agt serve`, `agt events` |
| **Crate** | `agt-service` |

## What it does

The tools agents use (messages, resources, tasks, memory and so on) are provided by one tool service. The same service is available outside agents in three ways:

| Command | Use it for |
| --- | --- |
| `agt tool call` | Calling one tool from a shell or script |
| `agt serve` | Exposing the tools to an MCP client over stdio |
| `agt events watch` | Following changes as a stream of JSON lines |

The caller's identity is fixed by whoever starts the command (`--principal`), never by tool arguments. `--manager` adds the resource and binding tools for a trusted local process.

## How to use it

```bash
agt tool list --principal me --manager
agt tool call --principal me --manager resource_list --args '{"kind":"agent"}'
agt events watch --principal me
```

MCP client configuration:

```json
{
  "mcpServers": {
    "agt": { "command": "/absolute/path/to/agt", "args": ["serve", "--principal", "me", "--manager"] }
  }
}
```

See [Use agt from MCP clients](../guide/mcp-clients.md).

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `--principal <id>` | The identity to act as; must exist |
| `--manager` | Add resource and binding tools |
| `--args <json>` | Tool arguments |

## All parameters

### `agt tool`

| Command | Options | Meaning |
| --- | --- | --- |
| `list` | `--principal <id>` (required), `--manager` | Tools available to this identity, with their JSON schemas |
| `call <tool>` | `--principal <id>` (required), `--manager`, `--args <json>` (default `{}`) | Call one tool. Business errors are returned as `{"error": {"code", "message", "field"}}`. |

### `agt serve`

| Option | Meaning |
| --- | --- |
| `--principal <id>` | Required. The identity of every call made through this server. |
| `--manager` | Add resource and binding tools |

`agt serve` stores and delivers messages but does not run agents. Agents reply only when a discussion is authorized (Web console or `agt schedule start`) and a scheduler is running.

### `agt events watch`

| Option | Meaning |
| --- | --- |
| `--principal <id>` | Required. Only changes this identity may see. |
| `--conversation <id>` | Only one conversation |
| `--after <cursor>` | Resume after a cursor from an earlier event |

Prints one JSON line per change, such as a new message or a task update, after an initial `ready` line. Events are hints: read the details with the normal tools. If you receive `resync_required`, reload your view and continue from the new cursor. Events are kept for 7 days.

### Which tools are available

| Group | Without `--manager` | With `--manager` | Details |
| --- | --- | --- | --- |
| Conversations and messages | yes | yes | [Identities and conversations](./conversations.md#agent-tools) |
| Memory | yes | yes | [Memory](./memory.md#all-parameters) |
| Tasks (own tasks only) and timers | yes | yes | [Tasks](./tasks.md#all-parameters) |
| Resources, categories | | yes | [Resources](./resources.md#agent-tools) |
| Identities and bindings | | yes | [Agent](./agent.md#identities-and-bindings) |
| Harness sessions | | yes | [Harness](./harness.md#agent-tools) |

Inside agent runs, tool names carry a `framework_` prefix, for example `framework_message_send`. Agents get the resource, identity and harness tools from `resource_management` instead of `--manager`.

### Error format

```json
{"error":{"code":"invalid_arguments","message":"Reply target must exist in this conversation; omit/null for a new message.","field":"reply_to"}}
```

Errors never echo argument values or secrets.

## Related modules

- [Agent](./agent.md): the same tools inside agent runs
- [Scheduler](./scheduler.md): what makes agents reply to messages sent through the service
