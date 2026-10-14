# 整体架构

最近代码核对：2026-10-10。箭头表示调用或数据流，不是严格的 crate 依赖关系；紫色虚线是工具调用和变更事件的回流。图里只画主干：agt CLI 还会启动长期运行的 scheduler，并执行显式 `agt agent run`，这两条没有单独画出；各模块的细节见下文和专题页。

![agt 整体架构：入口、服务层、执行层、执行引擎与存储](../../public/architecture/overview.svg)

## 模块与进程边界

- `agt-core`、`agt-storage`、`agt-config` 提供通用引用、文件操作、唤醒通知和 home 配置。
- 七类配置资源由 `agt-service::Resources` 统一管理；CLI 资源命令和 ToolService 都调用它，Web 经 `agt tool call` 进入 ToolService。各资源模块保留自己的配置类型与校验规则。
- 资源管理使用 `.resources.lock` 串行化读写与删除执行；`revision/expected_revision` 检测过期编辑。详情见[资源管理与能力生效](resource-flow.md)。
- 资源服务保存配置；Native Runtime 在成功自修改 MCP/Skill 后，于工具批次结束刷新能力。保存成功不等于当前 Run 已加载，刷新失败保留旧能力并单独报告。
- Agent 引用 Model 或 Harness，二者互斥；Model 再引用 Provider。Prompt、Skill 与工具能力附着于 Agent。
- Node 负责本地界面和 HTTP 适配，业务操作经 Rust CLI；scheduler 是长期运行的 Rust 子进程。
- 当前每个 Web 事件订阅启动一个长期运行的 `agt events watch`，将 JSONL 桥接成 SSE。
- IM 自身不调用模型；Runtime 负责执行，Scheduler 负责执行准入，WorkItem 记录工作责任。
- `agt-service` 不依赖 Runtime。它声明 `WorkControl`、`HarnessControl`、`ExecutionControl`，由 Runtime 实现并注入。
- `agt-inference` 只负责一次执行内的模型 I/O：Agent 配置解析、Chat Completions/Responses 协议与流式解析、上下文预算与压缩，以及共用的 `RuntimeError`/`StopCode`。它不读写数据库，也不了解调度；`agt-runtime` 原样再导出这些类型。
- 调度、WorkItem、审批、Harness 会话与 Run 存储共用 `runtime.db` 的事务与同线程连接，属于同一事务域，因此留在 `agt-runtime` 内按模块划分，而不拆成独立 crate。
- Harness 当前接入本机 Codex app-server，由它拥有内部模型和工具循环；AGT 不把它当作 Native 的 ModelClient。

## 持久化职责

以下路径均相对于资源 home。`runtime/runtime.db` 是 Runtime 侧的单一事务域； `im/im.db` 和 `memory/memory.db` 是独立领域，与它之间不构成全局事务。

| 路径 | 主要内容 |
| --- | --- |
| `config.yaml` 与资源目录 | 运行策略、Agent、Model、Provider、Prompt、Skill、MCP、Harness 配置 |
| `.resources.lock` | 配置资源管理读写、分类修改与删除执行的共享锁；不是数据库事务 |
| `im/im.db` | 参与者、会话、消息、投递和可见性 |
| `runtime/runtime.db` | 按模块分表：身份绑定与响应策略（bindings）、Discussion 与调度任务（scheduler）、WorkItem 与汇报（work）、Run 与调用诊断及检查点（runs）、会话摘要（contexts）、Harness 会话与执行围栏（harness_sessions）、审批（approvals）、事件投影（events）；共用 `change_outbox`，`schema_modules` 记录各模块版本 |
| `memory/memory.db` | Agent 私有、关联原消息来源的长期摘要 |

原始聊天、上下文压缩摘要和长期 Memory 各有职责；重置上下文不会删除聊天或长期记忆。事件通知用于触发刷新，SQLite 业务记录仍是状态依据。与 IM 之间的传播依靠 outbox、幂等键和对账。

同一线程内的 Runtime 存储共享一个 `runtime.db` 连接；调度围栏内的写入以 savepoint 加入围栏事务，因此围栏检查与受保护的写入原子提交。不同线程（调度并发任务）使用各自连接，由 SQLite 锁串行化写入。数据库版本按模块记录；当前开发阶段不提供旧库升级，旧的分库文件不再读取，需重建 Runtime 状态。

## 代码入口

- [Workspace](../../../Cargo.toml)：crate 列表与依赖。
- [Web 服务](../../../web/server.mjs)：`cliRunner`、`createConsole`、`watchEvents`。
- [共享资源服务](../../../crates/agt-service/src/resources/mod.rs)与[更新/版本检查](../../../crates/agt-service/src/resources/mutation.rs)。
- [能力刷新](../../../crates/agt-runtime/src/engine/native/refresh.rs)：批次边界装配与应用。
- [Agent 配置](../../../crates/agt-agent/src/config.rs)：`AgentConfig::validate`。
- [服务接口](../../../crates/agt-service/src/lib.rs)：Runtime 能力端口。
- [身份绑定](../../../crates/agt-service/src/bindings.rs)：`Binding`、`BindingStore`。
- [事件投影](../../../crates/agt-runtime/src/events.rs)：`Events`。
- [长期记忆](../../../crates/agt-service/src/memory/mod.rs)：身份隔离与来源可见性校验。

返回[架构图导航](index.md)。
