<p align="center">
  <img src="docs/public/favicon.svg" width="72" height="72" alt="agt">
</p>

<h1 align="center">agt</h1>

<p align="center">
  <b>Agents that build their own team.</b><br>
  English · <a href="README.zh-CN.md">简体中文</a> · <a href="https://blacsheep.github.io/agt/">Documentation</a>
</p>

<p align="center">
  <a href="https://github.com/blacsheep/agt/actions/workflows/ci.yml"><img src="https://github.com/blacsheep/agt/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/blacsheep/agt/releases/latest"><img src="https://img.shields.io/github/v/release/blacsheep/agt" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/blacsheep/agt" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey" alt="Linux | macOS | Windows">
  <a href="https://blacsheep.github.io/agt/"><img src="https://img.shields.io/badge/docs-online-blue" alt="Documentation"></a>
</p>

agt is a multi-agent orchestration framework that runs on your own machine. You talk to one agent, the lead.

Tell it what you want done, and it writes the role prompts, creates the agents, has a coding agent build any tool that is missing, puts everyone in a group chat and runs the work. You watch, step in when you want to, and approve what needs approving.

![Sped-up recording: one message to the lead, a three-person research team created, two rounds of debate and a conclusion, with no one stepping in](docs/public/showcase/research-run.webp)

## What is different about it

In most multi-agent frameworks you define the agents, their tools and how they connect before anything runs. In agt, setting up the team is work the agents do themselves.

**Agents manage resources with tools.** Prompts, models, MCP servers and agent definitions are files that agt manages, and an agent given the `resource_management` capability can create and change them through tool calls. That is how the lead gets from one sentence to a working team, and everything it creates is a file you can read and edit.

**Teams work in chats.** Agents have direct and group chats and @mention each other, the same way people do. A scheduler decides who replies, who waits and who stays quiet. There is no workflow graph to design: orchestration happens in the conversation, which takes whatever shape the work needs.

**Isolation is enforced by tools.** An agent can only do what its attached tools allow, and a new agent does not inherit the tools of the agent that created it. In the murder mystery below, each player's tool can only ever return that player's own clue. No prompt has to ask anyone not to look.

**Existing coding agents join the team.** An agent can run on [Codex](https://github.com/openai/codex), [Claude Code](https://claude.com/claude-code), [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (`dsh`) or another ACP agent instead of the built-in model loop. It keeps its own tools and sandbox, and works with the rest of the team through agt's messaging, tasks and memory. The lead can hand one a spec, get back a tested MCP server, and attach that server to the agents that need it.

**Work survives restarts.** Conversations, runs, tasks and approvals are stored in SQLite. Pending work picks up where it stopped, and work whose outcome is unclear is flagged for you instead of being run twice.

**You keep control.** Every tool call can wait for you, go to a reviewer model first, or run freely. The Web console shows who is working on what, including chats between agents that you are not part of.

## See it work

A complete project and two real runs, each started by a single message to the lead. The [examples page](docs/guide/examples.md) walks through all three.

**[AI Werewolf](https://github.com/blacsheep/werewolf-game), a full project on agt.** Twelve AI players and an AI God play a 12-seat Werewolf game. Each player has an engine MCP server of its own, which keeps the roles apart. One message to the lead runs a whole game; voice and replay agents then turn it into audio, a web replay and a video.

![God's-view replay of an AI Werewolf game](docs/public/showcase/werewolf-replay.webp)

**A murder mystery with a tool built by Codex.** The lead had a Codex agent write a clue server, then created a game master and three players, gave each player a copy of the server locked to that player, and ran the game.

![Sped-up recording: Codex writes the clue server, each role gets its own locked instance, and the game plays through to the reveal](docs/public/showcase/mystery-run.webp)

**A research team that argues.** Asked whether to migrate to Postgres 17, the lead created a performance researcher, an operations researcher and a reviewer, and opened a group chat. The reviewer challenged both researchers, and after two rounds the group reached a verdict. The recording at the top of this page is this run.

## Install

agt comes as a command-line tool and as a desktop app. Both use the same data, so you can switch between them.

### Command line

macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/blacsheep/agt/releases/latest/download/agt-cli-installer.ps1 | iex"
```

The installer puts a prebuilt `agt` in `~/.local/bin`; the Web console is built into it. Other ways to install, updating and uninstalling are in [Installation](docs/guide/installation.md).

### Desktop app

AGT Studio is the console as an app for macOS, Windows and Linux, with a tray icon and notifications. Download the installer for your system (`.dmg`, `-setup.exe`, `.AppImage` or `.deb`) from the [latest release](https://github.com/blacsheep/agt/releases/latest). See [Desktop app](docs/guide/desktop.md).

Either way, you need a model API that supports tool calls: OpenAI or a compatible service, Anthropic, or Google Gemini. Coding agents also need the tool they run on, such as the [Codex CLI](https://github.com/openai/codex), [Claude Code](https://claude.com/claude-code) or [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness).

## Quick start

```bash
agt web
```

Open the link it prints, including the token after `#`, or open AGT Studio. Connect a model, create the lead, and send it a goal; the [Quick start](docs/guide/quick-start.md) takes you through each step. The console only listens on `127.0.0.1`. To reach it on a server, use an [SSH tunnel](docs/guide/installation.md#running-on-a-remote-server).

## Documentation

The [documentation site](https://blacsheep.github.io/agt/) is in English and Chinese.

| If you want to | Read |
| --- | --- |
| Understand the idea | [What is agt?](docs/guide/what-is-agt.md) |
| Get a lead building a team | [Quick start](docs/guide/quick-start.md) |
| See two full runs | [Examples](docs/guide/examples.md) |
| Learn the moving parts | [Core concepts](docs/guide/concepts.md) |
| Set up models, agents, MCP, coding agents, tasks, memory or approvals | [Guides](docs/guide/models.md) |
| Look up a command or setting | [Modules](docs/modules/workspace.md) |
| Fix an agent that does not reply | [Troubleshooting](docs/help/troubleshooting.md) |
| Know what is safe to expose | [Security model](docs/help/security.md) |

## Where your data lives

Everything is kept in one directory, `~/.agt`, created the first time you run `agt`. Set `AGT_HOME` to keep it somewhere else, or run `agt init <dir>` to start a separate home for a project:

```text
~/.agt/
  config.yaml                       workspace settings
  providers/ models/ prompts/ skills/ mcps/ agents/ harnesses/
  im/im.db                          conversations and messages
  runtime/runtime.db                runs, tasks, approvals, harness sessions
```

agt is meant for one person on a trusted machine. It is not a hosted or multi-user service.

## Status and where it could go

agt is a first implementation of an idea: agents that build their own team and work together in chats. It is early, and it likely has bugs as well as design decisions that did not get enough thought.

If the ideas are useful to you, fork it and adapt it to your own needs. Some directions we have thought of:

- **One lead for many projects.** Each harness has its own working directory, so Codex, Claude Code and DeepSeek Harness sessions in different project directories can all join, each looking after its own project while you talk to a single lead. Some existing projects already manage sessions better; you are welcome to start from an idea like agt's and build something better.
- **A base for multi-agent setups that need isolation.** Each agent sees only the conversations it is in and the tools attached to it, which suits games where players hold different information, such as Werewolf or a murder mystery. [AI Werewolf](https://github.com/blacsheep/werewolf-game) is built this way.
- **Simulations in the style of an AI town.** Keep each character apart with agt, and put an interaction layer between them that decides who meets whom, when, and what they hear, then sends the messages to the right agents. Each character keeps its own memory and acts on its own, and the interaction layer drives how the world unfolds.

Issues and pull requests are welcome too.

## Contributing

Build and test instructions, the crate layout and the release process are in [CONTRIBUTING.md](CONTRIBUTING.md). The internal design is documented in [Architecture](docs/zh/architecture/index.md) (Chinese).

## Acknowledgements

Thanks to the [LINUX DO](https://linux.do) community for its support and discussions.

## Citation and license

If you use agt in research, cite it with [CITATION.cff](CITATION.cff). agt is released under the [MIT License](LICENSE).
