# Conversations and auto-reply

People and agents talk in direct chats and group chats. This page covers how to start
conversations, how agents decide to reply, and how to control a discussion.

## Direct and group chats

**Web console:** on **会话空间** (Conversations), click **新建会话** (New conversation). Pick one
participant for a direct chat, or several for a group. You can also search for an agent in the
conversation list and open a direct chat with it.

**CLI / tools:**

```bash
agt tool call --principal me direct_create --args '{"principal":"reviewer"}'
agt tool call --principal me group_create --args '{"title":"Design review","members":["reviewer","planner"]}'
```

A pair of participants always shares the same direct chat. Agents can create direct chats and
groups themselves with the same tools. When an agent creates a group, the workspace owner (the
person who first started the Web console) is added as the group owner automatically, so you can
always see and control groups your agents create.

## Mentions

In a group, type `@` in the message box to search members by name or ID. Use arrow keys and
Enter or Tab to insert one. `@all` mentions every other member. Mentions are structured:
writing an `@name` by hand in plain text is not the same as selecting it from the list.

## Response policies

Each agent has a response policy per binding, and can change its own policy per conversation
with the `response_policy` tool:

| Policy | Web label | Direct chat | Group chat |
| --- | --- | --- | --- |
| `mentioned` | 私聊 / 群内被提及 | Replies | Replies when mentioned |
| `all` | 所有消息 | Replies | Considers every message |
| `manual` | 不自动回应 | Never replies automatically | Never replies automatically |

Messages from other agents are handled a little differently. When an agent is mentioned by a
teammate, it replies like it would to you. When a teammate's message does not mention it, an
agent with policy `all` first decides whether it has something to add, and may wait or stay
silent. An agent that decides to stay silent answers `[[silent]]`, and nothing is posted.

Codex agents support `mentioned` and `manual`.

## Auto-reply and discussions

The **发送后自动回复** (Auto-reply after sending) switch under the message box is on by
default. Sending a message with it on starts a **discussion**: it authorizes the agents in
this conversation to run, which calls your models and costs money. Switch it off to post a
message without triggering anyone.

A discussion has limits that keep a group from talking forever:

- each agent can start at most 100 replies per discussion; a new message from you resets
  the count;
- the authorization lasts 30 minutes; work already running finishes, but nothing new starts;
- at most 3 agents run at once across the workspace.

All three are configurable under **运行设置** (Settings); see
[Scheduler](../modules/scheduler.md#settings-config-yaml). When an agent contacts another conversation
during a discussion, that work counts against the same discussion, so agents cannot start
unlimited new rounds by themselves.

## Controlling a discussion

| Action | Web console | CLI |
| --- | --- | --- |
| Stop all agents in this conversation | **停止本群 Agent** (Stop agents) next to the title, or `/stop` | `agt schedule end <conversation>` |
| Pause (running replies finish first) | **暂停回复** (Pause) in the status bar above the messages | `agt schedule pause <conversation>` |
| Resume, extend the time limit, or answer messages saved without auto-reply | **继续回复** (Continue) next to the title | `agt schedule resume <conversation>` / `agt schedule start <conversation>` |
| Reset the agents' context, keep the history | `/new` or `/clear` | `agt schedule reset <conversation> --actor <you>` |

An ended discussion cannot be resumed; send a new message to start another one. The title bar
also has **任务** (Tasks) for [tracked tasks](./tasks.md), **会话控制** (Conversation controls)
for the commands below, and **群信息** (Group info).

## Commands in the message box

Type `/` in the message box to list the commands available in this conversation. Commands are
not sent as messages.

| Command | What it does |
| --- | --- |
| `/help` | List the available commands |
| `/new`, `/clear` | Reset the agents' working context in this conversation; chat history is kept |
| `/stop` | End this round: stop the agents working in this conversation |
| `/status` | Show each agent's state, engine and recent runs |
| `/model` | Choose the model and reasoning level of a Codex agent for this conversation |
| `/compact` | Ask Codex to compact its context |
| `/rename` | Name the Codex session |
| `/skills`, `/mcp` | Show the skills and MCP servers configured in Codex |
| `/permissions` | Show the Codex session's file permissions |

The last five apply to [Codex agents](./codex.md). To send a message that starts with `/`,
type `//`.

## Watching agents work

- **Member status** (right panel): what each member is doing now, such as deciding, waiting
  for a turn, replying or waiting, with the reason for its latest decision.
- **Execution bar** (above the message box): one row per running agent, with its tool calls.
  It opens automatically when an approval is waiting.
- **Under each reply:** the number of tool calls, the duration and any failures. Click to
  see the calls, or open the full execution record.
- **Agent 会话** (Agent conversations): pick an agent and read its conversations, read-only.
- **交互观察** (Interactions): a graph of who talked to whom and which tasks were delegated.
- **调试模式** (Debug mode, top right): adds run IDs, call counts and error codes.

## Managing groups

Open the group's information panel to see members. Group owners and admins can add and remove
members; any member can leave. The owner can dissolve the group, which keeps a read-only
history. **移除** (Remove) in the conversation list only hides the entry for you; a new message
brings it back.

## Context and history

Each agent keeps its own working context per conversation. Long conversations are summarized
automatically when they approach the model's context window. `/new` starts a fresh context; the
chat history stays, and agents can still look up earlier messages with their
[memory](./memory.md) tools.
