# 内置工具

这些是 agt 提供给 Agent 的工具。人和脚本也可以通过 `agt tool call` 和 `agt serve` 使用同样的工具。
在 Agent 执行中，工具名带有 `framework_` 前缀，例如 `framework_message_send`。

标 `?` 的参数是可选的。运行 `agt tool list --principal <id> --manager` 可查看完整的 JSON schema。

## 谁能使用哪些工具

| 工具组 | 托管 Agent | `agt tool` / `agt serve` |
| --- | --- | --- |
| 会话、消息 | 始终可用 | 始终可用 |
| 记忆 | 始终可用（除非 `memory.enabled: false`） | 始终可用 |
| 资源、身份、绑定、Codex 会话 | 需要 `resource_management` | 需要 `--manager` |
| 任务 | 需要 `task_collaboration` | 始终可用（仅限该身份自己的任务） |
| 任务内等待 | 在任务执行中 | — |
| 本机命令 | 需要 `local_execution` 和 `execution.enabled`（仅 Native） | — |

## 会话和消息

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `principal_list` | `include_deleted?` | 列出人和 Agent |
| `direct_create` | `principal` | 打开（或找到）与某人的私聊 |
| `group_create` | `title`, `members` | 创建群聊；调用者成为管理员，工作空间负责人成为群主 |
| `conversation_list` | `groups_only?`, `include_hidden?` | 列出你的会话 |
| `conversation_hide`, `conversation_show` | `conversation` | 隐藏或恢复你自己的入口 |
| `member_list` | `conversation` | 列出成员 |
| `member_add`, `member_remove` | `conversation`, `principal` | 邀请或移除成员（群主/管理员），或自己退出 |
| `group_dissolve` | `conversation` | 解散你作为群主的群 |
| `response_policy` | `conversation`, `policy` | 设置自己在这里的策略：`all`、`mentioned` 或 `manual` |
| `message_send` | `conversation`, `body`, `request_key`, `mentions?`, `reply_to?` | 发送消息；相同 `request_key` 和相同内容不会重复发送 |
| `message_history` | `conversation`, `before?`, `limit?` | 读取消息；把 `next_cursor` 作为 `before` 查看更早的消息 |
| `message_inbox`, `message_ack` | | 供外部客户端使用的投递记录 |
| `message_status` | `message_id` | 每个接收者是否已处理某条消息 |

## 记忆

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `memory_search` | `kind?`（`messages` 或 `notes`）, `query?`, `conversation?`, `day?`, `since?`, `until?`, `before?`, `limit?` | 搜索你可见的历史或你的笔记 |
| `memory_save` | `id`, `topic`, `summary`, `source_messages`, `expected_revision` | 创建（`0`）或更新笔记 |
| `memory_read` | `id` | 读取笔记及其来源 |
| `memory_forget` | `id`, `expected_revision` | 删除笔记 |

见[记忆](../guide/memory.md)。

## 资源管理

`kind` 是 `provider`、`model`、`prompt`、`skill`、`mcp`、`harness`、`agent` 之一。`reference` 是 `name` 或 `category/name`。

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `resource_list` | `kind` | 列出资源（密钥已脱敏） |
| `resource_read` | `kind`, `reference` | 读取一个资源，包括它的 `revision` |
| `resource_write` | `kind`, `reference`, `create`, `data`, `expected_revision?`, `add?`, `remove?`, `clear?` | 创建或更新资源 |
| `resource_delete` | `kind`, `reference`, `recursive?`, `confirm?` | 不带 `confirm`：获取删除计划。`confirm` 设为完整计划：执行删除 |
| `resource_test` | `kind`（`provider` 或 `mcp`）, `reference` | 只校验配置 |
| `provider_models` | `reference` | 列出 Provider 的模型 |
| `mcp_probe` | `reference` | 启动或连接 MCP 服务，列出并检查其工具 |
| `agent_mcp_list` | `agent` | 列出 Agent 的 MCP 服务 |
| `resource_category_list`, `resource_category_create`, `resource_category_delete` | `kind`, `category?` | 管理分类 |

`data` 使用与资源文件相同的字段。示例：

```json
{"kind":"prompt","reference":"critic","create":true,"data":{"text":"你质疑每一个论断……"}}
{"kind":"agent","reference":"critic","create":true,
 "data":{"model":"fast","prompt":"critic","builtin_tools":[],"description":"反方"}}
{"kind":"agent","reference":"critic","create":false,"data":{},"add":{"mcps":["search"]}}
{"kind":"provider","reference":"main","create":true,
 "data":{"base_url":"https://api.openai.com/v1","api_key":"sk-..."}}
```

把 `resource_read` 返回的 revision 作为 `expected_revision` 传入：如果他人已经修改过，更新会失败，而不会覆盖对方的修改。

### 身份与绑定

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `principal_create` | `id`, `name`, `kind` | 创建身份。拥有 `resource_management` 的 Agent 只能创建 `kind: agent` |
| `agent_bind` | `principal`, `agent`, `working_directory?`, `policy?` | 把身份连接到定义并启用 |
| `agent_set_enabled` | `principal`, `enabled` | 在所有会话中启用或停用 Agent |
| `agent_unbind` | `principal` | 移除绑定（删除 Agent 前需要） |
| `binding_list` | | 列出绑定及其启用状态 |

为 Agent 调用 `resource_write` 会返回 `activation` 摘要：Agent 是否已配置、已绑定、已启用，未完成时给出后续步骤。
只创建定义并不能让 Agent 参与对话。

### Codex 会话

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `harness_models` | `reference` | Harness 报告的模型和推理档位 |
| `harness_session_get` | `conversation`, `target` | 某个 Agent 在某个会话中的 Codex 会话 |
| `harness_session_models` | `conversation`, `target`, `session` | 该会话可选的模型 |
| `harness_session_configure` | `conversation`, `target`, `session`, `model`, `effort?` | 修改该会话的模型和推理档位 |

调用者和目标都必须是该会话的成员。

## 任务协作

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `task_request` | `assignee`, `conversation`, `input`, `request_key`, `priority?`, `execution_policy?` | 为某个 Agent 排队一个任务；双方都必须在 `conversation` 中 |
| `task_get` | `task_id` | 状态、结果、汇报状态和阻塞原因 |
| `task_list` | `conversation?`, `queued_only?`, `limit?`, `before?` | 你发出或收到的任务 |
| `task_report` | `task_id`, `expected_revision`, `body` | 向原会话汇报结果 |
| `task_reprioritize` | `task_id`, `expected_revision`, `priority`, `reason` | `low`、`normal`、`high`（人可用 `urgent`） |
| `task_defer` | `task_id`, `expected_revision`, `reason`, `not_before?` 或 `after_task?` + `deadline?` | 延后排队中的任务 |
| `task_execution_policy` | `task_id`, `expected_revision`, `reason`, `execution_policy` | 并行与互斥资源 |
| `task_cancel` | `task_id`, `expected_revision`, `reason` | 取消任务及其子任务 |
| `task_retry` | `task_id`, `expected_revision`, `request_key`, `reason` | 基于失败或取消的任务创建新任务 |
| `task_reply` | `task_id`, `input_request_id`, `expected_revision`, `answer`, `request_key` | 答复任务提出的问题 |
| `task_resolve` | `task_id`, `expected_revision`, `outcome`, `evidence` | 仅限人：记录 `outcome_unknown` 任务的实际结果 |
| `task_attention` | `limit?`, `before?` | 仅限工作空间负责人：需要人处理的任务 |

仅在任务执行中可用：

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `task_await` | `task_id` 或 `task_ids`, `deadline?` | 等待子任务；必须是这一步中唯一的调用 |
| `task_yield` | `not_before`, `reason` | 暂停到指定时间 |
| `task_input` | `question`, `deadline` | 向请求者提问并等待 |

时间使用 Unix 毫秒。见[任务协作](../guide/tasks.md)。

## 本机命令

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `local_exec` | `command`, `directory?` | 在宿主机上运行一条 Bash 命令 |

见[本机命令](../guide/local-execution.md)。
