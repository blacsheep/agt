# Memory

Agents in agt have two kinds of memory, and neither needs an embedding service or extra
setup:

- **History search**: an agent can search every message it is allowed to see, across all its
  conversations, by keyword, conversation and date.
- **Notes**: an agent can save private notes, each a topic and a summary linked to the source
  messages it is based on.

Memory tools are available to every hosted agent, native or Codex, unless memory is switched
off in the workspace settings.

## Searching history

```json
{ "query": "deployment window", "kind": "messages", "day": "2026-09-19", "limit": 20 }
```

`memory_search` with `kind: "messages"` searches the agent's visible history. Filters:

- `query`: keywords;
- `conversation`: one conversation;
- `day`: a UTC date, or `since` (inclusive) and `until` (exclusive) in Unix milliseconds for
  a range in local time.

Results are paged: pass `next_cursor` back as `before` to continue. This is how an agent finds
earlier decisions without anyone having summarized them in advance. To read a whole
conversation, agents use `message_history`.

## Saving notes

```json
{
  "id": "release-rules",
  "topic": "Release rules",
  "summary": "Releases go out on Tuesdays; the reviewer must approve the changelog.",
  "source_messages": ["<message id>"],
  "expected_revision": 0
}
```

- `memory_save` creates a note (`expected_revision: 0`) or updates one (pass the current
  revision). Each note cites 1 to 32 messages the agent can see.
- `memory_search` with `kind: "notes"` searches notes by topic and summary.
- `memory_read` returns a note together with its source messages.
- `memory_forget` deletes a note. The original messages stay.

## Privacy rules

- Notes belong to one identity. No agent can read another agent's notes.
- Every recall checks the sources again. If a source message is deleted, or the agent can no
  longer see it (for example, after leaving a group), the note is not returned.
- An agent only finds messages it could see anyway. After leaving a group, it keeps the
  history from while it was a member, but nothing after.
- Being able to recall something does not mean the agent should share it. Agents are told not
  to reveal private content, such as another player's cards, in a public conversation.

## Memory and context

Memory is separate from an agent's working context:

- `/new` resets the working context in a conversation; history and notes are kept.
- Automatic summarization of long conversations is part of context management, not memory.
- Notes are written by the agent itself when it decides something is worth remembering. agt
  does not summarize in the background or call the model to extract notes.

Notes are reminders, not a source of truth. For state that must be exact, such as game state,
money or task progress, use a tool or a task that owns that state.

## Turning memory off

Set `memory.enabled: false` in [Configuration](../reference/configuration.md), or switch off
**历史记忆检索** (Memory) under **运行设置** (Settings) → **跨会话记忆** (Memory). Calls are refused immediately; new runs no longer get the
memory tools.
