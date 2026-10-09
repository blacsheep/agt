# 文档导航

现行说明同步于 2026-09-08，以当前工作区代码为准。

## 使用与开发

- [架构图与执行流程](architecture/README.md)：当前模块边界、资源管理与能力刷新、消息调度、Run 执行、异步协作及图表更新约定。
- [项目 README](../README.md)：模块、资源目录、CLI 和显式执行。
- [Web 使用指南](../web/README.md)：前后端一起启动、角色托管、会话与调试界面。
- [框架验收指南](framework-acceptance.md)：模型接入、CLI/MCP、调度控制和排错。
- [持久化调度设计](scheduler-plan.md)：并发、参与决策、提醒、预算与恢复边界。
- [共享资源管理](resource-management.md)：七类资源的统一入口、更新契约、版本检查与并发边界。
- [MCP 工具契约](mcp-tool-contracts.md)：注册探测、参数校验及 Native 能力刷新。
- [Native Agent 执行循环](native-agent-loop.md)：运行宿主、回合循环和工具阶段契约。
- [上下文管理](context-management.md)：模块边界、自动压缩、持久化摘要与重置语义。
- [Web 架构](web-console-plan.md)：Node 与 Rust 分工、身份和 UI 状态。

- [异步任务使用指南](async-task-guide.md)：挂载任务协作能力、跨会话委派、结果查询和当前边界。
- [外部 Harness 接入](harness-integration.md)：执行引擎、运行环境、Codex 会话与取消边界。

## 实施中设计

- [异步工作任务与事件驱动协作](async-work-orchestration.md)：任务队列、双边等待、优先级、可靠回报、SSE 和 Codex 进度；2026-09-17 已接入任务账本、统一执行与定向回报，已支持排队优先级和延后屏障；运行中挂起/事件推送等仍在后续阶段。

## 历史设计与实测

以下文档保留对应阶段的范围、提交号和验收事实；其中的“下一步”、测试数量及
当时未支持的功能，不代表当前状态。

- [Runtime 基础阶段](runtime-plan.md)
- [资源与消息框架阶段](messaging-tools-plan.md)
- [工具契约与调用诊断](tool-contract-plan.md)
- [2026-09-06 真实模型失败记录](live-acceptance-2026-09-06.md)
- [2026-09-06 真实模型复验](live-model-retest-2026-09-06.md)

最近实现验证：261 个 Rust 测试、7 个 Web 测试及浏览器模拟模型验收通过。
这是 2026-09-07 实现轮次的记录，不是本次文档更新重新执行的测试；历史真实模型
复验也不能替代新调度器在具体 Provider 上的真实验收。

2026-09-08 调度优化：五秒滑动安静期、并行参与判断与持久化回复排队、
100 次回复上限和新用户输入重置。完整 Rust 回归、Clippy、格式检查和 Web 集成测试通过；
本轮未调用真实模型，未重新进行浏览器交互验收。

自动消息深度限制已全局移除，三 Agent 调度测试覆盖从 0 报到 10。

开发阶段清理：移除旧 worker、Gateway/Lead 占位配置、未实现的模型参数、
非持久化技能来源、未接入的技能 catalog、旧 MCP stdio framing 和数据库升级链。
绑定只接受当前字段；现有旧绑定需重新保存，运行时不再忽略 max_depth。
历史实测记录保留，供查阅当时的行为，不代表仍维护旧代码。

本轮清理验证：244 个 Rust 测试、7 个 Web 测试通过，Clippy（禁止警告）、
格式检查及 CLI 构建通过；新版 Runtime 路径保留 Agent 建群/联系同伴测试。
只使用本地模型桩；未重启现有 Web 或重新触发真实模型讨论。
