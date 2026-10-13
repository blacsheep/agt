# CLI reference

Everything in agt is available through the `agt` command. Run `agt <command> --help` for the
exact options of any command.

## Global options

| Option | Meaning |
| --- | --- |
| `--home <dir>` | Use this home for this command only (default: `$AGT_HOME`, then `~/.agt`) |
| `--output human\|json\|jsonl` | Output format (default `human`) |
| `--yes` | Skip the confirmation prompt of destructive commands (required when not in a terminal) |

## Commands at a glance

| Command | Purpose |
| --- | --- |
| `init` | Initialize a new home directory |
| `provider`, `model`, `prompt`, `skill`, `mcp`, `agent` | Manage resources |
| `harness` | Manage Codex harnesses and their sessions |
| `im` | Identities, conversations and messages |
| `schedule` | Run the scheduler and control discussions |
| `tool`, `serve` | Call agt's tools directly, or expose them over MCP |
| `run` | Inspect agent runs |
| `permission` | List and decide approval requests |
| `config` | View and change workspace settings |
| `events` | Watch changes as JSON lines |

## Resources

All resource commands share the same shape: `create`, `list`, `info`, `update`, `delete` and
`category`. The aliases `add`, `ls`, `show`/`get`, `edit` and `remove` also work. A resource is
referenced as `name` (in the `default` category) or `category/name`, or with `--category`.

### provider

```bash
agt provider create <name> --base-url <url> (--api-key <key> | --api-key-stdin | --api-key-env <VAR>)
agt provider update <name> [--base-url <url>] [--api-key ... ]
agt provider test <name>          # validate the configuration
agt provider list-models <name>   # call the provider's /models endpoint
```

### model

```bash
agt model create <name> --provider <provider> --model <model-id> \
  [--api-format chat-completions|responses] [--tool-call true|false] \
  [--temperature <t>] [--reasoning-effort <level>] [--max-tokens <n>] \
  [--timeout <seconds>] [--context-window <tokens>]
agt model update <name> [same options]
```

See [Connect models](../guide/models.md#models) for defaults.

### prompt

```bash
agt prompt create <name> (--text <text> | --file <path> | --stdin)
agt prompt update <name> (--text ... | --file ... | --stdin)
agt prompt read <name>
```

### skill

```bash
agt skill create <name> --description <text>     # creates a SKILL.md skeleton
agt skill update <name> --file <SKILL.md or directory>
agt skill read <name>
```

### mcp

```bash
agt mcp create <name> --file <mcp.json>
agt mcp update <name> --file <mcp.json>
agt mcp test <name>      # validate the configuration only
agt mcp probe <name>     # start/connect, list tools, check schemas
```

The file format is in [MCP tools](../guide/mcp.md#register-a-server).

### agent

```bash
agt agent create <name> (--model <model> | --harness <harness>) [--prompt <prompt>] \
  [--skill <skill>]... [--mcp <mcp>]... [--builtin-tool <capability>]... [--description <text>]
agt agent update <name> [--model ... | --harness ...] [--prompt ...] [--description ...] \
  [--add-skill/--remove-skill <skill>] [--add-mcp/--remove-mcp <mcp>] \
  [--add-builtin-tool/--remove-builtin-tool <capability>]
agt agent run <name> (--input <text> | --stdin) [--request-key <key>] [--cwd <dir>] [--delegate <agent>]...
```

Capabilities are `resource_management`, `task_collaboration` and `local_execution`.
`agent run` executes the agent once, outside any conversation. With `--delegate`, it may hand
work to the listed agents.

### Deleting resources

```bash
agt <resource> delete <name> [--dry-run] [--recursive] [--yes]
```

A resource that others still use cannot be deleted unless you pass `--recursive`, which also
deletes the agents (and, for a provider, the models) that depend on it. `--dry-run` prints the
plan without changing anything. Deleted directories are moved to `<home>/.trash/`. The
`default/default` prompt cannot be deleted, and an agent must be unbound before it is deleted.

### Categories

```bash
agt <resource> category create <category>
agt <resource> category list
agt <resource> category delete <category>    # only empty, non-default categories
```

## harness

```bash
agt harness resource create <name> --file <harness.json>
agt harness resource update <name> --file <patch.json>
agt harness resource test <name>
agt harness models <name>          # models and reasoning levels Codex reports
agt harness skills <name>          # Codex skills in the project
agt harness mcp <name>             # Codex MCP servers
agt harness capabilities           # which engine operations agt supports

agt harness session list
agt harness session current --principal <agent> --conversation <id>
agt harness session configure <session> --model <model> [--effort <level>]
agt harness session rename <session> <title>
agt harness session compact <session>
agt harness session cancel <session>
```

The file format is in [Codex agents](../guide/codex.md#create-a-harness).

## im

```bash
agt im principal create <id> --name <name> [--kind user|agent|...]
agt im principal list
agt im owner get | ensure <user>         # the workspace owner added to agent-created groups

agt im direct create <a> <b>
agt im group create --owner <id> --title <title> --member <id>...
agt im group members add|remove|leave|list <conversation> ...
agt im group dissolve <conversation> --actor <owner>

agt im conversation list --principal <id>
agt im conversation info|hide|unhide|archive|reactivate|delete <conversation> ...

agt im message send <conversation> --sender <id> <body>
agt im message list <conversation> --viewer <id>
agt im message read <conversation> --principal <id> --through-seq <n>
```

These commands only store messages; they never run agents. Use `schedule` for that.

## schedule

```bash
agt schedule run [--watch] [--concurrency <n>]   # the scheduler; without --watch it drains ready work and exits
agt schedule start <conversation> [--message <id>]
agt schedule pause <conversation>
agt schedule resume <conversation>
agt schedule end <conversation>
agt schedule reset <conversation> --actor <you>   # reset agents' context, keep history
agt schedule delete <conversation> --actor <you>
agt schedule dissolve <conversation> --actor <owner>
agt schedule status
```

`start` authorizes a discussion: your pending messages (or just `--message`) may now be
answered. Authorizing does not call a model; the running scheduler does. The Web console runs
the scheduler for you; run only one per home.

## tool and serve

```bash
agt tool list --principal <id> [--manager]
agt tool call --principal <id> [--manager] <tool> --args '<json>'
agt serve --principal <id> [--manager]          # MCP stdio server
```

`--principal` is the identity to act as; `--manager` adds resource and binding tools. See
[Built-in tools](./builtin-tools.md) and [Use agt from MCP clients](../guide/mcp-clients.md).

## run

```bash
agt run list [--limit <n>] [--conversation <id>]
agt run info <run>
agt run calls <run> [--after <id>] [--limit <n>]   # model and tool calls, without private content
agt run children <run>                             # runs delegated by this run
agt run tool-detail <run> --activity <id>          # a tool call's arguments and result
```

## permission

```bash
agt permission list [--principal <id>]
agt permission info <id>
agt permission decide <id> --expected-revision <rev> --decision '<json value>' [--reason <text>]
```

## config

```bash
agt config describe              # every setting, its default and when it takes effect
agt config show                  # the settings in effect
agt config apply ./config.yaml   # validate and replace the whole file
```

See [Configuration](./configuration.md).

## events

```bash
agt events watch --principal <id> [--conversation <id>] [--after <cursor>]
```

Prints one JSON line per change the identity can see. Resume from the last `cursor` after a
restart.
