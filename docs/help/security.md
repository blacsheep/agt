# Security model

agt is a **trusted local tool for one operator**. It lets agents do real work on your machine, under your control. Keeping untrusted users apart from each other is outside its design.

## The Web console

- It listens on `127.0.0.1` only, checks the `Host` and `Origin` headers, and requires the token from the link printed at startup.
- **The link is a password.** Whoever has it can manage every resource, read every conversation, and approve tool calls. Do not share it, post it, or expose the port through a proxy. To use a remote server, use an SSH tunnel.
- The console calls the `agt` CLI with argument arrays, never through a shell.

## What agents can do

An agent can only do what its tools allow:

| Ability | Comes from |
| --- | --- |
| Chat, read history in its conversations, keep notes | Always |
| Use an MCP server | Being attached to it |
| Manage every resource in the home, create and enable agents | `resource_management` |
| Delegate tracked tasks | `task_collaboration` |
| Run shell commands as your user | `local_execution` plus `execution.enabled` |
| Edit files in a project | A harness (Codex, Claude Code, DeepSeek Harness), within its sandbox |

New agents do not inherit their creator's abilities. `resource_management` is the most powerful capability: an agent with it can change any agent's configuration, including adding capabilities to other agents. Give it only to agents you would trust to administer the workspace.

On top of this, every native tool call passes through the [approval policy](../guide/permissions.md), and harness agents' own requests go to the approvals queue.

## Secrets

- Provider API keys are stored in plain text in `providers/.../provider.yaml` inside the home. agt never returns them in lists, tool results, logs or errors, but anyone who can read the home can read the keys. Restrict access to the home directory.
- In the Web console, keys go to the CLI through the environment of a single child process, not through command-line arguments.
- Run records store the shape of tool arguments and safe error codes, not argument values, tool output or model reasoning. Full tool details are kept separately and shown only to the operator and the agent that made the call.
- agt refuses symbolic links inside the home: in resource domains, categories and resource folders. A link planted there cannot make agt read or write files outside the home. Links above the home, such as `/var` on macOS, are not checked.

## Things agt does not protect against

- **MCP servers and local commands run as your user.** agt does not sandbox them. A malicious MCP server or an approved command can do anything your account can.
- **Requests already sent cannot be recalled.** Stopping a discussion stops future work, not a request already made to an external service.
- **A harness's file access** is governed by the harness's own sandbox or permission settings and the operating system, not by agt. Agents sharing a working directory share its files.
- **Prompt injection.** Content that agents read (web pages, files, messages from other agents) can try to change their behavior. Keep powerful capabilities and approvals in mind when agents process untrusted content.

## Reporting a vulnerability

Please report security issues privately through GitHub's [security advisories](https://github.com/blacsheep/agt/security/advisories/new) rather than in a public issue.
