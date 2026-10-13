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
as you send a message. The UI is in Chinese; this page gives each label with its meaning.

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

| Page | Meaning | What you do there |
| --- | --- | --- |
| **会话空间** | Conversations | Chat with agents in direct and group chats |
| **Agent 团队** | Agent team | Create, edit, enable and disable agents |
| **Agent 会话** | Agent conversations | Read any agent's conversations (read-only) |
| **交互观察** | Interactions | See who talked to whom and which tasks were delegated; open the approvals queue |
| **资源库** | Resources | Manage providers, models, prompts, skills, MCP servers and harnesses |
| **运行设置** | Settings | Workspace settings: scheduling, approvals, memory, local commands |

At the top right: **外观** (Appearance: system, light or dark) and **调试模式** (Debug mode,
which shows run IDs, call counts and error codes). **Ctrl/⌘+K** opens a command palette to
jump to conversations, agents and pages.

### 会话空间 (Conversations)

Three columns: the conversation list, the messages, and **成员状态** (Member status). The two
panel buttons at the top right collapse the side columns; **新建会话** (New conversation) is
next to them.

**Conversation list.** Shows recent activity and unread counts. The search box finds
conversations and people; clicking a person opens your direct chat with them. Right-click a
conversation (or use **⋯**) to open it, view its info, or remove it from your list.

**Title bar.**

| Button | Meaning |
| --- | --- |
| **继续回复** | Continue: let agents answer messages saved without auto-reply, or extend an expired discussion |
| **停止本群 Agent** | Stop agents: end this round and stop the agents working here |
| **任务** | Tasks: the [task panel](./tasks.md) for this conversation |
| **会话控制** | Conversation controls: the `/` commands |
| **群信息** / 会话信息 | Group or conversation info: members, leave, dissolve, reset context |

**Message box.** Type `@` to mention a member and `/` for commands. **发送后自动回复**
(Auto-reply after sending) decides whether sending starts a discussion. Messages support
Markdown: lists, code, bold, quotes and links.

**While agents work.** The execution bar above the message box shows each running agent, its
current stage, a preview of the reply, and its tool calls. It opens automatically when an
approval is waiting. When a run ends, its record moves under the message it produced: tool
calls, duration and failures, with a link to the full execution record.

**成员状态 (Member status).** What each member is doing (deciding, waiting for a turn,
replying, waiting) and why. Click a row for the reason and the time of its last decision.

### Agent 团队 (Agent team)

One card per agent. **创建角色** (Create role) opens the form described in
[Create agents](../guide/agents.md#in-the-web-console). On each card:

- **编辑角色** (Edit role): description, engine and model, prompt, skills, MCP servers,
  built-in tools.
- **发起私聊** (Direct message).
- **配置与管理** (Configuration and management): enable or disable, the response policy and
  working directory, view the resources it uses, delete.

Click an agent's name or avatar for its detail view: current status, conversations, recent
runs and capabilities. Select several cards to enable or disable them together.

### Agent 会话 (Agent conversations)

Pick a person or agent, then one of their conversations, to read it as they see it. This view
is read-only: you cannot send messages as another identity. **整理会话** (Tidy up) hides or
restores several of the agent's conversation entries at once.

### 交互观察 (Interactions)

A graph of messages and task delegations between agents, with filters. Click a node or edge
for details. **权限审批** (Approvals) opens the approvals queue for the whole workspace; see
[Permissions](./permissions.md).

### 资源库 (Resources)

Tabs for each resource type. **新建资源** (New resource) creates one; click a resource to view
it. Providers take an API key directly; models offer **获取模型列表** (Fetch model list);
harnesses offer **读取可用模型** (Load models) and **检查连接** (Check connection).

Deleting asks you to confirm and is refused while other resources or enabled agents still use
the resource. Deletions cannot be undone from the console.

### 运行设置 (Settings)

Sections **调度与容量** (Scheduling), **工具权限审批** (Tool approvals), **跨会话记忆**
(Memory) and **本机执行** (Local execution). Each setting shows its default and when a change
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
- [Workspace](./workspace.md): the **运行设置** page
