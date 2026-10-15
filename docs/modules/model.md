# Model

| Overview | |
| --- | --- |
| **What it is** | A model on a provider, and how agt calls it |
| **CLI** | `agt model` |
| **Agent tools** | `resource_*` with `kind: "model"` |
| **Web console** | **Resources** → Model |
| **File** | `models/<category>/<name>/model.yaml` |
| **Crates** | `agt-model`, `agt-inference` |

## What it does

A model resource gives a name to one model on a [provider](./provider.md), with the protocol and settings to use. Native [agents](./agent.md) refer to a model; changing the model resource changes every agent that uses it from its next run. You can define several model resources for the same underlying model, for example a `fast` and a `careful` variant.

agt streams responses, retries transient failures up to 10 attempts, and summarizes old context automatically when a conversation nears the context window.

## How to use it

**Web console:** **Resources** → **New resource** → Model. Choose the provider, click **Fetch model list** (获取模型列表) or type the model ID, and choose the API format.

**CLI:**

```bash
agt model create fast --provider openai --model gpt-5-mini --api-format responses --tool-call true
agt model update fast --reasoning-effort low
agt model update fast --clear-reasoning-effort
```

**Agent tools:**

```json
{"kind":"model","reference":"fast","create":true,
 "data":{"provider":"openai","model":"gpt-5-mini","params":{"api_format":"responses","tool_call":true}}}
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `--model` | The model ID at the provider |
| `--api-format` | `chat-completions` or `responses`; must match what the provider supports |
| `--tool-call true` | Required for agents that use any tool |
| `--reasoning-effort` | Thinking level for reasoning models |
| `--context-window` | Input tokens the model accepts; set it from the provider's documentation |

## All parameters

### `agt model create <name>`

| Option | Required | Default | Meaning |
| --- | --- | --- | --- |
| `--provider <ref>` | yes | | The provider |
| `--model <id>` | yes | | Model ID at the provider |
| `--api-format <f>` | no | `chat-completions` | `chat-completions` or `responses`. agt never switches automatically. |
| `--temperature <t>` | no | `0.7` | 0 to 2. Not sent when a reasoning effort is set. |
| `--max-tokens <n>` | no | not sent | Output token limit |
| `--timeout <s>` | no | `60` | Seconds to wait for the response to start, for each read, and between output chunks once the reply is streaming; a stalled reply is retried |
| `--reasoning-effort <level>` | no | not sent | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, or any value the provider accepts |
| `--tool-call <bool>` | no | | `true` if the model supports function calling |
| `--context-window <n>` | no | `272000` | Input token window, 4096 to 2000000. Used for automatic compaction; does not limit output. |
| `--category <c>` | no | `default` | Category |

### `agt model update <name>`

Accepts the same options as `create` except `--provider` and `--model`, plus:

| Option | Meaning |
| --- | --- |
| `--clear-max-tokens` | Stop sending an output limit |
| `--clear-reasoning-effort` | Stop sending a reasoning effort |

`list`, `info`, `delete` and `category` are described in [Resources](./resources.md#shared-cli-commands).

### File: `model.yaml`

```yaml
name: fast
provider: { category: default, name: openai }
model: gpt-5-mini
api_format: responses
params:
  temperature: 0.7
  max_tokens: null
  timeout: 60
  reasoning_effort: null
capabilities:
  context_window: 272000
  tool_call: true
```

### Tool fields (`resource_write`, `kind: "model"`)

On create: `provider` (required), `model` (required), and optional `params` with any of the fields below. On update, give the fields directly in `data`.

| Field | Meaning |
| --- | --- |
| `temperature` | 0 to 2 |
| `max_tokens` | Positive integer |
| `timeout` | Seconds |
| `api_format` | `chat-completions` or `responses` |
| `reasoning_effort` | As for `--reasoning-effort` |
| `tool_call` | Boolean |
| `context_window` | 4096 to 2000000 |

`clear: ["max_tokens"]` or `clear: ["reasoning_effort"]` removes those settings.

### Request behavior

| Behavior | Details |
| --- | --- |
| Streaming | Requests use streaming. Providers that ignore it still work, without live preview. |
| Retries | Connection errors, HTTP errors, invalid JSON and empty replies are retried up to 10 attempts in total, waiting from 250 ms up to 4 s. Configuration errors and explicit refusals are not retried. |
| Compaction | At 90% of the context window, older history is summarized with the same model. |
| Input | Text only; images are not supported. |

## Related modules

- [Provider](./provider.md): where requests go
- [Agent](./agent.md): agents that use the model
- [Permissions](./permissions.md): a model can also act as the approval reviewer
