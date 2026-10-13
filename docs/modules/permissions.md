# Permissions

| Overview | |
| --- | --- |
| **What it is** | Approval of individual tool calls and of Codex's requests |
| **CLI** | `agt permission` |
| **Web console** | Execution bar in conversations; **交互观察** → **权限审批**; **运行设置** → **工具权限审批** |
| **Settings** | `permissions.*` |
| **Storage** | `runtime/runtime.db` |
| **Crate** | `agt-runtime` |

## What it does

Every tool call a native agent makes (built-in, MCP or local command) goes through the workspace
approval policy before it runs. Codex agents send their own approval requests when they need to
go beyond their sandbox. Both kinds end up in one approval queue.

| Native policy | Behavior |
| --- | --- |
| `model_review` (default) | A reviewer model allows or denies each call; anything unclear goes to a person |
| `manual` | Every call waits for a person |
| `auto_allow` | Every call runs |

With the default settings no reviewer model is configured, so **every call goes to a person**
until you choose a reviewer model or another policy.

## How to use it

**Web console:** open **运行设置** → **工具权限审批**, set **Native 权限审核策略** (Native policy),
**审核模型** (Reviewer model) and **Harness 权限处理模式** (Codex mode). Approve or deny requests
in the conversation's execution bar or under **交互观察** → **权限审批**.

**CLI:**

```bash
agt permission list
agt permission info <id>
agt permission decide <id> --expected-revision 1 --decision '"allow"' --reason 'routine read'
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `permissions.mode` | Native policy |
| `permissions.reviewer_model` | Model resource used for review |
| `permissions.harness_mode` | `manual` or `auto_allow` for Codex requests |

## All parameters

### Settings (`config.yaml`)

| Key | Default | Meaning |
| --- | --- | --- |
| `permissions.mode` | `model_review` | `model_review`, `manual` or `auto_allow` for native tool calls |
| `permissions.reviewer_model` | `null` | A model resource reference. Without it, `model_review` sends every call to a person. |
| `permissions.review_timeout_seconds` | `30` | Time limit for the reviewer, 1 to 120. On timeout the call goes to a person. |
| `permissions.request_timeout_seconds` | `600` | How long a native call waits for any decision. On timeout the run is cancelled. |
| `permissions.harness_mode` | `manual` | `manual`: each Codex request waits for a person. `auto_allow`: accept what Codex asks for, for that request or turn only. |

Changes apply to requests waiting now and to later ones. Decisions already made stand.

### `agt permission` commands

| Command | Options | Meaning |
| --- | --- | --- |
| `list` | `--principal <id>`, `--before <cursor>` | Approval requests, newest first |
| `info <id>` | `--principal <id>` | One request: tool, arguments, reason, options, revision |
| `decide <id>` | `--expected-revision <n>` (required), `--decision <json>` (required), `--reason <text>`, `--principal <id>` | Decide a request |

`--decision` is the JSON `value` of one of the options in `info`:

| Source | Options |
| --- | --- |
| Native call | `"allow"`, `"deny"`, `"cancel"` (cancel the run) |
| Codex command or file change | Accept once, accept for the session, decline, cancel the turn (as offered by Codex) |
| Codex permission request | Grant for this turn, deny, cancel the turn |

### How model review works

The reviewer receives the tool name, the arguments and the request the agent is working on, and
answers once, with no tools. A deny from the reviewer is final. Errors, timeouts, invalid output
and uncertain answers go to a person. A person can decide before the reviewer answers.

### What approvals cannot do

- Grant a tool the agent does not have, or bypass membership and discussion limits.
- Recall a call that was already approved and sent.
- Change Codex's sandbox.

## Related modules

- [Agent](./agent.md): which tools an agent has
- [Model](./model.md): the reviewer is an ordinary model resource
- [Harness](./harness.md): where Codex requests come from
