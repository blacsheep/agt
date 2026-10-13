# MCP tools

[Model Context Protocol](https://modelcontextprotocol.io) servers give agents tools: file
access, web search, databases, game state, or anything you write yourself. In agt an MCP
server is a resource that you attach to native agents.

## Register a server

An MCP resource is an `mcpServers` document with exactly one server, in the same format many
MCP clients use.

**stdio server** (agt starts the process):

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

- `cwd` must be an absolute path. The server always starts there, whichever agent uses it.
- `command` is a command name on the `PATH` or an absolute path. Relative paths in `args`
  are resolved against `cwd`, and `${cwd}` is replaced with it.

**HTTP server** (agt connects to a running server):

```json
{
  "mcpServers": {
    "search": {
      "url": "https://mcp.example.com/mcp",
      "bearer_token_env_var": "SEARCH_TOKEN",
      "http_headers": { "X-Team": "research" }
    }
  }
}
```

Credentials do not belong in the URL; use `bearer_token_env_var`, `http_headers`, or
`env_http_headers` (header name to environment variable). Environment variables are read when
agt connects, so set them in the environment that starts the Web console or the scheduler.

**Web console:** **资源库** → **新建资源** → MCP, then paste the document.

**CLI:**

```bash
agt mcp create files --file ./files.json
agt mcp update files --file ./files.json
```

## Check that it works

| Command | What it does |
| --- | --- |
| `agt mcp test files` | Validates the configuration only. Does not start the server. |
| `agt mcp probe files` | Starts or connects to the server exactly as a run would, lists its tools and checks their input schemas. Does not call any tool. |

A successful probe means agents will be able to connect and see the tools. Whether the tools
behave correctly is up to the server's own tests.

## Attach it to agents

```bash
agt agent update researcher --add-mcp files
agt agent update researcher --remove-mcp files
```

Or tick it under MCP in the agent's **编辑角色** (Edit role) form. Agents connect to their MCP
servers at the start of each run, so changes apply from the next run. When an agent with
`resource_management` changes its own MCP servers, the change takes effect right after its
current batch of tool calls.

Only attached servers are available to an agent. Registering a server in the home does not
give anyone access to it.

## Let agents build their own tools

An agent with `resource_management` can do all of the above with tools: register a server
with `resource_write(kind="mcp")`, check it with `resource_test` and `mcp_probe`, and attach it
with `resource_write(kind="agent", create=false, data={}, add={mcps:[...]})`. Combined with a
[Codex agent](./codex.md) that writes the server code, this is how a lead builds tools the
team is missing. See [Build a team with a lead](./lead-team.md).

If you ask an agent to write an MCP server, ask for:

- input schemas generated from the same types the server uses to parse arguments;
- contract tests that call the server with valid input, missing fields, wrong types,
  boundary values and business errors;
- a start command that works from the server's directory.

## How tool calls are checked

Each call an agent makes is validated against the tool's input schema before anything
happens. Invalid arguments are returned to the model as an error so it can correct itself;
they never reach the server or the approval queue. Valid calls then go through the
[approval policy](./permissions.md).

## Limits

- stdio servers run on the host with the permissions of the user running agt. agt does not
  sandbox them. Attach only servers you trust.
- A request already sent to a server cannot be recalled, even if you stop the discussion.
- Codex agents do not use agt's MCP resources; configure MCP servers in Codex itself.
