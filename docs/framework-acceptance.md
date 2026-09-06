# 资源与多 Agent 通信框架验收

## 当前架构

资源定义仍由 Provider / Model / Prompt / Skill / MCP / Agent 各库管理。
Agent 挂载 Model、Prompt、Skill、MCP，Model 引用 Provider。
`agt-service` 提供统一工具服务、托管绑定及权限；CLI 和 stdio MCP 复用它。
`agt-im` 只负责会话、消息及事务性收件人投递；`agt-runtime` 的 worker
消费投递、执行模型并发布结果。Agent 可以自己创建群聊、联系其他 Agent，
不要求父子 Run 关系。原有 `agent run --delegate` 仍可使用。

广播与回复是两回事：每条消息为发送时的其他当前成员创建投递记录；
是否推理由接收者绑定和响应策略决定，已读游标不代表已执行。
新加入成员不补发历史执行任务，发送者不接收自己的投递。

## 自动化验收（无需真实 API Key）

```bash
cargo test --workspace --offline
cargo test -p agt-cli --test runtime_cli mcp_manager_provisions_agents_and_group_then_worker_broadcasts_replies --offline
cargo test -p agt-runtime hosted_agent_creates_group_and_contacts_peer_without_parent_child_delegation --offline
```

第一条覆盖所有回归。第二条通过真实 stdio MCP 客户端配置资源、创建并绑定
两个 Agent、建群发消息，再由 CLI worker 调用本地 HTTP 模型桩回复。
第三条验证托管 Agent 自己使用通信工具建群、联系同伴，没有父子委派关系。
另外覆盖重复发送、并发 worker 单次推理、发布重试、失败隔离、权限和深度限制。

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
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner --manager agent_bind --args '{"principal":"reviewer","agent":"reviewer","working_directory":"/tmp/agt-framework-demo","policy":"all","max_steps":8,"max_depth":2,"allow_mcp":false}'
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
target/debug/agt --home /tmp/agt-framework-demo worker --watch
```

另一个终端查询历史；用发送结果的消息 ID 查询每个收件人的处理状态：

```bash
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner message_history --args '{"conversation":"CONVERSATION_ID","limit":50}'
target/debug/agt --home /tmp/agt-framework-demo tool call --principal owner message_status --args '{"message_id":"MESSAGE_ID"}'
```

`worker` 不加 `--watch` 只跑一批；一次不保证清空新产生的后续消息。
重复使用相同 `request_key` 必须保持内容完全一致，否则拒绝。

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
MCP 服务本身不启动 worker，需另行运行。

## 响应策略与安全边界

| policy | 私聊 | 群聊 |
| --- | --- | --- |
| mentioned（默认） | 回复 | 仅结构化 mentions 包含自己时回复 |
| all | 回复 | 回复每条符合条件的消息 |
| manual | 不自动回复 | 不自动回复 |

正文中的 `@名字` 不等同于 `mentions`。Agent 可用 `response_policy` 设置
自己在某个会话中的策略。精确输出 `[[silent]]` 不发布公开回复。
`max_depth` 默认 1，只回应外部发起消息；需要 Agent 之间接续沟通时设为
2 或更大（上限 8）。同时每个 Run 受 `max_steps` 限制。
挂载外部 MCP 的执行需要 binding 的 `allow_mcp` 与 worker 的 `--allow-mcp`
同时开启。外部工具可能执行命令或访问网络，务必只挂载可信配置。

这是可信本地主机框架，不是公网多租户服务。启动者能选择身份和 manager 权限；
底层 IM CLI 同样属于可信主机接口。资源摘要和工具错误不返回密钥，Provider
密钥仍按现有资源存储机制保存，不是专门的密钥保险库。

## 已知限制与下一阶段

- worker 在单进程内同步执行；多个 worker 不保证同一会话不同消息严格串行。
- 中断于模型/工具执行中的 Run 保持可见的 processing/running，不自动重放
  不确定副作用；目前没有强制恢复 API。应检查 `run` 与消息状态，人工判断后
  再发新请求。已完成 Run 的发布可重试而不重新推理，前提是本地资源和目录仍可用。
- 归档会话暂停投递，重新激活后继续；已删除触发消息跳过。修改绑定不会取消
  已经运行中的调用。
- 自动回复深度与单 Run 步数不是全群 token 预算；大量成员和 all 策略仍可能
  产生较多调用。公平调度、全局预算、显式中断恢复是后续重点。
- 资源文件、IM、Run、binding 是不同存储边界，没有跨库分布式事务。
- 本轮以模型桩验收，未调用付费真实模型；真实模型的工具选择质量需用户验收。
