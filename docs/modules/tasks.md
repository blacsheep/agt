# Tasks

| Overview | |
| --- | --- |
| **What it is** | Tracked work requests between agents, with results and reports |
| **Agent tools** | `task_*` (needs `task_collaboration`) |
| **CLI** | `agt tool call ... task_*` |
| **Web console** | **Tasks** (任务) in the conversation title bar; **Interactions** (交互观察) |
| **Settings** | `scheduler.parallel_work` |
| **Storage** | `runtime/runtime.db` |
| **Crates** | `agt-service`, `agt-runtime` |

## What it does

A task is a request from one agent (the requester) to another (the assignee) that agt tracks until it ends: it is queued, run, its result is delivered back to the requester, and the requester files a report to the original conversation. Tasks survive restarts, can wait for one another, and can run in parallel when they declare which shared things they touch.

Use messages for conversation and tasks for work whose result matters. See [Task collaboration](../guide/tasks.md) for the workflow.

## How to use it

1. Give the requester (usually the lead) `task_collaboration`. Assignees only need to be bound.
2. Make sure requester and assignee share a conversation (`direct_create` or a group).
3. The requester calls `task_request`, later reviews the result with `task_get`, and calls `task_report`.

```bash
agt tool call --principal lead task_list --args '{"queued_only":true}'
agt tool call --principal lead task_get --args '{"task_id":"<id>"}'
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `assignee`, `conversation`, `input` | Who does what, and where both are members |
| `request_key` | Repeating the same key returns the same task |
| `expected_revision` | Required for every change; get it from `task_get` |

## All parameters

### Requesting and reporting

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `task_request` | `assignee`, `conversation`, `input`, `request_key`, `priority?`, `execution_policy?`, `source_message?` | Queue a task. Returns `queued`, not completion. `input` is at most 120 KiB. `source_message` is for people calling from the CLI: the message that authorized the discussion. |
| `task_get` | `task_id` | State, result, blocking reason, report and notification state, revision |
| `task_list` | `conversation?`, `queued_only?`, `limit?`, `before?` | Your requested and assigned tasks, newest first. Use the last `list_cursor` as `before`. |
| `task_report` | `task_id`, `expected_revision`, `body` | Post the report to the task's original conversation. Required after a result; a normal message does not count. |

### Managing the queue

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `task_reprioritize` | `task_id`, `expected_revision`, `priority`, `reason` | `low`, `normal`, `high`; only people can set `urgent` |
| `task_defer` | `task_id`, `expected_revision`, `reason`, and either `not_before` or `after_task` + `deadline` | Start no earlier than a time, or only after another task of the same assignee ends |
| `task_execution_policy` | `task_id`, `expected_revision`, `reason`, `execution_policy` | Declare parallelism and exclusive resources for a queued task |
| `task_cancel` | `task_id`, `expected_revision`, `reason` | Cancel a task and the subtasks it created |
| `task_retry` | `task_id`, `expected_revision`, `request_key`, `reason` | New task from a failed, canceled or interrupted one (requester or people) |
| `task_resolve` | `task_id`, `expected_revision`, `outcome` (`succeeded` or `failed`), `evidence` | People only: record what really happened to an `outcome_unknown` task |
| `task_reply` | `task_id`, `input_request_id`, `expected_revision`, `answer`, `request_key` | Answer a question the task asked with `task_input` |
| `task_attention` | `limit?`, `before?` | Workspace owner only: tasks that need a person |

### Waiting inside a task

Available only while an agent is working on a task.

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `task_await` | `deadline` (required), `task_id` or `task_ids` (1–20) | Save a checkpoint and wait until the tasks end or the deadline passes. Must be the only call in its step. |
| `task_yield` | `not_before`, `reason` | Save a checkpoint and pause until a time |
| `task_input` | `question`, `deadline` | Ask the requester a question and wait for `task_reply` or the deadline |

Times are Unix milliseconds. While waiting, the agent's slot is free for other work. A deadline never cancels the awaited tasks and never invents a result.

### `execution_policy`

```json
{ "parallel": true, "exclusive_resources": ["repo:frontend"] }
```

| Field | Default | Meaning |
| --- | --- | --- |
| `parallel` | `false` | Allow this task to run alongside other parallel tasks of the same agent. Needs `scheduler.parallel_work: true`. |
| `exclusive_resources` | `[]` | Keys of shared things the task changes. Tasks with overlapping keys never run at once, across all agents. |

### Settings

| Key | Default | Meaning |
| --- | --- | --- |
| `scheduler.parallel_work` | `false` | Allow parallel tasks for one agent. Takes effect on scheduler restart. |

### Task states

| State | Meaning |
| --- | --- |
| `queued` | Waiting to start |
| `running` | Being worked on |
| `waiting_dependency` | Waiting with `task_await` |
| `waiting_until` | Waiting with `task_yield` |
| `waiting_input` | Waiting for an answer to `task_input` |
| `succeeded`, `failed`, `canceled` | Ended |
| `interrupted` | Stopped by a crash or restart, and the stop is confirmed. Can be retried with `task_retry`. |
| `outcome_unknown` | Cut off; whether the work happened is unknown. Never retried automatically. |

`blocked_reason` explains why a queued task does not start, for example a disabled or `manual` assignee, an expired discussion, a deferral or a dependency.

### Ordering

Queued tasks run by priority (`urgent`, `high`, `normal`, `low`), then in the order they were queued. A task gains one priority level for every five minutes it waits, up to `high`. Running tasks are never preempted.

## Related modules

- [Agent](./agent.md): granting `task_collaboration`
- [Scheduler](./scheduler.md): discussions and limits that tasks inherit
- [Runs](./runs.md): the runs that work on tasks
