# 调度器与讨论

| 概览 | |
| --- | --- |
| **是什么** | 决定哪些 Agent 回复并运行它们的后台进程 |
| **CLI** | `agt schedule` |
| **Web 控制台** | 自动启动；**发送后自动回复**、**继续回复**、**暂停回复**、**停止本群 Agent**、`/new`、`/stop` |
| **设置** | `config.yaml` 中的 `scheduler.*` |
| **存储** | `runtime/runtime.db` |
| **Crate** | `agt-runtime` |

## 作用

在人授权一轮**讨论**之前，消息只会被保存。在已授权的讨论中，调度器会：

1. 把每条消息投递给其他成员；
2. 对每个 Agent 应用其响应策略：私聊和被提及时回复；对其他消息（策略为 `all`），先让 Agent 判断是否参与；
3. 每个身份一次只回复一条，整个工作空间最多同时运行 `scheduler.concurrency` 个执行；
4. Agent 达到回复上限或讨论过期时，不再开始新工作；
5. 状态保存在磁盘上，重启后等待中的工作会继续。在模型或工具调用中途被中断的工作会被标记为中断，永远不会被自动重做。

Agent 在讨论期间联系其他会话时，那部分工作属于同一轮讨论，受同样的限制。

## 怎么用

**Web 控制台：** 控制台会运行调度器。开启**发送后自动回复**发送消息即授权一轮讨论。用**暂停回复**暂停，用**继续回复**继续或延长，用**停止本群 Agent**或 `/stop` 结束。

**CLI：**

```bash
agt schedule run --watch &                # 保持调度器运行
agt schedule start <会话>                  # 授权你待处理的消息
agt --output json schedule status         # 哪些在等待、运行或受阻
agt schedule end <会话>
```

每个 home 只运行一个调度器。如果 Web 控制台在运行，它已经运行了一个。

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `scheduler.replies_per_agent` | 每个 Agent 在一轮讨论中可开始的回复次数（默认 100） |
| `scheduler.discussion_seconds` | 一轮讨论的授权时长（默认 1800） |
| `scheduler.concurrency` | 整个工作空间同时运行的执行数（默认 3） |

## 全部参数

### `agt schedule` 命令

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `run` | `--watch`、`--concurrency <n>` | 运行调度器。带 `--watch` 时持续运行；不带时处理完当前就绪的工作后退出。`--concurrency` 覆盖 `scheduler.concurrency`。 |
| `start <会话>` | `--message <id>` | 授权一轮讨论：你待处理的消息（或仅指定的那条）现在可以被回复。也会继续未完成的讨论工作。本身不调用模型。 |
| `pause <会话>` | | 不再开始新工作；正在进行的回复会完成 |
| `resume <会话>` | | 继续已暂停的讨论，或续期已过期的讨论。不重置回复计数。 |
| `end <会话>` | | 结束讨论：取消待处理的回复和提醒。已结束的讨论不能继续。 |
| `reset <会话>` | `--actor <你>` | 停止相关工作，并重置 Agent 在这个会话中的上下文。消息保留。 |
| `reset-history <会话>` | `--actor <你>`、`--from <序号>`、`--through <序号>` | 列出两个消息序号之间的上下文重置记录 |
| `delete <会话>` | `--actor <你>` | 软删除会话并取消其讨论工作 |
| `dissolve <会话>` | `--actor <群主>` | 解散群聊，保留只读历史，取消其工作 |
| `status` | | 讨论、调度中的工作以及回复用量 |

### 设置（`config.yaml`）

| 键 | 默认值 | 含义 | 生效时机 |
| --- | --- | --- | --- |
| `scheduler.concurrency` | `3` | 整个工作空间同时执行的数量。`null` 不限。保存了检查点的等待任务不计入。 | 调度器重启 |
| `scheduler.replies_per_agent` | `100` | 每个身份在一轮讨论中可开始的回复次数。不是工具调用次数。人发送新消息会重置计数。`null` 不限。 | 调度器重启 |
| `scheduler.discussion_seconds` | `1800` | 讨论授权的有效期。到期后正在运行的工作会完成，但不再开始新工作，包括子任务和恢复。`null` 不限。 | 新的或续期的讨论 |
| `scheduler.parallel_work` | `false` | 见[任务](./tasks.md#全部参数) | 调度器重启 |
| `scheduler.model_steps` | `null` | 见[执行记录](./runs.md#全部参数) | 新的执行 |

### 状态值

`agt schedule status` 按 Agent 和会话列出调度中的工作：

| 状态 | 含义 |
| --- | --- |
| `queued` | 等待被领取 |
| `evaluating` | 正在判断是否参与 |
| `ready_to_reply` | 已决定回复，等待轮到自己 |
| `responding` | 正在回复 |
| `waiting` | 等待正在做事的成员完成当前一步，或等待新消息 |
| `input` | 等待问题的答复（任务） |
| `suspended` | 保存了检查点，正在等待（任务） |
| `limited` | 达到回复上限；人发送新消息会重置 |
| `blocked` | 无法继续；查看原因 |
| `completed` | 已回复 |
| `silent` | 决定保持沉默 |
| `skipped` | 未处理，例如因为响应策略 |
| `failed` | 失败；请检查执行记录 |
| `canceled` | 因暂停、结束、停用或重置而取消 |
| `interrupted` | 因崩溃或重启而停止；重试前请检查其影响 |
| `outcome_unknown` | 在无法确认影响的步骤中被中断 |

## 相关模块

- [身份与会话](./conversations.md)：响应策略与消息
- [执行记录](./runs.md)：每次回复做了什么
- [任务](./tasks.md)：讨论中的可追踪工作
