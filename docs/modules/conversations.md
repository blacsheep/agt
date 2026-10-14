# Identities and conversations

| Overview | |
| --- | --- |
| **What it is** | Participants, direct and group chats, and messages |
| **CLI** | `agt im` |
| **Agent tools** | `principal_list`, `direct_create`, `group_create`, `member_list`, `member_add`, `member_remove`, `conversation_*`, `group_dissolve`, `response_policy`, `message_*` |
| **Web console** | **Conversations** (会话空间), **Agent views** (Agent 会话) |
| **Storage** | `im/im.db` |
| **Crate** | `agt-im` |

## What it does

This module is a small instant-messaging system shared by people and agents:

- **Identities** (principals) are participants: users, agents, bots or services.
- **Direct chats** connect two identities; a pair always shares the same direct chat.
- **Groups** have an owner, admins and members. Messages go to every current member.
- **Messages** have a sequence number within their conversation, optional mentions and a reply
  target, and are stored in SQLite.

Storing a message never runs an agent by itself. Agents reply only when a discussion is
authorized; see [Scheduler](./scheduler.md).

When an agent creates a group, the **workspace owner** (the person who first started the Web
console) joins as group owner automatically, and the creating agent becomes an admin.

When a person creates a group, they are its owner and everyone they add is a member. Only the
owner and admins can add or remove members, so a lead you add to your group cannot invite
teammates until you make it an admin: **Group info** (群信息) → **Make admin** (设为管理员) next to
it, or `member_role_set`. Only the owner changes roles, and ownership cannot be transferred.

## How to use it

**Web console:** **Conversations** → **New conversation**; type `@` to mention and `/` for commands.

**CLI (as yourself, through tools):**

```bash
agt tool call --principal me direct_create --args '{"principal":"reviewer"}'
agt tool call --principal me group_create --args '{"title":"Design review","members":["reviewer","planner"]}'
agt tool call --principal me message_send --args '{"conversation":"<id>","body":"Ready?","mentions":["reviewer"],"request_key":"msg-1"}'
agt tool call --principal me message_history --args '{"conversation":"<id>"}'
```

**CLI (low-level store commands):** `agt im ...`, below.

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `mentions` | Identities asked to respond in a group |
| `request_key` | Makes sending idempotent: the same key with the same content sends once |
| `policy` | An identity's response policy in a conversation |

## All parameters

### Agent tools

Every identity has these tools; the identity is fixed by the host and cannot be changed through
arguments.

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `principal_list` | `include_deleted?` | List identities (deleted ones only for resolving old names) |
| `direct_create` | `principal` | Create or find your direct chat with someone |
| `group_create` | `title`, `members` | Create a group |
| `conversation_list` | `groups_only?`, `include_hidden?` | Your conversations, including read-only history of groups you left or that were dissolved |
| `conversation_hide` | `conversation` | Remove the entry from your list only; a new message brings it back |
| `conversation_show` | `conversation` | Restore a hidden entry |
| `member_list` | `conversation` | Members of a conversation you belong to |
| `member_add` | `conversation`, `principal` | Invite (owner or admin) |
| `member_remove` | `conversation`, `principal` | Remove someone (owner or admin), or leave yourself. The owner cannot leave. |
| `member_role_set` | `conversation`, `principal`, `role` (`admin` or `member`) | Group owner only: make a member an admin or an admin a member. Offered to people, not agents. |
| `group_dissolve` | `conversation` | Dissolve a group you own; history stays read-only |
| `response_policy` | `conversation`, `policy` | Set your own policy here: `all`, `mentioned` or `manual` |
| `message_send` | see below | Send a message |
| `message_history` | `conversation`, `before?`, `limit?` (1–200, default 50) | Messages, oldest first within a page. Pass `next_cursor` as `before` for older ones. |
| `message_inbox` | `pending_only?` (default true), `limit?` | Your deliveries and their processing state |
| `message_ack` | `id` | Acknowledge a delivery (for external clients) |
| `message_status` | `message_id` | Per-recipient processing state of a message |

**`message_send` parameters**

| Parameter | Required | Meaning |
| --- | --- | --- |
| `conversation` | yes | Target conversation |
| `body` | yes | Text (Markdown) |
| `request_key` | yes | Idempotency key. Reuse it only for identical content. |
| `mentions` | no | Identities to mention |
| `reply_to` | no | ID of a message in the same conversation |
| `input_mode` | no | When an agent that is already running receives it: `steering` (at its next tool boundary, default for people) or `follow_up` (after its current work, default for agents) |

### Response policies

| Policy | Direct chat | Group |
| --- | --- | --- |
| `mentioned` (default) | Replies | Replies when mentioned |
| `all` | Replies | Considers every message; may stay silent or wait |
| `manual` | Never automatically | Never automatically |

A mention is the structured `mentions` field; `@name` typed in the text is not a mention. An agent
that answers exactly `[[silent]]` posts nothing.

### `agt im` commands

These operate on the store directly, as a trusted local administrator.

| Command | Options | Meaning |
| --- | --- | --- |
| `principal create <id>` | `--name <n>`, `--kind <k>` (default `user`; also `agent`, `bot`, `service`, `other`) | Create an identity |
| `principal list` / `info <id>` | | List or show identities |
| `owner get` | | Show the workspace owner |
| `owner ensure <user>` | | Set the owner if none is set, and make it owner of existing agent-created groups |
| `direct create <a> <b>` | | Create or find a direct chat |
| `group create` | `--owner <id>` (required), `--title <t>`, `--member <id>` (repeatable) | Create a group |
| `group members list <conv>` | | List members |
| `group members add <conv> <id>` | `--actor <id>` | Add a member |
| `group members remove <conv> <id>` | `--actor <id>` | Remove a member |
| `group members role <conv> <id> <admin\|member>` | `--actor <owner>` | Change a member's role |
| `group members leave <conv> <id>` | | Leave |
| `group dissolve <conv>` | `--actor <owner>` | Dissolve |
| `conversation list` | `--principal <id>` (required), `--include-hidden`, `--groups-only` | List an identity's conversations |
| `conversation info <conv>` | | Show a conversation |
| `conversation hide` / `unhide <conv>` | `--actor <id>` | Hide or restore an entry |
| `conversation visit <conv>` | `--actor <id>` | Record a visit (for recent-history ordering) |
| `conversation archive` / `reactivate` / `delete <conv>` | `--actor <id>` | Archive, restore, or soft-delete a conversation |
| `message send <conv> <body>` | `--sender <id>` (required), `--idempotency-key <k>`, `--client-message-id <id>`, `--input-mode steering\|follow_up` | Store a message |
| `message list <conv>` | `--viewer <id>` (required), `--limit <n>` (default 50), `--before <cursor>`, `--after <cursor>` | List messages |
| `message read <conv>` | `--principal <id>`, `--through-seq <n>` (both required) | Mark messages as read |
| `message delete <message>` | `--actor <id>` | Soft-delete a message |

To delete a conversation and also stop agent work in it, use `agt schedule delete`; see
[Scheduler](./scheduler.md).

### Visibility rules

- New members can read earlier history; unread counts start from when they joined.
- Members who leave keep the history up to when they left, and see nothing after.
- Hiding an entry affects only your own list.
- Deletions are soft: records are marked, not erased.

## Related modules

- [Scheduler](./scheduler.md): when agents reply
- [Memory](./memory.md): searching history
- [Web console](./web-console.md): the chat interface
