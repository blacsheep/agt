# Changelog

## 0.6.1 - 2026-10-21

From running whole werewolf games on agt from a single request: the lead set up the team, a host agent ran the game, and other agents made the audio and the replay, with nobody watching.

### Goals

- An agent can own a goal (`goal_create` with what to do and when it is done). When the workspace goes quiet while the goal is open, agt posts a 🎯 Goal review in the goal's conversation, so the owner finishes it, unblocks the work or asks a person, instead of everything waiting until someone notices. The built-in lead sets a goal for multi-step requests. See [Goals](docs/guide/tasks.md#goals-keep-long-work-moving).
- Reviews pause after `scheduler.goal_stalled_reviews` (3) reviews in a row in which only the owner acted, until a person writes; `scheduler.goal_idle_seconds` (120) sets how long is quiet.
- `goal_wait_human` posts the question in the goal's conversation and mentions the person. With no answer, the goal is reviewed again after 30 minutes, then 1, 2, 4 and 8 hours.
- A refused timer or goal call tells the agent why, instead of a generic rejection.

### Scheduler and recovery

- A scheduler that hits an error logs it to `runtime/scheduler.log` and keeps going, and the console restarts a scheduler that exits. Before, one failed tick stopped every reply with no trace.
- After a stop, a run that had not called a tool yet is retried on its own (its task fails so it can be retried) instead of waiting as `outcome_unknown` for a person.
- Fixed a race in which a run that had just been claimed could fail with "cannot bind work Run", or stop the scheduler.
- A member who chose to wait for others decides again when they finish without a new message, instead of staying silent. A host that waited for the last speaker no longer stalls the conversation.

### Mentions

- Mentioning someone who is not a member is refused with the unknown mentions and the ids and names of every member, so the agent can correct it. Before, the error blamed the sender's membership.

## 0.6.0 - 2026-10-20

Fixes and changes from running a twelve-player werewolf game on agt for hours: a host agent, twelve player agents, a Codex agent that wrote the rules engine, and Claude Code and Codex agents that turned the game into audio and a replay.

### Behavior change: `mentioned` means mentioned

- An agent with response policy `mentioned` now answers only messages that mention it, from people and agents alike. Before, every message from another agent made it decide whether to speak, which cost one model call per member per message. Set members to `all` when a team should discuss freely.
- Agents mention members by writing `@id` in a reply, or `@["id"]` for IDs with spaces or punctuation, the same syntax as the Web console. Only current members count; an agent cannot mention itself or use `@all`.

### Tasks and recovery

- A run that stopped before it started, for example because binding it to its task failed, ends as `failed` and can be retried, instead of `outcome_unknown`. The task keeps the reason.
- After a crash or restart, runs that were executing become `outcome_unknown` instead of showing `running` forever.
- The workspace owner can view, cancel, retry and resolve any task from the console or with `--manager`, including tasks between two agents in a chat the owner is not part of. Resolving an `outcome_unknown` task releases the exclusive resources it held.

### Models

- Server errors (5xx), rate limits and timeouts are retried with a wait that grows to 30 seconds, about a minute and a half in all, so a short provider or gateway outage no longer fails the run. Other client errors such as 401 or 404 fail at once instead of being retried ten times.
- `agt model update --provider --model` points a model resource at another provider or model name; agents keep referring to it by name.

### MCP

- Servers that answer an older protocol version (2025-06-18, 2025-03-26 or 2024-11-05) are accepted.
- When a server cannot start or breaks the protocol, tools such as `mcp_probe` say which kind of failure it was (`mcp_server_error`) instead of a generic rejection.

### Codex

- A Codex harness can set `"inherit_mcp_servers": false` to start Codex without the MCP servers in its own config, so the agent uses agt's tools only and starts no extra processes.

### Timers

- Timer messages show as ⏰ Timer in the console instead of as your own message, and agents see who set the timer.

### Lead

- The built-in lead checks every step of a request before reporting it done, and carries on with the steps after a subtask's result.
- An edited built-in lead prompt is kept on upgrade, and its page in the console offers the new built-in version.

### Desktop app

- The macOS app icon is drawn on the macOS icon grid, so it is the same size as other apps in the Dock and Launchpad, and the menu bar shows a monochrome icon that follows light and dark mode.

### CLI

- `agt … | head` no longer panics when the pipe closes early.

## 0.5.0 - 2026-10-16

### Desktop app

AGT Studio is the agt console as a desktop app for macOS, Windows and Linux. It shows the same pages as `agt web` without a local web server, keeps agents and timers running from the system tray when you close the window, and notifies you of new messages, timers that fire and approvals waiting for you. It uses the same home as the CLI and updates itself.

Installers are attached to each release. They are not signed yet; see [Desktop app](docs/guide/desktop.md) for how to open them the first time.

### More engines

Besides Codex, an agent can now run on another agent harness, which brings its own tools and session, while agt keeps its framework tools, tasks and approvals:

- `claude-code`: [Claude Code](docs/guide/claude-code.md). Its permission prompts go through agt's approval queue.
- `deepseek-harness`: [DeepSeek Harness](docs/guide/deepseek-harness.md) (`dsh`, Node.js 24 or later).
- `acp`: any agent that speaks the Agent Client Protocol, such as `codex-acp`, with its command in `executable` and `args`.

### Models

- Two new API formats: `anthropic-messages` for Claude and `gemini` for the Google Gemini API, next to `chat-completions` and `responses`.
- The model input keeps fields that never change first and those that change on every call last, so providers can reuse more of their prompt cache.

### Permissions

- **Always allow this agent to use this tool** when approving a call. Later calls of that tool by that agent run without asking. See and remove rules in the Approvals panel or with `agt permission rules` and `agt permission forget`.
- An MCP server entry can set `"trust_read_only_hint": true`; tools that server marks read-only then run without approval. Its other tools are still reviewed.

### Time and timers

- `task_defer`, `task_yield`, `task_input` and `task_await` accept a local time such as `2026-11-01T09:00`, or one with an offset, instead of Unix milliseconds, which still work.
- Agents see when each message in the history was sent, including messages that arrive while they work.
- Create timers in the Web console from `/timers`, once or with a cron expression. A timer can set its own `discussion_seconds` for long jobs.

### Lead

- Built-in lead prompts you have not edited are upgraded to the current version when the scheduler starts. Edited prompts are left alone.

### Web console

- When a discussion is active but nobody is working, the reply status says it is idle and until when it stays open.

### Docs

- Desktop app, Claude Code and DeepSeek Harness guides, in English and Chinese.

## 0.4.0 - 2026-10-16

### Timers

Agents can now act at a set time, even when nobody is around. A timer posts a message to a conversation later, once or on a cron schedule, and counts as a message from you: it starts a new discussion, so agents get to work even if the last one expired long ago.

- Ask an agent with `task_collaboration` in plain words, such as "remind me in 5 minutes" or "at 9:00 on the 1st of every month, have the team send the data". It sets the timer with `timer_create` and sees the exact local time it resolved.
- Setting a timer follows your permission mode: in `manual` you approve it in the Web console. Timers you create from the CLI (`agt tool call timer_create`) apply at once.
- Type `/timers` in a conversation to see its timers and cancel them.
- If agt was not running at that time, the timer fires once when it starts again and says it is late.

See [Timers](docs/guide/timers.md).

### Time

- Agents see the local time with its offset and time zone, and when each new message was sent. Before, they only got a Unix timestamp.
- New setting `scheduler.timezone`, also in the Web console settings. By default agt uses the host's time zone.

### Scheduler

- Agents no longer wait on each other forever. An agent can wait only for a member that is actually working, and wakes as soon as that member finishes its step. When nobody is working, it must answer or stay silent. Before, members who were all "waiting for others" kept the discussion and the "thinking" indicator alive until the discussion expired.
- Messages from other agents that do not mention an agent join the decision it already has pending, instead of each starting its own.

### Permissions

- Built-in tools that only read agt's own data, such as listing tasks or reading messages, no longer ask for approval in any mode, including `manual`. Writes, local commands, MCP tools and calls that reach outside agt are still reviewed.

### Storage

- agt waits up to 30 seconds, instead of 5, for another process to release the database. This fixes "database is locked" errors on slow disks, mostly on Windows.

## 0.3.1 - 2026-10-15

### Scheduler

- The scheduler no longer drops its lease after a brief error. Its heartbeat stopped for good the first time renewing failed, for example while the database was busy, and fifteen seconds later the scheduler cancelled all running work with "scheduler lease lost". Errors are now retried on the next heartbeat.

## 0.3.0 - 2026-10-15

### The Web console no longer needs Node.js

`agt web` now serves the console from the `agt` binary itself. Install `agt` and run `agt web`; there is nothing else to install. The `--node` option is gone, and the copy of the console that earlier versions unpacked into `<home>/.web` is removed on first start.

### Lead

- The lead works in the language of your request: its replies, reports, task briefs and the role prompts it writes for teammates. Previously the built-in prompt pulled it into Chinese.
- When the results a step was waiting for arrive, the lead starts that step in the same turn. Before, it could announce the next round and then stop until someone nudged it.
- For tools where each role may only see its own data, the lead asks for one tool instance per role, locked by environment variables, instead of telling roles apart by tokens in tool arguments.

The built-in lead prompt is written when a home is created. To get these changes in an existing home, copy `crates/agt-prompt/src/lead.md` over `<home>/prompts/default/lead/PROMPT.md`, or create a new home.

### Models

- A reply that stops streaming after it has started is retried. Before, a stream kept alive by heartbeats could hold a run for over 20 minutes. The model's `timeout` now also bounds the gap between output chunks; silent reasoning is not limited.

### Docs

- The README and the examples pages show sped-up recordings of both example runs.
