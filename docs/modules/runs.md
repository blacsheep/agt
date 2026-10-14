# Runs

| Overview | |
| --- | --- |
| **What it is** | The record of each agent execution: model calls, tool calls and result |
| **CLI** | `agt run`, `agt agent run` |
| **Web console** | Execution bar, the record under each reply, **Debug** (调试模式) |
| **Settings** | `scheduler.model_steps` |
| **Storage** | `runtime/runtime.db` |
| **Crates** | `agt-runtime`, `agt-inference` |

## What it does

Every time an agent replies to a message or works on a task, agt records a **run**: its input,
status, model calls, tool calls and output. Runs let you see what an agent did, why it failed,
and what it cost in calls.

A native run loops between the model and tools until the model gives a final answer. Messages
that arrive meanwhile are delivered at the next tool boundary (`steering`) or after the current
work (`follow_up`). Codex runs are a Codex turn, with Codex's own tool loop.

`completed` means the model finished normally, not that the goal was achieved; check the tool
failures and the output. A run cut off during a model or tool call is never retried
automatically, because the call may already have had an effect.

## How to use it

**Web console:** each reply shows its tool calls, duration and failures; click to expand or open
the full record. Turn on **Debug** for run IDs, call counts and error codes.

**CLI:**

```bash
agt run list --limit 20
agt run info <run>
agt run calls <run>
agt run tool-detail <run> --activity <activity>
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `agt run info <run>` | Status, input, output, errors, tool failure count |
| `agt run calls <run>` | Each model and tool call, with timing and status |

## All parameters

### `agt run` commands

| Command | Options | Meaning |
| --- | --- | --- |
| `list` | `--limit <n>` (default 50), `--conversation <id>` | Recent runs; with `--conversation`, only top-level runs triggered there |
| `info <run>` | | One run |
| `calls <run>` | `--after <id>` (default 0), `--limit <n>` (default 50) | Model and tool calls. Records the tool name, timing, status, safe error code and the shape of the arguments, not their values. Page with the last ID as `--after`. |
| `children <run>` | | Runs delegated from this run (`agt agent run --delegate`) |
| `tool-detail <run>` | `--activity <id>` (required) | Full arguments and result of one tool call, truncated at 64 KiB per field. Local operator only. |

### `agt agent run`

Runs an agent once, outside any conversation. Options are listed in
[Agent](./agent.md#agt-agent-run-name).

### Settings

| Key | Default | Meaning |
| --- | --- | --- |
| `scheduler.model_steps` | `null` | Maximum model calls in one native run. `null` for unlimited. Does not limit Codex. Takes effect for new runs. |

### Run status

| Status | Meaning |
| --- | --- |
| `running` | In progress, or cut off before a final status was recorded |
| `suspended` | Waiting with a saved checkpoint (tasks only) |
| `completed` | The model finished |
| `failed` | An error ended the run |
| `canceled` | Stopped by a person, a disabled agent or an ended discussion |
| `interrupted` | Stopped by a crash or restart, and the stop is confirmed. Check its effects before trying again. |
| `outcome_unknown` | Cut off during a step whose effect cannot be confirmed; check before retrying |

### Call record status

| Status | Meaning |
| --- | --- |
| `started` | The call began and no result was recorded: the outcome is unknown |
| `succeeded`, `failed`, `canceled` | The call's result |

## Related modules

- [Scheduler](./scheduler.md): what starts runs
- [Tasks](./tasks.md): runs that work on tracked tasks
- [Model](./model.md): retries and context management inside a run
