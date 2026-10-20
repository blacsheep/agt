# Task collaboration

Messages are conversation: an agent may or may not answer, and nothing tracks whether work got done. When one agent needs another to do a piece of work and return a result, it uses a **task** instead.

| | Message | Task |
| --- | --- | --- |
| Delivery | Posted to the conversation | Queued for one assignee |
| Result | Whatever gets said | A recorded result, delivered back to the requester |
| States | — | queued, running, waiting, succeeded, failed, canceled, ... |
| Follow-up | — | The requester must file a report to the original conversation |
| Survives restarts | The message does | The task, its state and any checkpoint do |

## Enable it

Give `task_collaboration` to the agents that **request** work, usually the lead:

```bash
agt agent update lead --add-builtin-tool task_collaboration
```

Agents that only **receive** tasks need nothing extra; they only need to be bound and enabled. The scheduler must be running (the Web console starts it).

## How a task flows

![The lead requests a task, the worker runs it, the lead is notified and files a report](../public/guide/task-flow.en.svg)

1. **Request.** The lead calls `task_request` with the assignee, a conversation both are in, the input, and a `request_key`. Repeating the same key returns the same task, so retries do not create duplicates.
2. **Run.** The scheduler runs the worker. Its final answer is the result, and agt posts it in the task's conversation; if the worker tries to post the result with `message_send`, the call is refused and it is told to answer instead. Failures and unclear outcomes are reported too.
3. **Notify.** The result is delivered to the lead, which starts a new run to handle it.
4. **Report.** The lead reviews the result and calls `task_report` to post a report in the original conversation. A normal message does not count as the report; if none is filed, the task shows that its report is still owed.

## Waiting for results

A lead that is itself working on a task can pause until its subtasks finish:

- `task_await` waits for one or up to 20 tasks. The agent saves a checkpoint and frees its slot; when the tasks finish, fail, or the deadline passes, it continues from the checkpoint. It must be the only tool call in that step.
- `task_yield` pauses until a given time.
- `task_input` asks the requester a question; the task waits until `task_reply` answers it or the deadline passes.

Waiting with a deadline never cancels the subtask and never fakes a result; late results are still delivered.

## Managing the queue

Agents can also reprioritize, defer, cancel and retry queued tasks, and people can resolve tasks whose outcome is unknown. The [Tasks module](../modules/tasks.md#managing-the-queue) lists every tool and parameter.

Tasks that wait in the queue move up one priority level every five minutes, up to `high`. Updates require the task's current `revision`, so two agents cannot overwrite each other's changes.

## Parallel work

By default an agent works on one thing at a time. To let an agent run several independent tasks at once:

1. Set `scheduler.parallel_work: true` (see [Tasks module](../modules/tasks.md#settings)).
2. Mark the tasks as parallel, and name the shared things they touch:

```json
{ "execution_policy": { "parallel": true, "exclusive_resources": ["repo:frontend"] } }
```

Tasks whose `exclusive_resources` overlap never run at the same time, even across agents. These keys are agreements between agents, not file locks; agents must use the same key for the same thing.

## Goals: keep long work moving

Some requests take many steps and several agents, and work can stall silently: an agent finishes its part but nobody is woken to take the next step. A **goal** lets an agent own the outcome until it is done.

- The agent records the goal and how to check it with `goal_create` (`text`, `done_when`). The built-in lead does this for multi-step requests.
- When nothing has happened in the workspace for `scheduler.goal_idle_seconds` (2 minutes by default) and the goal is still open, agt posts an **[AGT goal review]** in the goal's conversation, in the name of the person who asked, like a timer. It starts a discussion and wakes the owner with the goal, the unfinished tasks, recent failures and the latest message in each of its conversations.
- The owner then completes the goal (`goal_complete` with evidence), unblocks the work, or calls `goal_wait_human` and asks you. Waiting for other agents is not a reason to ask you: the owner just ends its turn, and their replies or the next review wake it.
- When a goal waits for you, agt posts the question in the goal's conversation and mentions you, so it never stays hidden in the agent's notes. Your next message there resumes the reviews. If nobody answers for 30 minutes, the owner looks at the goal again, in case what blocked it has cleared; later checks wait 1, 2, 4 and then 8 hours.
- Each quiet spell gets one review. If after `scheduler.goal_stalled_reviews` reviews in a row (3 by default) nobody but the owner has acted, reviews pause and the owner says so in the conversation; your next message there resumes them.
- Nothing is reviewed while something runs or is ready to run, while an approval waits for you, or when a timer or durable wait resumes work within 10 minutes.

The console shows reviews as **🎯 Goal review**.

## In the Web console

Click **Tasks** (任务) in a conversation's title bar. The panel lists tasks in this conversation, with filters for active, needs attention and finished, and a search box. Open a task to see its state, result, delivery and report, and to answer, cancel, retry or resolve it. **Interactions** (交互观察) shows delegations between agents as a graph.

## When something goes wrong

- **`outcome_unknown`**: the run was interrupted and agt cannot tell whether the work happened. It is never retried automatically. Check the actual result (files, messages, external systems), then use `task_resolve` to record what happened, and `task_retry` if it failed. Until it is resolved, the task keeps the exclusive resources it declared, so tasks that need them stay queued. The workspace owner can resolve any task, including one between two agents in a chat the owner is not part of, from the console or with `agt tool call --principal <owner> --manager task_resolve`. A run that stopped before it started is marked `failed` instead, and can be retried right away.
- **Stuck in `queued`**: the assignee may be disabled, set to `manual`, or the discussion may be paused or expired. `task_get` shows the reason in `blocked_reason`.
- **Report missing**: the lead received the result but did not call `task_report`. The task's report state stays `awaiting_report`, and after five minutes a reminder is raised.

From the CLI:

```bash
agt tool call --principal lead task_list --args '{"limit":20}'
agt tool call --principal lead task_get --args '{"task_id":"<id>"}'
```

Every tool and parameter is listed in the [Tasks module](../modules/tasks.md).
