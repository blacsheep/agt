# Native Agent 执行循环

最近整理：2026-10-10。

Native 执行采用明确的阶段函数。调度器仍负责输入投递、参与判断、租约和任务责任；执行循环负责已经认领的一次 Run，不接管其他会话的消息，也不共享全局 transcript。Harness 保留自己的执行引擎，不作为 Native 模型请求的一种实现。

## 模块与状态归属

| 模块 | 职责 |
| --- | --- |
| `engine.rs` / `Runtime` | 公共 Run 认领、重放、终态持久化、显式子 Run 委派 |
| `engine/dispatch.rs` | 引擎选择、续跑引擎兼容检查、分派到 Native/Harness |
| `engine/native.rs` / `NativeRun` | 本次执行的上下文、工具会话、回合推进、完成与挂起结果 |
| `engine/native/prepare.rs` | 配置与工具快照、身份和能力绑定、恢复显式工作 checkpoint |
| `engine/native/inputs.rs` / `run_inputs.rs` | 安全边界轮询、参与判断、持久化接纳与输入终态 |
| `engine/native/request.rs` | 请求前检查、压缩、模型请求及审计、有限的超窗恢复 |
| `engine/native/tools.rs` | 整批验证以及每个工具的 prepare/permission/before/execute/after 流水线 |
| `permission.rs` / `permission/*` | Native 权限循环与策略插件；拒绝返回配对工具结果，取消终止运行 |
| `engine/native/framework.rs` | Framework 工具分派、消息去重和数据库副作用围栏 |
| `engine/native/mcp.rs` | MCP 连接、调用与关闭、外部结果规范化 |

`NativeRun` 借用宿主的存储、模型和取消信号，自己持有当前 transcript、usage、已使用的 call ID、成功消息发送的去重状态以及工具连接。委派子 Run 使用独立上下文，共享步数统计；这些状态不提升到 principal 全局。

## 公共执行边界

调度器为两种引擎共用准入、请求标识、任务绑定和结果结算。 `scheduler/backend.rs` 集中表达执行前的差异：参与策略、上下文准备、Native 模型预算与 MCP 准入检查。公共执行流程不再组合 `external` 布尔值与可空的 Native 配置。适配模块是固定的两种引擎分派，不是插件注册层。

Runtime 仍按 `claim → execute_claimed → finish` 执行。已保存的 Run 先返回，不会因引擎配置变化重新执行；新 Run 认领后才再次解析引擎。调度准入时的引擎选择不替代运行时配置检查。显式 Native 子 Run 继续使用原委派配置快照。Native 与 Harness 分别维护自己的执行阶段；Harness 的宿主、协议与取消确认边界见 [Harness 执行流程](harness.md#执行流程)。两种引擎返回原有执行结果，并由公共终态逻辑保留 completed / suspended / canceled / interrupted / outcome_unknown 等区别。

## 循环层次

```text
Runtime: claim → prepare NativeRun
  外层：follow-up 回合
    接收一条 steering（如有）
    内层：模型 / 工具 / steering
      request_turn → Final 或 Tools（超窗恢复与网络重试保持原请求）
      Tools → 完整工具批次 → 接收一条 steering → 继续
      Final → 若有 steering，加入上下文继续；否则结束内层
    有 follow-up → 发布当前回答并结算已完成输入 → 接收 follow-up → 继续外层
    无 follow-up → Complete(Text / Silent)
Runtime: 保存 Run 终态与剩余已接纳输入的终态
```

普通消息不再固定等待 5 秒，也不合并为一个调度任务。每条消息独立持久化。人类消息默认 steering，Agent 消息默认 follow-up；发送时可用 `input_mode` 或 CLI `im message send --input-mode steering|follow_up` 指定。模式不授予额外权限。仅同 principal、同 conversation、同 discussion 的普通输入可进入当前 Native Run。群内未被点名的 Agent 消息仍须参与判断；silent/wait 不进入执行上下文。

接纳先写 `run_inputs` 收据，再进入模型或工具阶段，并计入回复额度。暂停、额度、成员资格、删除和上下文重置仍生效。压缩保留已接纳输入原文。已接纳但未完成的输入在失败、取消或接管时随 Run 结算，不自动重放；尚未接纳的消息保留在队列，包含最后一次轮询之后到达的消息。已完成输入不被后续失败覆盖。follow-up 开始新的发送去重范围；steering 保留当前范围。回复关联当前处理的消息。

显式 work 继续使用 task_input/await；Harness 使用后续独立 Run 处理排队消息，不支持本机制的运行中注入。普通网络重试仍只重试同一个模型请求，不消费队列。

`prepare_turn` 检查取消、调度资格与步数预算；恢复请求也必须经过这些检查。 `prepare_request` 只在完整工具批次之间压缩并检查输入预算。 `request_model` 记录请求审计、处理流进度并验证完整响应；成功后移除 usage 元数据，不把计数内部字段写回模型上下文。

## 工具阶段契约

1. `prepare_tool_batch`：先验证完整响应、工具名、调用 ID 和委派权限。await/yield/input 必须独占批次。任何一项失败，都不开始该批次的工具副作用。
2. `prepare_tool_call`：解析参数，准备脱敏审计记录；不写数据库或调用工具。
3. `before_tool_call`：检查取消与调度围栏，然后持久化 Started 审计记录。
4. `execute_tool_call`：调用 delegate、Framework 或 MCP 实现。Framework 服务继续执行身份、成员和权限检查；数据库短事务围栏不包住外部请求。
5. `after_tool_call`：先保存成功/失败审计与失败计数，再把裁剪后的结果写入 transcript。可反馈给模型的业务错误保留为工具结果，后续调用继续；致命错误或取消终止批次。框架工具分类后的公开错误进入审计与模型上下文；被隐藏的根因只写入私有工具详情的 `diagnostic` 字段（`agt tool call` 失败时输出到 stderr），不进入调用记录、Web 投影或外部 MCP 客户端。成功的等待工具返回 Suspend，由循环保存包括该工具结果的 checkpoint。

这些阶段是内部 Rust 方法，不是任意注册的插件回调。新增工具后端接入 execute 阶段，通用审计或结果处理放在 before/after 阶段；强制权限和取消校验不能被扩展绕过。当前工具批次保持串行，以保留已有副作用顺序。

## 完成、沉默与恢复

循环用 `LoopOutcome::Complete(FinalReply)` 和 `LoopOutcome::Suspended` 区分正常结束与等待。 `FinalReply` 区分文本和沉默，只有在宿主边界转回现有 `[[silent]]` 发布协议。本次 Run 已向当前会话成功发送消息时，仍抑制自动重复发布最终文本。这些结果不代替工作任务账本的完成/汇报责任检查。

恢复继续校验配置指纹与工作目录；已完成工具结果、call ID 和成功发送去重状态一起恢复。工具连接随 NativeRun 释放，部分初始化失败时也关闭已连接的 MCP 客户端。不自动重试结果未知的副作用。已接纳输入以 `input` 状态关联当前 Run。

## 验证

`cargo test -p agt-runtime --offline` 使用本地模型桩及 MCP 测试进程。原有回归覆盖整批预验证、取消、委派、去重、挂起恢复与 checkpoint 崩溃间隙。 `tests/runtime/loop_stages.rs` 补充业务错误后继续批次、致命错误/取消阻止后续副作用，以及成功工具回合之后恢复额度重新生效的测试。

`src/scheduler/tests/run_input_tests.rs` 覆盖 steering/follow-up 顺序、工具批次边界、参与判断、跨会话隔离、删除围栏、重试快照、压缩、额度、暂停及接管后的终态。

Native 权限配置、人工队列与独立审核模型的边界见 [权限审批与交互观察](permissions.md)。
