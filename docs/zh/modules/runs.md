# 执行记录

| 概览 | |
| --- | --- |
| **是什么** | 每次 Agent 执行的记录：模型调用、工具调用和结果 |
| **CLI** | `agt run`、`agt agent run` |
| **Web 控制台** | 执行横条、每条回复下方的记录、**调试模式** |
| **设置** | `scheduler.model_steps` |
| **存储** | `runtime/runtime.db` |
| **Crate** | `agt-runtime`、`agt-inference` |

## 作用

Agent 每次回复消息或处理任务，agt 都会记录一次**执行**（Run）：输入、状态、模型调用、工具调用和输出。通过执行记录可以看到 Agent 做了什么、为什么失败、调用了多少次。

Native 执行在模型和工具之间循环，直到模型给出最终答复。期间到达的消息会在下一个工具边界（`steering`）或当前工作结束后（`follow_up`）送达。Harness 执行是 Harness（Codex、Claude Code、DeepSeek Harness）的一轮，使用它自己的工具循环。

`completed` 表示模型正常结束，不代表目标达成；请检查工具失败和输出。在模型或工具调用中途被中断的执行永远不会被自动重做，因为那次调用可能已经产生了效果。

## 怎么用

**Web 控制台：** 每条回复显示其工具调用、耗时和失败数；点击可展开或打开完整记录。打开**调试模式**可看到执行 ID、调用次数和错误码。

**CLI：**

```bash
agt run list --limit 20
agt run info <执行>
agt run calls <执行>
agt run tool-detail <执行> --activity <活动>
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `agt run info <执行>` | 状态、输入、输出、错误、工具失败次数 |
| `agt run calls <执行>` | 每次模型和工具调用，含时间和状态 |

## 全部参数

### `agt run` 命令

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `list` | `--limit <n>`（默认 50）、`--conversation <id>` | 最近的执行；带 `--conversation` 时只列出由该会话触发的顶层执行 |
| `info <执行>` | | 一次执行 |
| `calls <执行>` | `--after <id>`（默认 0）、`--limit <n>`（默认 50） | 模型和工具调用。记录工具名、时间、状态、安全的错误码和参数结构，不含参数值。用上一页最后一条的 ID 作为 `--after` 翻页。 |
| `children <执行>` | | 由这次执行委派出的执行（`agt agent run --delegate`） |
| `tool-detail <执行>` | `--activity <id>`（必填） | 一次工具调用的完整参数和结果，每个字段最多 64 KiB。仅本机操作者可用。 |

### `agt agent run`

在会话之外运行一次 Agent。选项见 [Agent](./agent.md#agt-agent-run-名称)。

### 设置

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `scheduler.model_steps` | `null` | 单次 Native 执行的最大模型调用次数。`null` 不限。不限制 Harness Agent。对新执行生效。 |

### 执行状态

| 状态 | 含义 |
| --- | --- |
| `running` | 进行中，或在记录最终状态前被中断 |
| `suspended` | 保存了检查点，正在等待（仅任务） |
| `completed` | 模型正常结束 |
| `failed` | 因错误结束 |
| `canceled` | 被人、停用的 Agent 或结束的讨论停止 |
| `interrupted` | 因崩溃或重启而停止，且已确认停止。重试前请检查其影响。 |
| `outcome_unknown` | 在无法确认影响的步骤中被中断；重试前请先检查 |

### 调用记录状态

| 状态 | 含义 |
| --- | --- |
| `started` | 调用已开始但没有记录结果：结果未知 |
| `succeeded`、`failed`、`canceled` | 调用的结果 |

## 相关模块

- [调度器与讨论](./scheduler.md)：什么会启动执行
- [任务](./tasks.md)：处理可追踪任务的执行
- [Model](./model.md)：执行中的重试与上下文管理
