# 持久化讨论调度器

最近整理：2026-10-10（由英文版翻译，并按当前代码修正过期描述）。使用方式见[对话与自动回复](../guide/conversations.md)。

调度器在 `runtime/runtime.db` 中管理讨论轮次、独立输入任务、等待、租约和回复额度。IM 保持通用，不感知调度。显式 `agent run` 沿用自己的执行规则；所有自动消息执行只走调度器。

## 模块边界

`scheduler.rs` 是协调者，保留原有公开入口。每次 tick 的顺序固定：心跳、轮询进行中的任务、对账 WorkItem、扫描输入、讨论过期、对账投递、派发就绪任务。各阶段保持原有的读写与事务边界。

| 模块 | 职责 |
| --- | --- |
| `scheduler/lease.rs` | 协调者归属、独立心跳与释放 |
| `scheduler/controls.rs` | 人工授权、删除/解散、上下文重置 |
| `scheduler/ingress.rs` | 把 IM 投递转换为持久化调度任务 |
| `scheduler/admission.rs` | 复查绑定、响应策略、讨论状态、快照与等待条件 |
| `scheduler/guard.rs` | 事务围栏与每次尝试的模型调用计数 |
| `scheduler/execution.rs` | 共用的输入封装、Run 请求与执行编排 |
| `scheduler/backend.rs` | Native/Harness 的策略、上下文、参与判断及引擎相关检查 |
| `scheduler/decision.rs` | 参与判断，产出 silent / wait / ready |
| `scheduler/completion.rs` | Run 结算、带围栏的发布与汇报义务 |

已领取任务的路径是 `admission → context → 参与判断或 Run → completion`。参与判断在进入回复准入前释放执行槽，不会直接落入执行。模型与外部调用都在短事务围栏之外；发布前仍复查领取状态。`ScheduleStore` 负责持久化、原子领取和计数状态转换。

## 引擎准备边界

准入阶段用与 Runtime 分派相同的 Agent 配置查询得到 `EngineKind`；Native 的完整配置仍在准入之后才解析。 `scheduler/backend.rs` 集中表达下列差异，本身不运行任何一种协议：

| 关注点 | Native | Harness |
| --- | --- | --- |
| 非 manual 响应策略 | Agent 同伴消息走参与判断；人类消息保留策略 | 仅私聊与被提及消息 |
| 参与判断与工作分诊 | 受限的 Native 模型调用 | 框架准入，无 Native 推理 |
| 上下文 | 按 token 预算的历史与压缩；触发消息不进历史 | 含触发消息的可见历史，供 Harness 渲染 |
| 预算与 MCP 准入 | Native 模型预算及绑定/讨论检查 | 执行时由 Harness 校验 |

公共代码仍负责创建输入封装和 Run 键、绑定调度任务、结算返回的 Run。Native 上下文失败在准备阶段结算任务；Harness 上下文错误传到执行边界。 `engine/dispatch.rs` 在 Run 领取后才选择引擎；Runtime 保留公共的重放与终态持久化。取消、检查点兼容性和外部协议事实不会被抹平成同一种表示。

## 任务状态转换

`schedule_state.rs` 负责领取/延后、人工唤醒、租约恢复、参与判断、输入接纳和 Run 结算时成组修改的字段。这些操作只修改 `ScheduledTask`；授权、额度扣减/退还、接纳收据和数据库写入留在各自事务中。没有通用的 `set_state` 接口或新的状态机引擎。

| 操作 | 保持的契约 |
| --- | --- |
| 领取 | 同时设置 evaluating/responding、owner、version 和额度代次 |
| 延后已领取任务 | 保留 owner/version，直到存储层校验并退还旧领取 |
| 人工唤醒 | 清除 quiet/reason/owner，递增 version，保留决策审计 |
| 恢复受阻工作 | 保留领取与等待字段，只重置 state/due/reason |
| 使就绪快照失效 | 清除 owner/reason，递增 version，保留 quiet 设置 |
| 回收过期租约 | 仅 evaluating 可重试；responding 变为 interrupted |
| 调度参与判断 | 持久化决定；participate 变为 ready，不在判断中直接执行 |
| 输入参与等待 | 保留决策审计，释放输入 owner；在调用方事务中移除收据 |
| 接纳运行中输入 | 执行前记录 owner、version、Run 和模式 |
| 完成一个输入回合 | 释放输入 owner，保留其 Run 和诊断原因 |
| 结算父任务 | 只有尚未完成的已接纳输入继承结果；已完成输入保持终态 |
| 完成回复 | 汇报义务与沉默语义分开保存，不与显式工作混淆 |

## 消息与运行中输入

人类消息需要显式授权自动回复。第一条授权消息创建讨论；同一未结束讨论中新授权的人类消息会重置所有身份的回复计数，并续期讨论。授权持久且幂等：重放授权或继续讨论不会重置计数。已结束的讨论保持结束，新的人类消息开启新讨论。

没有固定的 5 秒安静窗口。人类消息、提及和工作任务各自独立成任务；未被提及的同伴消息若遇到同一身份、同一会话、同一讨论中排队或等待的判断任务，会并入该任务（追加 triggers/deliveries，回复指向最新消息，最多 20 条），否则新建任务。等待不再是定时器：参与判断时先计算会话的工作集合（ready_to_reply、responding、input，以及无需判断即可执行的排队任务；evaluating 与 waiting 不算），集合为空时判断工具不提供 wait；否则 wait 记录 `blocked_on`。保存任务时，若 `blocked_on` 中任一成员已不在工作集合，等待任务立即到期；登记等待时同样检查，避免丢失唤醒。被唤醒后若会话没有比该决定更新的消息，直接以 silent 结束，不调用模型。300 秒看门狗只兜底从不结算的成员。因为等待者不在工作集合中，成员之间不会互相等待。人工授权会唤醒等待中的任务和受回复额度限制的任务。模型可见历史使用持久化摘要加按 token 预算截取的近期消息，见[上下文管理](context.md)。

定时任务（`timers` 表）是第二种授权来源。到点时，调度器在持有协调租约的 tick 里以定时任务的人类身份发送消息（客户端消息 ID 为 `timer:<id>:<计划时间>`），再对这条消息调用与人类授权相同的 `authorize_human`，因此它开启或续期讨论、重置回复额度，和人亲自发送完全一样。发送前先把消息正文和提及写入记录（`pending`），崩溃重试会得到同一条消息。错过的周期只补发一次并注明迟到；投递失败每 60 秒重试，5 次后取消。最近的触发时间并入调度器的唤醒计算，两次触发之间没有任何执行或轮询。Agent 设置的定时任务以工作空间所有者的名义发送，记录 `creator`；`timer_create` 是写操作，按权限模式审批。

Native 普通聊天在安全边界接纳“同身份、同会话、同讨论”的后续输入。人类消息默认 steering，Agent 消息默认 follow-up； `input_mode` 可显式指定，不改变授权。steering 在完整工具批次之间或最终回复提交前进入；follow-up 在当前回合结束后进入。重试保留原请求快照，不消费新输入。显式工作沿用 task_input/await；Harness 用后续独立 Run 处理新输入。队列与恢复契约见 [Native 执行循环](native-loop.md)。

## 参与判断与回复准入分离

每个 home 一个协调者，默认最多 3 个并发任务，可通过 `config.yaml` 的 `scheduler.concurrency` 或 `schedule run --concurrency` 调整。不同身份可以在同一会话中并行做参与判断，包括同伴正在回复时。判断使用一次受限的 participate/silent/wait 函数调用，不连接 MCP。

参与判断持久化 `ready_to_reply`、安全原因码和所考虑的历史序号，然后释放线程，此时尚未消耗回复额度。三种判断结果都会另存 `last_decision`：判断时间、所考虑的消息序号，以及可选的 `public_reason`（最多 240 个字符）。它跨状态变化和重启保留，是面向用户的简短说明，不是模型的私有推理（私有推理不保存也不重放）。正式回复的输入包含判断理由。就绪的回复比判断优先出队；进入回复准入前若出现新历史，决定失效并重新判断。

同一会话同时最多开始一个自主回复，并等待现有回复者结束。人类请求和被明确提及的同伴直接执行，不同身份之间可以并行，不经过安静窗口。响应策略、成员资格、消息删除和讨论过期仍是执行门槛。MCP 可用性跟随 Agent 装配，单次工具调用仍需审批。未被提及的非 manual 同伴消息走参与判断。

一个身份在所有会话中同时最多只有一个 evaluating、ready 或 responding 任务。ready 任务占用身份，不占线程。多个身份绑定同一 Agent 定义时可以并行执行。

## 额度与控制

| 配置项（`config.yaml`） | 默认 | 含义 |
| --- | --- | --- |
| `scheduler.replies_per_agent` | 100 | 每个身份在两次新的人类授权输入之间可开始的回复数；`null` 不限 |
| `scheduler.discussion_seconds` | 1800 | 讨论授权期限；`null` 不限 |
| `scheduler.model_steps` | `null`（不限） | 单个 Native Run 的模型步数上限 |
| `scheduler.timezone` | `null`（本机时区） | 模型输入中 `now`、`sent_at` 和定时任务使用的 IANA 时区 |

回复在执行准入时计数，判断、等待和排队时间不计。执行前延后会退还该次计数，且不影响更新一轮人类输入的计数。失败或中断的执行保留计数。Agent 消息和跨会话联系继承来源讨论，不重置额度。

没有整轮讨论的模型调用上限，也没有每身份判断次数上限；调用次数只作诊断。Run 请求里的 `max_steps` 仅为兼容字段，不再生效，步数上限只看 `scheduler.model_steps`。自动消息深度不受限制，绑定没有 `max_depth` 字段；因果深度仍记录在消息和 Run 请求中供诊断。旧版本保存的定时/安静等待记录仍可读取，唤醒后按上述规则处理。

暂停阻止新工作，在途回复可以完成。继续会续期，但不重置回复额度；新的授权人类消息才会重置。结束/删除会取消待处理、就绪和等待中的工作。控制作用于来源会话，包括在其他会话中的同伴任务。

## 持久化与恢复

短 SQLite 事务预留工作和计数，没有事务跨越模型或 MCP 请求。独立协调者心跳和带版本的领取用于隔离过期工作。过期的 evaluating 可以重试；responding 变为 interrupted，绝不自动重放。就绪和等待中的任务在重启后保留。人工授权在 human_messages 中去重，任务持久化计数代次和回复准入状态。

本地框架操作和最终发布都会复查讨论与领取状态。已发出的外部 MCP 请求无法撤回。发布失败记为 interrupted，需要人工检查；Run 结果不是自动重放计划。Web 启动一个 watch 协调者；异常退出后，接管可能需要等 15 秒租约过期。

存储：`im/im.db` 保存消息和投递；`runtime/runtime.db` 按模块分表保存身份与策略、正式 Run 与调用诊断、讨论、任务决定、等待和回复计数。判断/就绪/等待阶段可以没有 Run。`schedule status` 包含回复用量；Web 调试模式显示计数、安全原因和状态，从不显示模型推理。

## 模型请求重试

每个逻辑模型步骤（包括参与判断）对传输/HTTP 错误、无效 JSON、明确的 API 错误和空响应最多尝试 10 次。退避从 250ms 指数增长，上限 4 秒，每 50ms 检查一次取消。每次托管尝试都经过调度准入与围栏，并计入实际调用数。重试保留 transcript，不重放已完成的工具；Run 的 `model_steps` 统计逻辑步骤，`request_attempts` 记录在模型调用审计中。拒答、截断、配置错误和工具失败不触发这个重试循环。

## 发送去重与输入结算

同一 Run 内，会话、正文、规范化提及和回复目标都相同的成功 `framework_message_send`，即使模型给出新的调用 ID，也复用第一次成功的请求键。每次调用仍经过服务校验、成员检查和调度围栏；IM 返回已有消息并标记 `duplicate=true`。失败不缓存；这不是跨 Run 的文本去重，收件人或回复目标不同仍视为不同消息。

已接纳输入单独计数并在执行前记录，待处理输入状态为 `input` 并关联消费它的 Run ID。完成的回合结算其输入；之后的失败不会覆盖已完成输入。失败或租约被接管时，剩余已接纳输入随父任务结算，不自动重放；未接纳输入继续排队。follow-up 开启新的发送去重范围，steering 保留当前范围。

## CLI

- `schedule start <conversation> [--message <id>]`：带 `--message` 时幂等授权该条人类消息；不带时授权待处理人类投递并继续讨论。
- `schedule run --watch [--concurrency 3]`：持续处理；不带 `--watch` 只处理当前就绪的工作。
- `schedule status`、`schedule pause|end|resume <conversation>`。
- `schedule delete <conversation> --actor <principal>`。
- `schedule dissolve <conversation> --actor <principal>`：仅群主可执行，群聊终止并只读保留历史；同时结束目标为该会话的讨论，包括来自其他会话的工作。
- `schedule reset <conversation> --actor <principal>`：重置模型上下文，见[上下文管理](context.md)。

个人入口的隐藏/显示不影响调度。IM 历史与入口的生命周期见[会话生命周期](conversations.md)。

## 测试范围

覆盖：独立即时输入、steering/follow-up 边界、并行判断与直接回复、自主回复互斥、过期决定、100 次回复重置与幂等、等待与重启、拒绝不支持的 schema、暂停/结束/发布围栏。测试使用本地桩，不调用付费模型。
