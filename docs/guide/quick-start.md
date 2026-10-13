# Quick start

This guide takes you from a fresh install to a lead agent that builds its own team. It uses
the Web console; the [CLI](#doing-the-same-from-the-cli) section at the end shows the same
steps in a terminal.

::: tip The Web console is in Chinese
The console currently uses Chinese labels. This guide gives each label with its meaning, for
example **资源库** (Resources).
:::

Before you start, [install agt](./installation.md) and start the console with
`node web/server.mjs`. Open the full link it prints.

## 1. Connect a model

Open **资源库** (Resources) in the left sidebar and click **新建资源** (New resource).

1. Create a **Provider**: a name, your endpoint's base URL (for example
   `https://api.openai.com/v1`) and the API key.
2. Create a **Model** on that provider. Click **获取模型列表** (Fetch model list) to choose
   from the models the provider offers, or type the name. Pick the API your provider supports
   for that model: **Chat Completions** or **Responses**. Keep tool calling enabled.

See [Connect models](./models.md) for every option.

## 2. Choose how tool calls are approved

By default, every tool call an agent makes waits for approval. Open **运行设置** (Settings) →
**工具权限审批** (Tool approvals), and set **Native 权限审核策略** (Native approval policy):

| Mode | When to use it |
| --- | --- |
| **模型审核，异常转人工** (Model review) | A reviewer model approves routine calls and passes doubtful ones to you. Select the model from step 1 under **审核模型** (Reviewer model). |
| **人工审批** (Manual) | You approve each call yourself. Safest, but the lead makes many calls. |
| **自动放行** (Allow all) | No approvals. Only for experiments in a disposable home. |

Without a reviewer model, model review falls back to manual approval. Pending requests appear
above the message box in the conversation, where you can approve or deny them. See
[Permissions](./permissions.md).

## 3. Create the lead

Open **Agent 团队** (Agent team) and click **创建角色** (Create role):

- **资源名称 / 身份 ID** (Name / identity ID): `lead`
- **执行方式** (Engine): **AGT 内置** (Native), with the model from step 1
- **角色 Prompt** (Role prompt): **使用已有 Prompt** (Use an existing prompt), then
  `default/lead`
- **框架内置工具** (Built-in tools): tick **资源管理** (Resource management) and
  **任务协作** (Task collaboration)
- **响应策略** (Response policy): keep **私聊 / 群内被提及** (Direct messages and mentions)

Saving creates the agent definition, its chat identity and the binding that enables it, so
the lead is ready to talk to.

## 4. Give it a goal

Click **新建会话** (New conversation), choose a direct chat with `lead`, and describe what you
want, for example:

> Set up a two-person review team for this repository: one reviewer focused on correctness
> and one on readability. Put them in a group with me and have them review the latest change.

Keep **发送后自动回复** (Auto-reply after sending) switched on. Sending authorizes the agents
to run, which calls your model and costs money.

The lead checks existing resources, writes role prompts, creates and enables each teammate,
creates the group and briefs the team. While it works:

- The bar above the message box shows each running agent and its tool calls.
- Pending approvals appear there too, if you chose manual approval.
- **Agent 团队** lists the new teammates as they are created. Click one to see its
  configuration, status and recent runs.
- Turn on **调试模式** (Debug mode) in the top-right corner for run IDs and error codes.

## 5. Let it build tools

With a [Codex agent](./codex.md) available, the lead can also build tools. Ask for something
that needs a tool that does not exist yet, for example a small MCP server for a board game or
a data source. The lead delegates the code to a Codex agent, registers the result as an MCP
server, tests that it starts, and attaches it to the agents that need it. See
[Build a team with a lead](./lead-team.md).

## Doing the same from the CLI

Everything the console does goes through the `agt` CLI. The example below creates a
reviewer agent by hand and talks to it; the lead does these same steps through its tools.

```bash
export MODEL_API_KEY=sk-...
agt provider create main --base-url https://api.openai.com/v1 --api-key-env MODEL_API_KEY
agt model create fast --provider main --model gpt-5-mini --api-format responses --tool-call true
agt prompt create reviewer --text 'You review code changes. Stay silent with [[silent]] when you have nothing to add.'
agt agent create reviewer --model fast --prompt reviewer

# Chat identities: yourself and the agent, then enable the agent
agt im principal create me --name Me
agt tool call --principal me --manager principal_create --args '{"id":"reviewer","name":"Reviewer","kind":"agent"}'
agt tool call --principal me --manager agent_bind --args '{"principal":"reviewer","agent":"reviewer"}'

# Open a direct chat and send a message
agt tool call --principal me direct_create --args '{"principal":"reviewer"}'
agt tool call --principal me message_send --args '{"conversation":"<conversation id>","body":"Please review src/main.rs","request_key":"hello-1"}'

# Authorize the discussion and run the scheduler
agt schedule start <conversation id>
agt schedule run --watch
```

In a second terminal, read the reply with
`agt tool call --principal me message_history --args '{"conversation":"<conversation id>"}'`.
The [CLI reference](../reference/cli.md) covers every command.

## Next steps

- [Core concepts](./concepts.md): resources, identities, discussions and tasks
- [Create agents](./agents.md): prompts, skills, MCP servers and built-in tools
- [Troubleshooting](../help/troubleshooting.md): what to check when an agent does not reply
