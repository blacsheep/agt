# Claude Code agents

A Claude Code agent runs on [Claude Code](https://claude.com/claude-code) instead of agt's own model loop. Like a [Codex agent](./codex.md), it edits files and runs commands in a project directory, and it can use agt's messaging, memory and task tools.

## Requirements

- Claude Code installed on the machine that runs agt (`command -v claude`).
- Claude Code signed in under the same user account that runs agt. agt does not store Claude credentials; it uses Claude Code's own login and settings.

agt runs Claude Code in headless mode (`claude -p` with stream-json), one process per turn, and resumes the agent's session on the next turn.

## Create a harness

```json
{
  "engine": "claude-code",
  "executable": "/home/me/.local/bin/claude",
  "working_directory": "/home/me/projects/tools",
  "sandbox": "workspace_write",
  "model": "opus",
  "reasoning_effort": "high",
  "timeout_seconds": 1800,
  "description": "Builds and reviews code"
}
```

| Field | For Claude Code |
| --- | --- |
| `sandbox` | `workspace_write`: file edits in the project are accepted (`--permission-mode acceptEdits`), and other actions such as shell commands ask for approval. `read_only`: editing tools are disabled and every action asks. |
| `model` | A Claude Code model alias (`fable`, `opus`, `sonnet`, `haiku`) or a full model name. Empty uses Claude Code's default. |
| `reasoning_effort` | `low`, `medium`, `high`, `xhigh` or `max`, passed as `--effort`. |

**Web console:** **Resources** → **New resource** → Harness, then choose **Claude Code** as the engine.

**CLI:**

```bash
agt harness resource create reviewer --file ./reviewer.json
agt harness resource test reviewer      # checks that the executable is Claude Code; runs no model
```

Then create the agent on it, as for Codex: `agt agent create reviewer --harness reviewer --prompt reviewer`.

## How agt connects to Claude Code

- **Instructions:** the agent's role prompt and skills are added to Claude Code's system prompt (`--append-system-prompt`).
- **agt tools:** agt's messaging, memory, task and resource tools are offered to Claude Code as an MCP server named `agt` inside the agt process, so they appear as `mcp__agt__…`. Claude Code does not ask for approval for them; agt checks them itself.
- **Approvals:** when Claude Code wants to do something its permission mode does not allow, the request comes to agt's approval queue and follows the **Harness** approval mode; see [Permissions](./permissions.md#codex-approvals). You can approve once, deny, or cancel the turn.
- **Sessions:** each agent keeps one Claude Code session per conversation and resumes it each turn. `/new` starts a fresh session.

## What Claude Code agents can and cannot use

| Available | Not available |
| --- | --- |
| Role prompt and skills | agt MCP resources (configure MCP in Claude Code instead) |
| `resource_management` and `task_collaboration` tools | `local_execution` (Claude Code has its own shell) |
| Messaging, memory and task tools | Response policy `all` (only `mentioned` and `manual`) |
| `/stop`, `/new`, `/status`, live preview of the reply | `/compact`, `/rename`, `/skills`, `/mcp`, `/permissions` |

## Things to know

- **Files are shared.** Agents using the same working directory edit the same files.
- **Stopping is not undoing.** Cancelling a turn stops Claude Code, but changes already made stay.
- **Your Claude Code settings apply.** Project and user settings, CLAUDE.md and hooks in the working directory are loaded as they would be in a terminal.
