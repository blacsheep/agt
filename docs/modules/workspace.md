# Workspace and configuration

| Overview | |
| --- | --- |
| **What it is** | The home directory that holds one workspace, and its settings file `config.yaml` |
| **CLI** | `agt init`, `agt config`, global options |
| **Web console** | **Settings** (运行设置) |
| **Crate** | `agt-config` |

## What it does

A workspace is one *home* directory. Everything agt knows lives there: resources, conversations, runs, tasks and settings. Different homes are completely independent, so you can keep one per project or use a throwaway home for experiments.

`config.yaml` holds the workspace settings: scheduling limits, approval policy, memory and local commands. Only the operator changes it; agents cannot, even with `resource_management`.

## How to use it

**Choose the home.** agt uses, in order: the `--home` option of the command, the `AGT_HOME` environment variable, then `~/.agt`. The default home is created automatically on first use.

```bash
export AGT_HOME=/data/agt          # lasting change, in ~/.bashrc or ~/.zshrc
agt --home /tmp/demo agent list    # this command only
agt init ~/projects/game-night     # create a new home explicitly
```

**Change settings.** In the Web console, open **Settings**. Each setting shows its default and when a change takes effect. From the CLI:

```bash
agt config show > config.yaml      # current settings
# edit config.yaml
agt config apply config.yaml       # validate and replace the whole file
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `AGT_HOME` | Absolute path of the default home |
| `--home <dir>` | Home for one command |
| `--output json` | Machine-readable output, for scripts |
| `scheduler.replies_per_agent`, `scheduler.discussion_seconds` | How long agents may keep talking; see [Scheduler](./scheduler.md) |
| `permissions.mode`, `permissions.reviewer_model` | How tool calls are approved; see [Permissions](./permissions.md) |

## All parameters

### Global options

Accepted by every command.

| Option | Default | Meaning |
| --- | --- | --- |
| `--home <dir>` | `$AGT_HOME`, then `~/.agt` | Home directory for this command only |
| `--output <format>` | `human` | `human`, `json` or `jsonl` |
| `--yes` | off | Skip the confirmation of destructive commands. Required when not running in a terminal. |
| `-h`, `--help` | | Help for any command |
| `-V`, `--version` | | Print the version |

### Environment variables

| Variable | Meaning |
| --- | --- |
| `AGT_HOME` | Default home. Must be an absolute path; an empty value counts as unset. |

### `agt init <dir>`

Initializes a home: creates the resource directories, the conversation database, the default `config.yaml` and the built-in prompts `default/default` and `default/lead`. The other databases are created when first needed. Running it again on an existing home adds anything missing without overwriting existing files.

### `agt config`

| Command | Options | Meaning |
| --- | --- | --- |
| `config describe` | | Every setting with its description, default and when it takes effect |
| `config show` | | The settings in effect |
| `config apply <file>` | | Validate a YAML file and replace the whole configuration |
| `config update` | `--value <json>` `--expected <json>` | Replace the configuration with `value`, only if the current configuration still equals `expected` (used by the Web console to avoid overwriting concurrent changes) |

`apply` replaces the whole file: fields you leave out return to their defaults. Unknown fields and invalid values are rejected. Where a setting allows "unlimited", write `null`; `0` is not accepted.

### `config.yaml`

| Key | Default | Meaning | Details |
| --- | --- | --- | --- |
| `scheduler.concurrency` | `3` | Runs executing at once across the workspace | [Scheduler](./scheduler.md#all-parameters) |
| `scheduler.parallel_work` | `false` | Allow one agent to run parallel tasks | [Tasks](./tasks.md#all-parameters) |
| `scheduler.replies_per_agent` | `100` | Replies per agent per discussion | [Scheduler](./scheduler.md#all-parameters) |
| `scheduler.discussion_seconds` | `1800` | How long a discussion stays authorized | [Scheduler](./scheduler.md#all-parameters) |
| `scheduler.model_steps` | `null` | Model calls per native run | [Runs](./runs.md#all-parameters) |
| `scheduler.timezone` | `null` | Time zone for agents and timers; `null` uses the host's | [Scheduler](./scheduler.md#all-parameters) |
| `permissions.mode` | `model_review` | Approval policy for native tool calls | [Permissions](./permissions.md#all-parameters) |
| `permissions.reviewer_model` | `null` | Model used for model review | [Permissions](./permissions.md#all-parameters) |
| `permissions.review_timeout_seconds` | `30` | Time limit for the reviewer | [Permissions](./permissions.md#all-parameters) |
| `permissions.request_timeout_seconds` | `600` | Time a call waits for a decision | [Permissions](./permissions.md#all-parameters) |
| `permissions.harness_mode` | `manual` | Handling of Codex approval requests | [Permissions](./permissions.md#all-parameters) |
| `memory.enabled` | `true` | History search and notes | [Memory](./memory.md#all-parameters) |
| `execution.enabled` | `false` | Allow host commands | [Local commands](./local-execution.md#all-parameters) |
| `execution.timeout_seconds` | `300` | Time limit per command | [Local commands](./local-execution.md#all-parameters) |
| `execution.output_bytes` | `65536` | Output returned per command | [Local commands](./local-execution.md#all-parameters) |

### Home layout

```text
<home>/
  config.yaml
  providers/<category>/<name>/provider.yaml
  models/<category>/<name>/model.yaml
  prompts/<category>/<name>/PROMPT.md
  skills/<category>/<package>/SKILL.md
  mcps/<category>/<name>/mcp.json
  harnesses/<category>/<name>/harness.yaml
  agents/<category>/<name>/agent.yaml
  im/im.db               # identities, conversations, messages
  runtime/runtime.db     # bindings, discussions, runs, tasks, approvals, Codex sessions
  memory/memory.db       # agents' notes
  .trash/                # deleted resources
```

## Related modules

- [Resources](./resources.md): what the resource directories contain and how to manage them
- [Web console](./web-console.md): the **Settings** page
