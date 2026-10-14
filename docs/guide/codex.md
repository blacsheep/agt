# Codex agents

A Codex agent runs on a [Codex](https://github.com/openai/codex) session instead of agt's own model loop. Codex brings file editing, shell commands and its own sandbox, so Codex agents are the right choice for writing and testing code, including the MCP servers your other agents use.

## Requirements

- The Codex CLI installed on the machine that runs agt. agt starts it as `codex app-server`.
- Codex signed in under the same user account that runs agt (`codex login`). agt does not store Codex credentials; it uses Codex's own login and configuration, including any custom model provider configured there.

## Create a harness

A **harness** resource describes how to start Codex and where it works:

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

`working_directory` must be an existing absolute directory, and `sandbox` decides whether Codex may write files there. `model` and `reasoning_effort` are optional defaults for new sessions. All fields are listed in the [Harness module](../modules/harness.md#all-parameters).

**Web console:** **Resources** → **New resource** → Harness. After saving, use **Load models** (读取可用模型) to see the models and reasoning levels Codex reports, and **检查连接** (Check connection) to test the setup without running a model.

**CLI:**

```bash
agt harness resource create toolsmith --file ./toolsmith.json
agt harness resource test toolsmith
agt harness models toolsmith           # models and reasoning levels reported by Codex
```

## Create the agent

Choose the harness instead of a model:

```bash
agt agent create toolsmith --harness toolsmith --prompt toolsmith
```

In the Web form, set **Engine** (执行方式) to **External harness** (外部 Harness) and pick the harness. Then chat with the agent like any other: in a direct chat, or by mentioning it in a group.

## What Codex agents can and cannot use

| Available | Not available |
| --- | --- |
| Role prompt and skills (sent to Codex as instructions) | agt MCP resources (configure MCP in Codex instead) |
| `resource_management` and `task_collaboration` tools | `local_execution` (Codex has its own shell) |
| Messaging, memory and task tools | Response policy `all` (only `mentioned` and `manual`) |
| Streaming preview, `/stop` and `/new` | |

Codex asks for approval when it needs to go beyond its sandbox. These requests appear in the same approval queue as native tool calls, and are handled by the **Harness** approval mode; see [Permissions](./permissions.md#codex-approvals).

## Sessions

Each Codex agent keeps one session per conversation. The session remembers the model and reasoning level it started with:

- Changing the harness defaults affects only new sessions.
- `/model` in a conversation changes the model and reasoning level for that agent in that conversation only.
- `/new` ends the session and starts a fresh one with the current defaults. The chat history and the files in the project directory are kept.

Other commands for Codex agents: `/status`, `/permissions`, `/compact`, `/rename`, `/skills`, `/mcp`. See [Conversations](./conversations.md#commands-in-the-message-box).

From the CLI, `agt harness session list` shows the sessions, and `agt harness session cancel <id>` stops a running turn.

## Things to know

- **Files are shared.** Agents using the same working directory edit the same files. Give each Codex agent its own directory or git worktree when they work in parallel.
- **Stopping is not undoing.** Cancelling a turn stops Codex, but file changes already made stay. If a turn's outcome is unclear, its status shows **Needs attention** (需要处理): check the project files, then use `/new`.
- **Not yet supported:** `/review`, `/diff`, importing or forking existing Codex threads.
