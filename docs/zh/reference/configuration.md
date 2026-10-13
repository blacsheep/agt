# 运行配置

工作空间设置保存在 home 中的 `config.yaml`。它由操作者设置；Agent 即使拥有 `resource_management` 也不能修改。

## 修改方式

- **Web 控制台：** **运行设置**。每项设置都显示说明、默认值和修改的生效时机。
- **CLI：**

```bash
agt config describe              # 所有设置及默认值
agt config show                  # 当前生效的设置
agt config apply ./config.yaml   # 校验并替换整个文件
```

`apply` 会替换整个文件。请从 `agt config show`（或当前文件）开始修改，不要只写想改的字段；省略的字段会回到默认值。
未知字段和非法值会被拒绝。允许不限的字段用 `null` 表示不限；`0` 不表示不限。

## 完整示例

```yaml
scheduler:
  concurrency: 3
  parallel_work: false
  replies_per_agent: 100
  discussion_seconds: 1800
  model_steps: null
permissions:
  mode: model_review
  reviewer_model: null
  review_timeout_seconds: 30
  request_timeout_seconds: 600
  harness_mode: manual
memory:
  enabled: true
execution:
  enabled: false
  timeout_seconds: 300
  output_bytes: 65536
```

以上即默认值。

## scheduler

| 键 | 默认值 | 含义 | 生效时机 |
| --- | --- | --- | --- |
| `concurrency` | `3` | 整个工作空间同时执行的数量。`null` 不限。 | 调度器重启（`--concurrency` 可覆盖） |
| `parallel_work` | `false` | 任务标记为可并行时，允许同一 Agent 同时运行多个任务。普通聊天仍逐个处理。 | 调度器重启 |
| `replies_per_agent` | `100` | 每个 Agent 在一轮讨论中可以开始的回复次数。不是工具调用次数。`null` 不限。 | 调度器重启 |
| `discussion_seconds` | `1800` | 一轮讨论的授权时长。到期后正在运行的工作会完成，但不再开始新工作。`null` 不限。 | 新的或续期的讨论 |
| `model_steps` | `null` | 单次 Native 执行的最大模型调用次数。`null` 不限。不限制 Codex。 | 新的执行 |

对于长时间无人值守的工作，可以调高 `replies_per_agent` 并设置 `discussion_seconds: null`。其他保护仍然有效：
你可以随时停止讨论，审批也照常进行。

## permissions

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `mode` | `model_review` | Native 工具调用：`model_review`、`manual` 或 `auto_allow` |
| `reviewer_model` | `null` | `model_review` 使用的 Model 资源。未设置时调用转为人工审批。 |
| `review_timeout_seconds` | `30` | 审核模型的时限（1–120）；超时则转为人工审批 |
| `request_timeout_seconds` | `600` | 调用等待决定的时限，超时取消执行 |
| `harness_mode` | `manual` | Codex 审批请求：`manual` 或 `auto_allow` |

修改对正在等待和之后的请求生效。见[权限审批](../guide/permissions.md)。

## memory

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `enabled` | `true` | 为 Agent 提供历史搜索和笔记。关闭后，之后的调用会立即被拒绝。 |

见[记忆](../guide/memory.md)。

## execution

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `enabled` | `false` | 允许拥有 `local_execution` 的 Agent 执行宿主机命令 |
| `timeout_seconds` | `300` | 每条命令的时限。`null` 不限（命令仍可被取消）。 |
| `output_bytes` | `65536` | 每条命令返回的 stdout 和 stderr 总字节数，超出部分截断 |

见[本机命令](../guide/local-execution.md)。

## 环境变量

| 变量 | 含义 |
| --- | --- |
| `AGT_HOME` | 未指定 `--home` 时使用的 home 目录（绝对路径）。默认 `~/.agt`。 |

资源引用的环境变量（例如创建 Provider 时的 `--api-key-env`，或 MCP 服务的 `bearer_token_env_var`）从使用它们的进程环境中读取。
