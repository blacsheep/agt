# Resources

| Overview | |
| --- | --- |
| **What it is** | The shared operations for all seven resource types |
| **CLI** | `agt provider`, `model`, `prompt`, `skill`, `mcp`, `agent`, `harness resource` |
| **Agent tools** | `resource_*` (needs `resource_management`) |
| **Web console** | **Resources** (资源库), **Agent team** (Agent 团队) |
| **Crates** | `agt-core`, `agt-storage`, `agt-service` |

## What it does

Resources are the files that describe agents. There are seven types, each with its own page:

| Type | Describes | Page |
| --- | --- | --- |
| Provider | A model API endpoint and its key | [Provider](./provider.md) |
| Model | A model on a provider and how to call it | [Model](./model.md) |
| Prompt | A role description | [Prompt](./prompt.md) |
| Skill | A reusable instruction package | [Skill](./skill.md) |
| MCP | An MCP server that provides tools | [MCP](./mcp.md) |
| Harness | An external engine (Codex, Claude Code or DeepSeek Harness) and its project | [Harness](./harness.md) |
| Agent | A model or harness plus prompt, skills, MCP servers and capabilities | [Agent](./agent.md) |

All seven share the same naming, categories, deletion rules and agent tools, described here. People and agents edit resources through the same code path, with the same validation and locks.

## How to use it

```bash
agt model list                         # list
agt model info fast                    # show one
agt model category create experiments  # group resources
agt model create cheap --category experiments --provider main --model gpt-5-nano
agt model delete experiments/cheap --dry-run
```

An agent with `resource_management` does the same through tools:

```json
{"kind": "model", "reference": "experiments/cheap"}
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `name` or `category/name` | How a resource is referenced. A bare name means the `default` category. |
| `--category <c>` | Category of the resource being created or looked up |
| `--recursive` | When deleting, also delete what depends on the resource |
| `--dry-run` | When deleting, show the plan only |

## All parameters

### Shared CLI commands

Every resource type supports these subcommands (for harnesses: `agt harness resource ...`). Aliases: `add` = `create`, `ls` = `list`, `show`/`get` = `info`, `edit` = `update`, `remove` = `delete`.

| Command | Options | Meaning |
| --- | --- | --- |
| `list` | `--category <c>` | List resources, optionally in one category |
| `info <ref>` | `--category <c>` | Show one resource (secrets redacted) |
| `delete <ref>` | `--category <c>` `--recursive` `--dry-run` | Delete a resource |
| `category create <name>` | | Create a category |
| `category list` | | List categories |
| `category delete <name>` | | Delete an empty category other than `default` |

`create`, `update` and type-specific commands are on each type's page.

### Deletion rules

- Deleting a resource that others still reference is refused, with the list of dependents.
- `--recursive` deletes the dependents too: deleting a provider deletes its models and the agents using them; deleting a prompt, skill, MCP server, harness or model deletes the agents using it. Resources an agent uses are never deleted because the agent is deleted.
- An agent that is still bound must be unbound first (see [Agent](./agent.md#identities-and-bindings)).
- `default/default` (prompt) cannot be deleted.
- Before deleting, agt checks all seven types for broken configuration and dangling references, and refuses if anything is inconsistent.
- Deleted directories are moved to `<home>/.trash/`. In a terminal you are asked to confirm; elsewhere pass `--yes`.

### Agent tools

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `resource_list` | `kind` | List resources of one type (secrets redacted) |
| `resource_read` | `kind`, `reference` | Read one resource, including its `revision` |
| `resource_write` | `kind`, `reference`, `create`, `data`, `expected_revision?`, `add?`, `remove?`, `clear?` | Create (`create: true`) or update a resource |
| `resource_delete` | `kind`, `reference`, `recursive?`, `confirm?` | Delete in two steps (below) |
| `resource_category_list` | `kind` | List categories |
| `resource_category_create` | `kind`, `category` | Create a category |
| `resource_category_delete` | `kind`, `category` | Delete an empty category other than `default` |

`kind` is one of `provider`, `model`, `prompt`, `skill`, `mcp`, `agent`, `harness`.

**`resource_write` details.** The fields of `data` depend on the type and are listed on each type's page. On update:

- omitted fields stay unchanged;
- `expected_revision` (from `resource_read`) makes the update fail if someone changed the resource in the meantime;
- `add` / `remove` add or remove items of an agent's `skills`, `mcps` and `builtin_tools`;
- `clear` resets optional fields: an agent's `description`, `skills`, `mcps`, `builtin_tools`; a model's `max_tokens`, `reasoning_effort`; a harness's `model`, `reasoning_effort`.

Do not combine replacing a list, `clear`, and `add`/`remove` of the same item in one call. `add`, `remove` and `clear` are not allowed with `create: true`.

**`resource_delete` details.** Call it without `confirm` to get the deletion plan. Then call it again with `confirm` set to the exact, ordered list of items in the plan. An empty list never confirms a non-empty plan.

## Related modules

- [Workspace](./workspace.md): where resource files live
- [Agent](./agent.md): how resources come together into an agent
