# Local commands

| Overview | |
| --- | --- |
| **What it is** | Shell commands run by native agents on the host |
| **Agent tool** | `local_exec` (needs `local_execution`) |
| **Web console** | **Local commands** in the agent form; **Settings** → **Local execution** |
| **Settings** | `execution.*` |
| **Crates** | `agt-service`, `agt-runtime` |

## What it does

`local_exec` runs one Bash command on the machine running agt and returns its exit code, duration and output. It is off by default: the workspace must allow it and the agent must be granted it.

::: warning Host access
Commands run as the user running agt. The working directory is not a sandbox, and nothing is rolled back.
:::

## How to use it

```bash
agt config show > config.yaml   # set execution.enabled: true, then:
agt config apply config.yaml
agt agent update builder --add-builtin-tool local_execution
```

Or switch on **Allow local commands** under **Settings** → **Local execution**, and tick **Local commands** in the agent form. Every call still goes through the [approval policy](./permissions.md).

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `command` | The command line |
| `execution.timeout_seconds` | Time limit per command |

## All parameters

### `local_exec`

| Parameter | Required | Meaning |
| --- | --- | --- |
| `command` | yes | Run with `/bin/bash --noprofile --norc -c` |
| `directory` | no | Working directory. Defaults to the agent's working directory (from its binding); relative paths resolve against it. |

Result: exit code, duration, and stdout and stderr truncated to `execution.output_bytes`.

### Settings (`config.yaml`)

| Key | Default | Meaning | Takes effect |
| --- | --- | --- | --- |
| `execution.enabled` | `false` | Allow agents with `local_execution` to run commands | Off: the next command is refused. On: new runs get the tool. |
| `execution.timeout_seconds` | `300` | Time limit per command; the process group is killed. `null` for none (cancel still works). | New commands |
| `execution.output_bytes` | `65536` | Total stdout and stderr returned; the rest is truncated, the command is not stopped | New commands |

### Limits

- Native agents only; Codex agents use Codex's own shell and sandbox.
- No interactive input, no background services that outlive the command.
- Cancelling or stopping the discussion kills the process group. If agt itself is killed, a command may keep running.
- `resource_management` does not include this capability, and created agents do not inherit it.
- Conversations show only that a command ran; the command and output are in the operator's execution record.

## Related modules

- [Agent](./agent.md): granting `local_execution` and setting the working directory
- [Permissions](./permissions.md): approving each command
