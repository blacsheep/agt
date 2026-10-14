# Web console

| Overview | |
| --- | --- |
| **What it is** | AGT Studio, the local browser UI for agt |
| **Start** | `agt web` |
| **Needs** | Node.js 22 or newer |
| **Code** | `web/` (Node.js server and plain HTML/CSS/JavaScript, no npm dependencies), built into `agt` |

## What it does

The Web console is a thin local UI over the `agt` CLI: every action calls the CLI, so anything you
do here you can also do in a terminal. The server also runs the scheduler, so agents reply as soon
as you send a message.

The console is in English and Chinese. It follows your browser's language; switch with the
language menu at the top right, and the choice is remembered in that browser. This page names
each control in English, with the Chinese label in parentheses.

## How to use it

```bash
agt web
```

Open the full link printed in the terminal, including the token after `#`. On a remote server,
use an SSH tunnel (`ssh -N -L 4317:127.0.0.1:4317 user@server`); see
[Installation](../guide/installation.md#running-on-a-remote-server).

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `agt --home <dir> web` | Workspace to open (default `$AGT_HOME`, then `~/.agt`) |
| `--port <n>` | Port on `127.0.0.1` (default 4317) |

## All parameters

### `agt web`

| Option | Default | Meaning |
| --- | --- | --- |
| `--port <n>` | `4317` | Port. The console only listens on `127.0.0.1`. |
| `--principal <id>` | `web-operator` | Your identity. The first identity to start the console on a home becomes the workspace owner; later starts must use the same identity. |
| `--node <path>` | `node` | Node.js executable (22 or newer) |
| `--home <dir>` (global) | `$AGT_HOME`, then `~/.agt` | Home directory; write it before `web` |

`agt web` writes the console's files to `<home>/.web/<version hash>/` on first start, removes
copies from other versions, and runs them with Node.js, passing its own path so the console and
the scheduler use the same `agt`. Ctrl+C stops the console and the scheduler.

On first start the console creates your user identity. It never calls a model for old messages
on start.

### Running from source

In a repository clone, `node web/server.mjs` runs the console from `web/` directly. It accepts
`--home`, `--port`, `--principal`, and `--binary <path>` (the `agt` to call, default
`target/debug/agt`).

### Pages

The left sidebar has six pages. Click the menu button at the top left to show their names.

| Page | Chinese label | What you do there |
| --- | --- | --- |
| **Conversations** | 会话空间 | Chat with agents in direct and group chats |
| **Agent team** | Agent 团队 | Create, edit, enable and disable agents |
| **Agent views** | Agent 会话 | Read any agent's conversations (read-only) |
| **Interactions** | 交互观察 | See who talked to whom and which tasks were delegated; open the approvals queue |
| **Resources** | 资源库 | Manage providers, models, prompts, skills, MCP servers and harnesses |
| **Settings** | 运行设置 | Workspace settings: scheduling, approvals, memory, local commands |

At the top right: **Appearance** (外观: system, light or dark), the language menu, and **Debug** (调试模式,
which shows run IDs, call counts and error codes). **Ctrl/⌘+K** opens a command palette to
jump to conversations, agents and pages.

### 会话空间 (Conversations)

Three columns: the conversation list, the messages, and **Members** (成员状态). The two
panel buttons at the top right collapse the side columns; **New conversation** (新建会话) is
next to them.

**Conversation list.** Shows recent activity and unread counts. The search box finds
conversations and people; clicking a person opens your direct chat with them. Right-click a
conversation (or use **⋯**) to open it, view its info, or remove it from your list.

**Title bar.**

| Button | Meaning |
| --- | --- |
| **Continue** (继续回复) | Let agents answer messages saved without auto-reply, or extend an expired discussion |
| **Stop agents in this group** (停止本群 Agent) | End this round and stop the agents working here |
| **Tasks** (任务) | The [task panel](./tasks.md) for this conversation |
| **Conversation controls** (会话控制) | The `/` commands |
| **Group info** (群信息) / **Conversation info** (会话信息) | Members, leave, dissolve, reset context |

**Message box.** Type `@` to mention a member and `/` for commands. **Auto-reply after sending**
(发送后自动回复) decides whether sending starts a discussion. Messages support
Markdown: lists, code, bold, quotes and links.

**While agents work.** The execution bar above the message box shows each running agent, its
current stage, a preview of the reply, and its tool calls. It opens automatically when an
approval is waiting. When a run ends, its record moves under the message it produced: tool
calls, duration and failures, with a link to the full execution record.

**Members (成员状态).** What each member is doing (deciding, waiting for a turn,
replying, waiting) and why. Click a row for the reason and the time of its last decision.

### Agent 团队 (Agent team)

One card per agent. **Create agent** (创建 Agent) opens the form described in
[Create agents](../guide/agents.md#in-the-web-console). On each card:

- **Edit role** (编辑角色): description, engine and model, prompt, skills, MCP servers,
  built-in tools.
- **Direct chat** (发起私聊).
- **Configure** (配置与管理): enable or disable, the response policy and
  working directory, view the resources it uses, delete.

Click an agent's name or avatar for its detail view: current status, conversations, recent
runs and capabilities. Select several cards to enable or disable them together.

### Agent 会话 (Agent conversations)

Pick a person or agent, then one of their conversations, to read it as they see it. This view
is read-only: you cannot send messages as another identity. **Organize** (整理会话) hides or
restores several of the agent's conversation entries at once.

### 交互观察 (Interactions)

A graph of messages and task delegations between agents, with filters. Click a node or edge
for details. **Approvals** (权限审批) opens the approvals queue for the whole workspace; see
[Permissions](./permissions.md).

The page opens in **Global view** (全局视角), which shows every conversation any identity can
see, including chats between agents that you are not in, such as the lead briefing a Codex
agent. It refreshes every few seconds. To see exactly what one agent sees, pick it under
**Scope and filters** (观察范围与筛选).

### 资源库 (Resources)

Tabs for each resource type. **New resource** (新建资源) creates one; click a resource to view
it. Providers take an API key directly; models offer **Fetch model list** (获取模型列表);
harnesses offer **Load models** (读取可用模型) and **Check connection** (检查连接).

Deleting asks you to confirm and is refused while other resources or enabled agents still use
the resource. Deletions cannot be undone from the console.

### 运行设置 (Settings)

Sections **Scheduling and capacity** (调度与容量), **Tool approvals** (工具权限审批), **Memory across conversations**
(跨会话记忆) and **Local execution** (本机执行). Each setting shows its default and when a change
takes effect. See [Configuration](./workspace.md#config-yaml).

### Message box commands

Type `/` in the message box for the commands available in this conversation. Commands are not
sent as messages; type `//` to send text that starts with `/`.

| Command | Meaning |
| --- | --- |
| `/help` | List the commands |
| `/new`, `/clear` | Reset the agents' working context here; chat history is kept |
| `/stop` | End this round and stop the agents working here |
| `/status` | Each agent's state, engine and recent runs |
| `/model` | Codex agents: choose model and reasoning level for this conversation |
| `/compact` | Codex agents: compact the context |
| `/rename` | Codex agents: name the session |
| `/skills`, `/mcp` | Codex agents: skills and MCP servers configured in Codex |
| `/permissions` | Codex agents: the session's file permissions |

### Keyboard

| Keys | Action |
| --- | --- |
| Ctrl/⌘+K | Command palette |
| Ctrl/⌘+Enter | Send |
| `@` | Mention a member |
| `/` | Commands |
| Shift+F10 | Context menu of the focused conversation |

## Related modules

- [Identities and conversations](./conversations.md): what the chat pages show
- [Scheduler](./scheduler.md): the buttons that control discussions
- [Workspace](./workspace.md): the **Settings** page
