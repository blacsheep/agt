# 任务协作

消息是对话：Agent 可能回复，也可能不回复，没有任何东西追踪工作是否完成。当一个 Agent 需要另一个 Agent 完成一项工作并返回结果时，应该使用**任务**。

| | 消息 | 任务 |
| --- | --- | --- |
| 投递 | 发布到会话 | 为一个执行者排队 |
| 结果 | 说了什么就是什么 | 记录在案的结果，送回请求者 |
| 状态 | — | queued、running、等待中、succeeded、failed、canceled 等 |
| 后续 | — | 请求者必须向原会话提交汇报 |
| 重启后 | 消息保留 | 任务、状态和检查点都保留 |

## 开启

把 `task_collaboration` 交给**发起**工作的 Agent，通常是 lead：

```bash
agt agent update lead --add-builtin-tool task_collaboration
```

只**接收**任务的 Agent 不需要额外能力，只需已绑定并启用。调度器必须在运行（Web 控制台会启动它）。

## 任务的流程

![lead 发起任务，执行者完成后通知 lead，lead 提交汇报](../../public/guide/task-flow.zh.svg)

1. **请求。** lead 调用 `task_request`，传入执行者、双方都在的会话、输入内容和 `request_key`。重复使用同一个 key 会返回同一个任务，重试不会产生重复任务。
2. **执行。** 调度器运行执行者。它的最终回答就是任务结果，agt 会把它发到任务所在的会话；如果执行者试图用 `message_send` 自己发布结果，这次调用会被拒绝，并提示它改为直接回答。失败和结果不明确的情况也会通知。
3. **通知。** 结果送达 lead，lead 开始一次新的执行来处理它。
4. **汇报。** lead 审阅结果后调用 `task_report`，在原会话中发布汇报。普通消息不算汇报；如果没有提交，任务会显示汇报仍未完成。

## 等待结果

自身正在执行任务的 lead 可以暂停，直到子任务完成：

- `task_await` 等待一个或最多 20 个任务。Agent 保存检查点并释放执行槽；任务完成、失败或到达截止时间后，从检查点继续。它必须是这一步中唯一的工具调用。
- `task_yield` 暂停到指定时间。
- `task_input` 向请求者提问；任务会等待 `task_reply` 的答复或截止时间到达。

带截止时间的等待不会取消子任务，也不会伪造结果；迟到的结果仍会送达。

## 管理队列

Agent 还可以调整优先级、延后、取消和重试排队中的任务，人还可以核对结果不明确的任务。所有工具和参数见[任务模块](../modules/tasks.md#管理队列)。

排队中的任务每等待五分钟提升一个优先级，最高到 `high`。修改任务需要提供当前的 `revision`，两个 Agent 不会覆盖彼此的修改。

## 并行工作

默认情况下，一个 Agent 同一时间只处理一件事。要让一个 Agent 同时运行多个独立任务：

1. 设置 `scheduler.parallel_work: true`（见[任务模块](../modules/tasks.md#设置)）。
2. 把任务标记为可并行，并声明它们会操作的共享对象：

```json
{ "execution_policy": { "parallel": true, "exclusive_resources": ["repo:frontend"] } }
```

`exclusive_resources` 有重叠的任务永远不会同时运行，即使属于不同 Agent。这些键是 Agent 之间的约定，不是文件锁；操作同一对象的 Agent 必须使用相同的键。

## 在 Web 控制台中

点击会话标题栏的**任务**。面板列出本会话的任务，可以按进行中、需要处理、已结束筛选，并支持搜索。打开任务可查看状态、结果、投递和汇报，也可以在这里答复、取消、重试或核对。**交互观察**以关系图展示 Agent 之间的委派。

## 出现问题时

- **`outcome_unknown`**：执行被中断，agt 无法确定工作是否已发生。它永远不会自动重试。请检查实际结果（文件、消息、外部系统），然后用 `task_resolve` 记录实际情况；如果失败了，再用 `task_retry` 重试。
- **一直处于 `queued`**：执行者可能被停用、设为 `manual`，或讨论已暂停或过期。`task_get` 的 `blocked_reason` 会显示原因。
- **缺少汇报**：lead 收到了结果，但没有调用 `task_report`。任务的汇报状态停留在 `awaiting_report`，五分钟后会产生提醒。

在 CLI 中：

```bash
agt tool call --principal lead task_list --args '{"limit":20}'
agt tool call --principal lead task_get --args '{"task_id":"<任务 ID>"}'
```

所有工具和参数见[任务模块](../modules/tasks.md)。
