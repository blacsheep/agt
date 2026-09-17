# 异步任务协作（后端首版）

普通消息仍是聊天；需要可靠排队、完成/失败回报时使用显式任务。
需要运行 `agt schedule run --watch`；Web 服务原有的 scheduler 子进程也会处理任务。
本版没有新增任务页面，查询通过 CLI/MCP 或已挂载的内置工具完成。
更新并构建后需要重启服务，让 CLI 与 scheduler 使用相同版本；旧二进制不能读取 v6 调度库。

## 给 lead 挂载能力

```bash
agt agent update lead --add-builtin-tool task_collaboration
```

这里的 `lead` 是 Agent 配置名；调用任务工具使用的是已绑定的 principal 身份。
该能力独立于 `resource_management`，两者可同时挂载。资源管理 Agent 创建 lead 时，
也可在 `resource_write(kind="agent")` 的 data 中设置 `builtin_tools: ["task_collaboration"]`。
接收任务的 Agent 只需已有有效绑定；不要求挂载任务协作或资源管理能力。
Native 模型需开启 tool_call；Codex 使用框架注入的 dynamicTools。

## lead 的调用流程

1. 用 `principal_list` 确认执行者身份，必要时由有资源管理权限的 Agent 创建并绑定。
2. 用 `direct_create(principal="codex")` 找到自己与执行者的私聊；也可使用双方都在的群聊。
3. 调用 `task_request`。工具名在模型侧带 `framework_` 前缀。

```json
{
  "assignee": "codex",
  "conversation": "双方共享会话的 ID",
  "input": "实现登录表单校验，并运行相关测试，完成后给出变更与验证结果。",
  "request_key": "login-validation-001"
}
```

返回 `id` 和 `state: queued` 表示请求已持久化，不代表已经开始或完成。
同一请求者重用相同 request_key 和参数会返回原任务；不同参数复用该 key 会拒绝。
不要再用 message_send 重发同一工作请求。任务输入 JSON 编码后最多 120 KiB。

创建请求继承当前已授权的 discussion，不额外获得模型额度或 MCP 权限。
执行者停用/manual 时保持排队；暂停或过期讨论不会启动新执行。
执行完成后结果定向通知 lead，失败和结果不确定也会通知。lead 可以在收到结果后，
用 message_send 向通知注明的原会话汇报。普通进度不会广播成群聊消息。

这还不是 await：lead 当前 Run 正常结束后释放执行槽，结果到达会创建后续通知 Run。
优先级、暂停具体任务并恢复原检查点等能力将在后续阶段实现。

## 查询与排错

```bash
agt tool call --principal lead task_list --args '{"limit":20}'
agt tool call --principal lead task_get --args '{"task_id":"任务 ID"}'
```

CLI 和 stdio MCP 固定其启动身份，只允许查询本人发出或收到且当前仍有会话访问权的任务。
不能通过换一个 task_id 查看别人的任务；Agent 内置调用还须通过当前执行租约围栏。

关注以下字段：

| 字段 | 含义 |
| --- | --- |
| state | queued / running / succeeded / failed / canceled / outcome_unknown |
| blocked_reason | 排队任务的停用、manual 或讨论授权阻塞原因 |
| run_id | 开始执行后的 Run，可使用 `agt run info <ID>` 检查 |
| publication | not_ready / pending / published / blocked |
| publication_error | 发布失败的安全错误代码，原结果继续保留 |
| requester_notification | pending / handled；查询和消息已送达均不代表模型已经处理 |
| notification_blocked_reason | 请求者目前不能被自动唤醒的原因 |

outcome_unknown 表示需要检查实际执行与副作用，本版不会自动重跑。
会话 /new、结束讨论、解散/退出引起权限或上下文边界变化时，旧结果不会自动进入新上下文。
未完成发布保留在账本中；当前没有单任务取消/重试或更换汇报目的地的工具。

本地人类管理员也可发起任务，但须先发送本人消息并显式 `schedule start` 授权，
再通过 `agt tool call --principal <本人> --manager task_request` 传入对应的 `source_message`。
外部 stdio MCP 的 Agent 身份没有当前 scheduler 租约，不能仅靠启动 MCP 获得付费执行授权；
应使用已托管 Agent 的 `task_collaboration` 能力发起工作。
