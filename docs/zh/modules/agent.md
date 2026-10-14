# Agent

| 概览 | |
| --- | --- |
| **是什么** | Agent 定义，以及让它参与会话的身份和绑定 |
| **CLI** | `agt agent`；绑定通过 `agt tool call` |
| **Agent 工具** | `kind: "agent"` 的 `resource_*`、`principal_create`、`agent_bind`、`agent_set_enabled`、`agent_unbind`、`binding_list` |
| **Web 控制台** | **Agent 团队** |
| **文件** | `agents/<category>/<name>/agent.yaml` |
| **Crate** | `agt-agent`、`agt-service` |

## 作用

Agent 定义把执行引擎（[Model](./model.md) 或 [Harness](./harness.md)）与 [Prompt](./prompt.md)、[Skill](./skill.md)、
[MCP 服务](./mcp.md)和内置能力组合在一起。

只有定义什么也不会发生。要参与对话，Agent 还需要一个**身份**（会话中的参与者）和一个**绑定**：
绑定把身份和定义连接起来，并保存运行设置。多个身份可以绑定同一个定义，并行运行。

## 怎么用

**Web 控制台：** **Agent 团队** → **创建 Agent**一步创建定义、身份和绑定。用**编辑角色**修改定义，用**配置与管理**修改绑定。

**CLI：**

```bash
agt agent create reviewer --model fast --prompt reviewer --skill code-review --mcp github
agt tool call --principal me --manager principal_create --args '{"id":"reviewer","name":"Reviewer","kind":"agent"}'
agt tool call --principal me --manager agent_bind --args '{"principal":"reviewer","agent":"reviewer"}'
agt agent update reviewer --add-builtin-tool task_collaboration
```

**Agent 工具：** `resource_write`（定义）、`principal_create`（身份）、`agent_bind`（绑定）。`resource_write`
会返回 `activation` 摘要，说明 Agent 是否已绑定和启用，以及下一步该做什么。

## 常用参数

| 参数 | 含义 |
| --- | --- |
| `--model` 或 `--harness` | 执行引擎（二选一） |
| `--prompt` | 角色 Prompt |
| `--builtin-tool` | `resource_management`、`task_collaboration`、`local_execution` |
| `policy`（绑定） | 何时回复：`mentioned`、`all`、`manual` |

## 全部参数

### `agt agent create <名称>`

| 选项 | 可重复 | 含义 |
| --- | --- | --- |
| `--model <引用>` | | 使用该模型的 Native 引擎（或 `--harness`） |
| `--harness <引用>` | | 使用该 Harness 的 Codex 引擎（或 `--model`） |
| `--prompt <引用>` | | 角色 Prompt |
| `--skill <引用>` | 是 | 挂载 Skill |
| `--mcp <引用>` | 是 | 挂载 MCP 服务（仅 Native） |
| `--builtin-tool <能力>` | 是 | 授予能力 |
| `--description <文本>` | | 在控制台中和对其他 Agent 显示 |
| `--category <c>` | | 分类（默认 `default`） |

### `agt agent update <名称>`

| 选项 | 含义 |
| --- | --- |
| `--model <引用>` / `--harness <引用>` | 切换执行引擎 |
| `--prompt <引用>` | 修改 Prompt |
| `--description <文本>` | 修改说明 |
| `--add-skill`、`--remove-skill <引用>` | 增删一个 Skill（可重复） |
| `--add-mcp`、`--remove-mcp <引用>` | 增删一个 MCP 服务（可重复） |
| `--add-builtin-tool`、`--remove-builtin-tool <能力>` | 授予或收回能力（可重复） |
| `--category <c>` | 要更新的 Agent 所在分类 |

修改从 Agent 的下一次执行开始生效。正在进行的执行保留原配置；例外是 Agent 修改自己的 MCP 和 Skill 时，
会在当前这批工具调用结束后加载。

### `agt agent run <名称>`

在会话之外运行一次 Agent。

| 选项 | 默认值 | 含义 |
| --- | --- | --- |
| `--input <文本>` / `--stdin` | | 请求内容（二选一） |
| `--request-key <key>` | 自动生成 | 用相同的 key 和相同的请求重复调用时，返回之前的结果而不重新执行 |
| `--cwd <目录>` | `.` | 本次执行的工作目录 |
| `--delegate <引用>` | | 本次执行可以委派的 Agent（可重复）。被委派的 Agent 不能再委派。 |

结果记录为[执行记录](./runs.md)。状态不是 `completed` 时以非零退出码结束。

`list`、`info`、`delete` 和 `category` 见[资源通用操作](./resources.md#通用-cli-命令)。

### 文件：`agent.yaml`

```yaml
name: reviewer
description: 审查代码改动
model: { category: default, name: fast }
harness: null
prompt: { category: default, name: reviewer }
skills: [code-review]
mcps:
  - { category: default, name: github }
builtin_tools: [task_collaboration]
```

| 字段 | 含义 |
| --- | --- |
| `model` / `harness` | 二者只设置一个 |
| `prompt` | 角色 Prompt 引用 |
| `skills` | Skill 包 |
| `mcps` | MCP 服务引用 |
| `builtin_tools` | 授予的能力 |
| `description` | 说明文字 |

### 工具字段（`resource_write`，`kind: "agent"`）

| 字段 | 含义 |
| --- | --- |
| `model` 或 `harness` | 创建时二选一 |
| `prompt`、`skills`、`mcps`、`builtin_tools`、`description` | 可选。列表会替换当前列表；空列表表示全部移除。 |

用 `add` / `remove`（适用于 `skills`、`mcps`、`builtin_tools`）修改单项，用 `clear` 重置 `description`、`skills`、
`mcps` 或 `builtin_tools`。

### 内置能力

| 能力 | Web 标签 | 赋予 Agent |
| --- | --- | --- |
| `resource_management` | 资源管理 | 全部[资源](./resources.md)工具、Agent 身份与绑定、Codex 会话工具 |
| `task_collaboration` | 任务协作 | [任务](./tasks.md)工具 |
| `local_execution` | 本机命令 | [本机命令](./local-execution.md)工具（仅 Native；还需开启 `execution.enabled`） |

Agent 创建的其他 Agent 不会继承能力。消息和[记忆](./memory.md)工具始终可用。

### 身份与绑定

| 工具 | 参数 | 含义 |
| --- | --- | --- |
| `principal_create` | `id`, `name`, `kind` | 创建身份。拥有 `resource_management` 的 Agent 只能创建 `kind: "agent"`。 |
| `agent_bind` | `principal`, `agent`, `working_directory?`, `policy?` | 把身份绑定到定义。新绑定默认启用。对已有身份重新绑定会更新它，已停用的绑定保持停用。 |
| `agent_set_enabled` | `principal`, `enabled` | 在所有会话中启用或停用 Agent，保留设置和历史。启用可能处理待处理的消息。 |
| `agent_unbind` | `principal` | 移除绑定。删除仍有绑定的 Agent 前需要先执行。 |
| `binding_list` | | 所有绑定及其启用状态 |

绑定设置：

| 设置 | 默认值 | 含义 |
| --- | --- | --- |
| `working_directory` | home | 本机命令的默认目录。Codex Agent 始终使用 Harness 的目录。 |
| `policy` | `mentioned` | `mentioned`：私聊和被提及时。`all`：考虑每条消息。`manual`：从不自动回复。Codex Agent 会把 `all` 保存为 `mentioned`。 |

这些工具在 CLI 中需要 `--manager`，Agent 则需要 `resource_management`。

## 相关模块

- [身份与会话](./conversations.md)：Agent 在哪里对话
- [调度器与讨论](./scheduler.md)：Agent 何时运行
- [资源通用操作](./resources.md)：删除与分类
