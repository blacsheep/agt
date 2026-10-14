# Provider

| Overview | |
| --- | --- |
| **What it is** | An OpenAI-compatible endpoint and the API key to reach it |
| **CLI** | `agt provider` |
| **Agent tools** | `resource_*` with `kind: "provider"`, `provider_models`, `resource_test` |
| **Web console** | **Resources** → Provider |
| **File** | `providers/<category>/<name>/provider.yaml` |
| **Crate** | `agt-provider` |

## What it does

A provider tells agt where to send model requests. It works with OpenAI, API gateways, and local servers such as Ollama or vLLM that implement the OpenAI API. [Models](./model.md) refer to a provider; one provider can serve many models.

## How to use it

**Web console:** **Resources** → **New resource** → Provider. Enter a name, the base URL and the API key.

**CLI:**

```bash
agt provider create openai --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY
agt provider list-models openai
agt provider update openai --base-url https://gateway.example.com/v1
```

**Agent tools:**

```json
{"kind":"provider","reference":"main","create":true,
 "data":{"base_url":"https://api.openai.com/v1","api_key":"sk-..."}}
```

## Common parameters

| Parameter | Meaning |
| --- | --- |
| `--base-url` | The API root, usually ending in `/v1` |
| `--api-key-env <VAR>` | Read the key from an environment variable when the command runs |
| `--api-key-stdin` | Read the key from standard input, keeping it out of shell history |

## All parameters

### `agt provider create <name>`

| Option | Required | Default | Meaning |
| --- | --- | --- | --- |
| `--base-url <url>` | yes | | API root URL |
| `--api-key <key>` | one of three | | The key itself (visible in shell history and process lists) |
| `--api-key-stdin` | one of three | | Read the key from standard input |
| `--api-key-env <VAR>` | one of three | | Read the key from this environment variable |
| `--type <type>` | no | `openai-compatible` | Provider type; currently only `openai-compatible` |
| `--category <c>` | no | `default` | Category |

The key is read once, when the command runs, and stored in `provider.yaml`. Changing the environment variable later does not change the stored key.

### `agt provider update <name>`

| Option | Meaning |
| --- | --- |
| `--base-url <url>` | New base URL |
| `--api-key <key>`, `--api-key-stdin`, `--api-key-env <VAR>` | New key (at most one) |
| `--category <c>` | Category of the provider to update |

### Other commands

| Command | Meaning |
| --- | --- |
| `test <ref>` | Validate the stored configuration. Does not contact the server. |
| `list-models <ref>` | Call the provider's `/models` endpoint with the stored key. Does not run inference. |
| `list`, `info`, `delete`, `category` | See [Resources](./resources.md#shared-cli-commands) |

### File: `provider.yaml`

```yaml
name: openai
type: openai-compatible
base_url: https://api.openai.com/v1
api_key: sk-...
```

| Field | Meaning |
| --- | --- |
| `name` | Same as the directory name |
| `type` | `openai-compatible` |
| `base_url` | API root URL |
| `api_key` | The stored key, in plain text |

### Tool fields (`resource_write`, `kind: "provider"`)

| Field | On create | Meaning |
| --- | --- | --- |
| `base_url` | required | API root URL |
| `api_key` | one of two | The key |
| `api_key_env` | one of two | Name of an environment variable of the agt process; its current value is stored |

On update, any field may be given. Keys are never returned by any tool, list or error.

### Related tools

| Tool | Parameters | Meaning |
| --- | --- | --- |
| `provider_models` | `reference` | List the provider's models with the stored key |
| `resource_test` | `kind: "provider"`, `reference` | Validate the configuration only |

## Related modules

- [Model](./model.md): the models that use this provider
- [Resources](./resources.md): categories, deletion, shared tools
