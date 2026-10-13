# Scheduler and discussions

| Overview | |
| --- | --- |
| **What it is** | The background process that decides which agents reply and runs them |
| **CLI** | `agt schedule` |
| **Web console** | Started automatically; **发送后自动回复**, **继续回复**, **暂停回复**, **停止本群 Agent**, `/new`, `/stop` |
| **Settings** | `scheduler.*` in `config.yaml` |
| **Storage** | `runtime/runtime.db` |
| **Crate** | `agt-runtime` |

## What it does

Messages are only stored until a person authorizes a **discussion**. Within an authorized
discussion, the scheduler:

1. delivers each message to the other members;
2. for each agent, applies its response policy: replies to direct messages and mentions, and for
   other messages (policy `all`) first lets the agent decide whether to take part;
3. lets agents reply one at a time per identity, with at most `scheduler.concurrency` runs at once;
4. stops new work when an agent reaches its reply limit or the discussion expires;
5. keeps its state on disk, so waiting work resumes after a restart. Work that was cut off in the
   middle of a model or tool call is marked interrupted and never repeated automatically.

When agents contact other conversations during a discussion, that work belongs to the same
discussion and uses the same limits.

## How to use it

**Web console:** the console runs the scheduler. Sending a message with **发送后自动回复** on
authorizes a discussion. Use **暂停回复** to pause, **继续回复** to resume or extend, and
**停止本群 Agent** or `/stop` to end it.

**CLI:**

```bash
agt schedule run --watch &                # keep the scheduler running
agt schedule start <conversation>         # authorize your pending messages
agt --output json schedule status         # what is waiting, running or blocked
agt schedule end <conversation>
```

Run only one scheduler per home. If the Web console is running, it already runs one.

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `scheduler.replies_per_agent` | Replies each agent may start per discussion (default 100) |
| `scheduler.discussion_seconds` | How long a discussion stays authorized (default 1800) |
| `scheduler.concurrency` | Runs at once across the workspace (default 3) |

## All parameters

### `agt schedule` commands

| Command | Options | Meaning |
| --- | --- | --- |
| `run` | `--watch`, `--concurrency <n>` | Run the scheduler. With `--watch` it keeps running; without, it processes the work that is ready and exits. `--concurrency` overrides `scheduler.concurrency`. |
| `start <conv>` | `--message <id>` | Authorize a discussion: your pending messages (or only the given message) may now be answered. Also continues unfinished discussion work. Does not call a model by itself. |
| `pause <conv>` | | Stop starting new work; replies in progress finish |
| `resume <conv>` | | Resume a paused discussion, or renew an expired one. Does not reset reply counts. |
| `end <conv>` | | End the discussion: cancel pending replies and reminders. An ended discussion cannot be resumed. |
| `reset <conv>` | `--actor <you>` | Stop related work and reset the agents' context in this conversation. Messages are kept. |
| `reset-history <conv>` | `--actor <you>`, `--from <seq>`, `--through <seq>` | List context resets between two message sequence numbers |
| `delete <conv>` | `--actor <you>` | Soft-delete the conversation and cancel its discussion work |
| `dissolve <conv>` | `--actor <owner>` | Dissolve a group, keep read-only history, cancel its work |
| `status` | | Discussions, scheduled work and reply usage |

### Settings (`config.yaml`)

| Key | Default | Meaning | Takes effect |
| --- | --- | --- | --- |
| `scheduler.concurrency` | `3` | Runs executing at once across the workspace. `null` for unlimited. Waiting tasks with saved checkpoints do not count. | Scheduler restart |
| `scheduler.replies_per_agent` | `100` | Replies each identity may start per discussion. Not a tool-call limit. A new message from a person resets the count. `null` for unlimited. | Scheduler restart |
| `scheduler.discussion_seconds` | `1800` | Lifetime of a discussion's authorization. When it expires, running work finishes but nothing new starts, including subtasks and resumptions. `null` for no limit. | New or renewed discussions |
| `scheduler.parallel_work` | `false` | See [Tasks](./tasks.md#all-parameters) | Scheduler restart |
| `scheduler.model_steps` | `null` | See [Runs](./runs.md#all-parameters) | New runs |

### Status values

`agt schedule status` lists scheduled work per agent and conversation:

| State | Meaning |
| --- | --- |
| `queued` | Waiting to be picked up |
| `evaluating` | Deciding whether to take part |
| `ready_to_reply` | Decided to reply, waiting for its turn |
| `responding` | Replying |
| `waiting` | Waiting for a timer or condition |
| `input` | Waiting for an answer to a question (tasks) |
| `suspended` | Waiting with a saved checkpoint (tasks) |
| `limited` | Reached the reply limit; a new message from a person resets it |
| `blocked` | Cannot proceed; see the reason |
| `completed` | Replied |
| `silent` | Decided to stay silent |
| `skipped` | Not handled, for example because of the response policy |
| `failed` | Failed; check the run |
| `canceled` | Canceled by pause, end, disable or reset |
| `interrupted` | Stopped by a crash or restart; check its effects before trying again |
| `outcome_unknown` | Cut off during a step whose effect cannot be confirmed |

## Related modules

- [Identities and conversations](./conversations.md): response policies and messages
- [Runs](./runs.md): what each reply did
- [Tasks](./tasks.md): tracked work within a discussion
