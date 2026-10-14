# Harness

| Overview | |
| --- | --- |
| **What it is** | An external execution engine (Codex) and the project directory it works in |
| **CLI** | `agt harness` |
| **Agent tools** | `resource_*` with `kind: "harness"`, `harness_models`, `harness_session_*` |
| **Web console** | **Resources** → Harness; **External harness** in the agent form; `/model`, `/status` and other commands in conversations |
| **File** | `harnesses/<category>/<name>/harness.yaml` |
| **Crate** | `agt-harness` |

## What it does

A harness lets an agent run on [Codex](https://github.com/openai/codex) instead of agt's own model loop. Codex brings file editing, shell commands and its own sandbox, so Codex agents can write and test code, including MCP servers for the rest of the team.

agt starts `codex app-server`, keeps one Codex session per agent per conversation, forwards Codex's approval requests to the [approval queue](./permissions.md), and publishes Codex's final answer as the agent's reply. Codex uses its own login and configuration; agt stores no Codex credentials.

## How to use it

1. Install Codex and run `codex login` as the user that runs agt.
2. Create the harness:

```bash
agt harness resource create toolsmith --file ./toolsmith.json
agt harness resource test toolsmith
agt harness models toolsmith
```

3. Create an agent on it: `agt agent create toolsmith --harness toolsmith --prompt toolsmith`.
4. Talk to it in a direct chat, or mention it in a group.

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `working_directory` | The project Codex works in |
| `sandbox` | `read_only` or `workspace_write` |
| `model`, `reasoning_effort` | Defaults for new sessions |

## All parameters

### File: `harness.yaml` (JSON when created from the CLI)

```json
{
  "engine": "codex",
  "executable": "/usr/local/bin/codex",
  "working_directory": "/home/me/projects/tools",
  "sandbox": "workspace_write",
  "model": "gpt-5.5",
  "reasoning_effort": "medium",
  "timeout_seconds": 1800,
  "description": "Builds MCP servers for the team"
}
```

| Field | Required | Default | Meaning |
| --- | --- | --- | --- |
| `engine` | yes | | `codex` (the only engine so far) |
| `executable` | yes | | Absolute path of an existing Codex binary |
| `working_directory` | yes | | Absolute path of an existing directory |
| `sandbox` | yes | | `read_only` or `workspace_write` |
| `model` | no | Codex's default | Default model for new sessions |
| `reasoning_effort` | no | Codex's default | Default reasoning level for new sessions |
| `timeout_seconds` | no | `1800` | Time limit for one turn, 10 to 86400 |
| `description` | no | | At most 4096 bytes |

Unknown fields are rejected.

### `agt harness resource`

| Command | Options | Meaning |
| --- | --- | --- |
| `create <name>` | `--file <json>` (full object), `--category <c>` | Create a harness |
| `update <name>` | `--file <json>` (fields to change), `--category <c>` | Update a harness |
| `test <ref>` | `--category <c>` | Check the configuration without running a model |
| `list`, `info`, `delete`, `category` | | See [Resources](./resources.md#shared-cli-commands) |

### Inspecting Codex

| Command | Meaning |
| --- | --- |
| `agt harness models <ref>` | Models and reasoning levels Codex reports for this harness |
| `agt harness skills <ref>` | Skills Codex finds in the project |
| `agt harness mcp <ref>` | MCP servers configured in Codex, and their status |
| `agt harness capabilities` | Which engine operations agt supports |

### `agt harness session`

Each agent has one session per conversation. Sessions keep the configuration they started with; changing the harness affects new sessions only.

| Command | Options | Meaning |
| --- | --- | --- |
| `list` | | All sessions |
| `current` | `--principal <agent>` `--conversation <id>` | Get or prepare the session for an agent in a conversation (does not start Codex) |
| `info <id>` | | Session details |
| `models <id>` | | Models available to this session |
| `skills <id>`, `mcp <id>` | | Skills and MCP servers seen by this session |
| `configure <id>` | `--model <m>` (required), `--effort <level>` | Change model and reasoning level for this session, from the next turn |
| `rename <id> <title>` | | Name the Codex thread |
| `compact <id>` | | Ask Codex to compact its context and wait (may call the model) |
| `cancel <id>` | | Stop the running turn. File changes already made remain. |

### Conversation commands

`/model`, `/status`, `/permissions`, `/compact`, `/rename`, `/skills`, `/mcp`, `/new`, `/stop`. See [Conversations](./web-console.md#message-box-commands).

### Agent tools

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `harness_models` | `reference` | Models and reasoning levels of a harness |
| `harness_session_get` | `conversation`, `target` | The session of agent `target` in a conversation |
| `harness_session_models` | `conversation`, `target`, `session` | Choices for that session |
| `harness_session_configure` | `conversation`, `target`, `session`, `model`, `effort?` | Change that session's model and reasoning level |

These need `resource_management`. The caller and `target` must both be members of the conversation. Sessions that are running or need review reject changes.

### What Codex agents support

| Supported | Not supported |
| --- | --- |
| Role prompt and skills | agt MCP resources |
| `resource_management`, `task_collaboration`, messaging, memory, tasks | `local_execution` |
| Response policies `mentioned` and `manual` | Response policy `all` |
| Streaming preview, `/stop`, `/new` | `/review`, `/diff`, importing or forking Codex threads |

## Related modules

- [Agent](./agent.md): choosing a harness instead of a model
- [Permissions](./permissions.md#all-parameters): approving Codex's requests
