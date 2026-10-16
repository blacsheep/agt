# DeepSeek Harness agents

A DeepSeek Harness agent runs on [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (`dsh`) instead of agt's own model loop. agt drives it over the [Agent Client Protocol](https://agentclientprotocol.com) (ACP), the same open protocol editors use, so it keeps its own tools, plugins and sandbox while using agt's messaging, memory and task tools.

## Requirements

- DeepSeek Harness installed on the machine that runs agt, for example `npm install -g @deepseek-ai/dsh`. It needs Node.js 24 or later on the `PATH` of the agt process.
- A DeepSeek API key saved in DeepSeek Harness (its Web UI's Models page) or `DEEPSEEK_API_KEY` exported where agt runs. agt stores no DeepSeek credentials.

agt starts `dsh --profile acp` for each turn and resumes the agent's session on the next one.

## Create a harness

```json
{
  "engine": "deepseek-harness",
  "executable": "/usr/local/bin/dsh",
  "working_directory": "/home/me/projects/tools",
  "sandbox": "workspace_write",
  "model": "deepseek-v4-pro",
  "reasoning_effort": "high",
  "timeout_seconds": 1800,
  "description": "Builds tools for the team"
}
```

| Field | For DeepSeek Harness |
| --- | --- |
| `model` | A model DeepSeek Harness offers, by name (`deepseek-v4-pro`) or by its option value. Empty keeps DeepSeek Harness's default. An unknown name fails the turn and lists the available ones. |
| `reasoning_effort` | A level DeepSeek Harness offers: `off`, `low`, `high` or `max`. |
| `sandbox` | Kept for consistency with other engines; DeepSeek Harness applies its own permission preset. |

**Web console:** **Resources** → **New resource** → Harness, then choose **DeepSeek Harness** as the engine.

**CLI:**

```bash
agt harness resource create builder --file ./builder.json
agt harness resource test builder      # starts dsh and checks the ACP handshake; runs no model
```

Then create the agent on it: `agt agent create builder --harness builder --prompt builder`.

## How agt connects to DeepSeek Harness

- **agt tools:** for each turn agt opens a temporary MCP endpoint on `127.0.0.1` with a random token and mounts it in the ACP session as the server `agt`. Tool calls run inside agt with its usual checks.
- **Approvals:** permission requests (`session/request_permission`) go to agt's approval queue and follow the **Harness** approval mode; see [Permissions](./permissions.md#codex-approvals). You can approve once, deny, or cancel the turn.
- **Model and level:** set per session through ACP configuration options.
- **Sessions:** each agent keeps one session per conversation; agt closes it after each turn so the next process can resume it. `/new` starts a fresh session.

## What DeepSeek Harness agents can and cannot use

| Available | Not available |
| --- | --- |
| Role prompt and skills | agt MCP resources (install plugins or MCP in DeepSeek Harness instead) |
| `resource_management` and `task_collaboration` tools | `local_execution` |
| Messaging, memory and task tools | Response policy `all` (only `mentioned` and `manual`) |
| `/stop`, `/new`, `/status`, live preview of the reply | `/model`, `/compact`, `/rename`, `/skills`, `/mcp`, `/permissions` |

## Things to know

- **Developer preview.** DeepSeek Harness says breaking changes are expected; agt uses only the standard ACP v1 methods.
- **Files are shared** between agents that use the same working directory.
- **Stopping is not undoing.** Cancelling a turn stops DeepSeek Harness, but changes already made stay.
