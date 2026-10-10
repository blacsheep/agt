# 快速上手与验收

最近整理：2026-10-10。界面操作见 [Web 指南](../../web/README.md)，
并发和恢复设计见[调度器](../architecture/scheduler.md)，命令参考见 [CLI 参考](cli.md)。

## 当前架构

资源定义仍由 Provider / Model / Prompt / Skill / MCP / Agent 各库管理。
Agent 挂载 Model、Prompt、Skill、MCP，Model 引用 Provider。
`agt-service` 提供统一工具服务、托管绑定及权限；CLI 和 stdio MCP 复用它。
`agt-im` 只负责会话、消息及事务性收件人投递；`agt-runtime` 的持久化调度器
消费已授权讨论的投递、执行模型并发布结果。自动消息执行统一使用 scheduler。
Agent 可以自己创建群聊、联系其他 Agent，
不要求父子 Run 关系。原有 `agent run --delegate` 仍可使用。

广播与回复是两回事：每条消息为发送时的其他当前成员创建投递记录；
是否推理由接收者绑定和响应策略决定，已读游标不代表已执行。
新加入成员不补发历史执行任务，发送者不接收自己的投递。

## 自动化验收（无需真实 API Key）

```bash
cargo test --workspace --offline
cargo test -p agt-cli --test runtime_cli mcp_manager_provisions_agents_and_group_then_scheduler_broadcasts_replies --offline
cargo test -p agt-runtime hosted_agent_creates_group_and_contacts_peer_without_parent_child_delegation --offline
cargo test -p agt-runtime scheduler --offline
cargo test -p agt-runtime schedule_store --offline
cargo build -p agt-cli --offline
node --test web/test/*.test.mjs
```

第一条覆盖所有回归。第二条通过真实 stdio MCP 客户端配置资源、创建并绑定
两个 Agent、建群发消息，授权讨论后由 CLI scheduler 调用本地 HTTP 模型桩回复。
第三条验证托管 Agent 自己使用通信工具建群、联系同伴，没有父子委派关系。
另外覆盖重复发送、调度器互斥、发布拦截、权限与从 0 到 10 的连续接话。
调度测试验证并行判断、自主回复互斥、消息独立入队、同 principal 串行、等待恢复、100 次回复计数重置与结束拦截。
Web 测试需要 Node.js 22+。离线构建要求依赖已缓存。

## 手动连接模型

以下命令从仓库根目录执行，示例 home 应选择尚不存在的专用目录。
先在启动进程的环境中设置 `MODEL_API_KEY`；不要把密钥写进工具参数。

```bash
cargo build -p agt-cli
target/debug/agt init /tmp/agt-framework-demo
target/debug/agt --home /tmp/agt-framework-demo im principal create owner --name Owner
target/debug/agt --home /tmp/agt-framework-demo provider create provider --base-url https://YOUR_PROVIDER/v1 --api-key-env MODEL_API_KEY
target/debug/agt --home /tmp/agt-framework-demo model create default-model --provider provider --model YOUR_MODEL_NAME --tool-call true
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner --manager resource_write --args '{"kind":"prompt","reference":"review","create":true,"data":{"text":"你是审查者；无需发言时输出 [[silent]]。"}}'
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner --manager resource_write --args '{"kind":"agent","reference":"reviewer","create":true,"data":{"model":"default-model","prompt":"review","skills":[],"mcps":[]}}'
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner --manager principal_create --args '{"id":"reviewer","name":"Reviewer","kind":"agent"}'
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner --manager agent_bind --args '{"principal":"reviewer","agent":"reviewer","working_directory":"/tmp/agt-framework-demo","policy":"all"}'
```

按同样方法创建 `planner` Agent、principal 和 binding（可复用模型和 Prompt）。
资源定义与 IM 身份分离：仅创建 Agent 定义不会启动自动回复，必须显式绑定。

```bash
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner direct_create --args '{"principal":"reviewer"}'
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner group_create --args '{"title":"方案讨论","members":["reviewer","planner"]}'
```

用返回的私聊或群聊 ID 替换下面的 `CONVERSATION_ID`：

```bash
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner message_send --args '{"conversation":"CONVERSATION_ID","body":"请讨论这个项目的改进方案。","request_key":"demo-1","mentions":["reviewer","planner"]}'
target/debug/agt --home /tmp/agt-framework-demo schedule start CONVERSATION_ID --message MESSAGE_ID
target/debug/agt --home /tmp/agt-framework-demo schedule run --watch --concurrency 3
```

将发送结果的消息 ID 填入上面的 `MESSAGE_ID`。授权本身不调用模型；运行 watch
才开始执行并可能产生费用。另一个终端查询历史与状态：

```bash
target/debug/agt --home /tmp/agt-framework-demo --output json schedule status
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner message_history --args '{"conversation":"CONVERSATION_ID","limit":50}'
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner message_status --args '{"message_id":"MESSAGE_ID"}'
```

重复使用相同 `request_key` 必须保持内容完全一致，否则拒绝。
不带 `--message` 的 `schedule start` 会授权会话待处理用户消息并继续未结束讨论。
`schedule run` 不带 `--watch` 只处理当前可运行任务，不负责长期提醒。

控制讨论（以下 `agt` 代表已构建的 CLI，使用同一 home）：

```bash
agt --home /tmp/agt-framework-demo schedule pause CONVERSATION_ID
agt --home /tmp/agt-framework-demo schedule resume CONVERSATION_ID
agt --home /tmp/agt-framework-demo schedule end CONVERSATION_ID
agt --home /tmp/agt-framework-demo schedule delete CONVERSATION_ID --actor owner
agt --home /tmp/agt-framework-demo im conversation reactivate CONVERSATION_ID --actor owner
```

暂停让当前回复完成；结束取消后续回复和提醒。已结束轮次不能 resume，请发新消息。
删除保留历史并检查权限；恢复会话不会复活已结束讨论。

## 提供给大模型的 MCP 工具

在支持 stdio MCP 的客户端中配置（替换绝对路径）：

```json
{"mcpServers":{"agt":{"command":"/ABSOLUTE/PROJECT/target/debug/agt","args":["--home","/tmp/agt-framework-demo","serve","--principal","owner","--manager"]}}}
```

管理会话可通过 `resource_list/read/write/delete` 管理资源，通过
`principal_create`、`agent_bind/unbind` 管理托管身份。删除先返回计划，
再次提交完整 `confirm` 列表才执行；被绑定的 Agent 必须先解绑。
Category 管理仍使用原 CLI。使用 `tool list --principal owner --manager`
查看完整工具参数。Provider 工具在服务端读取 `api_key_env` 指定的环境变量。

去掉 `--manager` 即普通参与者：可建私聊/群聊、管理有权限的群成员、发送和
读取消息，但不能修改资源/绑定。身份由启动者固定，模型无法通过参数伪造发送者。
托管 Agent 获得同样的参与者工具（`framework_` 前缀），不会自动获得管理权限。
MCP 服务本身不启动调度器，也不授权消息；需通过 schedule 或 Web 开启讨论。

## 响应策略与安全边界

### 工具参数与诊断

`message_send.reply_to` 和 `message_history.before` 支持省略或 null，兼容
空串/纯空白值（按未提供处理）。非空回复 ID 必须存在于目标会话；非空游标
必须来自相同会话。`resource_delete.confirm` 的省略/null 只获取计划，空数组
不代表同意删除非空计划。普通布尔、数值、数组参数不把 null 当默认值。

CLI、MCP、托管 Agent 使用相同的结构化业务错误，例如：

```json
{"error":{"code":"invalid_arguments","message":"Reply target must exist in this conversation; omit/null for a new message.","field":"reply_to"}}
```

错误不会回显原始参数、存储错误或密钥。协议错误仍按 MCP JSON-RPC 返回；
业务错误仍为 `isError: true`。复杂资源配置无法安全分类时返回
`operation_rejected`，不是把所有底层错误文本直接交给模型。

Run 的 `completed` 只表示模型正常结束，不表示业务目标达成。检查
`tool_failures`（本 Run 的失败工具调用次数，纠正后也不清零，子 Run 单独统计），
并通过主机 CLI 查询调用记录：

```bash
agt run info RUN_ID
agt run calls RUN_ID --limit 50
# 下一页：把上一页最后一条 id 用作 --after
agt run calls RUN_ID --after LAST_CALL_ID --limit 50
```

记录包含模型步骤、工具名称、开始/结束时间、状态、安全错误，以及允许字段的
参数类型/长度/是否空白。不会默认保存参数原文、返回正文、模型推理、任意字段名
或外部 MCP 内容；外部工具只记录参数容器的形状。它不是完整会话录制。
现有 Run input/output 与 IM 消息仍按原机制存储，不能因此把整个数据库视为脱敏文件。

调用前持久化 `started`，返回后写入 `succeeded/failed/canceled`；中断留下的
`started` 表示结果不确定，不允许据此自动重试。记录供主机查看，不开放给参与者
工具。连接外部 MCP 的准备阶段失败仍记录为 Run 错误，不生成工具调用条目。
Runtime 数据库按模块记录结构版本，不兼容的旧版本会被拒绝，不在运行时迁移。

### 自动响应

下表描述用户消息的响应策略：

| policy | 私聊 | 群聊 |
| --- | --- | --- |
| mentioned（默认） | 回复 | 仅结构化 mentions 包含自己时回复 |
| all | 回复 | 回复每条符合条件的消息 |
| manual | 不自动回复 | 不自动回复 |

正文中的 `@名字` 不等同于 `mentions`。Agent 可用 `response_policy` 设置
自己在某个会话中的策略。精确输出 `[[silent]]` 不发布公开回复。
调度器不限制自动消息深度。绑定不提供 `max_depth`；
消息深度仅用于历史追踪，不参与响应判定。
模型步数上限只由 `config.yaml` 的 `scheduler.model_steps` 控制（默认不限）；绑定和请求里的 `max_steps` 仅兼容读取，不再约束执行。
未被点名的 Agent 消息在非 manual 策略下进入参与判断；显式 mentions 点名的
同伴消息绕过判断，成员资格与回复上限仍然生效。每条消息独立入队，没有固定的安静等待。
外部 MCP 按 Native Agent 的装配加载，无需绑定或讨论级额外开关；
未装配的工具不可调用，具体调用仍经过权限审批。务必只挂载可信配置。

这是可信本地主机框架，不是公网多租户服务。启动者能选择身份和 manager 权限；
底层 IM CLI 同样属于可信主机接口。资源摘要和工具错误不返回密钥，Provider
密钥仍按现有资源存储机制保存，不是专门的密钥保险库。

## 调度状态与排错

- `schedule status` 返回 online、discussions、tasks；没有 Run 的任务也可能在
  判断、等待回复名额、等待唤醒或因授权受阻，不要仅凭 Run 列表判断是否收到消息。
- `evaluating / ready_to_reply / responding / waiting` 表示参与判断、等待回复名额、正式回复、等待唤醒。
  同会话判断并行，自主回复依次开始；等待名额不占线程，但保留身份。每个 principal 最多开始 100 次回复，新的用户消息获准自动回复后计数归零。
  模型调用次数仅供诊断，不再限制整轮调用或判断次数。单次 Native Run 的步数上限见 `scheduler.model_steps`。
- 任务 `limited` 表示 100 次回复达限，发送新用户消息可重置；讨论超时可 resume 续期，但不重置回复计数。`blocked` 检查任务记录中的阻塞原因；`failed / interrupted`
  先检查 Run 与实际外部结果，不会自动重跑。Web debug_mode 展示更多诊断信息。
- 不接话时检查绑定、manual/mentions、讨论是否授权/暂停/结束。
  模型协议必须与 Provider 实际接口一致；支持文本 Chat Completions 和 Responses，
  不会自动探测切换。调度判断在 runtime.db 中记录，正式 Run 使用 `run calls`。

## 已知限制

- 调度默认 3 个任务并行（`scheduler.concurrency` 或 `--concurrency` 调整）；互斥单位是 principal，不是 Agent 定义。
  多个身份绑定同一定义可同时执行。
- 中断于模型/工具执行中的 Run 保持可见的 processing/running，不自动重放
  不确定副作用；目前没有强制恢复 API。应检查 `run` 与消息状态，人工判断后
  再发新请求。
  调度等待会持久化，租约过期后纯判断可重试，正式回复任务标记 interrupted。
- 归档/删除、触发消息删除、成员移除和解绑会阻止调度任务后续执行或发布。
  恢复会话/绑定不保证恢复已取消任务，应检查状态后发送新消息。
- 同一未结束讨论中新授权的用户输入会重置各身份回复计数；重复授权或 resume 不重置。
  跨会话联系继承原轮次的回复计数与控制，Agent 消息不能重置。结束后的新用户消息开启新讨论。
  消息批次正文上限 128 KiB，超过时明确失败；最近历史另保留最多 20 条/128 KiB。
- 已发出的外部 MCP 请求不保证撤回。资源文件、IM、Run、binding、scheduler
  是不同存储边界，不承诺跨库事务或外部副作用 exactly-once。
- 自动化使用模型桩；历史真实模型报告只验证当时版本，不代表新调度器已做真实验收。
