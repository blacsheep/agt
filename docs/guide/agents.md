# Create agents

An agent definition combines an engine with a role and abilities:

```yaml
# agents/default/reviewer/agent.yaml
name: reviewer
description: Reviews code changes
model: { category: default, name: fast }   # either a model...
harness: null                              # ...or a harness, not both
prompt: { category: default, name: reviewer }
skills: [code-review]
mcps:
  - { category: default, name: github }
builtin_tools: [task_collaboration]
```

You rarely edit this file by hand: the Web console, the CLI and agents with
`resource_management` all write it through the same validation.

## In the Web console

Open **Agent team** (Agent 团队) and click **Create agent** (创建 Agent). The form has these
parts:

| Field | Meaning |
| --- | --- |
| **Name / identity ID** (资源名称 / 身份 ID) | The definition name and the agent's chat identity |
| **Engine** (执行方式) | **Native** (AGT 内置) or **External harness** (外部 Harness) |
| **Role prompt** (角色 Prompt) | **Use an existing prompt** (使用已有 Prompt) or **Write a new one** (新写一份) |
| Skills / MCP | Optional skills and MCP servers to attach |
| **Built-in tools** (框架内置工具) | **Resource management**, **Task collaboration**, **Local commands** |
| **Response policy** (响应策略) | When the agent replies; see [Conversations](./conversations.md#response-policies) |

Saving creates the definition, the identity and the binding, so the agent is ready to chat.
If a step fails, the form shows which steps succeeded; fix the problem and save again to
finish the rest.

To change an agent later, open its card and use **Edit role** (编辑角色). Changes apply from
the agent's next run. Enabling, disabling and the response policy are under
**Configure** (配置与管理) on the same card.

## Prompts

A prompt is the agent's role: what it is responsible for, who it works with, and when it
should stay quiet. agt ships two prompts:

- `default/default`: a minimal general-purpose prompt.
- `default/lead`: a prompt that knows the whole framework and can build and run a team. See
  [Build a team with a lead](./lead-team.md).

```bash
agt prompt create reviewer --text 'You review code for correctness...'
agt prompt update reviewer --file ./reviewer.md
agt prompt read reviewer
```

Good prompts for group work say when to answer `[[silent]]`, so agents do not reply to every
message in a group.

## Skills

A skill is a reusable instruction package (`SKILL.md` with a name and description in its
front matter). The text of every attached skill is added to the agent's system prompt when a
run starts.

```bash
agt skill create code-review --description 'Review source code'   # creates a skeleton
agt skill update code-review --file ./code-review/SKILL.md         # replace the body
```

Skills that declare `allowed-tools` are not supported yet; attaching one makes the run fail.

## MCP servers

Attach MCP servers to give an agent tools. The agent connects to them at the start of each
run and can call the tools they list. See [MCP tools](./mcp.md).

```bash
agt agent update reviewer --add-mcp github
agt agent update reviewer --remove-mcp github
```

## Built-in capabilities

| Capability | Web label | Gives the agent |
| --- | --- | --- |
| `resource_management` | 资源管理 | Manage all resources in the home, create agent identities, bind and enable agents |
| `task_collaboration` | 任务协作 | Delegate tracked tasks, wait for and report results |
| `local_execution` | 本机命令 | Run shell commands on the host (native agents only; also needs `execution.enabled`) |

```bash
agt agent create steward --model fast --builtin-tool resource_management
agt agent update reviewer --add-builtin-tool task_collaboration
```

`resource_management` is powerful: it covers every resource in the home, including other
agents' definitions. Give it only to agents you trust to manage the workspace, such as a
lead. Details are in the [Agent module](../modules/agent.md#built-in-capabilities).

## Enabling an agent from the CLI

`agt agent create` only writes the definition. To make it chat, create its identity and bind
it:

```bash
agt agent create reviewer --model fast --prompt reviewer
agt tool call --principal me --manager principal_create \
  --args '{"id":"reviewer","name":"Reviewer","kind":"agent"}'
agt tool call --principal me --manager agent_bind \
  --args '{"principal":"reviewer","agent":"reviewer"}'
```

`agent_bind` accepts optional `working_directory` (default: the home) and `policy`
(default: `mentioned`). Use `agent_set_enabled` to switch an agent off and on without losing
its settings.

## Running an agent once

To run an agent outside any conversation, for scripts or testing:

```bash
agt agent run reviewer --input 'Summarize the risks in this plan: ...'
echo 'Summarize this' | agt agent run reviewer --stdin
```

The run is recorded like any other (`agt run list`). Pass `--request-key` to make a retry
return the earlier result instead of calling the model again.

## Deleting

```bash
agt agent delete reviewer --dry-run      # show what would be deleted
agt agent delete reviewer --yes
```

An agent that still has a binding must be unbound first (**Settings** on its card, or
`agent_unbind`). Deleting a provider, model, prompt, skill or MCP server that agents still
use is refused unless you pass `--recursive`, which also deletes the dependent agents.
Deleted resource directories are moved to `<home>/.trash/`.
