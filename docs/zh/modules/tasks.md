# 任务

| 概览 | |
| --- | --- |
| **是什么** | Agent 之间可追踪的工作请求，带结果和汇报 |
| **Agent 工具** | `task_*`（需要 `task_collaboration`） |
| **CLI** | `agt tool call ... task_*` |
| **Web 控制台** | 会话标题栏的**任务**；**交互观察** |
| **设置** | `scheduler.parallel_work` |
| **存储** | `runtime/runtime.db` |
| **Crate** | `agt-service`、`agt-runtime` |

## 作用

任务是一个 Agent（请求者）向另一个 Agent（执行者）发出的请求，agt 会一直追踪到它结束：排队、执行、把结果送回请求者，再由请求者向原会话提交汇报。任务在重启后依然保留，可以相互等待；声明了共享对象后还可以并行执行。

对话用消息，结果重要的工作用任务。工作流程见[任务协作](../guide/tasks.md)。

## 怎么用

1. 把 `task_collaboration` 交给请求者（通常是 lead）。执行者只需已绑定。
2. 确认请求者和执行者在同一个会话中（`direct_create` 或群聊）。
3. 请求者调用 `task_request`，之后用 `task_get` 审阅结果，再调用 `task_report`。

```bash
agt tool call --principal lead task_list --args '{"queued_only":true}'
agt tool call --principal lead task_get --args '{"task_id":"<id>"}'
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `assignee`、`conversation`、`input` | 谁做什么，以及双方都在的会话 |
| `request_key` | 重复使用同一个 key 会返回同一个任务 |
| `expected_revision` | 每次修改都需要；从 `task_get` 获取 |

## 全部参数

### 请求与汇报

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `task_request` | `assignee`, `conversation`, `input`, `request_key`, `priority?`, `execution_policy?`, `source_message?` | 为任务排队。返回 `queued`，不代表完成。`input` 最多 120 KiB。`source_message` 供人从 CLI 调用时使用：授权讨论的那条消息。 |
| `task_get` | `task_id` | 状态、结果、阻塞原因、汇报和通知状态、revision |
| `task_list` | `conversation?`, `queued_only?`, `limit?`, `before?` | 你发出和收到的任务，从新到旧。把最后的 `list_cursor` 作为 `before`。 |
| `task_report` | `task_id`, `expected_revision`, `body` | 向任务的原会话发布汇报。收到结果后必须执行；普通消息不算。 |

### 管理队列

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `task_reprioritize` | `task_id`, `expected_revision`, `priority`, `reason` | `low`、`normal`、`high`；只有人可以设为 `urgent` |
| `task_defer` | `task_id`, `expected_revision`, `reason`，以及 `not_before` 或 `after_task` + `deadline` 之一 | 不早于某个时间开始，或等同一执行者的另一个任务结束后再开始 |
| `task_execution_policy` | `task_id`, `expected_revision`, `reason`, `execution_policy` | 为排队中的任务声明并行和互斥资源 |
| `task_cancel` | `task_id`, `expected_revision`, `reason` | 取消任务及它创建的子任务 |
| `task_retry` | `task_id`, `expected_revision`, `request_key`, `reason` | 基于失败、取消或中断的任务创建新任务（请求者或人） |
| `task_resolve` | `task_id`, `expected_revision`, `outcome`（`succeeded` 或 `failed`）, `evidence` | 仅限人：记录 `outcome_unknown` 任务的实际结果。需为任务所在会话中有管理权限的成员，或工作空间所有者。 |
| `task_reply` | `task_id`, `input_request_id`, `expected_revision`, `answer`, `request_key` | 答复任务通过 `task_input` 提出的问题 |
| `task_attention` | `limit?`, `before?` | 仅限工作空间负责人：需要人处理的任务 |

### 任务内等待

只在 Agent 处理任务期间可用。

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `task_await` | `deadline`（必填）、`task_id` 或 `task_ids`（1–20 个） | 保存检查点，等待这些任务结束或截止时间到达。必须是这一步中唯一的调用。 |
| `task_yield` | `not_before`, `reason` | 保存检查点，暂停到指定时间 |
| `task_input` | `question`, `deadline` | 向请求者提问，等待 `task_reply` 或截止时间 |

时间使用本地时间，例如工作空间时区（`scheduler.timezone`）下的 `2026-11-01T09:00`，也可以带偏移，例如 `+08:00`；`task_defer` 同样适用。早期版本要求的 Unix 毫秒仍然可用。等待期间 Agent 的执行槽会被释放给其他工作。截止时间到达不会取消被等待的任务，也不会编造结果。

### 定时任务

定时任务随 `task_collaboration` 一起提供。行为说明见[定时任务](../guide/timers.md)。

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `timer_create` | `text`，`after_seconds` / `at` / `cron` 三选一，`mentions?`、`name?`、`timezone?`、`discussion_seconds?`、`conversation?` | 稍后把 `text` 发到会话里，只发一次或按 cron 周期发送。Agent 只能在当前会话设置；人从 CLI 调用时必须指定会话。返回换算后的本地时间 `next_at`。 |
| `timer_list` | `conversation?`, `include_finished?` | 你所在会话中等待触发的定时任务；`include_finished` 同时列出已触发和已取消的 |
| `timer_cancel` | `timer_id`, `reason?` | Agent 只能取消自己设置的；人可以取消所在会话里的任何定时任务 |
| `goal_create` | `text`, `done_when`, `conversation?` | 对一个结果负责到底；工作空间安静时会审核未完成的目标。每个身份最多 5 个未完成目标 |
| `goal_get` / `goal_list` | `goal_id` / `include_finished?` | 负责人和目标所在会话中的人可以查看 |
| `goal_complete` | `goal_id`, `evidence` | 标记完成；不再审核 |
| `goal_wait_human` | `goal_id`, `question` | 暂停审核，直到有人在目标所在会话发言。只用于必须由人回答的问题，等 Agent 时不要用 |
| `goal_abandon` | `goal_id`, `reason` | 放弃该目标；不再审核 |

### `execution_policy`

```json
{ "parallel": true, "exclusive_resources": ["repo:frontend"] }
```

| 字段 | 默认值 | 含义 |
| --- | --- | --- |
| `parallel` | `false` | 允许该任务与同一 Agent 的其他并行任务同时运行。需要 `scheduler.parallel_work: true`。 |
| `exclusive_resources` | `[]` | 任务会修改的共享对象的键。键有重叠的任务永远不会同时运行，即使属于不同 Agent。 |

### 设置

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `scheduler.parallel_work` | `false` | 允许单个 Agent 并行处理任务。调度器重启后生效。 |

### 任务状态

| 状态 | 含义 |
| --- | --- |
| `queued` | 等待开始 |
| `running` | 正在处理 |
| `waiting_dependency` | 通过 `task_await` 等待 |
| `waiting_until` | 通过 `task_yield` 等待 |
| `waiting_input` | 等待 `task_input` 的答复 |
| `succeeded`、`failed`、`canceled` | 已结束 |
| `interrupted` | 因崩溃或重启而停止，且已确认停止。可用 `task_retry` 重试。 |
| `outcome_unknown` | 被中断；不确定工作是否已发生。永远不会自动重试。 |

`blocked_reason` 说明排队中的任务为什么没有开始，例如执行者被停用或设为 `manual`、讨论已过期、任务被延后或在等待依赖。

### 执行顺序

排队中的任务按优先级（`urgent`、`high`、`normal`、`low`）执行，同级按入队顺序。任务每等待五分钟提升一个优先级，最高到 `high`。正在运行的任务不会被抢占。

## 相关模块

- [Agent](./agent.md)：授予 `task_collaboration`
- [调度器与讨论](./scheduler.md)：任务继承的讨论与限制
- [执行记录](./runs.md)：处理任务的执行
