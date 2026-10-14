# Prompt

| Overview | |
| --- | --- |
| **What it is** | A role description, used as an agent's system prompt |
| **CLI** | `agt prompt` |
| **Agent tools** | `resource_*` with `kind: "prompt"` |
| **Web console** | **Resources** → Prompt; **Role prompt** in the agent form |
| **File** | `prompts/<category>/<name>/PROMPT.md` |
| **Crate** | `agt-prompt` |

## What it does

A prompt defines an agent's role: what it is responsible for, who it works with, and when it
should stay quiet. Several agents can share one prompt; editing it changes all of them from their
next run.

agt ships two prompts:

| Prompt | Purpose |
| --- | --- |
| `default/default` | A minimal general-purpose prompt. Cannot be deleted. |
| `default/lead` | Knows the whole framework and can build and run a team. See [Build a team with a lead](../guide/lead-team.md). |

`agt init` creates both, and never overwrites a prompt that already exists.

## How to use it

**Web console:** **Resources** → **New resource** → Prompt, or **Write a new one** (新写一份) in the agent
form. Prompts show a 2,000-character preview that you can expand.

**CLI:**

```bash
agt prompt create reviewer --text 'You review code for correctness...'
agt prompt update reviewer --file ./reviewer.md
cat reviewer.md | agt prompt update reviewer --stdin
agt prompt read reviewer
```

**Agent tools:**

```json
{"kind":"prompt","reference":"reviewer","create":true,"data":{"text":"You review code..."}}
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `--text` / `--file` / `--stdin` | Where the prompt text comes from (exactly one) |

## All parameters

### `agt prompt create <name>` and `agt prompt update <name>`

| Option | Meaning |
| --- | --- |
| `--text <text>` | Prompt text |
| `--file <path>` | Read the text from a file |
| `--stdin` | Read the text from standard input |
| `--category <c>` | Category (default `default`) |

Give exactly one of `--text`, `--file` and `--stdin`. Update replaces the whole text.

### Other commands

| Command | Meaning |
| --- | --- |
| `read <ref>` | Print the prompt text |
| `list`, `info`, `delete`, `category` | See [Resources](./resources.md#shared-cli-commands) |

### File: `PROMPT.md`

Plain Markdown. The whole file is the prompt.

### Tool fields (`resource_write`, `kind: "prompt"`)

| Field | Meaning |
| --- | --- |
| `text` | The full prompt text |

## Writing good prompts

- State the role, the people and agents it works with, and what "done" means.
- In groups, say when to answer exactly `[[silent]]`, so the agent does not reply to everything.
- Describe the tools it should use for important actions, for example `task_request` for work
  that must produce a result.

## Related modules

- [Agent](./agent.md): attaching a prompt to an agent
- [Skill](./skill.md): reusable instructions added on top of the prompt
