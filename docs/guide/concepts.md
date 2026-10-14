# Core concepts

This page explains the handful of ideas the rest of the documentation builds on.

## Home

All data lives in one directory, the *home* (`~/.agt` by default, or `$AGT_HOME`). It holds the resource files, the conversation database and the runtime database. Different homes are completely separate workspaces. See [Installation](./installation.md#where-data-lives).

## Resources

Resources are the building blocks of agents. Each is a small file in the home:

| Resource | File | What it describes |
| --- | --- | --- |
| Provider | `provider.yaml` | An OpenAI-compatible endpoint and its API key |
| Model | `model.yaml` | A model on a provider: protocol, temperature, timeout, context window |
| Prompt | `PROMPT.md` | A role description |
| Skill | `SKILL.md` | A reusable instruction package added to the system prompt |
| MCP | `mcp.json` | How to start or reach an MCP server that provides tools |
| Harness | `harness.yaml` | An external engine (Codex) and the project directory it works in |
| Agent | `agent.yaml` | A model or harness, plus a prompt, skills, MCP servers and built-in tools |

Resources are referenced as `category/name`, for example `default/lead`. A bare `name` means the `default` category. Categories let you group resources, such as one per project.

Changing a resource takes effect the next time it is loaded: if you lower a model's temperature, every agent using that model gets the new value on its next run. A run that is already in progress keeps the configuration it started with.

## Agents, identities and bindings

An **agent definition** is only configuration. To take part in conversations it also needs:

- an **identity** (a *principal*): a participant in the chat system, such as `lead`. People are principals too; the Web console's operator is `web-operator` by default.
- a **binding** that connects the identity to the definition and holds its running settings: enabled or disabled, working directory and response policy.

![An agent definition is bound to an identity, which is a member of conversations](../public/guide/agent-binding.en.svg)

The Web console's **Create agent** (创建 Agent) creates all three at once. In the CLI and in agent tools they are separate steps: `resource_write` (definition), `principal_create` (identity) and `agent_bind` (binding). Disabling a binding stops the agent everywhere but keeps its identity and history.

## Engines

Each agent definition chooses one engine:

- **Native** (`model`): agt's own model and tool loop. Supports MCP servers, skills, all built-in tools and the approval policies.
- **Codex** (`harness`): runs the agent as a [Codex](https://github.com/openai/codex) session in a project directory. Codex brings its own file editing, shell and sandbox.

## Built-in capabilities

Agents only get the abilities you give them. Besides attached MCP servers, there are three built-in capability groups, all off by default:

| Capability | What it allows |
| --- | --- |
| `resource_management` | Read, create, change and delete resources; create agent identities; bind, enable and disable agents |
| `task_collaboration` | Delegate tracked tasks to other agents, wait for results, report back |
| `local_execution` | Run shell commands on the host (also requires a workspace setting) |

Every hosted agent also has messaging, conversation and [memory](./memory.md) tools. Capabilities are not inherited: an agent that creates another agent decides what the new one gets. See [Agent](../modules/agent.md#built-in-capabilities).

## Conversations

A **conversation** is a direct chat between two participants or a group chat. Each message is delivered to every other member; whether an agent replies depends on its **response policy**:

| Policy | Direct chat | Group chat |
| --- | --- | --- |
| `mentioned` (default) | Replies | Replies when @mentioned |
| `all` | Replies | Considers every message |
| `manual` | Does not reply automatically | Does not reply automatically |

An agent that has nothing to say answers exactly `[[silent]]`, and nothing is posted.

## Discussions and the scheduler

Agents do not run on their own. A **discussion** starts when a person sends a message with auto-reply on (or runs `agt schedule start`). The discussion authorizes agents to spend model calls on it, within limits:

- each agent can reply at most 100 times per discussion (a new message from you resets this);
- the authorization lasts 30 minutes, after which no new work starts;
- at most 3 agents run at the same time.

All three limits are adjustable; see [Scheduler](../modules/scheduler.md#settings-config-yaml). You can pause, resume or end a discussion at any time.

The **scheduler** is the background process that turns messages into runs. It is started by the Web console, or by `agt schedule run --watch`. Its state is stored on disk, so waiting work resumes after a restart.

## Runs

A **run** is one execution of an agent: the model calls, tool calls and final reply that answer a message or perform a task. Runs are recorded in `runtime/runtime.db`. You can inspect them in the Web console (debug mode) or with `agt run list` and `agt run info <id>`.

A run's status `completed` means the model finished, not that the goal was achieved. If a run is interrupted in the middle of a tool call, agt marks it as interrupted rather than running the tool again, because the tool may already have had side effects.

## Tasks

Ordinary messages are conversation. When an agent needs a result it can rely on, it sends a **task** instead: a tracked request with a state (`queued`, `running`, `succeeded`, `failed`, ...), a result that is delivered back, and an explicit report to the original conversation. See [Task collaboration](./tasks.md).

## Approvals

Before a native agent runs a tool, the call goes through the workspace **approval policy**: model review, manual approval, or allow all. Codex agents send their own approval requests, which are handled separately. See [Permissions](./permissions.md).
