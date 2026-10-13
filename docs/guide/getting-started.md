# Getting started

This guide takes you from a fresh clone to a lead agent that builds its own team.

::: tip The Web console is in Chinese
The console UI currently uses Chinese labels. This guide shows each label with its meaning,
for example **资源库** (Resources).
:::

## Requirements

- Rust 1.96.1 or newer
- Node.js 22 or newer (no npm packages are needed for the console)
- An OpenAI-compatible model endpoint whose model supports tool calls
- Optional: the [Codex CLI](https://github.com/openai/codex) on your `PATH`, if you want
  Codex-backed agents

## 1. Build and start

```bash
git clone https://github.com/blacsheep/agt.git
cd agt
cargo build -p agt-cli
node web/server.mjs
```

Open the full link printed in the terminal, including the token after `#`. The server
listens on `127.0.0.1:4317`, starts the scheduler in the background, and uses `~/.agt` as
its home. Set `AGT_HOME` to an absolute path first if you want the data somewhere else.

On Windows, pass the binary explicitly:
`node web/server.mjs --binary .\target\debug\agt.exe`.

## 2. Connect a model

Open **资源库** (Resources) and use **新建资源** (New resource):

1. Create a **Provider** with your endpoint's base URL and API key.
2. Create a **Model** on that provider. Click **获取模型列表** (Fetch model list) to pick
   from the models the provider offers, or type the name. Choose the API your provider
   supports for that model: Chat Completions or Responses.

## 3. Create the lead

Open **Agent 团队** (Agent team) and click **创建角色** (Create role):

- **资源名称 / 身份 ID** (Name / identity ID): for example `lead`.
- Model: the model from step 2.
- **角色 Prompt** (Role prompt): choose **使用已有 Prompt** (Use an existing prompt), then
  `default/lead`.
- **框架内置工具** (Built-in tools): tick **资源管理** (Resource management) and
  **任务协作** (Task collaboration).
- Keep the default **响应策略** (Response policy), which replies to direct messages and
  mentions.

Saving creates the agent definition, its identity and its binding, so the lead is ready to
chat.

## 4. Give it a goal

Click **新建会话** (New conversation), start a direct chat with the lead, and describe what
you want. For example:

> Set up a two-person code review team for this repository: one reviewer focused on
> correctness and one on readability. Put them in a group with me and have them review
> the latest change.

Keep **发送后自动回复** (Auto-reply after sending) on. Sending then authorizes the
agents to run, which calls your model and may cost money.

Watch the lead work. It will check existing resources, write role prompts, create and bind
each teammate, create the group, and brief the team. Open a teammate from **Agent 团队**
to see its configuration, current status and recent runs. Turn on debug mode in the top
right corner to see run IDs, tool calls and error codes.

## 5. Let it build tools

Codex-backed agents let the team build its own tools. Make sure the Codex CLI is installed,
then ask the lead for something that needs a new tool, for example a small MCP server for a
game or a data source. The lead asks a Codex agent to write it with tests, registers it as
an MCP resource with an absolute `cwd`, probes it the way a real run would start it, and
attaches it to the agents that need it.

## Using the CLI instead

Everything the console does goes through the `agt` CLI, so the same steps work from a
terminal. `agt --help` lists the commands, and `agt tool call` invokes the same tools the
lead uses:

```bash
target/debug/agt agent list
target/debug/agt tool list --principal web-operator --manager
target/debug/agt tool call --principal web-operator --manager resource_list --args '{"kind":"prompt"}'
```

## Where to go next

The detailed guides are currently written in Chinese:

- [Web console guide](../zh/guide/web-console.md): every screen, auto-reply rules and
  troubleshooting
- [CLI reference](../zh/guide/cli.md)
- [Built-in tools](../zh/guide/builtin-tools.md) and [task collaboration](../zh/guide/tasks.md)
- [Architecture](../zh/architecture/index.md)
