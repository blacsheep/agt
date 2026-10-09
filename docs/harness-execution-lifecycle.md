# Harness 执行状态与租约

本模块将执行结果和并发执行权分开。`harness_sessions` 保存配置、引擎线程 ID 和已接收输入进度；
`harness_operations` 保存每个会话最近一次操作的状态和租约。Run 继续承担模型执行审计，任务队列继续管理依赖和汇报。
操作的私有输出不通过会话查询、SSE 或 Web 状态接口返回。

## 状态

| 状态 | 含义 | 能否开始下一操作 |
|---|---|---|
| preparing | 连接、准备和提交过程；submitted 标记是否进入可能已发送的边界 | 否 |
| running | 引擎已接受 | 否 |
| canceling | 请求取消，等待确认 | 否 |
| reconciling | 执行者租约过期，检查持久状态 | 否 |
| succeeded | 结果已保存 | 是，原持有者已释放时 |
| failed | 提交前失败、明确拒绝或引擎确认失败 | 是，原持有者已释放时 |
| canceled | 提交前取消，或请求停止后引擎确认中断 | 是，原持有者已释放时 |
| interrupted | 引擎主动确认中断，未收到本地停止请求 | 是，原持有者已释放时 |
| needs_review | 已提交但无法确认结果，或未能可靠保存结果 | 否；先检查原线程及副作用 |

失败和取消不回滚项目文件，不自动重做旧任务。界面 busy 是状态与持有者的派生值，不是另一套持久状态。
配置和重命名也使用同一执行权，避免与模型执行并发修改会话快照。

## 获取、续租和隔离

原子获取执行权，分配 UUID 执行归属和递增 generation。租期 30 秒，独立线程每 5 秒续租，
因此长工具调用不会仅因为没有流式消息而失去执行权。续租不得复活过期租约。
提交请求之前先保存可能已提交的标记，避免发送成功但标记尚未写入的窗口。
会话写入和释放均核对 owner、generation 和 expires。过期持有者的写入、释放均被拒绝。
运行进程仍须尊重 scheduler 的授权；租约不提供新的资源或工具权限。

## 正常完成、失败与取消

正常完成先保存会话输入进度和结果，再释放执行权。失败或中断得到确认后，先记录引擎已收到的输入，
再结束操作，避免新请求再次注入同一批历史。明确拒绝不推进输入进度。

取消不是终态：发送 turn/interrupt 后，最多等待 2 秒匹配原 thread/turn 的终态通知。
RPC 应答本身不算中断确认；无关 turn 的通知不能解除执行保护。确认中断后释放执行权，允许继续会话。
若取消前已经完成且成功取得结果，保留完成结果；讨论停止后的公开发布仍受 scheduler 授权限制。
压缩同样区分请求接受、终态和取消；只有得到可识别的 turn 才能发送定向中断。

## 进程强杀与恢复边界

SIGKILL 不运行析构函数。过期租约由 scheduler 的修复扫描、会话查询或下次获取执行权时回收：

- 未进入提交边界：结束为 failed，可以继续。
- 已持久保存明确失败/取消：保留终态，移除失效持有者。
- 可能已提交且无可靠终态：经过 reconciling 转为 needs_review，不自动发送新请求。

当前 Codex 适配器没有实现跨进程重新附着活跃 turn，因此不能把失联解释为远端已停止，
也不能从不存在的连接中自动取回完成结果。needs_review 时保留原线程 ID，用户应检查 Codex 线程和项目产出，
确认旧执行停止后通过现有 /new 开始新上下文。旧执行仍可能留下外部副作用。
这释放的是无主的执行租约，不是绕过未确认结果的业务保护。

旧 dirty=1 数据升级为 needs_review，旧配置、线程 ID 和进度保留，不自动清理或重放。
状态变化进入已有权限隔离事件源；续租不会产生进度事件或被解释成模型工作进展。

## 验证

`crates/agt-runtime/src/scheduler/tests/harness_session_lock_tests.rs` 集中覆盖启动失败、明确拒绝、失败后继续、
取消确认、压缩取消、真实子进程强杀、租约到期、旧归属隔离和旧数据迁移。
协议桩位于 `crates/agt-harness/tests/fixtures/codex.py`，不调用真实模型。

## 任务终止诊断

讨论期限只限制新的 admission（包括子任务、恢复片段和通知模型调用）。在途执行继续；
自然到期不能撤销其运行权限、调用计数或结果投递。停止、上下文重置、权限变化仍撤销执行权限。
对账单独检查执行归属，允许仍持有有效租约的执行者完成停止确认。

Run 与 Work 使用 succeeded/failed/canceled/interrupted/outcome_unknown 对应的明确终态
（Run 成功名为 completed）。超时且确认中断为 failed；主动取消且确认停止为 canceled；
引擎自行中断为 interrupted；提交后断线、失去归属或缺失可保存结果为 outcome_unknown。
取消请求期间 execution_status=canceling；该值由取消意图与运行事实派生，不新增独立任务状态机。

termination 保存 code/source/observed_at/interrupt_requested_at/confirmed_at/confirmation/side_effects/actor。
首个原因保留，不用清理错误覆盖；副作用可能存在与停止是否确认分开表达。task_get 提供安全诊断
和 run/thread/turn/session 标识，不携带进程 stderr。主机 run info 的 execution.diagnostics
可查看子进程退出码、信号和最多 8 KiB 的 stderr 尾部；常见凭据行被过滤，仍按主机私有日志处理。
已有已完成结果不会因迟到取消丢失，但向失效上下文的发布仍受权限检查。

验证位于 tests/scheduling/expiry.rs、tests/harness/termination.rs 和 session_lock.rs。
