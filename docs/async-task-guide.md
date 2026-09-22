# 异步任务协作（后端首版）

普通消息仍是聊天；需要可靠排队、完成/失败回报时使用显式任务。
需要运行 `agt schedule run --watch`；Web 服务原有的 scheduler 子进程也会处理任务。
Web 会话标题栏的「任务」可查看和管理任务；也可通过 CLI/MCP 或已挂载的内置工具操作。
更新并构建后需要重启服务，让 CLI 与 scheduler 使用相同版本；旧二进制不能读取 v14 调度库、v5 IM 库、v5 Run 库及 v4 绑定库。

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
用 task_report 向通知注明的原会话提交持久汇报；message_send 不会消除汇报义务。普通进度不会广播成群聊消息。

不调用 await 时，lead 当前 Run 正常结束后释放执行槽，结果到达会创建后续通知 Run。
若 lead 正在执行一项显式任务，也可使用下面的检查点等待。

## 运行中的任务等待依赖

内置执行器与 Codex 的显式 WorkItem 均提供 `task_await`。
普通聊天、结果通知 Run、CLI/外部 MCP 不暴露此工具。Codex 登记等待后必须结束当前 turn，
只有协议确认正常完成才保存检查点；下一段使用同一 thread，任务 session 与普通聊天独立。
例如：人类先委派显式任务给 lead，lead 用 task_request 委派给 Codex，然后单独调用：

```json
{"task_id":"刚创建的子任务 ID","deadline":1893456000000}
```

deadline 使用实际的未来 Unix 毫秒时间。可用 task_id 等待一个请求，或 task_ids 数组一次等待 1..20 个本人发出、同一 discussion 的请求。
两者不能同时填写；默认 all_terminal，同一依赖只允许一个父任务等待。结果不确定会提前唤醒处理异常，
剩余任务仍继续执行并可通知。已完成的结果不会漏掉。
`task_await` 必须是该轮唯一工具调用，不能与文件写入等工具放进并行批次。

Runtime 保存模型上下文、已完成工具结果和发送去重信息，然后结束本段 Run 为 `suspended`。
执行返回后才释放该 Agent 的槽位；WorkItem 显示 `waiting_dependency`，其他任务仍可运行。
检查点留在 runs.db 私有表中，不通过 task_get 或结果通知返回原始上下文。
依赖成功、失败、取消、结果不确定或截止时间到达，会排队一个新的 continuation Run。
新 Run 读取检查点和明确的依赖状态；用 task_get 查看完整结果，之前成功的工具不会自动重放。
父任务直到真正返回最终结果才对上游汇报成功；“正在等待”不是任务完成。

等待超时不会取消子任务，也不会伪造成功，迟到结果仍可通知 lead。
已由 continuation 接收的结果会标记 handled，不再额外创建同一结果的通知 Run。
handled 在这里表示已持久交接给父任务，尚不代表父任务已经完成或用户已收到汇报。
暂停、manual、停用、预算仍限制恢复；停止、/new 或授权失效会阻止旧任务恢复。
等待期间若变更模型、工具、工作目录或执行引擎，恢复会失败并留存诊断，不从头重做外部操作。
服务重启可恢复已提交检查点；只有等待意图而没有检查点的中断执行仍按 outcome_unknown 处理。


## 排序和延后尚未开始的任务

给负责队列的 lead 挂载 `task_collaboration` 后，它可在当前托管运行中：

1. `task_list({"queued_only":true,"limit":20})` 查看待办，用 task_get 读取最新 revision。
2. `task_reprioritize` 调整自己负责、同一 discussion 中的排队任务。
3. `task_defer` 指定执行时间，或明确要求 A 在 B 得出结果后执行。

```json
{
  "task_id": "任务 B 的 ID",
  "expected_revision": 1,
  "priority": "high",
  "reason": "先完成阻塞发布的修复"
}
```

上面是 task_reprioritize 的参数。task_defer 的参数例如：

```json
{
  "task_id": "任务 A 的 ID",
  "expected_revision": 1,
  "after_task": "任务 B 的 ID",
  "deadline": 1893456000000,
  "reason": "等待 B 完成后处理 A"
}
```

时间使用 Unix 毫秒，示例 deadline 必须替换为实际截止时间。
也可用 `not_before` 代替 `after_task`/`deadline`，表示最早执行时间。
两种模式不能同时使用。修改失败提示旧 revision 时，重新读取任务再做决定。

普通 Agent 只可设置 low/normal/high，不能授予或改写人类设置的 urgent。
任务每等待五分钟提升一个有效档位，最高 high；同档按最初入队顺序执行。
排序不抢占正在执行的工作，已被调度器领取的任务也不能改成等待。

A 和 B 必须属于同一执行身份及授权 discussion，调用者必须能访问两项任务。
B 成功、失败或取消均可解除屏障，结果不确定则继续阻塞。
若截止前没有结果，`blocked_reason` 显示 `dependency_deadline_exceeded`，
框架会发出一次定向超时通知，需要重新决定等待时间或条件；不会直接忽略 B。人工停止和 /new 仍优先于等待条件。

请求者和 lead 查询 A 会看到相同的 `deferred`（原因、时间、目标任务）及 `waiting_for_task`。
这只控制 A 的排队，不会冻结请求者 Agent。挂载任务协作的 Native lead 有至少两个同讨论的新待办时，会先自动分诊，最多读取 20 条摘要，
只允许提交队列决定，不开放项目工具。每批只调用一次；失败保留原优先级/FIFO，记录 framework_fallback。
Codex 不运行这类 Native 模型分诊，可通过 task_list/排序工具管理队列。队列更新不会逐条触发额外模型回复。
同身份连续处理四次结果通知后，会优先给可运行工作一次 admission 机会。

## 查询与排错

```bash
agt tool call --principal lead task_list --args '{"limit":20,"queued_only":true}'
agt tool call --principal lead task_get --args '{"task_id":"任务 ID"}'
```

CLI 和 stdio MCP 固定其启动身份，只允许查询本人发出或收到且当前仍有会话访问权的任务。
不能通过换一个 task_id 查看别人的任务；Agent 内置调用还须通过当前执行租约围栏。

关注以下字段：

| 字段 | 含义 |
| --- | --- |
| state | queued / running / waiting_dependency / waiting_until / waiting_input / succeeded / failed / canceled / outcome_unknown |
| blocked_reason | 停用、manual、授权变化、deferred_until、waiting_for_task、dependency_outcome_unknown 或 dependency_deadline_exceeded |
| triage | 分诊时间、来源（model/framework_fallback）及公开理由 |
| priority / priority_reason | 明确设置的优先级与原因（不含自动老化） |
| deferred / waiting_for_task | 双方共享的排队等待条件 |
| run_id / resume_from | 当前 Run / 恢复所依据的前段 Run；可使用 `agt run info <ID>` 检查 |
| publication | not_ready / pending / published / blocked |
| publication_error | 发布失败的安全错误代码，原结果继续保留 |
| requester_notification | pending / handled；查询和消息已送达均不代表模型已经处理 |
| notification_blocked_reason | 请求者目前不能被自动唤醒的原因 |

outcome_unknown 表示需要检查实际执行与副作用，本版不会自动重跑。
会话 /new、结束讨论、解散/退出引起权限或上下文边界变化时，旧结果不会自动进入新上下文。
未完成发布保留在账本中；支持下述单任务取消、核对和重试；更换汇报目的地的工具暂未开放。

本地人类管理员也可发起任务，但须先发送本人消息并显式 `schedule start` 授权，
再通过 `agt tool call --principal <本人> --manager task_request` 传入对应的 `source_message`。
外部 stdio MCP 的 Agent 身份没有当前 scheduler 租约，不能仅靠启动 MCP 获得付费执行授权；
应使用已托管 Agent 的 `task_collaboration` 能力发起工作。

## 任务取消、核对与重试

- `task_cancel({task_id, expected_revision, reason})`：取消未运行/等待中的任务，运行中的任务先登记取消请求。
  框架停止后若仍无法确认外部副作用，保留 outcome_unknown；不把断线当作取消成功。
  任务内 task_request 创建的子任务记录 parent_id，取消父任务会联动其独占后代。
- `task_resolve({task_id, expected_revision, outcome, evidence})`：仅人类 Manager 可对 outcome_unknown
  记录 succeeded/failed 核对结论及依据。原始状态保留，resolution 单独保存并定向通知请求者。
- `task_retry({task_id, expected_revision, request_key, reason})`：从已失败/取消或核对为失败的任务创建新任务，
  保留 retry_of；重复 request_key 幂等。结果未核对时禁止重试。讨论授权必须仍有效。

调用前使用 task_get 获取最新 revision。托管请求者/执行者可取消；重试只限请求者或人类 Manager。
本地未托管 Agent 不可通过 CLI/MCP 绕过执行授权。人类 Manager 只能管理自己当前仍为成员的共享会话任务。

## 主动让出与业务答复

显式任务可单独调用 `task_yield({not_before, reason})`，保存检查点并释放执行槽，
到达指定的未来 Unix 毫秒时间后恢复；其他任务可在等待期间执行。

需要上游补充业务信息时，单独调用 `task_input({question, deadline})`。
任务进入 waiting_input 后，问题定向通知请求者；即使请求者正在等待该子任务，也不会吞掉问题通知。
请求者的托管运行或当前共享会话的人类 Manager 可使用：

```json
{"task_id":"任务 ID","input_request_id":"waiting_input 中的 run_id","expected_revision":4,"answer":"使用 feature/login 分支","request_key":"answer-login-1"}
```

以上为 `task_reply` 参数，调用前用 task_get 读取最新状态。相同 key 和答复可幂等重放；
过期、冲突或旧版本答复被拒绝。答复到达后从原检查点恢复；超时则携带 input_timeout 恢复，
由执行者决定后续处理，不伪造答复。这是业务信息交换，不授予工具、沙箱或系统权限。
Native 与 Codex 均支持；Codex 仍须正常结束当前 turn 才能安全挂起。

## 本机事件订阅

`agt events watch --principal <身份> [--conversation <会话>] [--after <游标>]`
输出持续 JSONL，初次 ready 后加载快照，再按 cursor 重连。事件只是有权限的变化提示，
正文仍从现有查询接口取得。游标绑定身份和会话范围；resync_required 表示须重新加载快照。
Feed 保留 7 天，源变化在投影和消费游标同事务提交后清理；不会删除任务收件箱、结果或待发布记录。
此命令不发起模型请求；本机通知丢失时每 15 秒补偿检查。

## Web 任务面板与执行进度

会话标题栏点「任务」，默认显示当前会话的进行中任务。可切换全部可见任务、需要处理或已结束，
按任务文本/执行者搜索，加载更早记录。详情区分执行状态、结果投递和请求者处理状态。
操作使用详情中的最新版本，冲突后刷新重试；答复/核对/取消和重试均在面板内填写。
Agent 会话观察模式只读。Codex 公开工具活动不包含命令原文或输出，三个时间分别代表本地检查、
协议接收和有效工作进展；本地检查时间更新不代表模型有新进展。

## 原会话汇报与异常处理

结果送达 lead 后，task_get 的 `report.state` 独立显示 awaiting_report/pending/published/blocked。
lead 应调用 `task_report({task_id, expected_revision, body})` 向固定的原始会话汇报；不要用普通
message_send 代替这个可追踪流程。相同正文重复提交幂等，已提交正文不能覆盖。
停止、重置、退群后不会绕过原上下文授权发布，受阻内容保留以供核查。

Web 任务详情可填写汇报；「工作空间异常」仅供启动时指定的工作空间负责人查看。
五分钟未汇报会产生一次提醒；私有任务仅显示身份和异常类型，无权限时不能展开正文。
可用 `task_attention` 从 CLI/MCP 查询同一异常清单，不能通过 Agent 的 Manager 标记冒充人类负责人。

## Harness 执行状态与恢复

会话显示 preparing、running、canceling、reconciling、succeeded、failed、canceled 或 needs_review。
失败或取消得到确认后允许继续使用会话；断线不等于取消成功。
执行权通过带归属和代际编号的租约维护，进程强杀后会回收过期租约，不会自动重跑旧任务。
可能已提交但结果无法确认时显示「需要处理」，保留原线程以供检查。

在 `/status` 面板可刷新状态。确认旧执行停止并检查产出后，可点击「开始新上下文」或使用 `/new`，
聊天记录保留。独立任务的结果待核对状态仍应在任务面板人工核对，不通过修改数据库绕过保护。
完整状态和能力边界见 [Harness 执行状态与租约](harness-execution-lifecycle.md)。

## 上下文重置分隔标记

执行 `/new`、`/clear` 或「重置上下文」后，消息时间线显示持久的「新上下文」分隔线及时间。
标记不作为聊天消息发送，不触发 Agent 回复；刷新、重新打开或加载历史仍可显示。
分隔线表示模型工作上下文重置，不代表聊天记录或长期记忆被删除。
升级前没有保存重置时间线，无法补出此前每次重置的准确时间；新标记从升级后开始记录。


### 沉默与工作处理

普通消息执行先检查是否需要处理请求结果、登记状态或推进流程，最后再决定是否发布正文。
`[[silent]]` 仅表示没有自动发布的正文；它不证明业务完成，也不会登记等待。
普通聊天没有任务关联，框架不会凭自然语言推断并创建任务。需可靠追踪结果的协作应使用
`task_request`；显式工作任务可通过 `task_await` 登记依赖等待。

结果通知的输入包含仅属于当前请求者及通知事件的 `report_obligation`。
执行结束时读取持久化汇报记录：未提交报告的通知标为处理失败，原因 `task_report_required`，
但该次 Run 仍可为正常完成，原工作任务的执行结果也不改变。通知待办和汇报义务保留，
不会自动重跑模型。普通正文或 `message_send` 都不能代替 `task_report`；
报告提交后返回 `[[silent]]` 可以正常完成通知处理。报告发布受阻仍由独立发布状态追踪。
