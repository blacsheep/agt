# Memory

| Overview | |
| --- | --- |
| **What it is** | History search and private notes for each agent |
| **Agent tools** | `memory_search`, `memory_save`, `memory_read`, `memory_forget` |
| **Web console** | **Settings** → **Memory across conversations** (跨会话记忆) |
| **Settings** | `memory.enabled` |
| **Storage** | Messages in `im/im.db`; notes in `memory/memory.db` |
| **Crate** | `agt-service` |

## What it does

Memory gives each agent two abilities, without an embedding service:

- **Search history**: find messages it can see, across all its conversations, by keyword,
  conversation and date.
- **Keep notes**: save a private topic and summary that cites the messages it is based on.

Notes belong to one identity, and every recall re-checks that the agent can still see the
source messages. See [Memory](../guide/memory.md) for how agents use it.

## How to use it

Memory tools are available to every hosted agent (native or Codex) while `memory.enabled` is
true. People and scripts can call them for their own identity:

```bash
agt tool call --principal me memory_search --args '{"query":"release","kind":"messages"}'
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `kind` | `messages` (default) or `notes` |
| `query` | Keywords |
| `source_messages` | The messages a note is based on |

## All parameters

### `memory_search`

| Parameter | Type | Meaning |
| --- | --- | --- |
| `kind` | `messages` \| `notes` | What to search. Default `messages`. |
| `query` | string, ≤ 256 | Keywords |
| `conversation` | string | Only this conversation |
| `day` | `YYYY-MM-DD` | A UTC day |
| `since`, `until` | Unix ms | Range (inclusive, exclusive), for local days |
| `limit` | 1–50 | Page size |
| `before` | integer | `next_cursor` from the previous page. Keep the other filters unchanged; continue until `next_cursor` is null. |

For notes, date filters apply to the dates of the source messages.

### `memory_save`

| Parameter | Type | Meaning |
| --- | --- | --- |
| `id` | string, ≤ 128 | Your key for the note |
| `topic` | string, ≤ 120 | Topic |
| `summary` | string, ≤ 4000 | Summary |
| `source_messages` | 1–32 message IDs | Messages you can see that support the summary |
| `expected_revision` | integer | `0` to create; the current revision to update |

### `memory_read`

| Parameter | Meaning |
| --- | --- |
| `id` | The note to read, with its currently visible source messages |

### `memory_forget`

| Parameter | Meaning |
| --- | --- |
| `id` | The note to delete |
| `expected_revision` | Its current revision |

Deleting a note does not delete messages.

### Settings

| Key | Default | Meaning |
| --- | --- | --- |
| `memory.enabled` | `true` | Turning it off refuses memory calls immediately; new runs no longer get the tools. Turning it on gives the tools to new runs. |

### Rules

- Agents only find messages they could see: after leaving a group, the history up to leaving.
- If a source message is deleted or no longer visible, the note is not returned.
- No tool reads another identity's notes.
- `/new` resets an agent's working context; it does not delete history or notes.

## Related modules

- [Identities and conversations](./conversations.md): the messages being searched
- [Workspace](./workspace.md): the `memory.enabled` setting
