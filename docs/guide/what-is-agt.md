# What is agt?

Most agent frameworks ask you to build every agent by hand: write its prompt, wire up its
tools, decide who talks to whom. agt turns that around. **Resource management is itself a
capability you can give to an agent.** An agent that has it can create the prompts, tools,
skills and teammates it needs, and then use them to get the work done.

In practice you talk to one agent, the *lead*. You tell it the goal; it designs the team,
creates each member, has Codex build any missing tools, and coordinates the work in a group
chat while you watch.

## Resources

Everything an agent is made of is a resource stored in the agt home directory
(`~/.agt` by default):

| Resource | What it is |
| --- | --- |
| Provider | An OpenAI-compatible endpoint and the API key to reach it |
| Model | A named model on a provider, with its protocol and options |
| Prompt | A role description (`PROMPT.md`) |
| Skill | A reusable instruction package (`SKILL.md`) |
| MCP | An MCP server definition (stdio or HTTP) that provides tools |
| Harness | An external execution engine, currently Codex |
| Agent | A model or harness, plus a prompt, skills, MCP servers and built-in tools |

People edit these through the Web console or the CLI. Agents edit them through the
`resource_management` tools, which go through the same code path, validation and locks.

## Built-in capabilities

Agents only get the abilities you mount on them. Besides the MCP servers you attach, there
are three built-in tool groups:

| Capability | What it allows |
| --- | --- |
| `resource_management` | Create and change resources, create agent identities, bind and enable agents |
| `task_collaboration` | Delegate tracked tasks to other agents, wait for results, and file reports |
| `local_execution` | Run commands on the local machine |

A new agent does not inherit its creator's capabilities. The lead decides what each
teammate actually needs, so isolation is enforced by tools, not by asking the model to
behave.

## Identities and conversations

An agent definition is just configuration. To take part in chats it needs an identity
(a *principal*) and a *binding* that connects the identity to the definition. Bound agents
then join direct and group conversations alongside you.

By default an agent replies to direct messages and to group messages that mention it.
A persistent scheduler decides who runs, who waits and who stays silent, enforces reply
limits, and resumes unfinished work after a restart.

## Engines

Each agent runs on one of two engines:

- **Native**: agt's own model and tool loop, talking to any OpenAI-compatible model with
  Chat Completions or Responses.
- **Codex**: a [Codex](https://github.com/openai/codex) session started through a
  harness, useful for agents that write and test code, such as the ones that build MCP
  tools for the rest of the team.

## The lead

`agt init` creates a built-in `lead` prompt that knows the whole framework. It tells the
agent how to:

1. Check what already exists before creating anything.
2. Build a working teammate: prompt, agent definition, identity, binding, conversation.
3. Have Codex write an MCP server, register it, probe it, and attach it to agents.
4. Delegate tracked tasks and review the results.
5. Set standing rules so the team coordinates directly instead of relaying every step.
6. Report honestly, based only on what the tools actually returned.

Give an agent this prompt plus `resource_management` and `task_collaboration`, and you
have a lead. [Getting started](./getting-started.md) walks through it.

## What agt is not

agt is a trusted local tool. The Web console listens on `127.0.0.1` only, and whoever holds
its link can manage every resource. It is not a hosted or multi-tenant service.
