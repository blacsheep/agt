# 身份与会话

| 概览 | |
| --- | --- |
| **是什么** | 参与者、私聊与群聊、消息 |
| **CLI** | `agt im` |
| **Agent 工具** | `principal_list`、`direct_create`、`group_create`、`member_list`、`member_add`、`member_remove`、`conversation_*`、`group_dissolve`、`response_policy`、`message_*` |
| **Web 控制台** | **会话空间**、**Agent 会话** |
| **存储** | `im/im.db` |
| **Crate** | `agt-im` |

## 作用

这个模块是人和 Agent 共用的小型即时通讯系统：

- **身份**（principal）是参与者：用户、Agent、Bot 或服务。
- **私聊**连接两个身份；同一对身份始终共用同一个私聊。
- **群聊**有群主、管理员和成员。消息会发给所有当前成员。
- **消息**在会话内有序号，可带提及和回复对象，保存在 SQLite 中。

保存消息本身不会运行任何 Agent。只有讨论被授权后，Agent 才会回复；见[调度器与讨论](./scheduler.md)。

Agent 建群时，**工作空间负责人**（第一次启动 Web 控制台的人）会自动作为群主加入，建群的 Agent 成为管理员。

人建群时，建群的人是群主，被拉进来的都是成员。只有群主和管理员能拉人或移除成员，所以你拉进群的 lead 在被设为管理员之前不能邀请队友：
在**群信息**中点它旁边的**设为管理员**，或调用 `member_role_set`。只有群主能修改角色，群主身份不能转让。

## 怎么用

**Web 控制台：** **会话空间** → **新建会话**；输入 `@` 提及成员，输入 `/` 使用命令。

**CLI（以自己的身份，通过工具）：**

```bash
agt tool call --principal me direct_create --args '{"principal":"reviewer"}'
agt tool call --principal me group_create --args '{"title":"方案评审","members":["reviewer","planner"]}'
agt tool call --principal me message_send --args '{"conversation":"<id>","body":"准备好了吗？","mentions":["reviewer"],"request_key":"msg-1"}'
agt tool call --principal me message_history --args '{"conversation":"<id>"}'
```

**CLI（底层存储命令）：** `agt im ...`，见下文。

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `mentions` | 群里被要求回应的身份 |
| `request_key` | 使发送幂等：相同 key 和相同内容只发送一次 |
| `policy` | 某个身份在某个会话中的响应策略 |

## 全部参数

### Agent 工具

每个身份都有这些工具；身份由宿主固定，不能通过参数更改。

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `principal_list` | `include_deleted?` | 列出身份（已删除的身份仅用于解析旧名称） |
| `direct_create` | `principal` | 创建或找到你与某人的私聊 |
| `group_create` | `title`, `members` | 创建群聊 |
| `conversation_list` | `groups_only?`, `include_hidden?` | 你的会话，包括已退出或已解散群聊的只读历史 |
| `conversation_hide` | `conversation` | 只从你的列表中移除入口；有新消息时会重新出现 |
| `conversation_show` | `conversation` | 恢复被隐藏的入口 |
| `member_list` | `conversation` | 你所在会话的成员 |
| `member_add` | `conversation`, `principal` | 邀请成员（群主或管理员） |
| `member_remove` | `conversation`, `principal` | 移除成员（群主或管理员），或自己退出。群主不能退出。 |
| `member_role_set` | `conversation`, `principal`, `role`（`admin` 或 `member`） | 仅群主：把成员设为管理员，或把管理员改回成员。只对人开放，Agent 看不到这个工具。 |
| `group_dissolve` | `conversation` | 解散你作为群主的群；历史保留为只读 |
| `response_policy` | `conversation`, `policy` | 设置自己在这里的策略：`all`、`mentioned` 或 `manual` |
| `message_send` | 见下文 | 发送消息 |
| `message_history` | `conversation`, `before?`, `limit?`（1–200，默认 50） | 消息，每页内从旧到新。把 `next_cursor` 作为 `before` 查看更早的消息。 |
| `message_inbox` | `pending_only?`（默认 true）, `limit?` | 你的投递及处理状态 |
| `message_ack` | `id` | 确认一条投递（供外部客户端使用） |
| `message_status` | `message_id` | 一条消息在每个接收者处的处理状态 |

**`message_send` 参数**

| 参数 | 必填 | 含义 |
| --- | --- | --- |
| `conversation` | 是 | 目标会话 |
| `body` | 是 | 正文（Markdown） |
| `request_key` | 是 | 幂等键。只在内容完全相同时复用。 |
| `mentions` | 否 | 要提及的身份 |
| `reply_to` | 否 | 同一会话中某条消息的 ID |
| `input_mode` | 否 | 正在运行的 Agent 何时收到这条消息：`steering`（下一个工具边界，人发送时的默认值）或 `follow_up`（当前工作结束后，Agent 发送时的默认值） |

### 响应策略

| 策略 | 私聊 | 群聊 |
| --- | --- | --- |
| `mentioned`（默认） | 回复 | 被提及时回复 |
| `all` | 回复 | 考虑每条消息；可能沉默或等待 |
| `manual` | 从不自动回复 | 从不自动回复 |

提及指结构化的 `mentions` 字段；正文中手打的 `@名字` 不算提及。只回复 `[[silent]]` 的 Agent 不会发布任何内容。

### `agt im` 命令

这些命令以可信本地管理员身份直接操作存储。

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `principal create <id>` | `--name <名称>`、`--kind <类型>`（默认 `user`；也可为 `agent`、`bot`、`service`、`other`） | 创建身份 |
| `principal list` / `info <id>` | | 列出或查看身份 |
| `owner get` | | 查看工作空间负责人 |
| `owner ensure <用户>` | | 尚未设置时设置负责人，并让其成为已有 Agent 所建群聊的群主 |
| `direct create <a> <b>` | | 创建或找到私聊 |
| `group create` | `--owner <id>`（必填）、`--title <标题>`、`--member <id>`（可重复） | 创建群聊 |
| `group members list <会话>` | | 列出成员 |
| `group members add <会话> <id>` | `--actor <id>` | 添加成员 |
| `group members remove <会话> <id>` | `--actor <id>` | 移除成员 |
| `group members role <会话> <id> <admin\|member>` | `--actor <群主>` | 修改成员角色 |
| `group members leave <会话> <id>` | | 退出 |
| `group dissolve <会话>` | `--actor <群主>` | 解散 |
| `conversation list` | `--principal <id>`（必填）、`--include-hidden`、`--groups-only` | 列出某身份的会话 |
| `conversation info <会话>` | | 查看会话 |
| `conversation hide` / `unhide <会话>` | `--actor <id>` | 隐藏或恢复入口 |
| `conversation visit <会话>` | `--actor <id>` | 记录一次访问（用于最近历史排序） |
| `conversation archive` / `reactivate` / `delete <会话>` | `--actor <id>` | 归档、恢复或软删除会话 |
| `message send <会话> <正文>` | `--sender <id>`（必填）、`--idempotency-key <k>`、`--client-message-id <id>`、`--input-mode steering\|follow_up` | 保存一条消息 |
| `message list <会话>` | `--viewer <id>`（必填）、`--limit <n>`（默认 50）、`--before <游标>`、`--after <游标>` | 列出消息 |
| `message read <会话>` | `--principal <id>`、`--through-seq <n>`（均必填） | 标记为已读 |
| `message delete <消息>` | `--actor <id>` | 软删除一条消息 |

要删除会话并同时停止其中 Agent 的工作，请使用 `agt schedule delete`；见[调度器与讨论](./scheduler.md)。

### 可见性规则

- 新成员可以阅读之前的历史；未读数从加入时开始计算。
- 退出的成员保留到退出时为止的历史，之后的消息不可见。
- 隐藏入口只影响你自己的列表。
- 删除都是软删除：记录被标记，而不是被擦除。

## 相关模块

- [调度器与讨论](./scheduler.md)：Agent 何时回复
- [记忆](./memory.md)：搜索历史
- [Web 控制台](./web-console.md)：聊天界面
