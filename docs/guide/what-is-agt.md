# What is agt?

agt is a framework for running a team of AI agents on your own machine. Its central idea:
**resource management is a capability you can give to an agent.** An agent that has it can
write prompts, register tools, create other agents and put them in a group chat, then use
that team to get the work done.

In practice you talk to one agent, the *lead*. You describe the goal; the lead designs the
team, creates each member, has a coding agent build any tool that is missing, and
coordinates the work while you watch.

## What you can do with it

| Feature | What it gives you | Guide |
| --- | --- | --- |
| Any OpenAI-compatible model | Connect OpenAI, a gateway, or a local server with Chat Completions or Responses | [Connect models](./models.md) |
| Agents as configuration | An agent is a model plus a prompt, skills, MCP servers and built-in tools | [Create agents](./agents.md) |
| Direct and group chats | Talk to agents in an IM; agents answer direct messages and @mentions | [Conversations](./conversations.md) |
| A lead that builds its team | A built-in prompt that knows how to create and coordinate agents | [Build a team with a lead](./lead-team.md) |
| MCP tools | Attach stdio or HTTP MCP servers to agents, or let agents register new ones | [MCP tools](./mcp.md) |
| Codex agents | Run an agent on a [Codex](https://github.com/openai/codex) session that can edit code | [Codex agents](./codex.md) |
| Tracked tasks | Delegate work with queuing, waiting, results and reports | [Task collaboration](./tasks.md) |
| Memory | Agents search their own history and keep notes linked to the source messages | [Memory](./memory.md) |
| Local commands | Optionally let an agent run shell commands on the host | [Local commands](./local-execution.md) |
| Approvals | Review each tool call by hand, with a reviewer model, or allow all | [Permissions](./permissions.md) |
| Use from other clients | Expose agt's tools to any MCP client, such as an editor or another agent | [Use agt from MCP clients](./mcp-clients.md) |

Everything is available both in the Web console and in the `agt` CLI.

## How it fits together

```mermaid
flowchart LR
  you([You]) -- chat --> im[Conversations]
  im -- messages --> sched[Scheduler]
  sched -- runs --> agents[Agents]
  agents -- reply / delegate --> im
  agents -- create & change --> res[(Resources)]
  res -- configure --> agents
  agents -- call --> tools[MCP servers, Codex, shell]
```

- **Resources** are the files that describe agents: providers, models, prompts, skills, MCP
  servers, harnesses and agent definitions.
- **Conversations** are direct and group chats between people and agents.
- The **scheduler** decides which agents should reply to a message, runs them, and keeps
  pending work across restarts.
- Agents with the `resource_management` capability can change the resources, which is how a
  lead builds its own team.

[Core concepts](./concepts.md) explains each of these in more detail.

## Design principles

- **Capabilities are tools, not instructions.** An agent can only do what its attached tools
  allow. A new agent does not inherit the capabilities of the agent that created it.
- **Durable by default.** Conversations, runs, tasks and approvals are stored in SQLite, so
  pending work resumes after a restart instead of being lost or silently repeated.
- **One core, two front ends.** All logic lives in the Rust core. The Web console is a thin
  local UI over the same CLI, so anything you do in one you can do in the other.

## What agt is not

agt is a trusted local tool for one operator. The Web console listens on `127.0.0.1` only,
and whoever holds its link can manage every resource. It is not a hosted or multi-tenant
service. See [Security model](../help/security.md).

## Next steps

- [Install agt](./installation.md)
- [Quick start](./quick-start.md): from a fresh clone to a lead that builds its own team
