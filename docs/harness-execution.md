# Harness 执行流程

Harness 接管整个任务，外部 Codex 引擎拥有自己的模型与工具循环。AGT 负责身份、
权限、任务责任、会话和运行记录，不通过 Native 的 ModelClient 或模型重试执行它。
普通后续消息仍排队为独立 Run；本次整理不增加运行中的 steering 注入。

## 阶段与状态归属

```text
Runtime: claim → execute_claimed → finish
  Harness: prepare → acquire session lock → prepare input → connect
    Codex: start/resume thread → submit turn → receive protocol events
      event → check fence → persist progress/session facts
      framework tool → prepare → before → execute → after
      native approval → persist request → poll human decision → reply to original RPC
    failure → notify stopping → recover submitted turn → interrupt/confirm
  Harness: collect diagnostics → close connection → settle result/checkpoint
```

| 模块 | 职责 |
| --- | --- |
| `agt-runtime/src/harness.rs` | 宿主编排；连接与关闭外部引擎，串联各阶段 |
| `harness/prepare.rs` | `PreparedHarness` 持有配置、提示词、工具服务、历史和会话库；准备输入与校验续跑指纹 |
| `harness/events.rs` | `HarnessRun` 的取消/租约检查，按事件顺序持久化线程与执行状态 |
| `harness/tools.rs` | `ToolBridge` 的 Framework 工具校验、审计、执行围栏与结果处理 |
| `harness/approval.rs` | 绑定原执行实例的人工审批桥，不运行 Native 审核策略 |
| `harness/finish.rs` | 完成与取消竞态、调用审计结算、会话进度推进和等待 checkpoint |
| `harness_input.rs` | IM 历史到外部输入的转换与去重 |
| `harness_sessions.rs` / `harness_sessions/lifecycle.rs` | 会话配置与所有权租约、执行状态、恢复责任 |
| `agt-harness/src/codex.rs` | 稳定的 `Codex` 接口和事件类型 |
| `codex/transport.rs` | 子进程、限长协议帧、RPC、超时、stderr 诊断和进程组回收 |
| `codex/turn.rs` | 线程准备、正常回合事件循环和动态工具 RPC 响应 |
| `codex/approval.rs` | 原生审批选项、请求关联及响应编码 |
| `codex/cancellation.rs` | 已提交回合的停止恢复、中断请求及终态确认 |
| `codex/control.rs` | 元数据操作与显式压缩 |

`PreparedHarness` 拥有准备资源；`HarnessRun` 借用它并持有会话执行锁。
配置快照必须在取得锁之后读取。事件回调修改 Run 记录，工具桥独立持有等待标志和
失败计数，结束时再交给收尾阶段。没有插件注册表，也不共享 Native transcript。

## 工具阶段

- `prepare_tool_call`：检查取消、等待状态、服务可用性、工具前缀和当前聊天发送限制。
- `before_tool_call`：创建脱敏审计并保存 Started。
- `execute_tool_call`：调用固定身份的 ToolService；保留普通工具的 scheduler 围栏，以及
  task/harness 控制工具已有的专用保护路径。
- `after_tool_call`：记录等待状态、成功/失败审计与失败计数，将结果交回协议适配器。

Codex 协议层先检查动态工具是否在暴露列表中，再进入宿主工具桥。工具业务失败以
失败工具结果回复引擎，不重新提交回合。这些阶段只管理 AGT Framework 工具；Codex
内部的命令、文件、MCP 等工具由 Codex 自己驱动，AGT 接收其进度事件。

## 不可改变的时序契约

1. 线程 ID 在收到 Thread 事件后持久化；默认模型与推理配置在对应事件后保存。
2. Submitted、Turn、Settled 分别推进执行状态。失败/中断的已确认输入进度必须在
   释放执行围栏前保存，不能统一挪到成功收尾中。
3. 中断 RPC 的响应不等于中断完成。只接受匹配线程与回合的终态通知。
4. 取消发生在读取协议帧之后时保留 pending 帧；尚无 turn ID 时进行有界恢复。
   停止恢复只读协议事实，不执行工具、不重放已提交任务。
5. Stopping 是诊断通知，其失败不能跳过中断清理；继续保留原始停止错误。
   InterruptRequested 与 Settled 的控制回调仍保留原有检查。
6. 已确认完成可以赢得与取消的竞态；连接丢失不视为已确认失败，也不自动重放。
7. 等待续跑保存原有 session/thread/config 指纹 checkpoint；恢复仍检查三者一致。
8. 外部进程在收尾前关闭；连接释放时回收进程组，包括外部工具创建的子进程。

## 验证

协议 fixture 是本地 Python app-server，不连接真实模型。
`agt-harness` 测试覆盖成功与工具失败、工具白名单、取消前后 start ack、错回合终态、
完成/取消竞态，以及执行和压缩的停止通知失败后仍中断确认。
Runtime 原有测试覆盖权限、会话锁、终态分类、输入历史、工具控制和等待恢复。

原生人工审批、取消与回传状态见 [权限审批与交互观察](permissions.md)。
