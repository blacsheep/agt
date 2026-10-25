# 资源通用操作

| 概览 | |
| --- | --- |
| **是什么** | 七种资源共用的操作 |
| **CLI** | `agt provider`、`model`、`prompt`、`skill`、`mcp`、`agent`、`harness resource` |
| **Agent 工具** | `resource_*`（需要 `resource_management`） |
| **Web 控制台** | **资源库**、**Agent 团队** |
| **Crate** | `agt-core`、`agt-storage`、`agt-service` |

## 作用

资源是描述 Agent 的文件。共有七种，每种都有自己的页面：

| 类型 | 描述的内容 | 页面 |
| --- | --- | --- |
| Provider | 一个模型接口及其密钥 | [Provider](./provider.md) |
| Model | Provider 上的一个模型及调用方式 | [Model](./model.md) |
| Prompt | 角色描述 | [Prompt](./prompt.md) |
| Skill | 可复用的指令包 | [Skill](./skill.md) |
| MCP | 提供工具的 MCP 服务 | [MCP](./mcp.md) |
| Harness | 外部执行引擎（Codex、Claude Code 或 DeepSeek Harness）及其项目 | [Harness](./harness.md) |
| Agent | Model 或 Harness，加上 Prompt、Skill、MCP 服务和能力 | [Agent](./agent.md) |

七种资源共用同样的命名、分类、删除规则和 Agent 工具，本页统一说明。人和 Agent 通过同一套代码修改资源，校验和锁也相同。

## 怎么用

```bash
agt model list                         # 列出
agt model info fast                    # 查看一个
agt model category create experiments  # 给资源分组
agt model create cheap --category experiments --provider main --model gpt-5-nano
agt model delete experiments/cheap --dry-run
```

拥有 `resource_management` 的 Agent 通过工具做同样的事：

```json
{"kind": "model", "reference": "experiments/cheap"}
```

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `name` 或 `category/name` | 资源的引用方式。只写名称表示 `default` 分类。 |
| `--category <c>` | 创建或查找资源时的分类 |
| `--recursive` | 删除时一并删除依赖它的资源 |
| `--dry-run` | 删除时只显示计划 |

## 全部参数

### 通用 CLI 命令

每种资源都支持这些子命令（Harness 为 `agt harness resource ...`）。别名：`add` = `create`、`ls` = `list`、 `show`/`get` = `info`、`edit` = `update`、`remove` = `delete`。

| 命令 | 选项 | 含义 |
| --- | --- | --- |
| `list` | `--category <c>` | 列出资源，可限定分类 |
| `info <引用>` | `--category <c>` | 查看一个资源（密钥已脱敏） |
| `delete <引用>` | `--category <c>` `--recursive` `--dry-run` | 删除资源 |
| `category create <名称>` | | 创建分类 |
| `category list` | | 列出分类 |
| `category delete <名称>` | | 删除空的、非 `default` 的分类 |

`create`、`update` 以及各类型特有的命令见各类型的页面。

### 删除规则

- 删除仍被其他资源引用的资源会被拒绝，并列出依赖者。
- `--recursive` 会一并删除依赖者：删除 Provider 会删除它的 Model 和使用这些 Model 的 Agent；删除 Prompt、Skill、MCP 服务、Harness 或 Model 会删除使用它的 Agent。删除 Agent 不会删除它使用的资源。
- 仍有绑定的 Agent 需要先解除绑定（见 [Agent](./agent.md#身份与绑定)）。
- `default/default`（Prompt）不能删除。
- 删除前，agt 会检查全部七种资源是否有损坏的配置和悬空引用，发现不一致就拒绝删除。
- 被删除的目录会移动到 `<home>/.trash/`。在终端中会要求确认；其他环境需要传入 `--yes`。

### Agent 工具

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `resource_list` | `kind` | 列出一种资源（密钥已脱敏） |
| `resource_read` | `kind`, `reference` | 读取一个资源，包括它的 `revision` |
| `resource_write` | `kind`, `reference`, `create`, `data`, `expected_revision?`, `add?`, `remove?`, `clear?` | 创建（`create: true`）或更新资源 |
| `resource_delete` | `kind`, `reference`, `recursive?`, `confirm?` | 分两步删除（见下文） |
| `resource_category_list` | `kind` | 列出分类 |
| `resource_category_create` | `kind`, `category` | 创建分类 |
| `resource_category_delete` | `kind`, `category` | 删除空的、非 `default` 的分类 |

`kind` 为 `provider`、`model`、`prompt`、`skill`、`mcp`、`agent`、`harness` 之一。

**`resource_write` 详细说明。** `data` 的字段取决于资源类型，见各类型的页面。更新时：

- 省略的字段保持不变；
- `expected_revision`（来自 `resource_read`）使更新在他人已修改时失败；
- `add` / `remove` 增删 Agent 的 `skills`、`mcps` 和 `builtin_tools` 中的单项；
- `clear` 重置可选字段：Agent 的 `description`、`skills`、`mcps`、`builtin_tools`；Model 的 `max_tokens`、 `reasoning_effort`；Harness 的 `model`、`reasoning_effort`。

不要在一次调用中同时替换列表、`clear` 以及对同一项 `add`/`remove`。`create: true` 时不能使用 `add`、`remove` 和 `clear`。

**`resource_delete` 详细说明。** 不带 `confirm` 调用以获取删除计划。然后再次调用，把 `confirm` 设为计划中完整、有序的条目列表。空列表永远不能确认非空计划。

## 相关模块

- [工作空间与配置](./workspace.md)：资源文件存放在哪里
- [Agent](./agent.md)：资源如何组合成 Agent
