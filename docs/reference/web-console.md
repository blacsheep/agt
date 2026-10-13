# Web console

The Web console (AGT Studio) is a local UI over the `agt` CLI. Start it with
`node web/server.mjs` and open the full link it prints; see
[Installation](../guide/installation.md#start-the-web-console). The UI is in Chinese; this page
gives each label with its meaning.

## Pages

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

## 会话空间 (Conversations)

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
| **任务** | Tasks: the [task panel](../guide/tasks.md#in-the-web-console) for this conversation |
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

## Agent 团队 (Agent team)

One card per agent. **创建角色** (Create role) opens the form described in
[Create agents](../guide/agents.md#in-the-web-console). On each card:

- **编辑角色** (Edit role): description, engine and model, prompt, skills, MCP servers,
  built-in tools.
- **发起私聊** (Direct message).
- **配置与管理** (Configuration and management): enable or disable, the response policy and
  working directory, view the resources it uses, delete.

Click an agent's name or avatar for its detail view: current status, conversations, recent
runs and capabilities. Select several cards to enable or disable them together.

## Agent 会话 (Agent conversations)

Pick a person or agent, then one of their conversations, to read it as they see it. This view
is read-only: you cannot send messages as another identity. **整理会话** (Tidy up) hides or
restores several of the agent's conversation entries at once.

## 交互观察 (Interactions)

A graph of messages and task delegations between agents, with filters. Click a node or edge
for details. **权限审批** (Approvals) opens the approvals queue for the whole workspace; see
[Permissions](../guide/permissions.md).

## 资源库 (Resources)

Tabs for each resource type. **新建资源** (New resource) creates one; click a resource to view
it. Providers take an API key directly; models offer **获取模型列表** (Fetch model list);
harnesses offer **读取可用模型** (Load models) and **检查连接** (Check connection).

Deleting asks you to confirm and is refused while other resources or enabled agents still use
the resource. Deletions cannot be undone from the console.

## 运行设置 (Settings)

Sections **调度与容量** (Scheduling), **工具权限审批** (Tool approvals), **跨会话记忆**
(Memory) and **本机执行** (Local execution). Each setting shows its default and when a change
takes effect. See [Configuration](./configuration.md).

## Keyboard

| Keys | Action |
| --- | --- |
| Ctrl/⌘+K | Command palette |
| Ctrl/⌘+Enter | Send |
| `@` | Mention a member |
| `/` | Commands |
| Shift+F10 | Context menu of the focused conversation |
