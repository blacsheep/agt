# Local commands

The `local_execution` capability lets a native agent run shell commands on the machine that runs agt. It is off by default and needs two switches.

::: warning This is host access
Commands run with the permissions of the user running agt. The working directory is not a sandbox, and nothing a command does is rolled back. Enable this only for agents and models you trust, in a home and account where that is acceptable.
:::

## Enable it

1. Allow it in the workspace: set `execution.enabled: true` (see [Local commands module](../modules/local-execution.md#settings-config-yaml)), or switch on **Allow local commands** (允许本机命令) under **Settings** (运行设置) → **Local execution** (本机执行).
2. Give it to an agent: tick **Local commands** (本机命令) under **Built-in tools** when editing the agent, or:

```bash
agt agent update builder --add-builtin-tool local_execution
```

`resource_management` does not include this capability; it must be granted separately.

## What the agent gets

One tool, `local_exec`:

```json
{ "command": "cargo test -p agt-core", "directory": "/home/me/project" }
```

- The command runs in `/bin/bash --noprofile --norc -c`.
- `directory` is optional and defaults to the agent's working directory (set in its binding).
- The result contains the exit code, duration, and stdout and stderr, truncated to `execution.output_bytes` (64 KiB by default).
- Commands are stopped after `execution.timeout_seconds` (300 by default).

Each call also goes through the [approval policy](./permissions.md), so with manual approval you see every command before it runs.

## Limits

- No interactive input, and no background services that outlive the command.
- For long jobs, delegate a [task](./tasks.md) instead, so it can be waited on and cancelled.
- Stopping a discussion or cancelling a run terminates the command's process group. If agt itself is killed, the command may keep running; check the host before trying again.
- Harness agents (Codex, Claude Code, DeepSeek Harness) do not use this capability. They run commands through their own shell, within the harness's sandbox.
- The conversation shows only that a command was run, not the command or its output. The full details are available to the operator in the execution record.
