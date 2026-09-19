# 运行策略、Agent Memory 与本机执行

本阶段将运行默认值集中到 AGT home 的 `config.yaml`。配置只由本机管理员修改，不作为资源管理 Agent 的可写资源。

## 配置

- `agt config describe`：中文用途、默认值、单位和生效时机。
- `agt config show`：工作空间生效默认值；命令行的临时覆盖不在此输出中。
- `agt config apply ./config.yaml`：校验后原子替换配置；未知字段和非法值直接拒绝。
- `--home` 与服务保持一致。Web 启动的 scheduler 读取同一个配置。

省略字段采用默认值；支持不限的字段用 YAML `null` 表示，`0` 无效。
配置文件是整份替换，修改前先读取现有配置，不要把部分配置误当补丁。

```yaml
scheduler:
  concurrency: 8
  parallel_work: false
  replies_per_agent: null
  discussion_seconds: null
  model_steps: null
memory:
  enabled: true
execution:
  enabled: false
  timeout_seconds: 300
  output_bytes: 65536
```

示例适合较长的多 Agent 工作：取消讨论回复额度和授权到期，但保留容量控制。
默认仍为并发 3、每身份每讨论回复额度 100、讨论授权 1800 秒；单次原生 Run 模型步数默认不限。
`discussion_seconds` 是授权期限，不是空闲检测。有限期限到期会撤销在途授权；长任务应在授权前设为不限。
已有讨论的到期时间不会因改配置自动延长，需要显式继续/续期。配置不改变 HTTP 重试次数或 Codex 资源自己的超时。

调度容量及并行开关需重启调度器；回复额度由其加载的策略控制，新请求检查也会读取配置，建议统一重启服务避免新旧策略混用。
`--concurrency` 临时覆盖全局容量。不限容量仍不绕过身份/资源冲突、权限和 Harness 租约。

## Memory 的两层数据

原始聊天仍由 IM 保存，Memory 不复制正文。`memory_search(kind=messages)` 查询当前身份可见历史，
按关键词、会话、日期筛选；默认不进行模型推理，不依赖向量服务。
`memory_save` 保存 Agent 私有的主题摘要，必须关联 1..32 条可见原消息。
`memory_search(kind=notes)` 按主题/摘要搜索；`memory_read` 返回摘要和有界的原文片段。

示例：

```json
{"query":"麻将","kind":"messages","day":"2026-09-19","limit":20}
```

`day` 按 UTC 日期解释。按用户本地日期查找时，用 `since`（含）和 `until`（不含）的 Unix 毫秒范围。
摘要的日期筛选基于来源消息日期，而不是 Agent 保存摘要的时间。
分页将 `next_cursor` 作为下次 `before`，保持其他条件不变；扫描有界，空页但 cursor 非空时应继续。
这也是无需预先生成摘要就可查旧对话的入口。完整会话用已有 `message_history` 读取。

```json
{"id":"mahjong-rules","topic":"牌局约定","summary":"采用约定的赤牌规则，四人各 25000 点。","source_messages":["原消息 ID"],"expected_revision":0}
```

`id` 是身份内稳定键；0 创建，修改/删除须带查询返回的 revision，防止并发覆盖。
`memory_forget` 删除摘要，不删除聊天；`/new` 重置工作上下文，不删除长期摘要或旧消息。
摘要内容由 Agent 主动写入，不自动推测主题或无声调用模型。模型通过工具说明获知何时应检索和保存。
摘要不能替代牌局、资金或任务账本等权威状态，也不作为更高优先级指令。

所有工具固定当前 principal。每次召回都重新校验来源可见性；任一来源被删除/不可见时不返回该摘要。
退群前自己可见的历史仍按 IM 的历史可见区间保留，离开期间的消息不可见；隐藏入口不删除记忆。
Agent 身份删除后禁止调用。没有“用参数指定另一个 Agent 并读取其私有摘要”的入口。
跨会话可访问不代表应向当前群公开；不要在公开回复中暴露私有手牌、秘密或凭据。

模块边界：`agt-service/memory` 负责固定身份工具和 `memory/memory.db` 的摘要存储，IM 负责消息权限；
runtime 的 `ContextStore` 继续只负责上下文压缩。原生/Codex 托管 Agent、CLI/MCP 共用工具入口。

## 本机执行

原生 Agent 需要同时满足：管理员配置 `execution.enabled: true`，Agent 挂载 `local_execution`。
工具 `local_exec(command, directory?)` 运行 `/bin/bash --noprofile --norc -c`，返回退出码、耗时和有界 stdout/stderr。
默认工作目录来自运行设置；可显式提供已存在的目录。工具与模型调用分离，命令执行期间不消耗重复查询的模型请求。

这是宿主机执行权限，不是目录沙箱；命令拥有服务运行账号的权限。默认关闭，不由 resource_management 隐式开启。
不支持交互输入和脱离管理的后台服务；长任务作为显式任务委派，使用现有任务取消/等待机制。
命令期间轮询的是本地进程状态，每 100ms 检查取消/授权；不是轮询模型服务。
超时或正常取消终止进程组，输出管道不阻止取消返回；外部副作用不会回滚。
SIGKILL 无法执行析构清理，不能保证宿主机命令已停止，仍须按结果不确定处理并检查实际进程和文件。
公开横条只显示“执行本机命令”，不暴露命令、路径或输出。Codex 继续使用自己的命令能力与沙箱，本工具不绕过其限制。

## 模型安排并行，runtime 执行互斥

`scheduler.parallel_work: true` 允许显式工作声明并行。普通聊天和通知仍保留身份串行保护。
任务请求可在创建时携带：

```json
{"execution_policy":{"parallel":true,"exclusive_resources":["repo:frontend"]}}
```

`task_execution_policy` 可修改本人请求/负责的排队任务，需最新 revision 和 reason；已领取任务不能修改。
只有同身份两个任务都选择 parallel，且全局开关允许，才可并行。资源键重叠时跨身份也互斥。
键是逻辑约定、精确匹配，不是自动推断文件依赖或 OS 文件锁；操作相同共享状态必须使用相同键。
未声明资源的外部副作用不受这类锁保护。默认 serial 不会自动锁住其他身份。

检查与领取在 `BEGIN IMMEDIATE` 同一事务中，竞争者不能同时通过。任务结束或安全挂起后释放活动占用；
结果不确定的任务保留声明资源，核对后才释放。继续任务重新领取，不假定等待期间状态未变化。
任务查询返回 `execution_policy` 和 `identity_busy/resource_busy/resource_outcome_unknown` 原因。
依赖顺序仍用 `task_defer/task_await`，现有环检测保留；不是让两个模型先并行修改后再决定是否冲突。
Harness 会话的独立执行租约、工作 revision 与停止/重置围栏均保留。

调度库升级为 v16；请统一升级 CLI、Web 和 scheduler，不要让旧二进制继续访问新库。
