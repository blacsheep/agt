# Use agt from MCP clients

`agt serve` exposes agt's tools over MCP stdio, so any MCP client (an editor, a desktop
assistant, or another agent framework) can manage resources and take part in conversations.

## Configure the client

Most clients accept an `mcpServers` entry:

```json
{
  "mcpServers": {
    "agt": {
      "command": "/absolute/path/to/agt",
      "args": ["serve", "--principal", "me", "--manager"]
    }
  }
}
```

- `--principal` sets the identity the client acts as. It must already exist
  (`agt im principal create me --name Me`). The model cannot change it through tool arguments.
- `--manager` adds the resource and binding tools. Leave it out to give the client only the
  participant tools.
- Add `--home <dir>` before `serve` to use a home other than `$AGT_HOME` or `~/.agt`.

## What the client can do

Without `--manager`, the client gets the participant tools: conversations, messages, memory and
its own tasks. With `--manager` it also gets resources, identities and bindings. The
[Tool service module](../modules/tool-service.md#which-tools-are-available) has the full list.

`agt tool list --principal me --manager` prints every tool with its parameters.

## Things to know

- **`agt serve` does not run agents.** Messages sent through it are stored and delivered, but
  agents only reply when a discussion is authorized and the scheduler is running. Start
  discussions from the Web console or with `agt schedule start`, and keep the console or
  `agt schedule run --watch` running.
- **Trust.** Whoever starts `agt serve` chooses the identity and whether it is a manager. Treat
  it like a local admin tool.
- **Same rules as agents.** Deletions return a plan first and need an exact confirmation, and
  secrets are never returned.

## Calling tools from scripts

The same tools are available without an MCP client:

```bash
agt tool list --principal me --manager
agt tool call --principal me --manager resource_list --args '{"kind":"agent"}'
agt --output json tool call --principal me message_history --args '{"conversation":"<id>"}'
```

To react to changes, `agt events watch --principal me` prints a JSON line whenever something
the identity can see changes, such as a new message or a task update.
