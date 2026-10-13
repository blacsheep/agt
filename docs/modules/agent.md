# Agent

| Overview | |
| --- | --- |
| **What it is** | An agent definition, plus the identity and binding that let it take part in conversations |
| **CLI** | `agt agent`; bindings through `agt tool call` |
| **Agent tools** | `resource_*` with `kind: "agent"`, `principal_create`, `agent_bind`, `agent_set_enabled`, `agent_unbind`, `binding_list` |
| **Web console** | **Agent 团队** (Agent team) |
| **File** | `agents/<category>/<name>/agent.yaml` |
| **Crate** | `agt-agent`, `agt-service` |

## What it does

An agent definition combines an engine (a [model](./model.md) or a [harness](./harness.md)) with
a [prompt](./prompt.md), [skills](./skill.md), [MCP servers](./mcp.md) and built-in capabilities.

A definition alone does nothing. To chat, an agent also needs an **identity** (a participant in
conversations) and a **binding** that connects the identity to the definition and holds its
running settings. Several identities can be bound to the same definition and run in parallel.

## How to use it

**Web console:** **Agent 团队** → **创建角色** creates the definition, the identity and the
binding in one step. Use **编辑角色** to change the definition and **配置与管理** for the binding.

**CLI:**

```bash
agt agent create reviewer --model fast --prompt reviewer --skill code-review --mcp github
agt tool call --principal me --manager principal_create --args '{"id":"reviewer","name":"Reviewer","kind":"agent"}'
agt tool call --principal me --manager agent_bind --args '{"principal":"reviewer","agent":"reviewer"}'
agt agent update reviewer --add-builtin-tool task_collaboration
```

**Agent tools:** `resource_write` (definition), `principal_create` (identity), `agent_bind`
(binding). `resource_write` returns an `activation` summary that says whether the agent is
bound and enabled, and what to do next.

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `--model` or `--harness` | The engine (exactly one) |
| `--prompt` | Role prompt |
| `--builtin-tool` | `resource_management`, `task_collaboration`, `local_execution` |
| `policy` (binding) | When it replies: `mentioned`, `all`, `manual` |

## All parameters

### `agt agent create <name>`

| Option | Repeatable | Meaning |
| --- | --- | --- |
| `--model <ref>` | | Native engine with this model (or `--harness`) |
| `--harness <ref>` | | Codex engine with this harness (or `--model`) |
| `--prompt <ref>` | | Role prompt |
| `--skill <ref>` | yes | Attach a skill |
| `--mcp <ref>` | yes | Attach an MCP server (native only) |
| `--builtin-tool <capability>` | yes | Grant a capability |
| `--description <text>` | | Shown in the console and to other agents |
| `--category <c>` | | Category (default `default`) |

### `agt agent update <name>`

| Option | Meaning |
| --- | --- |
| `--model <ref>` / `--harness <ref>` | Switch the engine |
| `--prompt <ref>` | Change the prompt |
| `--description <text>` | Change the description |
| `--add-skill`, `--remove-skill <ref>` | Add or remove one skill (repeatable) |
| `--add-mcp`, `--remove-mcp <ref>` | Add or remove one MCP server (repeatable) |
| `--add-builtin-tool`, `--remove-builtin-tool <capability>` | Grant or revoke a capability (repeatable) |
| `--category <c>` | Category of the agent to update |

Changes apply from the agent's next run. A run in progress keeps its configuration, except that
an agent's own MCP and skill changes load after its current batch of tool calls.

### `agt agent run <name>`

Runs the agent once outside any conversation.

| Option | Default | Meaning |
| --- | --- | --- |
| `--input <text>` / `--stdin` | | The request (exactly one) |
| `--request-key <key>` | generated | Repeating the same key with the same request returns the earlier result instead of running again |
| `--cwd <dir>` | `.` | Working directory for the run |
| `--delegate <ref>` | | Agents this run may hand work to (repeatable). Delegated agents cannot delegate further. |

Results are recorded as [runs](./runs.md). A non-`completed` status exits with a non-zero code.

`list`, `info`, `delete` and `category` are described in [Resources](./resources.md#shared-cli-commands).

### File: `agent.yaml`

```yaml
name: reviewer
description: Reviews code changes
model: { category: default, name: fast }
harness: null
prompt: { category: default, name: reviewer }
skills: [code-review]
mcps:
  - { category: default, name: github }
builtin_tools: [task_collaboration]
```

| Field | Meaning |
| --- | --- |
| `model` / `harness` | Exactly one is set |
| `prompt` | Role prompt reference |
| `skills` | Skill packages |
| `mcps` | MCP server references |
| `builtin_tools` | Granted capabilities |
| `description` | Free text |

### Tool fields (`resource_write`, `kind: "agent"`)

| Field | Meaning |
| --- | --- |
| `model` or `harness` | Exactly one on create |
| `prompt`, `skills`, `mcps`, `builtin_tools`, `description` | Optional. Lists replace the current list; an empty list removes everything. |

Use `add` / `remove` (for `skills`, `mcps`, `builtin_tools`) to change single items, and
`clear` to reset `description`, `skills`, `mcps` or `builtin_tools`.

### Built-in capabilities

| Capability | Web label | Gives the agent |
| --- | --- | --- |
| `resource_management` | 资源管理 | All [resource](./resources.md) tools, agent identities and bindings, Codex session tools |
| `task_collaboration` | 任务协作 | The [task](./tasks.md) tools |
| `local_execution` | 本机命令 | The [local command](./local-execution.md) tool (native only; also needs `execution.enabled`) |

Capabilities are not inherited by agents an agent creates. Messaging and
[memory](./memory.md) tools are always available.

### Identities and bindings

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `principal_create` | `id`, `name`, `kind` | Create an identity. Agents with `resource_management` can only create `kind: "agent"`. |
| `agent_bind` | `principal`, `agent`, `working_directory?`, `policy?` | Bind an identity to a definition. New bindings start enabled. Rebinding an existing identity updates it and keeps a disabled binding disabled. |
| `agent_set_enabled` | `principal`, `enabled` | Enable or disable the agent in all conversations, keeping its settings and history. Enabling may process pending messages. |
| `agent_unbind` | `principal` | Remove the binding. Required before deleting a bound agent. |
| `binding_list` | | All bindings and whether they are enabled |

Binding settings:

| Setting | Default | Meaning |
| --- | --- | --- |
| `working_directory` | the home | Default directory for local commands. Codex agents always use the harness directory. |
| `policy` | `mentioned` | `mentioned`: direct chats and mentions. `all`: considers every message. `manual`: never replies automatically. Codex agents save `all` as `mentioned`. |

These tools need `--manager` from the CLI, or `resource_management` for agents.

## Related modules

- [Identities and conversations](./conversations.md): where agents talk
- [Scheduler](./scheduler.md): when agents run
- [Resources](./resources.md): deleting and categories
