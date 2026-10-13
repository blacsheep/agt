# Installation

## Requirements

| Requirement | Notes |
| --- | --- |
| Rust 1.96.1 or newer | Builds the `agt` CLI, which contains the whole framework |
| Node.js 22 or newer | Runs the Web console; it has no npm dependencies |
| A model endpoint | Any OpenAI-compatible API whose model supports tool calls |
| Codex CLI (optional) | Only for [Codex agents](./codex.md); agt starts it as `codex app-server` |

agt runs on Linux and macOS. On Windows, pass the binary path to the Web console
explicitly (see below).

## Build from source

```bash
git clone https://github.com/blacsheep/agt.git
cd agt
cargo build -p agt-cli
```

The binary is `target/debug/agt`. To put `agt` on your `PATH` instead:

```bash
cargo install --path crates/agt-cli
agt --version
```

## Start the Web console

From the repository root:

```bash
node web/server.mjs
```

If you installed `agt` with `cargo install`, point the console at it:

```bash
node web/server.mjs --binary "$(command -v agt)"
```

On Windows PowerShell:

```powershell
node web/server.mjs --binary .\target\debug\agt.exe
```

The server prints a link such as `http://127.0.0.1:4317/#<token>`. Open the **full** link,
including the token after `#`; the token is what authorizes your browser. The server also
starts the scheduler in the background, so agents can reply as soon as you configure them.

| Option | Default | Purpose |
| --- | --- | --- |
| `--port` | `4317` | Port to listen on (always `127.0.0.1`) |
| `--home` | `$AGT_HOME`, then `~/.agt` | Data directory to use |
| `--binary` | `target/debug/agt` | The `agt` CLI to call |
| `--principal` | `web-operator` | Your identity in conversations |

### Running on a remote server

The console only listens on the server's `127.0.0.1`. Use an SSH tunnel from your own
computer:

```bash
ssh -N -L 4317:127.0.0.1:4317 user@server
```

Then open the full link printed on the server in your local browser. Do not expose the port
to the internet: the token gives full control of the workspace.

## Where data lives

agt keeps everything in one *home* directory, `~/.agt` by default, created automatically on
first use:

```text
~/.agt/
  config.yaml          # workspace settings (scheduler, permissions, memory, local commands)
  providers/ models/ prompts/ skills/ mcps/ agents/ harnesses/   # resources
  im/im.db             # conversations and messages
  runtime/runtime.db   # bindings, runs, tasks, approvals, Codex sessions
```

To keep the data somewhere else, set `AGT_HOME` to an absolute path in your shell profile:

```bash
export AGT_HOME=/data/agt
```

The home is chosen in this order: `--home` > `AGT_HOME` > `~/.agt`. `--home` applies only to
the command it is passed to, so use `AGT_HOME` for a lasting change. To set up a separate
home for a project, run `agt init <dir>`.

## Updating

```bash
git pull
cargo build -p agt-cli
```

Then restart `node web/server.mjs` and reload the page with the new link. The Web console,
the CLI and the scheduler must all use the same build.

## Next steps

Continue with the [Quick start](./quick-start.md).
