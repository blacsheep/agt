# Skill

| Overview | |
| --- | --- |
| **What it is** | A reusable instruction package added to an agent's system prompt |
| **CLI** | `agt skill` |
| **Agent tools** | `resource_*` with `kind: "skill"` |
| **Web console** | **Resources** → Skill; Skills in the agent form |
| **File** | `skills/<category>/<package>/SKILL.md` |
| **Crate** | `agt-skill` |

## What it does

A skill packages instructions for one kind of work, such as "how we review code" or "how to run
the release checklist", so you can attach it to any agent instead of copying it into every
prompt. When a run starts, the full text of every attached skill is added to the agent's
instructions. Codex agents receive skills too.

Skills use the `SKILL.md` format: Markdown with a YAML front matter. Packages can be scoped,
such as `@team/review`.

## How to use it

**Web console:** **Resources** → **New resource** → Skill creates a skeleton with a name and description.
Write the full instructions with the CLI.

**CLI:**

```bash
agt skill create code-review --description 'Review source code for correctness and style'
agt skill update code-review --file ./code-review/SKILL.md
agt skill read code-review
agt agent update reviewer --add-skill code-review
```

**Agent tools:**

```json
{"kind":"skill","reference":"code-review","create":true,"data":{"description":"Review source code"}}
{"kind":"skill","reference":"code-review","create":false,"data":{"text":"---\nname: code-review\n..."}}
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `--description` | One-line summary, shown in lists |
| `--file` | A `SKILL.md` file, or a directory that contains one |

## All parameters

### `agt skill create <package>`

| Option | Required | Meaning |
| --- | --- | --- |
| `--description <text>` | yes | Summary, at most 1024 characters |
| `--category <c>` | no | Category (default `default`) |

Creates a `SKILL.md` with the name, the description and a placeholder body.

### `agt skill update <package>`

| Option | Required | Meaning |
| --- | --- | --- |
| `--file <path>` | yes | A `SKILL.md` file or a directory containing it. Replaces the whole file. |
| `--category <c>` | no | Category |

### Other commands

| Command | Meaning |
| --- | --- |
| `read <ref>` | Print `SKILL.md` |
| `list`, `info`, `delete`, `category` | See [Resources](./resources.md#shared-cli-commands) |

### File: `SKILL.md`

```markdown
---
name: code-review
description: Review source code for correctness and style
license: MIT
---
# Code review

1. Read the whole change before commenting.
2. ...
```

| Front matter field | Required | Meaning |
| --- | --- | --- |
| `name` | yes | Package name; must match the directory |
| `description` | yes | Summary, at most 1024 characters |
| `license` | no | License of the instructions |
| `compatibility` | no | Compatibility notes, at most 500 characters |
| `metadata` | no | String map. For scoped packages, `agt.package` holds the full name, such as `@team/review`; its last part must equal `name`. |
| `allowed-tools` | no | Not supported yet: an agent with such a skill fails to run |

### Tool fields (`resource_write`, `kind: "skill"`)

| Field | When | Meaning |
| --- | --- | --- |
| `description` | create | Summary; creates the skeleton |
| `text` | update | The complete new `SKILL.md`, front matter included |

## Related modules

- [Agent](./agent.md): attaching skills
- [Prompt](./prompt.md): the base role description
