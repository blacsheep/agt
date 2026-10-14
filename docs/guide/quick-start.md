# Quick start

This guide takes you from a fresh install to a lead agent that builds its own team. It uses
the Web console; the [CLI](#doing-the-same-from-the-cli) section at the end shows the same
steps in a terminal.

::: tip Console language
The console is in English and Chinese and follows your browser's language; switch it with the
language menu at the top right. This guide names each control in English, with the Chinese
label in parentheses, for example **Resources** (资源库).
:::

Before you start, [install agt](./installation.md) and start the console with `agt web`.
Open the full link it prints.

## 1. Connect a model

Open **Resources** (资源库) in the left sidebar and click **New resource** (新建资源).

1. Create a **Provider**: a name, your endpoint's base URL (for example
   `https://api.openai.com/v1`) and the API key.
2. Create a **Model** on that provider. Click **Fetch model list** (获取模型列表) to choose
   from the models the provider offers, or type the name. Pick the API your provider supports
   for that model: **Chat Completions** or **Responses**. Keep tool calling enabled.

See [Connect models](./models.md) for every option.

## 2. Choose how tool calls are approved

By default, every tool call an agent makes waits for approval. Open **Settings** (运行设置) →
**Tool approvals** (工具权限审批), and set **Native approval policy** (Native 权限审核策略):

| Mode | When to use it |
| --- | --- |
| **Model review, doubtful calls to you** (模型审核，异常转人工) | A reviewer model approves routine calls and passes doubtful ones to you. Select the model from step 1 under **Reviewer model** (审核模型). |
| **Manual** (人工审批) | You approve each call yourself. Safest, but the lead makes many calls. |
| **Allow all** (自动放行) | No approvals. Only for experiments in a disposable home. |

Without a reviewer model, model review falls back to manual approval. Pending requests appear
above the message box in the conversation, where you can approve or deny them. See
[Permissions](./permissions.md).

## 3. Create the lead

Open **Agent team** (Agent 团队) and click **Create agent** (创建 Agent):

- **Name / identity ID** (资源名称 / 身份 ID): `lead`
- **Engine** (执行方式): **Native** (AGT 内置), with the model from step 1
- **Role prompt** (角色 Prompt): **Use an existing prompt** (使用已有 Prompt), then
  `default/lead`
- **Built-in tools** (框架内置工具): tick **Resource management** (资源管理) and
  **Task collaboration** (任务协作)
- **Response policy** (响应策略): keep **Direct chats and mentions** (私聊 / 群内被提及)

Saving creates the agent definition, its chat identity and the binding that enables it, so
the lead is ready to talk to.

## 4. Give it a goal

Click **New conversation** (新建会话), choose a direct chat with `lead`, and describe what you
want, for example:

> Set up a two-person review team for this repository: one reviewer focused on correctness
> and one on readability. Put them in a group with me and have them review the latest change.

Keep **Auto-reply after sending** (发送后自动回复) switched on. Sending authorizes the agents
to run, which calls your model and costs money.

The lead checks existing resources, writes role prompts, creates and enables each teammate,
creates the group and briefs the team. While it works:

- The bar above the message box shows each running agent and its tool calls.
- Pending approvals appear there too, if you chose manual approval.
- **Agent team** lists the new teammates as they are created. Click one to see its
  configuration, status and recent runs.
- Turn on **Debug** (调试模式) in the top-right corner for run IDs and error codes.

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
The [module pages](../modules/workspace.md) list every command and parameter.

## Next steps

- [Core concepts](./concepts.md): resources, identities, discussions and tasks
- [Create agents](./agents.md): prompts, skills, MCP servers and built-in tools
- [Troubleshooting](../help/troubleshooting.md): what to check when an agent does not reply
