# Run 执行流程

最近代码核对：2026-10-09。显式 `agt agent run` 直接调用 Runtime；聊天调度也调用同一执行入口。

第一张图是总览：认领请求、按执行方式分流，以及 Harness 路径。Native 的模型与工具循环在第二张图展开。

![Run 执行总览](../../public/architecture/run-flow.svg)

## Native 循环

红色节点是直接进入终态的出口；紫色虚线汇总了回到下一次模型请求之前的路径。

![Native 执行循环：模型请求、工具批次与能力刷新](../../public/architecture/run-flow-native.svg)

## 关键约束

- claim 在模型和工具副作用之前提交；同一请求键重放返回已有记录，不重新执行。
- Native 由 AGT 控制模型、工具和压缩循环；Harness 由外部引擎控制内部循环。
- Native 工具来源包括 Framework、MCP 和允许名单内的同步 `delegate`；本机命令需管理员配置及 Agent 能力同时开启。
- Native 工具先做批次校验，再逐项审批和执行。拒绝可作为工具结果返回模型；致命错误或取消终止执行。
- 模型传输重试只重试模型请求，不重放已完成工具。
- 成功的 `framework_resource_write` 若修改自身 MCP/Skill 挂载，或当前挂载的 MCP/Skill 资源，会标记本批次需要刷新；批次内剩余调用继续使用旧快照，下一次模型请求才使用新能力。
- 刷新失败不撤销已保存配置，不自动重试；模型和调用审计都会收到独立的加载结果。Skill 单独变化复用现有 MCP 连接；MCP 工具别名由资源引用与远端工具名确定。
- 刷新不更换当前 Model、Provider、基础 Prompt、Framework 能力或委派配置。管理员实时切换审批策略的既有行为独立保留。MCP 使用范围由 Agent 装配决定，具体调用仍须审批。
- 其他 Agent、Web/CLI 修改配置不会隐式刷新当前 Run。Harness 不使用这套 Native 刷新路径。详见[资源管理与能力生效](resource-flow.md)。
- 普通聊天 Native 可在安全边界接纳 steering，在当前回合完成后处理 follow-up；Harness 后续普通消息使用独立 Run。
- 有限模型步数由当前运行配置控制；不要把保留的 legacy `max_steps` 字段当作生效限制。
- Harness 当前使用 Codex app-server，桥接框架工具和审批；不支持将 AGT MCP 挂载或显式 delegate 允许名单直接传入该执行路径。
- 挂起记录已采用能力的执行指纹；恢复重新装配并核对指纹与工作目录，不匹配则拒绝续跑。
- 挂起必须保存有效检查点；失败、取消、中断和结果不确定分别保留，不统一视为成功或可重试。

## 代码入口

- [CLI 执行](../../../crates/agt-cli/src/runtime.rs)：`execute`。
- [公共生命周期](../../../crates/agt-runtime/src/engine.rs)：`Runtime::run`、`finish`。
- [引擎分派](../../../crates/agt-runtime/src/engine/dispatch.rs)：`EngineKind`、`execute_claimed`。
- [Native 循环](../../../crates/agt-runtime/src/engine/native.rs)：`NativeRun`。
- [模型请求](../../../crates/agt-runtime/src/engine/native/request.rs)与[工具阶段](../../../crates/agt-runtime/src/engine/native/tools.rs)。
- [能力刷新](../../../crates/agt-runtime/src/engine/native/refresh.rs)与[MCP 工具快照](../../../crates/agt-runtime/src/engine/native/mcp.rs)。
- [Native 审批](../../../crates/agt-runtime/src/permission.rs)：`PermissionLoop`。
- [Harness 宿主](../../../crates/agt-runtime/src/harness.rs)与[Codex 协议](../../../crates/agt-harness/src/codex.rs)。

返回[架构图导航](index.md)。
