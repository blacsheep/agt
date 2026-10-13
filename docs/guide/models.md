# Connect models

Native agents talk to models through two resources: a **provider** (where to send requests)
and a **model** (which model to use there, and how).

## Providers

A provider is an OpenAI-compatible endpoint. It works with OpenAI, API gateways, and local
servers such as Ollama or vLLM, as long as they implement the OpenAI API.

**Web console:** **资源库** (Resources) → **新建资源** (New resource) → Provider. Enter a
name, the base URL and the API key.

**CLI:**

```bash
# Read the key from an environment variable (the value is stored in the provider file)
agt provider create openai --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY

# Or pipe it in, which keeps it out of your shell history
echo "$KEY" | agt provider create local --base-url http://localhost:11434/v1 --api-key-stdin

agt provider list-models openai     # list the models the endpoint offers
agt provider update openai --base-url https://gateway.example.com/v1
```

The key is saved in the provider's `provider.yaml` inside your home. agt never returns it in
lists, tool results or errors, but it is not an encrypted vault: protect the home directory
like any file containing credentials.

## Models

A model resource names a model on a provider and sets how agt calls it.

**Web console:** **资源库** → **新建资源** → Model. Choose the provider, click
**获取模型列表** (Fetch model list) or type the model name, and choose the API format.

**CLI:**

```bash
agt model create fast --provider openai --model gpt-5-mini \
  --api-format responses --tool-call true
agt model update fast --temperature 0.2 --timeout 120
agt model info fast
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--api-format` | `chat-completions` | `chat-completions` or `responses`. Must match what the provider supports for this model; agt does not switch automatically. |
| `--tool-call` | | Whether the model supports function calling. Agents need it to use any tool. |
| `--temperature` | `0.7` | Sampling temperature. Omitted when a reasoning effort is set. |
| `--reasoning-effort` | not sent | For reasoning models, for example `low`, `medium`, `high`. Any value your provider accepts. |
| `--max-tokens` | not sent | Output token limit. |
| `--timeout` | `60` | Seconds per request. |
| `--context-window` | `272000` | Input tokens the model accepts. Used to decide when to compact the conversation; it does not limit output. Set it from your provider's documentation. |

Several model resources can point to the same provider model with different settings, for
example a `fast` and a `careful` variant.

## How agt calls the model

- **Streaming.** Requests use streaming, so the console can show progress and a preview of the
  reply while the model is generating. Providers that ignore streaming still work, without
  the preview.
- **Retries.** Connection errors, server errors, invalid responses and empty replies are
  retried, up to 10 attempts with backoff. Errors such as invalid configuration or an explicit
  refusal are not retried.
- **Context management.** When a conversation approaches 90% of the context window, agt
  summarizes older history automatically (this calls the model). You can reset an agent's
  context in a conversation with `/new`; chat history is kept.
- **Text only.** Image input is not supported yet.

## Checking a connection

```bash
agt provider test openai          # validates the configuration (no network call)
agt provider list-models openai   # calls the provider's /models endpoint with the stored key
```

The surest test is a short direct chat with an agent that uses the model. If it fails, turn
on debug mode in the console, or run `agt run info <run id>` to see the HTTP status and the
number of attempts. See [Troubleshooting](../help/troubleshooting.md).
