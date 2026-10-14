# Model

| 概览 | |
| --- | --- |
| **是什么** | Provider 上的一个模型，以及 agt 如何调用它 |
| **CLI** | `agt model` |
| **Agent 工具** | `kind: "model"` 的 `resource_*` |
| **Web 控制台** | **资源库** → Model |
| **文件** | `models/<category>/<name>/model.yaml` |
| **Crate** | `agt-model`、`agt-inference` |

## 作用

Model 资源为 [Provider](./provider.md) 上的一个模型命名，并指定使用的协议和设置。Native [Agent](./agent.md) 引用 Model；修改 Model 资源后，所有使用它的 Agent 从下一次执行开始生效。同一个底层模型可以定义多个 Model 资源，例如 `fast` 和 `careful` 两个版本。

agt 以流式方式接收响应，对临时故障最多重试 10 次，并在对话接近上下文窗口时自动摘要旧的上下文。

## 怎么用

**Web 控制台：** **资源库** → **新建资源** → Model。选择 Provider，点击**获取模型列表**或填写模型 ID，并选择 API 格式。

**CLI：**

```bash
agt model create fast --provider openai --model gpt-5-mini --api-format responses --tool-call true
agt model update fast --reasoning-effort low
agt model update fast --clear-reasoning-effort
```

**Agent 工具：**

```json
{"kind":"model","reference":"fast","create":true,
 "data":{"provider":"openai","model":"gpt-5-mini","params":{"api_format":"responses","tool_call":true}}}
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `--model` | Provider 上的模型 ID |
| `--api-format` | `chat-completions` 或 `responses`；必须与 Provider 支持的一致 |
| `--tool-call true` | 使用任何工具的 Agent 都需要 |
| `--reasoning-effort` | 推理模型的思考档位 |
| `--context-window` | 模型接受的输入 token 数；按 Provider 文档设置 |

## 全部参数

### `agt model create <名称>`

| 选项 | 必填 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `--provider <引用>` | 是 | | Provider |
| `--model <id>` | 是 | | Provider 上的模型 ID |
| `--api-format <格式>` | 否 | `chat-completions` | `chat-completions` 或 `responses`。agt 从不自动切换。 |
| `--temperature <t>` | 否 | `0.7` | 0 到 2。设置推理强度时不发送。 |
| `--max-tokens <n>` | 否 | 不发送 | 输出 token 上限 |
| `--timeout <秒>` | 否 | `60` | 每次请求的秒数 |
| `--reasoning-effort <档位>` | 否 | 不发送 | `none`、`minimal`、`low`、`medium`、`high`、`xhigh`、`max`，或 Provider 接受的任何值 |
| `--tool-call <bool>` | 否 | | 模型支持函数调用时为 `true` |
| `--context-window <n>` | 否 | `272000` | 输入 token 窗口，4096 到 2000000。用于自动压缩，不限制输出。 |
| `--category <c>` | 否 | `default` | 分类 |

### `agt model update <名称>`

接受 `create` 中除 `--provider` 和 `--model` 以外的所有选项，另外还有：

| 选项 | 含义 |
| --- | --- |
| `--clear-max-tokens` | 不再发送输出上限 |
| `--clear-reasoning-effort` | 不再发送推理强度 |

`list`、`info`、`delete` 和 `category` 见[资源通用操作](./resources.md#通用-cli-命令)。

### 文件：`model.yaml`

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

### 工具字段（`resource_write`，`kind: "model"`）

创建时：`provider`（必填）、`model`（必填），以及可选的 `params`，可包含下表任意字段。更新时直接在 `data` 中给出字段。

| 字段 | 含义 |
| --- | --- |
| `temperature` | 0 到 2 |
| `max_tokens` | 正整数 |
| `timeout` | 秒 |
| `api_format` | `chat-completions` 或 `responses` |
| `reasoning_effort` | 同 `--reasoning-effort` |
| `tool_call` | 布尔值 |
| `context_window` | 4096 到 2000000 |

`clear: ["max_tokens"]` 或 `clear: ["reasoning_effort"]` 会移除这些设置。

### 请求行为

| 行为 | 说明 |
| --- | --- |
| 流式输出 | 请求使用流式模式。忽略该参数的 Provider 也能用，只是没有实时预览。 |
| 重试 | 连接错误、HTTP 错误、无效 JSON 和空回复会重试，总共最多 10 次，间隔从 250 毫秒增加到 4 秒。配置错误和明确拒绝不重试。 |
| 压缩 | 达到上下文窗口的 90% 时，用同一个模型摘要较早的历史。 |
| 输入 | 仅文本，不支持图片。 |

## 相关模块

- [Provider](./provider.md)：请求发往哪里
- [Agent](./agent.md)：使用该模型的 Agent
- [权限审批](./permissions.md)：模型也可以作为审批的审核者
