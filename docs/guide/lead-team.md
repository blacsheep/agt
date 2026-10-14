# Build a team with a lead

You can create every agent yourself, but usually it is easier to give one agent the ability to manage resources and let it build the team your goal needs. That agent is the lead.

## What the lead is

A lead is an ordinary agent with three things:

1. the built-in prompt `default/lead`;
2. the `resource_management` capability, to create prompts, agents and tools;
3. the `task_collaboration` capability, to delegate tracked work and collect results.

The `default/lead` prompt (created by `agt init` and on first start) teaches the agent to:

- **check before creating**: list existing models, prompts, MCP servers and agents, and reuse them;
- **build working teammates**: write a role prompt, define the agent, create its identity, bind it, confirm it is enabled, and add it to a conversation;
- **grant the minimum**: new agents do not inherit the lead's capabilities, and the lead adds only what each role needs;
- **have Codex build missing tools**: delegate an MCP server to a Codex agent with contract tests, register it, probe it, and attach it to the agents that need it;
- **delegate and follow up**: use tracked tasks for work that must produce a result, and report back to the original conversation;
- **let the team run itself**: give standing rules once (who does what, who talks to whom, when to escalate) instead of relaying every step;
- **join groups properly**: in a group someone else created, the lead is a plain member and cannot invite anyone, so it asks the owner to make it an admin or to add the members;
- **report honestly**: confirm the setup and deliver results in the conversation where you asked, base every claim on what the tools actually returned, and ask you before deleting things, overwriting others' configuration or making expensive decisions.

## Set it up

Follow [Quick start](./quick-start.md#_3-create-the-lead), or from the CLI:

```bash
agt agent create lead --model fast --prompt lead \
  --builtin-tool resource_management --builtin-tool task_collaboration
agt tool call --principal me --manager principal_create --args '{"id":"lead","name":"Lead","kind":"agent"}'
agt tool call --principal me --manager agent_bind --args '{"principal":"lead","agent":"lead"}'
```

The lead makes many tool calls. Set an approval policy you are comfortable with before you start; see [Permissions](./permissions.md). A strong model with reliable tool calling works best for the lead; teammates can use cheaper models.

### Optional: a Codex agent for building tools

For the lead to build tools, create a Codex agent it can delegate to:

1. Create a [harness](./codex.md) that points at a working directory for tools, with `workspace_write` access.
2. Create an agent on that harness, for example `toolsmith`, with a short prompt such as "You build and test MCP servers in this directory."

Tell the lead it exists, or let it find it with `resource_list`.

## Give it a goal

Talk to the lead in a direct chat. Say what you want to achieve, not how to configure agt. Good goals name the outcome, the roles if you have an opinion, and how you want to be involved:

> Build a three-person research team for the question "Should we migrate to Postgres 17?": one researcher for performance, one for operational risk, and a critic who challenges both. Put them in a group with me, let them discuss, and send me a one-page summary at the end.

> We want to play a murder-mystery game. Build an MCP server that holds the secret clues and hands each player only their own. Create a game master and four player agents, attach the tool, and start the game in a group with me.

## What to expect

- The lead first lists what exists, then creates prompts, agents and identities. Each step is a tool call that you can see in the execution bar, and approve if you use manual approval.
- New agents appear under **Agent team** as soon as they are created.
- The lead creates a group, adds you as owner, and posts the briefing. From then on, team members talk to each other directly with mentions.
- When it delegates a tracked task, you can follow it under **Tasks** (任务) in the conversation title bar.
- The lead reports back in your direct chat.

Everything the lead creates is an ordinary resource. You can inspect it, edit it or delete it like anything you created yourself.

## Tips

- **Review the first team.** Open each new agent and check its prompt and capabilities. If a role is wrong, tell the lead what to change; it edits the resources itself.
- **Control cost.** Each agent can reply at most 100 times per discussion, and a discussion expires after 30 minutes by default. Stop everything at any time with **Stop agents in this group** or `/stop`. See [Scheduler](../modules/scheduler.md#settings-config-yaml) for longer jobs.
- **Use separate homes for experiments.** `agt init ~/experiments/game` creates a clean workspace, and `agt --home ~/experiments/game web --port 4318` runs a console for it.
- **Write your own lead.** `default/lead` is a starting point. Copy it into a new prompt and adapt it to your domain, such as a lead that always builds a reviewer into every team.
