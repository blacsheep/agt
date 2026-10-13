# Run 执行流程

最近代码核对：2026-10-09。显式 `agt agent run` 直接调用 Runtime；聊天调度也调用同一执行入口。

```mermaid
flowchart TD
    A["RunRequest"] --> B["规范化参数并持久化 claim"]
    B --> C{"请求键已存在？"}
    C -->|参数相同| D["返回已有记录"]
    C -->|参数冲突| E["拒绝"]
    C -->|新请求| F{"Agent 执行方式"}
    F -->|Model| N1["Native：加载配置快照<br/>准备上下文、工具和权限"]
    N1 --> N2["检查取消与执行资格<br/>必要时压缩上下文"]
    N2 --> N3["请求模型"]
    N3 --> N4{"返回类型"}
    N4 -->|工具调用| N5["整批校验 → 逐项权限检查<br/>审计 → 执行工具 → 保存结果"]
    N5 --> N6{"工具要求挂起？"}
    N6 -->|否| N7["工具结果加入上下文"]
    N7 --> R1{"本批次有成功的<br/>自身 MCP / Skill 配置写入？"}
    R1 -->|否| N2
    R1 -->|是| R2["准备候选能力<br/>检查模型工具能力、连接、schema 与工具数量"]
    R2 --> R3{"候选能力就绪？"}
    R3 -->|是| R4["切换能力快照与系统提示词<br/>更新执行指纹，报告 applied"]
    R3 -->|加载失败| R5["保留旧能力<br/>配置仍已保存，报告 failed"]
    R4 --> N2
    R5 --> N2
    R2 -.->|取消或失去执行资格| Z
    N6 -->|是| W["保存检查点，返回 suspended"]
    N4 -->|最终回答| N8{"还有可接纳的新输入？"}
    N8 -->|有| N2
    N8 -->|无| Z["保存 Run 终态"]
    N3 -.->|请求失败或取消| Z
    N5 -.->|致命错误或取消| Z
    F -->|Harness| H1["获取会话执行锁<br/>读取配置与线程映射"]
    H1 --> H2["启动 Codex app-server<br/>建立或恢复 thread"]
    H2 --> H3["执行 turn<br/>桥接框架工具、审批和公开进度"]
    H3 --> H4{"协议确认结果"}
    H4 -->|正常完成| Z
    H4 -->|确认安全等待| W
    H4 -->|失败或结果不确定| Z
```

## 关键约束

- claim 在模型和工具副作用之前提交；同一请求键重放返回已有记录，不重新执行。
- Native 由 AGT 控制模型、工具和压缩循环；Harness 由外部引擎控制内部循环。
- Native 工具来源包括 Framework、MCP 和允许名单内的同步 `delegate`；本机命令需管理员配置及 Agent 能力同时开启。
- Native 工具先做批次校验，再逐项审批和执行。拒绝可作为工具结果返回模型；致命错误或取消终止执行。
- 模型传输重试只重试模型请求，不重放已完成工具。
- 成功的 `framework_resource_write` 若修改自身 MCP/Skill 挂载，或当前挂载的 MCP/Skill
  资源，会标记本批次需要刷新；批次内剩余调用继续使用旧快照，下一次模型请求才使用新能力。
- 刷新失败不撤销已保存配置，不自动重试；模型和调用审计都会收到独立的加载结果。
  Skill 单独变化复用现有 MCP 连接；MCP 工具别名由资源引用与远端工具名确定。
- 刷新不更换当前 Model、Provider、基础 Prompt、Framework 能力或委派配置。
  管理员实时切换审批策略的既有行为独立保留。MCP 使用范围由 Agent 装配决定，具体调用仍须审批。
- 其他 Agent、Web/CLI 修改配置不会隐式刷新当前 Run。Harness 不使用这套 Native 刷新路径。
  详见[资源管理与能力生效](resource-flow.md)。
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
