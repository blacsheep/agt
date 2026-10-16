# Changelog

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
