# Installation

## Requirements

| Requirement | Notes |
| --- | --- |
| Linux, macOS or Windows | Prebuilt binaries for x86_64 and ARM64 (Windows: x86_64) |
| A model API | OpenAI or a compatible service, Anthropic, or Google Gemini, with a model that supports tool calls; see [Connect models](./models.md) |
| A coding agent tool (optional) | Only for agents that run on one: the Codex CLI for [Codex agents](./codex.md), [Claude Code](./claude-code.md) or [DeepSeek Harness](./deepseek-harness.md) |

## Install

agt comes in two forms. Both use the same [home](#where-data-lives), so you can use either or both:

- **Command line (`agt`)**, installed with one command below. It includes the Web console (`agt web`).
- **Desktop app (AGT Studio)** for macOS, Windows and Linux: download the installer for your system from the [latest release](https://github.com/blacsheep/agt/releases/latest). See [Desktop app](./desktop.md).

The rest of this page is about the command line.

**macOS and Linux:**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.sh | sh
```

**Windows (PowerShell):**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.ps1 | iex"
```

The installer detects your platform, downloads the matching `agt` binary from [GitHub Releases](https://github.com/blacsheep/agt/releases), checks its SHA-256 checksum and puts it in `~/.local/bin` (on Windows, `%USERPROFILE%\.local\bin`). It also adds that directory to your `PATH`; open a new terminal, or run the `source` command it prints, then check:

```bash
agt --version
```

::: details Prefer to read the script first?
Download it, read it, then run it:

```bash
curl --proto '=https' --tlsv1.2 -LsSf -o agt-installer.sh https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.sh
less agt-installer.sh
sh agt-installer.sh
```

You can also download an archive for your platform from the [Releases page](https://github.com/blacsheep/agt/releases), verify it against `sha256.sum`, and put `agt` anywhere on your `PATH`.
:::

### Other ways to install

**With Cargo** (Rust 1.96.1 or newer), from the latest source:

```bash
cargo install --git https://github.com/blacsheep/agt agt-cli --locked
```

**From a clone**, for development:

```bash
git clone https://github.com/blacsheep/agt.git
cd agt
cargo build -p agt-cli        # target/debug/agt
```

## Start the Web console

```bash
agt web
```

The command prints a link such as `http://127.0.0.1:4317/#<token>`. Open the **full** link, including the token after `#`; the token is what authorizes your browser. The console also runs the scheduler, so agents can reply as soon as you configure them. Press Ctrl+C to stop it.

| Option | Default | Purpose |
| --- | --- | --- |
| `--port` | `4317` | Port to listen on (always `127.0.0.1`) |
| `--principal` | `web-operator` | Your identity in conversations |
| `--home` (before `web`) | `$AGT_HOME`, then `~/.agt` | Data directory to use |

The console is built into `agt`; it needs nothing else. In a repository clone, `cargo run -p agt-cli -- web` starts it from the source.

### Running on a remote server

The console only listens on the server's `127.0.0.1`. Use an SSH tunnel from your own computer:

```bash
ssh -N -L 4317:127.0.0.1:4317 user@server
```

Then open the full link printed on the server in your local browser. Do not expose the port to the internet: the token gives full control of the workspace.

## Where data lives

agt keeps everything in one *home* directory, `~/.agt` by default, created automatically on first use:

```text
~/.agt/
  config.yaml          # workspace settings (scheduler, permissions, memory, local commands)
  providers/ models/ prompts/ skills/ mcps/ agents/ harnesses/   # resources
  im/im.db             # conversations and messages
  runtime/runtime.db   # bindings, runs, tasks, approvals, harness sessions
```

To keep the data somewhere else, set `AGT_HOME` to an absolute path in your shell profile:

```bash
export AGT_HOME=/data/agt
```

The home is chosen in this order: `--home` > `AGT_HOME` > `~/.agt`. `--home` applies only to the command it is passed to, so use `AGT_HOME` for a lasting change. To set up a separate home for a project, run `agt init <dir>`.

## Updating

agt tells you when a new release is out, and changes nothing until you ask:

- At most once a day, a command you run in a terminal ends with a line such as `agt 0.6.2 is available (you have 0.6.1); run agt upgrade to update`. The check runs alongside the command and is cached for a day. It is skipped for JSON output, for scripts and agents whose output is not a terminal, and in CI.
- The Web console shows **New version** at the bottom of the navigation, with the command to run.

To upgrade:

```bash
agt upgrade --check   # only report whether a newer release exists
agt upgrade           # show both versions and ask before replacing agt
```

`agt upgrade` runs the new release's installer into the directory the installer used before, so it downloads and checks the binary the same way. Add `--yes` to skip the question. Restart `agt web` afterwards to use the new version. Your data in the home directory is kept.

An `agt` installed with Cargo or built from a clone has no installer receipt; `agt upgrade` leaves it alone and tells you to upgrade it the way you installed it. Running the installer again also works for any installation.

To turn off the notice and the console's button, set `AGT_NO_UPDATE_CHECK=1`.

## Uninstalling

Delete the binary (`~/.local/bin/agt`) and the `PATH` setup the installer created (`~/.config/agt-cli/`, and the line that sources it in your shell profile). Your data stays in `~/.agt` (or `$AGT_HOME`) until you delete it.

## Next steps

Continue with the [Quick start](./quick-start.md).
