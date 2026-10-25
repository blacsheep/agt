# MCP

| Overview | |
| --- | --- |
| **What it is** | An MCP server that gives agents tools |
| **CLI** | `agt mcp` |
| **Agent tools** | `resource_*` with `kind: "mcp"`, `resource_test`, `mcp_probe`, `agent_mcp_list` |
| **Web console** | **Resources** → MCP; MCP in the agent form |
| **File** | `mcps/<category>/<name>/mcp.json` |
| **Crate** | `agt-mcp` |

## What it does

An MCP resource describes how to start (stdio) or reach (HTTP) one [Model Context Protocol](https://modelcontextprotocol.io) server. Attach it to native agents, and they can call its tools. Each run connects to the agent's MCP servers at the start, and every call is checked against the tool's input schema and the [approval policy](./permissions.md).

Registering a server does not give anyone access; only agents it is attached to can use it. Harness agents (Codex, Claude Code, DeepSeek Harness) do not use these resources; configure MCP in the harness instead.

## How to use it

**Web console:** **Resources** → **New resource** → MCP, then paste the configuration.

**CLI:**

```bash
agt mcp create files --file ./files.json
agt mcp probe files                       # start it and list its tools
agt agent update researcher --add-mcp files
```

**Agent tools:** register with `resource_write`, check with `mcp_probe`, attach with `resource_write(kind="agent", data={}, add={"mcps":["files"]})`. See [MCP tools](../guide/mcp.md#let-agents-build-their-own-tools).

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `command`, `args`, `cwd` | How to start a stdio server; `cwd` must be absolute |
| `url` | Address of an HTTP server |
| `bearer_token_env_var` | Environment variable holding the HTTP bearer token |

## All parameters

### Commands

| Command | Options | Meaning |
| --- | --- | --- |
| `create <name>` | `--file <path>` (required), `--category <c>` | Register a server from a JSON file |
| `update <name>` | `--file <path>` (required), `--category <c>` | Replace the configuration |
| `test <ref>` | `--category <c>` | Validate the configuration. Does not start or contact the server. |
| `probe <ref>` | `--category <c>` | Start or connect exactly as a run would, initialize, list tools and compile their input schemas, then disconnect. Calls no tool. agt asks for MCP protocol 2025-11-25 and accepts servers that answer 2024-11-05 or later. |
| `list`, `info`, `delete`, `category` | | See [Resources](./resources.md#shared-cli-commands) |

### File: `mcp.json`

The file is an `mcpServers` object with exactly one server, whose key must equal the resource name.

**stdio server**

```json
{
  "mcpServers": {
    "files": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/srv/shared"],
      "cwd": "/srv/shared",
      "env": { "LOG_LEVEL": "warn" }
    }
  }
}
```

| Field | Required | Meaning |
| --- | --- | --- |
| `command` | yes | Program name on the `PATH`, or an absolute path |
| `args` | no | Array of strings. Relative paths resolve against `cwd`; `${cwd}` and `${workspace}` are replaced with `cwd`. |
| `cwd` | yes | Absolute working directory. The server always starts here, whichever agent uses it. |
| `env` | no | Extra environment variables (string values) |
| `type` / `transport` | no | `stdio`. Inferred when absent. |

**HTTP server**

```json
{
  "mcpServers": {
    "search": {
      "url": "https://mcp.example.com/mcp",
      "bearer_token_env_var": "SEARCH_TOKEN",
      "http_headers": { "X-Team": "research" },
      "env_http_headers": { "X-Account": "SEARCH_ACCOUNT" }
    }
  }
}
```

| Field | Required | Meaning |
| --- | --- | --- |
| `url` | yes | Absolute `http` or `https` URL. Must not contain a user name or password. |
| `bearer_token_env_var` | no | Environment variable whose value is sent as `Authorization: Bearer ...` |
| `http_headers` | no | Fixed headers |
| `env_http_headers` | no | Headers whose values come from environment variables (header name → variable name) |
| `type` / `transport` | no | `http`. Inferred from `url`. |

Environment variables are read when agt connects, from the environment of the Web console or scheduler process.

### Agent tools

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `resource_write` | `kind: "mcp"`, `data: {mcpServers: {...}}` | Register or replace a server |
| `resource_test` | `kind: "mcp"`, `reference` | Validate the configuration only |
| `mcp_probe` | `reference` | Start or connect and check the tools, as `agt mcp probe` |
| `agent_mcp_list` | `agent` | List the MCP servers attached to an agent |

### How tool calls are checked

- Tool arguments are validated against the server's input schema (JSON Schema 2020-12 by default) before anything else. Invalid calls return an error to the model and never reach the server or the approval queue.
- Schemas may not load external references.
- Every call goes through [approval](./permissions.md). To let a server's own `readOnlyHint` count, add `"trust_read_only_hint": true` to its entry in `mcpServers` (stdio or HTTP). Tools that server marks read-only then run without approval; its other tools are still reviewed. Only set this for servers you trust, since the hint comes from the server itself.
- When an agent with `resource_management` changes its own MCP servers, the new tools load after its current batch of tool calls. If loading fails, it keeps the old tools and is told why.

## Related modules

- [Agent](./agent.md): attaching servers
- [Permissions](./permissions.md): approval of each call
- [Harness](./harness.md): harness agents, which use their own MCP configuration
