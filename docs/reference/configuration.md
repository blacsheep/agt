# Configuration

Workspace settings live in `config.yaml` in the home. They are set by the operator; agents
cannot change them, even with `resource_management`.

## Ways to change settings

- **Web console:** **运行设置** (Settings). Each setting shows its description, default and
  when a change takes effect.
- **CLI:**

```bash
agt config describe              # all settings with defaults
agt config show                  # the settings in effect
agt config apply ./config.yaml   # validate and replace the whole file
```

`apply` replaces the whole file. Start from `agt config show` (or the current file) rather
than writing only the fields you want to change; omitted fields fall back to their defaults.
Unknown fields and invalid values are rejected. Use `null` for "unlimited" where allowed;
`0` is not accepted as unlimited.

## Full example

```yaml
scheduler:
  concurrency: 3
  parallel_work: false
  replies_per_agent: 100
  discussion_seconds: 1800
  model_steps: null
permissions:
  mode: model_review
  reviewer_model: null
  review_timeout_seconds: 30
  request_timeout_seconds: 600
  harness_mode: manual
memory:
  enabled: true
execution:
  enabled: false
  timeout_seconds: 300
  output_bytes: 65536
```

These are the defaults.

## scheduler

| Key | Default | Meaning | Takes effect |
| --- | --- | --- | --- |
| `concurrency` | `3` | How many runs may execute at once across the workspace. `null` for unlimited. | Scheduler restart (`--concurrency` overrides it) |
| `parallel_work` | `false` | Let one agent run several tasks at once when they are marked parallel. Ordinary chat stays one at a time. | Scheduler restart |
| `replies_per_agent` | `100` | How many replies each agent may start per discussion. Not a tool-call limit. `null` for unlimited. | Scheduler restart |
| `discussion_seconds` | `1800` | How long a discussion stays authorized. When it expires, running work finishes but nothing new starts. `null` for no limit. | New or renewed discussions |
| `model_steps` | `null` | Maximum model calls in one native run. `null` for unlimited. Does not limit Codex. | New runs |

For long, unattended work, raise `replies_per_agent` and set `discussion_seconds: null`. The
other safeguards stay in place: you can stop a discussion at any time, and approvals still
apply.

## permissions

| Key | Default | Meaning |
| --- | --- | --- |
| `mode` | `model_review` | Native tool calls: `model_review`, `manual` or `auto_allow` |
| `reviewer_model` | `null` | Model resource used for `model_review`. Without it, calls go to manual approval. |
| `review_timeout_seconds` | `30` | How long the reviewer may take (1–120); then the call goes to manual approval |
| `request_timeout_seconds` | `600` | How long a call waits for a decision before the run is cancelled |
| `harness_mode` | `manual` | Codex approval requests: `manual` or `auto_allow` |

Changes apply to waiting and later requests. See [Permissions](../guide/permissions.md).

## memory

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | Give agents history search and notes. Turning it off refuses calls immediately. |

See [Memory](../guide/memory.md).

## execution

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `false` | Allow agents with `local_execution` to run host commands |
| `timeout_seconds` | `300` | Time limit per command. `null` for unlimited (commands can still be cancelled). |
| `output_bytes` | `65536` | Combined stdout and stderr returned per command; the rest is truncated |

See [Local commands](../guide/local-execution.md).

## Environment variables

| Variable | Meaning |
| --- | --- |
| `AGT_HOME` | Home directory (absolute path) used when `--home` is not given. Default `~/.agt`. |

Environment variables referenced by resources, such as a provider's `--api-key-env` at creation
or an MCP server's `bearer_token_env_var`, are read from the environment of the process that
uses them.
