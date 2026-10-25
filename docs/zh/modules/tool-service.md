# 工具服务

| 概览 | |
| --- | --- |
| **是什么** | 供脚本和外部 MCP 客户端使用的 agt 工具 |
| **CLI** | `agt tool`、`agt serve`、`agt events` |
| **Crate** | `agt-service` |

## 作用

Agent 使用的工具（消息、资源、任务、记忆等）由同一个工具服务提供。在 Agent 之外也可以通过三种方式使用它：

| 命令 | 用途 |
| --- | --- |
| `agt tool call` | 在 shell 或脚本中调用一个工具 |
| `agt serve` | 通过 stdio 把工具开放给 MCP 客户端 |
| `agt events watch` | 以 JSON 行流的形式跟踪变化 |

调用者的身份由启动命令的人决定（`--principal`），永远不能通过工具参数指定。`--manager` 为可信的本地进程增加资源和绑定工具。

## 怎么用

```bash
agt tool list --principal me --manager
agt tool call --principal me --manager resource_list --args '{"kind":"agent"}'
agt events watch --principal me
```

MCP 客户端配置：

```json
{
  "mcpServers": {
    "agt": { "command": "/absolute/path/to/agt", "args": ["serve", "--principal", "me", "--manager"] }
  }
}
```

见[在 MCP 客户端中使用 agt](../guide/mcp-clients.md)。

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `--principal <id>` | 要使用的身份；必须已存在 |
| `--manager` | 增加资源和绑定工具 |
| `--args <json>` | 工具参数 |

## 全部参数

### `agt tool`

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `list` | `--principal <id>`（必填）、`--manager` | 该身份可用的工具及其 JSON schema |
| `call <工具>` | `--principal <id>`（必填）、`--manager`、`--args <json>`（默认 `{}`） | 调用一个工具。业务错误以 `{"error": {"code", "message", "field"}}` 返回。 |

### `agt serve`

| 选项 | 含义 |
| --- | --- |
| `--principal <id>` | 必填。通过该服务发出的所有调用都使用这个身份。 |
| `--manager` | 增加资源和绑定工具 |

`agt serve` 会保存和投递消息，但不运行 Agent。只有讨论已授权（Web 控制台或 `agt schedule start`）且调度器在运行时，Agent 才会回复。

### `agt events watch`

| 选项 | 含义 |
| --- | --- |
| `--principal <id>` | 必填。只输出该身份可见的变化。 |
| `--conversation <id>` | 只看一个会话 |
| `--after <游标>` | 从之前某个事件的游标之后继续 |

先输出一行 `ready`，之后每个变化（例如新消息或任务更新）输出一行 JSON。事件只是提示：详情请用普通工具读取。收到 `resync_required` 时，重新加载你的视图，再从新游标继续。事件保留 7 天。

### 可用的工具

| 工具组 | 不带 `--manager` | 带 `--manager` | 详情 |
| --- | --- | --- | --- |
| 会话与消息 | 是 | 是 | [身份与会话](./conversations.md#agent-工具) |
| 记忆 | 是 | 是 | [记忆](./memory.md#全部参数) |
| 任务（仅自己的任务）与定时任务 | 是 | 是 | [任务](./tasks.md#全部参数) |
| 资源、分类 | | 是 | [资源通用操作](./resources.md#agent-工具) |
| 身份与绑定 | | 是 | [Agent](./agent.md#身份与绑定) |
| Harness 会话 | | 是 | [Harness](./harness.md#agent-工具) |

在 Agent 执行中，工具名带有 `framework_` 前缀，例如 `framework_message_send`。Agent 通过 `resource_management` 获得资源、身份和 Harness 工具，而不是 `--manager`。

### 错误格式

```json
{"error":{"code":"invalid_arguments","message":"Reply target must exist in this conversation; omit/null for a new message.","field":"reply_to"}}
```

错误永远不会回显参数值或密钥。

## 相关模块

- [Agent](./agent.md)：Agent 执行中的同一套工具
- [调度器与讨论](./scheduler.md)：什么让 Agent 回复通过工具服务发送的消息
