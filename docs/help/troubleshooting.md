# Troubleshooting

Start with **Debug** (调试模式) in the top-right corner of the console: it shows run IDs, call counts and error codes on every reply and status row. From a terminal, `agt --output json schedule status` shows the scheduler's view, and `agt run info <run id>` shows one run in detail.

## An agent does not reply

Check these in order:

1. **Is the agent enabled?** On its card under **Agent team**, or `binding_list`. Creating a definition is not enough; it needs an identity and an enabled binding.
2. **Does its response policy match?** With `mentioned` (the default) it replies in direct chats and when mentioned in a group. A typed `@name` that was not selected from the list is not a mention. `manual` never replies automatically.
3. **Was the discussion authorized?** Messages sent with **Auto-reply after sending** off are only stored. Click **Continue** (继续回复).
4. **Is the discussion still open?** It may be paused, ended, past its 30-minute limit, or the agent may have used its 100 replies. The status bar above the messages says so. Send a new message, or click **Continue** to extend.
5. **Is an approval waiting?** Look at the execution bar above the message box, or **Interactions** → **Approvals**.
6. **Did it decide to stay silent?** In a group, the member status panel shows the agent's decision and reason. `[[silent]]` means it had nothing to add.
7. **Is the scheduler running?** If the console shows **回复服务未连接** (Reply service not connected), restart `agt web`. If you run it from a clone, rebuild first so the console and the scheduler come from the same `agt`.

## The model call fails

- **Check the protocol.** The model's API format (Chat Completions or Responses) must be one the provider supports for that model. agt does not switch automatically.
- **Check tool calling.** Agents with any tools need a model with tool calling enabled.
- **Read the error.** In debug mode, the run shows the HTTP status and the number of attempts. agt retries connection errors and server errors up to 10 times; configuration errors and explicit refusals fail immediately.
- **List the models.** `agt provider list-models <provider>` confirms the base URL and key.
- **Reasoning effort.** When a reasoning effort is set, temperature is omitted. Some providers reject reasoning parameters for models that do not support them.

## An MCP server does not work

- `agt mcp probe <name>` starts the server exactly as a run would and reports the error.
- stdio servers need an absolute `cwd`, and their command must be on the `PATH` of the process running agt (the Web console's environment, not your interactive shell's).
- Environment variables for tokens must be set where the console or scheduler is started.
- An agent only sees servers attached to it. Check `agent_mcp_list` or the agent's card.

## A Codex agent does not work

- Make sure `codex login` was done under the same user account that runs agt.
- `agt harness resource test <name>` and `agt harness models <name>` check the setup without running a model.
- If a turn's status is **Needs attention** (需要处理), agt could not confirm how the turn ended. Check the project files, then use `/new`.

## A run or task shows an unknown outcome

`interrupted` runs and `outcome_unknown` tasks were cut off while a tool or model call was in progress, for example by a restart. agt never repeats them automatically, because the tool may already have had an effect. Check what actually happened, then send a new message, or resolve and retry the task (see [Task collaboration](../guide/tasks.md#when-something-goes-wrong)).

## The console says "此工作空间的负责人是 …" (The workspace owner is …)

The first person to start the console on a home becomes its owner. Start the console with the same identity: `agt web --principal <owner>`. `agt im owner get` shows the owner.

## After updating agt

Stop `agt web`, run the installer again (or rebuild), start `agt web`, and open the new link. An old console or scheduler left running against a newer `agt` causes errors such as missing commands.

## Getting help

Open an issue at <https://github.com/blacsheep/agt/issues> with the agt version (`agt --version`), what you did, and the error code shown in debug mode. Do not paste API keys or the console link.
