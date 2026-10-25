# 记忆

| 概览 | |
| --- | --- |
| **是什么** | 每个 Agent 的历史搜索和私有笔记 |
| **Agent 工具** | `memory_search`、`memory_save`、`memory_read`、`memory_forget` |
| **Web 控制台** | **运行设置** → **跨会话记忆**（开或关） |
| **设置** | `memory.enabled` |
| **存储** | 消息在 `im/im.db`；笔记在 `memory/memory.db` |
| **Crate** | `agt-service` |

## 作用

记忆给每个 Agent 两种能力，不需要向量服务：

- **搜索历史**：按关键词、会话和日期，在它的所有会话中查找它能看到的消息。
- **保存笔记**：保存私有的主题和摘要，并引用它所依据的消息。

笔记属于单个身份，每次召回都会重新检查 Agent 是否仍能看到来源消息。Agent 如何使用记忆见[记忆](../guide/memory.md)。

## 怎么用

`memory.enabled` 为 true 时，每个托管 Agent（Native 或 Harness）都有记忆工具。人和脚本也可以为自己的身份调用：

```bash
agt tool call --principal me memory_search --args '{"query":"发布","kind":"messages"}'
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `kind` | `messages`（默认）或 `notes` |
| `query` | 关键词 |
| `source_messages` | 笔记依据的消息 |

## 全部参数

### `memory_search`

| 参数 | 类型 | 含义 |
| --- | --- | --- |
| `kind` | `messages` \| `notes` | 搜索什么。默认 `messages`。 |
| `query` | 字符串，≤ 256 | 关键词 |
| `conversation` | 字符串 | 只搜索这个会话 |
| `day` | `YYYY-MM-DD` | 一个 UTC 日期 |
| `since`、`until` | Unix 毫秒 | 时间范围（含起点，不含终点），用于本地日期 |
| `limit` | 1–50 | 每页数量 |
| `before` | 整数 | 上一页的 `next_cursor`。保持其他条件不变；持续翻页直到 `next_cursor` 为 null。 |

搜索笔记时，日期条件作用于来源消息的日期。

### `memory_save`

| 参数 | 类型 | 含义 |
| --- | --- | --- |
| `id` | 字符串，≤ 128 | 笔记的键 |
| `topic` | 字符串，≤ 120 | 主题 |
| `summary` | 字符串，≤ 4000 | 摘要 |
| `source_messages` | 1–32 个消息 ID | 支持该摘要、且你能看到的消息 |
| `expected_revision` | 整数 | `0` 表示创建；更新时传当前 revision |

### `memory_read`

| 参数 | 含义 |
| --- | --- |
| `id` | 要读取的笔记，连同当前可见的来源消息 |

### `memory_forget`

| 参数 | 含义 |
| --- | --- |
| `id` | 要删除的笔记 |
| `expected_revision` | 它的当前 revision |

删除笔记不会删除消息。

### 设置

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `memory.enabled` | `true` | 关闭后，记忆调用立即被拒绝，新的执行不再提供这些工具。开启后，新的执行会获得这些工具。 |

### 规则

- Agent 只能找到它本来就能看到的消息：退出群聊后，只能看到退出前的历史。
- 来源消息被删除或不再可见时，不会返回该笔记。
- 没有任何工具能读取其他身份的笔记。
- `/new` 重置 Agent 的工作上下文，不删除历史或笔记。

## 相关模块

- [身份与会话](./conversations.md)：被搜索的消息
- [工作空间与配置](./workspace.md)：`memory.enabled` 设置
