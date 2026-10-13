# agt: a local framework for multi-agent teams

English | [简体中文](README.zh-CN.md)

agt runs a team of AI agents on your own machine. It manages the resources agents
need (providers, models, prompts, skills, MCP servers, harnesses and agent
definitions), gives them a shared IM with direct and group chats, and drives them
with a persistent scheduler so they can collaborate, delegate tasks to each other
and report back.

- **Two execution engines.** Agents run on the built-in native model/tool loop, or
  on an external [Codex](https://github.com/openai/codex) session through a harness.
- **A lead that builds its own team.** The built-in `lead` prompt knows the whole
  framework: it can write prompts, create and enable agents, ask Codex to build MCP
  tools, attach them to agents, and coordinate the work in a group chat.
- **Real isolation between agents.** What each agent can see and do is defined by
  the tools and MCP servers attached to it, not by prompt instructions.
- **Durable by design.** Runs, tasks, approvals and conversations live in SQLite, so
  pending work resumes after a restart.
- **Rust core, thin Web UI.** All resources, identities, permissions and execution
  live in Rust. The Web console is a local UI over the `agt` CLI, and anything you
  can do there you can also do from the terminal.

agt is a trusted local tool. It is not a public or multi-tenant service.

## Requirements

- Rust 1.96.1 or newer
- Node.js 22 or newer for the Web console (no npm packages to install)
- An OpenAI-compatible model endpoint
- Optional: the [Codex CLI](https://github.com/openai/codex) on your `PATH`, if you
  want agents backed by Codex (agt starts it as `codex app-server`)

## Quick start

```bash
git clone https://github.com/blacsheep/agt.git
cd agt
cargo build -p agt-cli
node web/server.mjs
```

Open the full link printed in the terminal, including the token after `#`. The
Web server listens on `127.0.0.1:4317` and starts the scheduler in the background.
After pulling new code, rebuild the CLI, restart the server and reload the page.

On Windows PowerShell, point the server at the built binary:

```powershell
cargo build -p agt-cli
node web/server.mjs --binary .\target\debug\agt.exe
```

To put `agt` on your `PATH` instead:

```bash
cargo install --path crates/agt-cli
node web/server.mjs --binary "$(command -v agt)"
```

See [Installation](docs/guide/installation.md#running-on-a-remote-server) for remote access over an SSH
tunnel, and [Security model](docs/help/security.md).

## Where data lives

agt keeps everything in a single home directory, `~/.agt` by default, which is set up
automatically the first time you run any command. To use another directory, set
`AGT_HOME` to an absolute path, for example in `~/.zshrc` or `~/.bashrc`:

```bash
export AGT_HOME=/data/agt
```

The home is chosen in this order: `--home` > `AGT_HOME` > `~/.agt`. `--home` only
applies to the command it is passed to and is not remembered, so use `AGT_HOME` for a
lasting change. An empty `AGT_HOME` counts as unset.

To create a separate home for a project, initialize it explicitly:

```bash
agt init ~/my_agent_project
```

```text
<home>/
  config.yaml
  providers/  models/  prompts/  skills/  mcps/  agents/  harnesses/
  im/im.db                  # conversations and messages
  runtime/runtime.db        # bindings, tasks, runs, approvals, harness sessions
```

## Documentation

**Docs site: <https://blacsheep.github.io/agt/>**, in English and Chinese. The site's source is
in [`docs/`](docs/README.md).

| Topic | Link |
| --- | --- |
| The idea and the feature list | [What is agt?](docs/guide/what-is-agt.md) |
| From install to a lead that builds its own team | [Quick start](docs/guide/quick-start.md) |
| Resources, identities, discussions and tasks | [Core concepts](docs/guide/concepts.md) |
| Models, agents, MCP tools, Codex, tasks, memory, approvals | [Guides](docs/guide/models.md) |
| Every module: what it does, how to use it, all parameters | [Modules](docs/modules/workspace.md) |
| When an agent does not reply | [Troubleshooting](docs/help/troubleshooting.md) |
| How it is built (Chinese) | [Architecture](docs/zh/architecture/index.md) |

## Crates

```text
agt-core       shared IDs, categories and typed references
agt-storage    safe filesystem access, shared SQLite connections, local wakeups
agt-config     home directory layout and initialization
agt-provider   OpenAI-compatible providers
agt-model      model aliases that reference providers
agt-prompt     PROMPT.md resources, including the built-in default and lead prompts
agt-skill      SKILL.md resources (scoped packages supported)
agt-mcp        mcp.json resources (stdio and HTTP servers)
agt-agent      agent.yaml: model or harness, prompt, skills, MCP servers
agt-harness    harness resources, the Codex protocol and execution control
agt-im         participants, direct and group chats, messages, read state (SQLite)
agt-service    tool service, identities and permissions, agent bindings, stdio MCP server
agt-inference  agent resolution, model protocols, streaming, context budget and compaction
agt-runtime    model/tool loop, persistent runs, delivery scheduling, delegation
agt-cli        the agt command-line interface
```

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p agt-cli && node --test web/test/*.test.mjs
```

None of the tests need a real API key. Browser tests (`web/test/*.browser.mjs`) need
a local Chrome and are run separately; see [Web console architecture](docs/zh/architecture/web-console.md) (Chinese).

## Citation

If you use agt in research, please cite it using [CITATION.cff](CITATION.cff)
(GitHub shows a "Cite this repository" button for it).

## License

[MIT](LICENSE)
