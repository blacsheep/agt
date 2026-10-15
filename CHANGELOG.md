# Changelog

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
